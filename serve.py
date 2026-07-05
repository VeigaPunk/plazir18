#!/usr/bin/env python3
"""Stdlib-only server: serves index.html, /sessions (live tmux tiles, the
primary feed), /agents.json (legacy job-scan feed, kept alive), and the
tp/kill control-plane actions.

Run: python3 serve.py [port]   (default 7777)
Reachable from Windows Chrome at http://localhost:7777/ via WSL2 localhost forwarding.
"""
import json
import subprocess
import sys
import time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 7777
DIR = Path(__file__).resolve().parent
JOBS_DIR = Path.home() / ".claude" / "jobs"
PANE_LINES = 30


def fake_agents():
    now = time.time()
    return [
        {"name": "ccs-scout-priorart", "status": "active", "started": now - 42, "sourceAge": 0},
        {"name": "ccs-labrat-statefs", "status": "busy", "started": now - 190, "sourceAge": 0},
        {"name": "ccs-connector-pip", "status": "idle", "started": now - 610, "sourceAge": 0},
    ]


# Schema confirmed against a live ~/.claude/jobs/<id>/state.json: top-level
# state/tempo describe the job, fan[] lists currently-running Agent-tool
# subagents/shells (id/kind/label/startedAt in epoch ms) inside it. The
# sibling ~/.claude/teams/session-*/config.json under-reports liveness —
# it only lists team-lead + backend-attached members, not Agent-tool
# subagents — so it is deliberately not used here.
def real_agents():
    agents = []
    for state_path in sorted(JOBS_DIR.glob("*/state.json")):
        job_id = state_path.parent.name
        try:
            data = json.loads(state_path.read_text())
        except (OSError, json.JSONDecodeError):
            continue

        tempo = data.get("tempo")
        state = data.get("state")
        if state == "done":
            continue  # finished jobs drop off the live wall entirely

        # needs-input is first-class: a job/tempo stuck on "blocked" means a
        # human is being waited on, which is the signal these walls exist to
        # surface — it must outrank plain idle.
        if state == "working":
            status = "active" if tempo == "active" else "busy"
        elif state == "blocked" or tempo == "blocked":
            status = "needs-input"
        else:
            status = "idle"

        mtime = state_path.stat().st_mtime
        source_age = round(time.time() - mtime, 1)
        fan = data.get("fan") or []
        # fan[] entries carry no per-agent liveness marker of their own
        # (just id/kind/label/startedAt) — they're only meaningful while
        # the parent job is still state=="working".
        if fan and state == "working":
            for entry in fan:
                label = (entry.get("label") or entry.get("kind") or "agent")[:40]
                started_ms = entry.get("startedAt")
                started = started_ms / 1000 if started_ms else mtime
                agents.append({
                    "name": f"{job_id[:8]}:{label}",
                    "status": status,
                    "started": started,
                    "sourceAge": source_age,
                })
        else:
            name = (data.get("name") or job_id)[:40]
            agents.append({
                "name": name,
                "status": status,
                "started": mtime,
                "sourceAge": source_age,
            })
    return agents


def get_agents():
    try:
        agents = real_agents()
    except OSError:
        agents = []
    return agents if agents else fake_agents()


# tmux is both the data source (list-sessions + capture-pane) and the
# control plane (attach/kill) for the live tile wall. capture-pane without
# -e already strips escape/color sequences, so no ANSI renderer is needed —
# the frontend still HTML-escapes it before inserting into a <pre>, since
# arbitrary program output must never be trusted as markup.
def get_sessions():
    try:
        raw = subprocess.check_output(
            ["tmux", "list-sessions", "-F", "#{session_id}\t#{session_name}\t#{session_created}\t#{session_attached}"],
            stderr=subprocess.DEVNULL, text=True,
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
                ["tmux", "capture-pane", "-p", "-t", sid, "-S", str(-PANE_LINES)],
                stderr=subprocess.DEVNULL, text=True,
            )
        except (subprocess.CalledProcessError, FileNotFoundError):
            pane = ""  # session vanished between list and capture
        sessions.append({
            "name": name,
            "created": int(created),
            "attached": attached != "0",
            "pane": pane,
        })
    return sessions


def launch_tp(name):
    if not name:
        return False, "missing session name"
    try:
        # wt.exe/wsl.exe are reachable from WSL via Windows interop PATH.
        # tmux supports multiple attached clients, so this doesn't disturb
        # anyone already attached to the session.
        subprocess.Popen(
            ["wt.exe", "-w", "_new", "wsl.exe", "--", "tmux", "attach", "-t", name],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        return True, f"launched terminal for {name}"
    except OSError as e:
        return False, f"failed to launch wt.exe: {e}"


def kill_session(name):
    if not name:
        return False, "missing session name"
    result = subprocess.run(["tmux", "kill-session", "-t", name], capture_output=True, text=True)
    if result.returncode == 0:
        return True, f"killed {name}"
    return False, result.stderr.strip() or "kill failed"


class Handler(SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/sessions":
            body = json.dumps(get_sessions()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path == "/agents.json":
            body = json.dumps(get_agents()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        return super().do_GET()

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        try:
            payload = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            payload = {}
        name = payload.get("name", "")

        if self.path == "/tp":
            ok, message = launch_tp(name)
        elif self.path == "/kill":
            ok, message = kill_session(name)
        else:
            self.send_response(404)
            self.end_headers()
            return

        body = json.dumps({"ok": ok, "message": message}).encode()
        self.send_response(200 if ok else 500)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, fmt, *args):
        pass  # keep stdout quiet; comment out to debug


def main():
    import os
    os.chdir(DIR)
    with ThreadingHTTPServer(("127.0.0.1", PORT), Handler) as httpd:
        print(f"Serving on http://127.0.0.1:{PORT}  (endpoints: /sessions, /agents.json, /tp, /kill)")
        httpd.serve_forever()


if __name__ == "__main__":
    main()
