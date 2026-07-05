#!/usr/bin/env python3
"""Native tkinter agent wall. One tile per live tmux session, stacked in a
single vertical column (a narrow always-running strip, not a dashboard) —
each tile is a live tmux capture-pane preview, translated directly from
the earlier HTML/PiP prototype's tile CSS (see TILE_WIDTH/TILE_HEIGHT).

Just a display and a window selector: no keyboard interaction at all.
60s timer poll, left-click a tile = attach a terminal to it, right-click
a tile = kill it. That's the entire interface.

`claude agents --json` was tried as an enrichment source (status/pid per
tile) but dropped: it has no field that joins to a tmux session name, so
adding it would mean guessing a match instead of a real one. Tmux is the
whole data source and the whole control plane here.

Runs two ways:
- inside WSL:      python3 wall.py            (renders via WSLg/X11)
- on Windows:      python.exe wall.py         (native Win32 window — the
  better-looking option; tmux itself only exists inside WSL, so tmux
  calls are transparently routed through `wsl.exe --` when `tmux` isn't
  directly on PATH, i.e. whenever this isn't running inside WSL)

Requires: python3-tk (bundled in the python.org Windows installer too),
tmux inside WSL, and — when run from Windows — wsl.exe/wt.exe on PATH
(both ship with Windows by default).
"""
import shutil
import subprocess
import threading
import time
import tkinter as tk

POLL_SECONDS = 60
PANE_LINES = 30

# Same tmux calls work from inside WSL (direct) or from native Windows
# Python (relayed through wsl.exe, since tmux itself only exists in WSL).
# Must use `-e` (exec, no shell) rather than `--`: plain `wsl.exe -- cmd`
# pipes the command through the default Linux shell, which expands a
# literal "$0" argument (a tmux session id) to that shell's own $0
# ("/bin/bash") before tmux ever sees it — confirmed by reproducing
# "can't find pane: /bin/bash" and fixing it with -e. Detected once at
# import time — whichever process runs this script.
TMUX = ["tmux"] if shutil.which("tmux") else ["wsl.exe", "-e", "tmux"]

# Literal translation of index.html's .tile/.pane CSS (min-height:150px,
# grid-template-columns: minmax(220px,1fr), gap:10px) into fixed tkinter
# pixel dimensions — not invented numbers.
TILE_WIDTH = 220
TILE_HEIGHT = 150
GAP = 10
WINDOW_PAD = 12
STATUS_HEIGHT = 20
TP_DEBOUNCE_SEC = 0.5  # collapses a double-click into one tp launch

BG = "#0b0e14"
CARD = "#161b26"
TEXT = "#c9d6e6"
MUTED = "#8b95ab"
ATTACHED = "#3ee08b"
BORDER = "#232a38"


def get_sessions():
    """tmux list-sessions + capture-pane, ported from the earlier serve.py
    scan. capture-pane without -e already strips ANSI/color escapes."""
    try:
        raw = subprocess.check_output(
            TMUX + ["list-sessions", "-F", "#{session_id}\t#{session_name}\t#{session_created}\t#{session_attached}"],
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []  # no tmux server running, or tmux not installed

    sessions = []
    for line in raw.splitlines():
        parts = line.split("\t")
        if len(parts) != 4:
            continue
        sid, name, created, attached = parts
        try:
            # capture by $-prefixed session id, never by name: -t takes a
            # target-PANE, and a bare name like "0" resolves as a pane index
            # of the current session before it matches the session named "0"
            pane = subprocess.check_output(
                TMUX + ["capture-pane", "-p", "-t", sid, "-S", str(-PANE_LINES)],
                stderr=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            pane = ""  # session vanished between list and capture
        sessions.append({"name": name, "created": int(created), "attached": attached != "0", "pane": pane})
    return sessions


def launch_tp(name):
    """Attach a new Windows Terminal window to the session (multi-client,
    doesn't disturb anyone already attached)."""
    if not name:
        return False, "missing session name"
    try:
        subprocess.Popen(
            ["wt.exe", "-w", "_new", "wsl.exe", "--", "tmux", "attach", "-t", name],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        return True, f"launched terminal for {name}"
    except OSError as e:
        return False, f"failed to launch wt.exe: {e}"


def kill_tmux_session(name):
    """Pure function (no Tk dependency) so it's exercisable standalone."""
    if not name:
        return False, "missing session name"
    result = subprocess.run(TMUX + ["kill-session", "-t", name], capture_output=True,
                             text=True, encoding="utf-8", errors="replace")
    if result.returncode == 0:
        return True, f"killed {name}"
    return False, result.stderr.strip() or "kill failed"


class AgentWall(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("Agent Wall")
        self.configure(bg=BG)
        self.resizable(False, True)

        self.sessions = []
        self.last_poll_ts = None
        self._last_tp_click = {}
        self.stop_event = threading.Event()

        self.container = tk.Frame(self, bg=BG)
        self.container.pack(fill="both", expand=True, padx=WINDOW_PAD, pady=(WINDOW_PAD, 0))

        self.status_var = tk.StringVar(value="starting…")
        tk.Label(self, textvariable=self.status_var, bg=BG, fg=MUTED, anchor="w",
                 font=("Segoe UI", 8)).pack(fill="x", padx=8, pady=4)

        # No keyboard bindings at all — display + left-click + right-click
        # is the entire interface. WM_DELETE_WINDOW is the OS close button
        # (mouse), not a keyboard shortcut, so it stays.
        self.protocol("WM_DELETE_WINDOW", self._on_close)

        self._resize_window(0)
        threading.Thread(target=self._poll_loop, daemon=True).start()
        self._tick_title()

    # ---- polling (worker thread does the tmux subprocess calls; only
    # .after() ever touches Tk state, from the mainloop thread) ----
    def _poll_loop(self):
        while not self.stop_event.is_set():
            self._do_poll()
            self.stop_event.wait(POLL_SECONDS)

    def _trigger_repoll(self):
        """Immediate re-poll after a kill — not user-triggered, there's no
        manual-refresh interaction anymore, just the 60s timer."""
        threading.Thread(target=self._do_poll, daemon=True).start()

    def _do_poll(self):
        sessions = get_sessions()
        self.after(0, self._apply_poll, sessions)

    def _apply_poll(self, sessions):
        self.sessions = sessions
        self.last_poll_ts = time.time()
        self.status_var.set(f"{len(sessions)} sessions — poll ok" if sessions else "no tmux sessions")
        self._render()

    # ---- rendering: strictly vertical, one tile per session ----
    def _render(self):
        for child in self.container.winfo_children():
            child.destroy()

        if not self.sessions:
            tk.Label(self.container, text="no tmux sessions — F7 tmux ON, then F6",
                      bg=BG, fg=MUTED, font=("Segoe UI", 9), wraplength=TILE_WIDTH,
                      justify="left").pack(side="top", anchor="w")
        else:
            for s in self.sessions:
                self._make_tile(s)

        self._resize_window(len(self.sessions))

    def _make_tile(self, s):
        border = ATTACHED if s["attached"] else BORDER
        tile = tk.Frame(self.container, width=TILE_WIDTH, height=TILE_HEIGHT, bg=CARD,
                         highlightthickness=1, highlightbackground=border, highlightcolor=border)
        tile.pack_propagate(False)
        tile.pack(side="top", pady=(0, GAP))

        pane_text = (s.get("pane") or "").rstrip("\n")
        txt = tk.Text(tile, bg=CARD, fg=TEXT, font=("Consolas", 8), bd=0, padx=8, pady=8,
                      highlightthickness=0, wrap="none", cursor="hand2")
        txt.insert("1.0", pane_text)
        txt.configure(state="disabled")
        txt.place(x=0, y=0, width=TILE_WIDTH, height=TILE_HEIGHT)

        nameplate = tk.Label(tile, text=s["name"], bg="#000000", fg="#ffffff",
                              font=("Segoe UI", 8, "bold"), padx=7, pady=1)
        nameplate.place(x=6, rely=1.0, y=-6, anchor="sw")

        for widget in (tile, txt, nameplate):
            widget.bind("<Button-1>", lambda _e, n=s["name"]: self._on_left_click(n))
            widget.bind("<Button-3>", lambda _e, n=s["name"]: self._on_right_click(n))

    def _resize_window(self, count):
        rows = max(count, 1)
        content_h = rows * TILE_HEIGHT + max(rows - 1, 0) * GAP
        self.geometry(f"{TILE_WIDTH + WINDOW_PAD * 2}x{content_h + WINDOW_PAD * 2 + STATUS_HEIGHT}")

    def _tick_title(self):
        age = "never" if self.last_poll_ts is None else f"{int(time.time() - self.last_poll_ts)}s ago"
        self.title(f"Agent Wall — {len(self.sessions)} sessions — polled {age}")
        self.after(1000, self._tick_title)

    # ---- interactions: left-click (debounced against double-click) = tp,
    # right-click = kill. Nothing else — no keyboard, no dialogs. ----
    def _on_left_click(self, name):
        now = time.monotonic()
        if now - self._last_tp_click.get(name, 0) < TP_DEBOUNCE_SEC:
            return  # a double-click fires two Button-1 events; collapse to one tp
        self._last_tp_click[name] = now

        def worker():
            ok, message = launch_tp(name)
            self.after(0, lambda: self.status_var.set(message))
        threading.Thread(target=worker, daemon=True).start()

    def _on_right_click(self, name):
        # guard against killing a session that's already gone since the
        # last poll (never act on a name outside the current snapshot)
        if name not in {s["name"] for s in self.sessions}:
            return
        ok, message = kill_tmux_session(name)
        self.status_var.set(message)
        if ok:
            self._trigger_repoll()

    def _on_close(self):
        self.stop_event.set()
        self.destroy()


if __name__ == "__main__":
    AgentWall().mainloop()
