# agent-pip (agent-wall)

Multi-panel agent terminal dashboard in Rust (eframe/egui): one **panel per live
tmux session**, up to **24 concurrent** tiles in a grid. Live pane tails,
DPI-aware rendering, always-on-top optional.

**tmux is a must** — sessions only get a tile if they run inside tmux
(`capture-pane` is the feed). Anything launched outside tmux is invisible
to the wall.

## Layout

**Default: multi-panel grid** (dashboard of terminals)

- Up to **24** concurrent panels (hard ceiling; no soft nudge below that).
- Adaptive columns: 1 / 2 / 3 / 4 / 6 cols by count.
- Each panel: title + attach dot + last ~10 non-empty pane lines.
- Left-click → attach in terminal · right-click → kill.

**Legacy strip** (top bar of compact rows):

```bash
cargo run -- --strip
```

## Linux: clone, build, and run

```bash
git clone https://github.com/VeigaPunk/agent-pip.git
cd agent-pip
cargo run
```

`TERMINAL` (if set) is used as primary launcher for left-click attach; otherwise
the app falls back to `alacritty`, `kitty`, `foot`, and `xterm`.

## Flags

| Flag | Effect |
|------|--------|
| *(none)* | Multi-panel dashboard, max 24 tiles |
| `--strip` | Legacy horizontal strip |
| `--no-top` | Disable always-on-top |
| `--toggle` | Reveal/hide running instance (Hyprland special workspace or IPC) |

## Build & deploy (Windows)

```bash
# inside WSL — cross-compiles to Windows and installs into shell:startup
./deploy.sh
```

## Dev

```bash
cargo test
cargo run              # multi-panel
cargo run -- --strip   # strip mode
```

Homepage / marketplace brand: https://ds4cc.com/
