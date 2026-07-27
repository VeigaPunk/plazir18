---
name: agent-pip-docs
description: Install, run, and control the agent-pip multi-panel tmux agent dashboard.
---

agent-pip is a Rust (`eframe`/`egui`) always-on-top dashboard that shows one tile per live tmux session. Each tile displays the session name, attach state, and the last few pane lines.

## Install

```bash
git clone https://github.com/VeigaPunk/agent-pip.git
cd agent-pip
cargo build --release
# binary is at target/release/agent-wall
cp target/release/agent-wall ~/.local/bin/agent-pip
```

## Run the dashboard

```bash
agent-pip
```

## Run the legacy strip

```bash
agent-pip --strip
```

## Toggle visibility (Hyprland)

Add a key bind to `~/.config/hypr/hyprland.conf`:

```ini
bind = $mainMod, A, exec, agent-pip --toggle
```

On other compositors, use the IPC toggle directly:

```bash
agent-pip --toggle
```

## Attach or kill a session

- **Left-click** a tile to attach in your terminal (`TERMINAL` env, or `alacritty` / `kitty` / `foot` / `xterm` fallback).
- **Right-click** a tile to kill that tmux session.

## Requirements

- tmux server running with at least one session.
- A Wayland or X11 desktop with a terminal emulator installed.
