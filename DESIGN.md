# agent-wall — Design

A tiny always-on-top dashboard that shows one compact row per live **tmux**
session, with a live tail of each session's pane. Built in Rust on
[`eframe`/`egui`](https://github.com/emilk/egui). Mouse-only, event-driven, and
close to zero CPU while nothing is happening.

---

## 1. Goals & non-goals

**Goals**

- One glanceable tile per tmux session, with attach-state and a live pane tail.
- **Event-driven**: the wall reacts to real tmux activity within milliseconds and
  burns no CPU while every session sits idle. No fixed-interval polling.
- Native, crisp, DPI-aware rendering; always-on-top so it stays visible.
- Mouse-only control: click to attach, right-click to kill, a mouse button to
  reveal/hide.
- Survive restarts: window geometry persists to disk.

**Non-goals**

- No scrollback browsing, no input into sessions, no multiplexing — `capture-pane`
  snapshots are the only feed.
- No config file / theming UI. The palette and dimensions are compile-time
  constants (`theme.rs`).
- Non-tmux processes are invisible by design.

---

## 2. Process & thread model

`main()` (`src/main.rs`) wires everything, then hands the main thread to
`eframe::run_native`. Four long-lived threads plus per-session helper processes:

| Thread / process | Source | Role |
|---|---|---|
| **UI** (main thread) | `ui::Wall` | egui render + input; drains channels; owns window geometry |
| **Poller** | `poller::run` | lists sessions, wires streams, captures panes, drives repaints |
| **FIFO reader** | `wake::spawn_reader` | reads the wake FIFO, forwards coalesced pokes to the poller |
| **IPC server** (unix) | `ipc::serve` | accepts `toggle` on a unix socket, forwards `IpcCmd::Toggle` to the UI |
| **`pipe-pane` cat** (0..N, external) | `tmux pipe-pane` | one per session; streams pane bytes into the wake FIFO |

### Channels (all `std::sync::mpsc::sync_channel`, bounded)

```
FIFO reader ─┐
             ├─ wake  (cap 8, ())        ─▶ Poller.recv()
UI (kill/    │
   reveal) ──┘

Poller ── tx (cap 32, AppMsg) ──▶ UI.rx           AppMsg::Sessions | AppMsg::Pane
IPC server ── ipc (cap 4, IpcCmd) ──▶ UI.ipc_rx   IpcCmd::Toggle
```

Bounds are deliberate. The `AppMsg` channel is `sync_channel(32)` and the poller
uses `try_send`, so if the UI ever falls behind, updates are **dropped** rather
than queued — the wall always shows fresh state, never a backlog. The wake
channel is `sync_channel(8)` with `try_send` on the writer side, which is exactly
the coalescing we want under a burst (see §4).

### Shared state

- `visible: Arc<AtomicBool>` — the UI flips it on reveal/hide; the poller reads it
  to **skip the expensive `capture-pane`** while the window is hidden (it still
  tracks the session list).
- Single-instance guard: `TcpListener::bind("127.0.0.1:47819")`. A second launch
  fails the bind and exits silently. `--toggle` short-circuits before the guard
  and just pokes the running instance over IPC.

---

## 3. The event source (`wake.rs` + `tmux.rs`)

The wall never polls tmux on a timer for content. Instead a **named FIFO**
(`$XDG_RUNTIME_DIR/agent-wall.wake`, fallback `/tmp`) is poked by two classes of
external writer, and the poller blocks on the resulting channel.

1. **Lifecycle hooks** (`tmux::install_hooks`). Global tmux hooks on
   `session-created`, `session-closed`, `client-attached`, `client-detached`,
   `client-session-changed`, each running
   `run-shell -b "printf . >> <fifo>"`. These make session **appearance,
   disappearance, and attach-state flips** event-driven.
2. **`pipe-pane` streams** (`tmux::pipe_pane`). Each session's active pane is
   streamed with `pipe-pane -o -t <id> "cat >> <fifo>"`. Any output byte pokes
   the FIFO. The `-o` flag makes re-piping a no-op, and the poller only pipes a
   session once (tracked in a `HashSet<id>`), forgetting ids that die so a
   recycled id re-pipes cleanly.

The wall never parses the raw stream — a poke just means "something changed",
and the poller responds with a clean `capture-pane` snapshot.

### FIFO reader details

`spawn_reader` opens the FIFO **read + write** (`O_RDWR`). Holding a writer end
open on our own side means the read end never sees EOF as tmux writer processes
(`printf`, the `pipe-pane` cat) come and go. Each `read()` of a burst forwards a
single `try_send(())` — `try_send` dropping when a poke is already pending is the
first stage of coalescing.

---

## 4. The poll loop (`poller.rs`) — the heart of the design

```
loop {
    sessions = tmux::list_sessions()
    tx.try_send(Sessions(sessions))          // UI updates the rows
    pipe-pane any new session; forget dead ids
    if visible: for s in sessions: tx.try_send(Pane(capture_pane(s)))
    ctx.request_repaint()
    wait_for_wake(&wake, !sessions.is_empty(), RECONNECT)   // ◀── park here
}
```

### `wait_for_wake` — no active-session timer

This is the load-bearing decision. The wait strategy is chosen purely by
**liveness**:

- **Active** (`has_sessions == true`): `wake.recv()` — a plain, **untimed** block.
  While sessions exist, the FIFO (hooks + `pipe-pane`) and UI pokes are the *only*
  things that can wake the loop. A wall of idle sessions therefore consumes **zero
  CPU** and does no tmux I/O until something actually changes. There is no
  periodic "safety" reconcile.
- **Idle** (`has_sessions == false`): `wake.recv_timeout(RECONNECT)` with
  `RECONNECT = 2s`. When there are no sessions there is nothing to install a hook
  on and nothing to stream, so *nothing can poke the FIFO*. A bounded reconnect
  probe is the only way to notice a freshly-started tmux server (or one that just
  came back). It is a **reconnect probe, not a content poll** — long enough not to
  busy-loop, short enough to feel instant.

After waking, `while wake.try_recv().is_ok() {}` drains the backlog so a burst
from a chatty pane collapses into a single refresh (second coalescing stage).

> **Why no safety timer?** An earlier revision kept a slow 15 s `recv_timeout`
> even with live sessions, as a backstop for a missed hook. It was removed: the
> hooks + `pipe-pane` cover every user-visible transition, and the timer meant the
> process woke, re-listed, and re-captured every 15 s forever even when the wall
> was untouched. Blocking on `recv()` makes "idle" genuinely idle. The tradeoff —
> a silently `kill-server`'d tmux (which fires no `session-closed` hook) leaves
> stale rows until the next UI poke — is accepted deliberately; normal session
> close *does* fire the hook.

### Visibility gate

`capture-pane` is the expensive per-session call. When the window is hidden
(`visible == false`) the poller still lists sessions (cheap, keeps the row count
and title live) but skips all pane captures.

---

## 5. UI (`ui.rs`, `tile.rs`, `theme.rs`)

### Layout

A single `CentralPanel` filled with `BG`, `PAD = 10` inner margin, containing a
vertical `ScrollArea` of session rows plus a one-line status footer
(`"N sessions · Xs ago"` / `"no sessions"` / `"starting…"`). Default window is
`244 × 194`; rows are `STRIP_W × STRIP_TILE_H = 220 × 28` with `GAP = 4`.

Each row (`strip_tile`) is hand-painted (not widgets) for density:

- A card rect (`CARD` fill, `BORDER` stroke).
- **Attachment dot** (radius-4 circle) at the left: filled `ATTACHED` green when a
  client is attached, hollow `BORDER` stroke when detached.
- **Session name** in 9 pt proportional `TEXT`, clipped to the left ~90 px.
- **Pane tail**: the last 2 non-empty lines of `capture-pane`, 9 pt monospace
  `MUTED`, right-aligned and clipped to the right half of the row.

### Palette (`theme.rs`) — compact black / blue / green

| Token | Hex | Use |
|---|---|---|
| `BG` | `#0b0e14` | window background (near-black navy) |
| `CARD` | `#161b26` | row fill |
| `BORDER` | `#232a38` | row border, detached dot |
| `TEXT` | `#c9d6e6` | session name (pale blue-grey) |
| `MUTED` | `#8b95ab` | pane tail, status footer |
| `ATTACHED` | `#3ee08b` | attached dot (green) |
| window icon | `#ff8c00` | orange 32×32 |

The result is a dark blue-black panel, blue-grey labels, and a green attach
indicator — see `HIGHLIGHTS.md` for a captured screenshot.

### Interactions (mouse-only)

| Input | Effect |
|---|---|
| **Left-click** a row | `launch::alacritty` → `alacritty -e tmux attach -t <id>`; 500 ms `TP_DEBOUNCE` guards double-launch |
| **Right-click** a row | kill the session (deferred past row iteration to avoid a borrow conflict, then poke the poller) |
| **MB5 / Extra2** on a row | toggle window visibility |
| **MB5 global** (Hyprland bind → `agent-wall --toggle`) | Hyprland: `hyprctl dispatch hl.dsp.workspace.toggle_special("agent-wall")`; elsewhere: IPC toggle |

Session ids are always the `$`-prefixed form and are passed to `tmux`/`alacritty`
as **discrete arguments, never through a shell** (`launch.rs`, `tmux.rs`), so a
hostile session name cannot inject a command.

### App loop

`eframe::App::logic` runs every frame (even hidden — the poller calls
`request_repaint`): it drains `rx`/`ipc_rx`, tracks the outer window rect for
persistence, and updates the title to `"Agent Wall — N sessions"`. `ui` paints
the rows and processes deferred kills. `on_exit` saves geometry and removes the
tmux hooks.

---

## 6. Persistence (`persist.rs`)

`WinState { x, y, w, h }` is JSON at `~/.local/share/agent-wall/state.json`.
Loaded at startup: a saved `w > 0` restores both size **and** position via
`ViewportBuilder::with_inner_size` / `with_position`; otherwise the `244 × 194`
default is used. The UI tracks the live outer rect each frame (with a 2 px
dead-zone) and `on_exit` writes it back, so user resizes survive restarts.

On Wayland the compositor owns floating-window placement, so `x/y` may not be
honored on restore even though they are persisted; **size is restored reliably**.

---

## 7. IPC (`ipc.rs`, unix only)

A `UnixListener` at `$XDG_RUNTIME_DIR/agent-wall.sock` (fallback `/tmp`). Protocol
is one line: a client writes `"toggle\n"`, the server reads it, sends
`IpcCmd::Toggle`, and requests a repaint. `send_toggle()` (used by
`agent-wall --toggle`) connects and writes that line. A stale socket from a
crashed run is removed before bind.

This is how the global Hyprland mouse bind reaches a running instance:
`mouse:276 → agent-wall --toggle → socket → IpcCmd::Toggle → toggle_visibility`.

---

## 8. Platform notes

- **Linux/Wayland (primary, incl. Omarchy/Hyprland).** `cargo run` talks to local
  tmux directly. Recommended Hyprland rule: float + pin on `title:^Agent Wall`,
  and **do not** force a `size` rule (it would clobber the app's persisted
  geometry — the app owns its size via winit).
- **Reveal/hide is compositor-limited.** Hide uses
  `ViewportCommand::Minimized(true)` because `Visible` is a no-op on Wayland
  (winit 0.30 xdg_toplevel). Hyprland/wlroots does not implement `xdg_toplevel`
  minimize either, so on Hyprland the toggle round-trips correctly over IPC but
  does not visually unmap an always-on-top pinned overlay; its practical use there
  is reveal/focus. The IPC transport, hooks, `pipe-pane`, and persistence are all
  fully functional.
- **Windows (WSL).** `deploy.sh` cross-compiles to
  `x86_64-pc-windows-gnu` and drops the exe into the Startup folder; tmux calls
  hop through `wsl.exe -e tmux`.

---

## 9. Testing

Unit tests live beside each module (`cargo test`):

- `tmux::parse_sessions` — well-formed, malformed, and empty `list-sessions`
  output.
- `persist` — `WinState` JSON round-trip.
- `launch` — session ids are passed as discrete args (injection-safe).
- `ipc` — `IpcCmd::Toggle` constructibility (socket path is env-bound; full
  round-trip is covered manually — see `HIGHLIGHTS.md`).
- **`poller::wait_for_wake`** — the wait logic is extracted into a pure,
  duration-parameterized function so the two invariants are provable without a
  live tmux or GUI:
  - `active_mode_blocks_until_wake` — with `has_sessions = true` the call does not
    return for 200 ms (≫ the injected reconnect) and releases only on a poke.
  - `idle_mode_reconnect_probe_does_not_busy_loop` — three idle probes consume
    ≥ 3× the reconnect interval, proving it sleeps rather than spins.
  - `queued_burst_returns_immediately_and_coalesces` — a pre-queued burst returns
    at once and is drained to a single refresh.
