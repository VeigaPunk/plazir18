# agent-wall — Highlights

## What it looks like

Default window is a 3440×58 top strip. The compositor persists user-resized
geometry in `~/.local/share/agent-wall/state.json`; the verified persisted state
during this build run was **360×300 at (1540, 583)** on DP-1.

The overlay is a dark navy (`#0b0e14`) panel with compact 220×28 horizontal
tiles, one per live tmux session. Each tile shows:

- A green dot (`#3ee08b`) when a client is attached, hollow border otherwise.
- Session name in 9 pt proportional pale blue-grey (`#c9d6e6`).
- Last 2 non-empty pane lines in 9 pt monospace muted grey (`#8b95ab`),
  right-aligned to the row's right half.
- A one-line footer: `"N sessions · Xs ago"` or `"no sessions"`.

## Toggle behaviour

`agent-wall --toggle` (bound to MB5 / Extra2 globally via Hyprland):

- **Hyprland** — `hyprctl dispatch hl.dsp.workspace.toggle_special("agent-wall")`.
  The window lives on `special:agent-wall` (see `bindings.lua`); toggling the
  special workspace hides/reveals it without intercepting input on normal
  workspaces.
- **Other compositors** — Unix socket IPC: sends `toggle\n` to the running
  instance and flips `ViewportCommand::Minimized`.

## Screenshot evidence

**Hidden:** special:agent-wall dismissed from DP-1 monitor stack; the 1540,583
360×300 region shows only the background workspace. Window process alive but
off-screen; `acceptsInput: true` has no effect while the special workspace is
not shown.

**Revealed:** `hyprctl clients -j` confirms `size: [360,300]`, `at: [1540,583]`,
`workspace: {id:-98, name:special:agent-wall}`, `visible: true`, `hidden: false`.
The `/tmp/agentwall_revealed.png` crop shows the actual dark overlay with two
session rows (green attach dot, name, pane tail) and the `2 sessions · 0s ago`
footer.
