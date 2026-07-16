# agent-pip

Agent wall in Rust (eframe/egui): one tile per live tmux session, crisp DPI-aware rendering, always-on-top.

This repo supports Linux desktops (including Omarchy) for local tmux usage.

**tmux is a must** — sessions only get a tile if they run inside tmux
(`capture-pane` is the feed). Anything launched outside tmux is invisible
to the wall.

## Linux: clone, build, and run

```bash
git clone https://github.com/VeigaPunk/agent-pip.git
cd agent-pip
cargo run
```

`TERMINAL` (if set) is used as primary launcher for left-click attach; otherwise
the app falls back to `alacritty`, `kitty`, `foot`, and `xterm`.

## Build & deploy (Windows)

```bash
# inside WSL — cross-compiles to Windows and installs into shell:startup
./deploy.sh
```

The exe lands in the Windows Startup folder, so it runs at logon and can
be double-clicked for a manual launch. A second launch exits silently
(single-instance guard).

## Interactions

Mouse-only:
- **left-click** a tile → open a terminal and attach to that session
- **right-click** a tile → kill it

Flags: `--no-top` disables always-on-top.

## Dev (Linux)

```bash
cargo test           # parser unit tests
cargo run            # runs against local tmux directly (no wsl.exe hop)
```
