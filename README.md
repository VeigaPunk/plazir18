# agent-pip

**Current tool: `wall.py`** — a native tkinter window, one file, stdlib
+ tmux only. Just a display and a window selector: no keyboard
interaction at all. A narrow, always-running vertical strip: one tile per
live tmux session, each showing a live terminal preview (the last 30
lines of `tmux capture-pane`). Polls every 60s on a timer only —
deliberately slow, this is for glancing, not dashboarding. Tile size
(220×150px) is a literal translation of the original HTML prototype's
`.tile`/`.pane` CSS.

Runs two ways — same file, no flags needed, it auto-detects which:

```bash
# inside WSL — renders via WSLg/X11
cd agent-pip
python3 wall.py
```

```powershell
# on Windows directly — native Win32 window, better fonts/DPI/titlebar
# than the WSLg-rendered version. Needs python.org's Windows installer
# (bundles tkinter) and wt.exe/wsl.exe on PATH (both ship with Windows).
python.exe \\wsl.localhost\Ubuntu-24.04\home\vhpnk\repos\agent-pip\wall.py
```

`tmux` itself only exists inside WSL, so when the script detects it isn't
running inside WSL (`tmux` not directly on PATH), every tmux call is
transparently relayed through `wsl.exe -e tmux ...` instead. Two gotchas
that cost real debugging time and are now baked into the code as
comments, not just this README:
- must be `wsl.exe -e` (exec), not `wsl.exe --` — the latter pipes the
  command through the default Linux shell, which expands a literal `$0`
  argument (a tmux session id) to that shell's own `$0` before tmux ever
  sees it.
- subprocess text-mode decoding must be forced to `encoding="utf-8"` —
  Windows Python's default is the console's codepage (e.g. cp1252),
  which can't decode UTF-8 bytes that show up in real tmux pane content.

**Interactions — exactly two, mouse only, no confirm dialogs:**
- **left-click** a tile → attach a new Windows Terminal window to that
  session (`tmux attach`; non-disruptive, tmux allows multiple attached
  clients). A double-click is debounced to a single attach.
- **right-click** a tile → `tmux kill-session` it, immediately.

That's the whole interface — no refresh key, no quit key. Close the
window with its normal OS close button if you need to stop it; otherwise
it's meant to just stay running (see autostart below).

A green border marks a session that already has a client attached
("speaking"). The window has no fixed height — it grows by one tile per
session (typically capped around ~6 in practice) and never wraps to a
second column.

`claude agents --json` was tried as a source of extra per-tile status/pid
info but dropped: nothing in its output joins to a tmux session name, so
tmux is the whole data source and the whole control plane here.

## Autostarting it

Since the window is meant to just always be running, start it once at
login instead of launching it by hand. Recipe only — nothing here edits
your actual `.bashrc` or launcher config:

```bash
# ~/.bashrc, guarded so it only starts once per login shell tree
pgrep -f "python3 .*agent-pip/wall.py" >/dev/null || \
  (DISPLAY=:0 python3 ~/repos/agent-pip/wall.py &) 2>/dev/null
```

or as a `claude-launcher`-style toml entry:

```toml
[[app]]
name = "agent-wall"
command = "python3"
args = ["/home/vhpnk/repos/agent-pip/wall.py"]
env = { DISPLAY = ":0" }
```

## Legacy: the HTML/PiP version

`serve.py` + `index.html` are the earlier browser-based iteration
(superseded by `wall.py`, not deleted). They served a live tmux
session grid over HTTP with a Chrome `documentPictureInPicture` pop-out,
click-to-attach, and right-click-to-kill. Kept on disk for reference;
not the maintained path going forward.

```bash
python3 serve.py 7777    # then open http://localhost:7777/ from Windows Chrome
```

If reviving that path, the same `chrome --app=http://localhost:7777`
persistent-opener trick (or a `claude-launcher`-style toml entry running
that command) still applies for keeping its window around across
restarts — see the git history of this file for the fuller writeup.
