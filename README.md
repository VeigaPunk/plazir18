# agent-pip

Native Windows agent wall in Rust (eframe/egui): one tile per live tmux
session, crisp DPI-aware rendering, always-on-top.

**tmux is a must** — sessions only get a tile if they run inside tmux
(`capture-pane` is the feed). Anything launched outside tmux is invisible
to the wall.

## Build & deploy

```bash
# inside WSL — cross-compiles to Windows and installs into shell:startup
./deploy.sh
```

The exe lands in the Windows Startup folder, so it runs at logon and can
be double-clicked for a manual launch. A second launch exits silently
(single-instance guard).

## Interactions

Mouse-only:
- **left-click** a tile → attach a Windows Terminal to that session
- **right-click** a tile → kill it

Flags: `--no-top` disables always-on-top.

## Dev (Linux)

```bash
cargo test           # parser unit tests
cargo run            # runs against local tmux directly (no wsl.exe hop)
```
