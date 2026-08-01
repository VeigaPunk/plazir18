# agent-wall — Highlights

## What it looks like

**Default** is the multi-panel **18-tile grid** (adaptive columns up to 6×3 at
capacity), not the legacy strip. Prefer `cargo run` with no flags for the
dashboard; use `--strip` for the compact top bar.

The legacy strip is a thin horizontal bar of compact tiles (historically
tuned around ultrawide widths). The compositor can persist user-resized
geometry in `~/.local/share/agent-wall/state.json`.

The overlay is a dark navy (`#0b0e14`) panel. Each tile shows:

- A green dot (`#3ee08b`) when a client is attached, hollow border otherwise.
- Session name in proportional pale blue-grey (`#c9d6e6`).
- Last non-empty pane lines in monospace muted grey (`#8b95ab`).
- Footer status (session count / idle age) or `"no sessions"`.

## Toggle behaviour

`agent-wall --toggle` (bound to MB5 / Extra2 globally via Hyprland):

- **Hyprland** — `hyprctl dispatch hl.dsp.workspace.toggle_special("agent-wall")`.
  The window lives on `special:agent-wall` (see `bindings.lua`); toggling the
  special workspace hides/reveals it without intercepting input on normal
  workspaces.
- **Other compositors** — Unix socket IPC: sends `toggle\n` to the running
  instance and flips `ViewportCommand::Minimized`.

## Screenshot evidence (historical strip verification)

**Hidden:** special:agent-wall dismissed from DP-1 monitor stack; a prior
360×300 region showed only the background workspace. Window process alive but
off-screen; `acceptsInput: true` has no effect while the special workspace is
not shown.

**Revealed:** `hyprctl clients -j` confirmed size/position on
`special:agent-wall`. Crop evidence showed dark overlay with session rows
(green attach dot, name, pane tail) and a `N sessions · Xs ago` footer.

## Dual-mode note

Wall is default. Coding agent: `cargo run --features agent -- --agent`
(see README). Agent feature compiles and unit-tests under `--features full`;
interactive provider E2E remains roadmap.
