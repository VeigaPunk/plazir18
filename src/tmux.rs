//! All tmux I/O: session listing, pane capture, session kill.
use std::process::{Command, Stdio};

const PANE_LINES: i32 = 30;

#[derive(Clone)]
pub struct Meta {
    pub id: String,
    pub name: String,
    pub attached: bool,
}

pub fn parse_sessions(raw: &str) -> Vec<Meta> {
    raw.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 4 {
                return None;
            }
            Some(Meta {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                attached: parts[3] != "0",
            })
        })
        .collect()
}

pub fn list_sessions() -> Vec<Meta> {
    tmux(&[
        "list-sessions",
        "-F",
        "#{session_id}\t#{session_name}\t#{session_created}\t#{session_attached}",
    ])
    .map(|raw| parse_sessions(&raw))
    .unwrap_or_default()
}

/// Capture pane output. `session_id` must be the $-prefixed form to avoid
/// ambiguity when a numeric name resolves as a pane index first.
pub fn capture_pane(session_id: &str) -> String {
    tmux(&[
        "capture-pane",
        "-p",
        "-t",
        session_id,
        "-S",
        &format!("-{PANE_LINES}"),
    ])
    .unwrap_or_default()
}

/// Lifecycle events that should wake the wall: session appear/disappear and
/// attach-state flips (the green dot). Content changes ride `pipe_pane`.
const HOOK_EVENTS: [&str; 5] = [
    "session-created",
    "session-closed",
    "client-attached",
    "client-detached",
    "client-session-changed",
];

/// Install global hooks that poke the wake FIFO on session lifecycle changes,
/// so appearance/disappearance/attach are event-driven with no polling. Run at
/// startup and on every reconnect (a fresh tmux server has no hooks).
pub fn install_hooks(fifo: &str) {
    let quoted_fifo = shell_quote(fifo);
    let poke = format!("run-shell -b \"printf . >> {quoted_fifo}\"");
    for ev in HOOK_EVENTS {
        let _ = tmux(&["set-hook", "-g", ev, &poke]);
    }
}

/// Remove the hooks we installed (best-effort, on exit).
pub fn remove_hooks() {
    for ev in HOOK_EVENTS {
        let _ = tmux(&["set-hook", "-gu", ev]);
    }
}

/// Stream a session's active-pane output to the wake FIFO. Any output byte
/// wakes the poller, which then `capture_pane`s for a clean snapshot — so we
/// never parse the raw stream. `-o` makes this a no-op if already piped.
pub fn pipe_pane(session_id: &str, fifo: &str) {
    let command = format!("cat >> {}", shell_quote(fifo));
    let _ = tmux(&["pipe-pane", "-o", "-t", session_id, &command]);
}

/// Quote one argument for the POSIX shell used internally by tmux hooks.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Kill a tmux session by its $-prefixed id.
pub fn kill_session(session_id: &str) {
    let base = tmux_base();
    let mut cmd = Command::new(base[0]);
    cmd.args(&base[1..])
        .args(["kill-session", "-t", session_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _ = no_window(&mut cmd).spawn();
}

fn tmux(args: &[&str]) -> Option<String> {
    let base = tmux_base();
    let mut cmd = Command::new(base[0]);
    cmd.args(&base[1..]).args(args).stdin(Stdio::null());
    let out = no_window(&mut cmd).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tmux_base() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["wsl.exe", "-e", "tmux"]
    } else {
        vec!["tmux"]
    }
}

/// Suppress console window flash on Windows background commands.
#[cfg(windows)]
fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

#[cfg(test)]
mod tests {
    use super::{parse_sessions, shell_quote};

    #[test]
    fn parses_well_formed_lines() {
        let raw = "$0\tmain\t1751800000\t1\n$3\tscout\t1751800100\t0\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].id, "$0");
        assert_eq!(s[0].name, "main");
        assert!(s[0].attached);
        assert!(!s[1].attached);
    }

    #[test]
    fn skips_malformed_lines() {
        let raw = "garbage\n$1\tok\t123\t0\n\ttoo\tfew\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "ok");
    }

    #[test]
    fn empty_input() {
        assert!(parse_sessions("").is_empty());
    }

    #[test]
    fn fifo_path_is_shell_quoted() {
        assert_eq!(
            shell_quote("/tmp/$(touch /tmp/pwned)' wall"),
            "'/tmp/$(touch /tmp/pwned)'\\'' wall'"
        );
    }

    #[test]
    fn shell_quote_backtick_injection() {
        // backtick is not special inside single quotes
        assert_eq!(shell_quote("/tmp/`rm -rf /`"), "'/tmp/`rm -rf /`'");
    }

    #[test]
    fn shell_quote_double_quote() {
        // double quotes are literal inside single quotes
        assert_eq!(shell_quote("/tmp/foo\"bar"), "'/tmp/foo\"bar'");
    }

    #[test]
    fn shell_quote_spaces() {
        assert_eq!(
            shell_quote("/run/user/1000/agent wall.wake"),
            "'/run/user/1000/agent wall.wake'"
        );
    }

    #[test]
    fn shell_quote_embedded_single_quote() {
        // each ' → '\'' so the shell reconstructs the literal apostrophe
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }

    #[test]
    fn shell_quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }
}
