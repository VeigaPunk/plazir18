#!/usr/bin/env python3
"""Native tkinter agent wall: one tile per live tmux session. A fixed
POLL_SECONDS heartbeat, independent of session count, advances a
round-robin index by one and refreshes just that tile each tick.
Mouse-only: left-click attaches a terminal, right-click kills the
session. No keyboard, no dialogs.

Run inside WSL, or from native Windows Python (better rendering); tmux
calls route through wsl.exe -e in the latter case.
"""
import shutil
import subprocess
import threading
import time
import tkinter as tk

POLL_SECONDS = 60
PANE_LINES = 30

# wsl.exe -e (exec), not --: `--` pipes the command through the default
# shell, which expands a literal "$0" session-id argument to its own $0.
TMUX = ["tmux"] if shutil.which("tmux") else ["wsl.exe", "-e", "tmux"]

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


def list_sessions():
    # encoding forced to utf-8: Windows Python's subprocess text mode
    # otherwise defaults to the console codepage (e.g. cp1252), which
    # can't decode real UTF-8 tmux pane content.
    try:
        raw = subprocess.check_output(
            TMUX + ["list-sessions", "-F", "#{session_id}\t#{session_name}\t#{session_created}\t#{session_attached}"],
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return []
    sessions = []
    for line in raw.splitlines():
        parts = line.split("\t")
        if len(parts) != 4:
            continue
        sid, name, created, attached = parts
        sessions.append({"id": sid, "name": name, "created": int(created), "attached": attached != "0"})
    return sessions


def capture_pane(session_id):
    # Target by $-prefixed session id, not name: -t takes a target-pane,
    # and a bare numeric name like "0" resolves as a pane index first.
    try:
        return subprocess.check_output(
            TMUX + ["capture-pane", "-p", "-t", session_id, "-S", str(-PANE_LINES)],
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8", errors="replace",
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""  # session vanished between list and capture


def get_sessions():
    """One-shot snapshot for testing; the UI calls list_sessions()/capture_pane() separately."""
    sessions = list_sessions()
    for s in sessions:
        s["pane"] = capture_pane(s["id"])
    return sessions


def launch_tp(name):
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

        self.session_meta = {}   # name -> {id, created, attached}
        self.tiles = {}          # name -> {"frame": Frame, "text": Text}
        self.last_tick_ts = None
        self._rotation_index = 0
        self._last_tp_click = {}
        self.stop_event = threading.Event()

        self.container = tk.Frame(self, bg=BG)
        self.container.pack(fill="both", expand=True, padx=WINDOW_PAD, pady=(WINDOW_PAD, 0))
        self.empty_label = None

        self.status_var = tk.StringVar(value="starting…")
        tk.Label(self, textvariable=self.status_var, bg=BG, fg=MUTED, anchor="w",
                 font=("Segoe UI", 8)).pack(fill="x", padx=8, pady=4)

        self.protocol("WM_DELETE_WINDOW", self._on_close)  # OS close button, not a keyboard shortcut

        self._resize_window(0)
        threading.Thread(target=self._poll_loop, daemon=True).start()
        self._tick_title()

    # ---- polling: worker thread does tmux subprocess calls; only
    # .after() ever touches Tk state. Fixed POLL_SECONDS heartbeat — not
    # derived from session count — advances one round-robin step per tick. ----
    def _poll_loop(self):
        while not self.stop_event.is_set():
            meta_list = list_sessions()
            self.after(0, self._reconcile_tiles, meta_list)
            self.last_tick_ts = time.time()
            if meta_list:
                self._rotation_index %= len(meta_list)
                meta = meta_list[self._rotation_index]
                self._rotation_index += 1
                pane = capture_pane(meta["id"])
                self.after(0, self._update_tile_pane, meta["name"], pane)
            if self.stop_event.wait(POLL_SECONDS):
                break

    def _fetch_initial_pane(self, session_id, name):
        def worker():
            pane = capture_pane(session_id)
            self.after(0, self._update_tile_pane, name, pane)
        threading.Thread(target=worker, daemon=True).start()

    def _reconcile_tiles(self, meta_list):
        self.session_meta = {m["name"]: m for m in meta_list}
        self.status_var.set(f"{len(meta_list)} sessions" if meta_list else "no tmux sessions")

        for name in list(self.tiles):
            if name not in self.session_meta:
                self._destroy_tile(name)
        for meta in meta_list:
            if meta["name"] not in self.tiles:
                self._make_tile(meta["name"], meta["attached"])
                self._fetch_initial_pane(meta["id"], meta["name"])  # don't wait for its round-robin turn
            else:
                self._set_tile_border(meta["name"], meta["attached"])

        if not meta_list and self.empty_label is None:
            self.empty_label = tk.Label(self.container, text="no tmux sessions — F7 tmux ON, then F6",
                                         bg=BG, fg=MUTED, font=("Segoe UI", 9), wraplength=TILE_WIDTH, justify="left")
            self.empty_label.pack(side="top", anchor="w")
        elif meta_list and self.empty_label is not None:
            self.empty_label.destroy()
            self.empty_label = None

        self._resize_window(len(meta_list))

    def _update_tile_pane(self, name, pane_text):
        tile = self.tiles.get(name)
        if not tile:
            return  # session was removed mid-cycle
        txt = tile["text"]
        view_top = txt.yview()[0]  # preserve manual scroll/pan across the refresh
        txt.configure(state="normal")
        txt.delete("1.0", "end")
        txt.insert("1.0", pane_text.rstrip("\n"))
        txt.yview_moveto(view_top)
        txt.configure(state="disabled")

    # ---- tiles: created/destroyed once per cycle, text mutated per-tile ----
    def _make_tile(self, name, attached):
        border = ATTACHED if attached else BORDER
        tile = tk.Frame(self.container, width=TILE_WIDTH, height=TILE_HEIGHT, bg=CARD,
                         highlightthickness=1, highlightbackground=border, highlightcolor=border)
        tile.pack_propagate(False)
        tile.pack(side="top", pady=(0, GAP))

        txt = tk.Text(tile, bg=CARD, fg=TEXT, font=("Consolas", 8), bd=0, padx=8, pady=8,
                      highlightthickness=0, wrap="none", cursor="hand2", state="disabled")
        txt.place(x=0, y=0, width=TILE_WIDTH, height=TILE_HEIGHT)

        nameplate = tk.Label(tile, text=name, bg="#000000", fg="#ffffff",
                              font=("Segoe UI", 8, "bold"), padx=7, pady=1)
        nameplate.place(x=6, rely=1.0, y=-6, anchor="sw")

        for widget in (tile, txt, nameplate):
            widget.bind("<Button-1>", lambda _e, n=name: self._on_left_click(n))
            widget.bind("<Button-3>", lambda _e, n=name: self._on_right_click(n))

        self.tiles[name] = {"frame": tile, "text": txt, "border": border}

    def _destroy_tile(self, name):
        tile = self.tiles.pop(name, None)
        if tile:
            tile["frame"].destroy()

    def _set_tile_border(self, name, attached):
        tile = self.tiles.get(name)
        if not tile:
            return
        border = ATTACHED if attached else BORDER
        if tile["border"] != border:
            tile["frame"].configure(highlightbackground=border, highlightcolor=border)
            tile["border"] = border

    def _resize_window(self, count):
        rows = max(count, 1)
        content_h = rows * TILE_HEIGHT + max(rows - 1, 0) * GAP
        self.geometry(f"{TILE_WIDTH + WINDOW_PAD * 2}x{content_h + WINDOW_PAD * 2 + STATUS_HEIGHT}")

    def _tick_title(self):
        age = "never" if self.last_tick_ts is None else f"{int(time.time() - self.last_tick_ts)}s ago"
        self.title(f"Agent Wall — {len(self.session_meta)} sessions — last tick {age}")
        self.after(1000, self._tick_title)

    # ---- interactions: left-click (debounced) = tp, right-click = kill ----
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
        if name not in self.session_meta:
            return  # already gone since the last list refresh
        ok, message = kill_tmux_session(name)
        self.status_var.set(message)
        if ok:
            self.session_meta.pop(name, None)
            self._destroy_tile(name)
            self._resize_window(len(self.session_meta))

    def _on_close(self):
        self.stop_event.set()
        self.destroy()


if __name__ == "__main__":
    AgentWall().mainloop()
