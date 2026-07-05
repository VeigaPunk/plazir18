# agent-pip

Native tkinter agent wall: one tile per live tmux session.

```bash
# inside WSL
cd agent-pip
python3 wall.py
```

```powershell
# on Windows directly (better rendering than WSLg)
python.exe \\wsl.localhost\Ubuntu-24.04\home\vhpnk\repos\agent-pip\wall.py
```

**Interactions:**
- **left-click** a tile → attach a terminal to that session
- **right-click** a tile → kill it

## Autostart

```bash
# ~/.bashrc
pgrep -f "python3 .*agent-pip/wall.py" >/dev/null || \
  (DISPLAY=:0 python3 ~/repos/agent-pip/wall.py &) 2>/dev/null
```

```toml
[[app]]
name = "agent-wall"
command = "python3"
args = ["/home/vhpnk/repos/agent-pip/wall.py"]
env = { DISPLAY = ":0" }
```
