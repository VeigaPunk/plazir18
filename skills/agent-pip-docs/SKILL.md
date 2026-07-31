---
name: agent-pip-docs
description: Install, run, and control the plazir18 multi-panel tmux agent dashboard.
---

plazir18 is a Rust (`eframe`/`egui`) always-on-top dashboard that shows one tile per live tmux session. Each tile displays the session name, attach state, and the last few pane lines.

## Install

```bash
git clone https://github.com/VeigaPunk/plazir18.git
cd plazir18
cargo build --release --locked
# binary is at target/release/agent-wall
install -Dm755 target/release/agent-wall ~/.local/bin/plazir18
```

## Run the dashboard

```bash
uwsm-app -- plazir18
```

## Run the legacy strip

```bash
plazir18 --strip
```

## Toggle visibility (Hyprland)

Add a key bind to `~/.config/hypr/hyprland.conf`:

```ini
bind = $mainMod, A, exec, plazir18 --toggle
```

On other compositors, use the IPC toggle directly:

```bash
plazir18 --toggle
```

## Attach or kill a session

- **Left-click** a tile to attach in `alacritty`.
- **Right-click** a tile to kill that tmux session.

## Requirements

- tmux server running with at least one session.
- A Wayland or X11 desktop with a terminal emulator installed.
