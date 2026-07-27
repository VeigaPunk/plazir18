//! All tmux I/O: session listing, pane capture, session kill.
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

const PANE_LINES: i32 = 30;

#[derive(Clone)]
pub struct Meta {
    pub id: String,
    pub name: String,
    /// Epoch seconds of the session's most recent pane output, max across its
    /// windows (`#{window_activity}`); `None` when unknown.
    pub activity: Option<i64>,
    pub attached: bool,
}

static ACTIVITY_PARSE_WARN: Once = Once::new();

impl Meta {
    /// `mm:ss` since this session last did anything. Minutes are not capped, so
    /// a two-hour idle reads `120:00`; unknown activity reads `--:--`.
    pub fn idle_label(&self, now: i64) -> String {
        let Some(activity) = self.activity else {
            return "--:--".to_string();
        };
        let secs = now.saturating_sub(activity).max(0);
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

/// Wall-clock seconds since the epoch — the `now` side of the idle counter.
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub fn parse_sessions(raw: &str) -> Vec<Meta> {
    raw.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 3 {
                return None;
            }
            Some(Meta {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                activity: None,
                attached: parts[2] != "0",
            })
        })
        .collect()
}

/// List tmux sessions and their most recent pane-output activity.
///
/// Each refresh makes two tmux calls; both are event-driven because the poller
/// calls this only on wake events, never on a timer.
pub fn list_sessions() -> Vec<Meta> {
    let sessions_raw = tmux(&[
        "list-sessions",
        "-F",
        "#{session_id}\t#{session_name}\t#{session_attached}",
    ])
    .unwrap_or_default();

    let windows_raw = tmux(&[
        "list-windows",
        "-a",
        "-F",
        "#{session_id}\t#{window_activity}",
    ])
    .unwrap_or_default();

    let mut sessions = parse_sessions(&sessions_raw);
    merge_window_activity(&mut sessions, parse_window_activity(&windows_raw));
    sessions
}

fn parse_window_activity(raw: &str) -> Vec<(String, i64)> {
    raw.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 2 {
                return None;
            }
            let activity = parts[1].parse::<i64>().map_err(|_| {
                ACTIVITY_PARSE_WARN.call_once(|| {
                    eprintln!("tmux: failed to parse #{{window_activity}}; idle may show --:--");
                });
            });
            Some((parts[0].to_string(), activity.ok()?))
        })
        .collect()
}

fn merge_window_activity(sessions: &mut [Meta], rows: Vec<(String, i64)>) {
    let mut activity_by_session = HashMap::new();
    for (session_id, activity) in rows {
        activity_by_session
            .entry(session_id)
            .and_modify(|current: &mut i64| *current = (*current).max(activity))
            .or_insert(activity);
    }
    for session in sessions {
        session.activity = activity_by_session.get(&session.id).copied();
    }
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

/// One tmux pane: an independently renderable screen with its own cell grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    /// `%`-prefixed pane id — the capture target for this exact screen.
    pub pane_id: String,
    pub session_name: String,
    pub window_index: u16,
    pub pane_index: u16,
    pub cols: u16,
    pub rows: u16,
    pub active: bool,
    pub command: String,
}

/// Tab-separated because the last two fields are free text: a session named
/// `my project` or a command like `vim foo.rs` would silently shift every
/// positional field under whitespace splitting. Tabs cannot occur in either.
const PANES_FORMAT: &str = "#{pane_id}\t#{session_name}\t#{window_index}\t#{pane_index}\t\
                            #{pane_width}\t#{pane_height}\t#{pane_active}\t#{pane_current_command}";

/// Every pane of every session, each with the `%id` that captures it.
///
/// This is the list the TUI renders from. A session id resolves to only its
/// current window's active pane, while this retains every window and split as
/// a separate live terminal with its own geometry.
pub fn list_panes() -> Vec<PaneInfo> {
    let raw = tmux(&["list-panes", "-a", "-F", PANES_FORMAT]).unwrap_or_default();
    parse_panes(&raw)
}

fn parse_panes(raw: &str) -> Vec<PaneInfo> {
    raw.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [
                pane_id,
                session_name,
                window_index,
                pane_index,
                cols,
                rows,
                active,
                command,
                ..,
            ] = fields.as_slice()
            else {
                return None;
            };
            Some(PaneInfo {
                pane_id: (*pane_id).to_string(),
                session_name: (*session_name).to_string(),
                window_index: window_index.parse().ok()?,
                pane_index: pane_index.parse().ok()?,
                cols: cols.parse().ok()?,
                rows: rows.parse().ok()?,
                active: *active == "1",
                command: (*command).to_string(),
            })
        })
        .collect()
}

/// Raw bytes of a target's visible screen, escape sequences intact, ready to
/// feed a vt100 parser.
///
/// `target` is any tmux target: a `$`-prefixed session id (resolves to that
/// session's current active pane) or, preferably, a `%`-prefixed pane id from
/// [`list_panes`], which names one exact screen with no resolution step.
///
/// Returns bytes rather than `String`: lossy UTF-8 conversion would corrupt the
/// escape sequences that carry every color and attribute.
pub fn capture_pane_styled(target: &str) -> Vec<u8> {
    let args = styled_capture_args(target);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    tmux_bytes(&borrowed).unwrap_or_default()
}

/// Each flag is load-bearing for the vt100 path; do not "simplify" them.
///
/// `-e` emits the SGR escapes that are the whole point of a styled capture.
/// `-N` preserves trailing spaces, without which short lines misalign the cell
/// grid. `-J` is omitted because joining wrapped lines breaks the
/// one-row-per-screen-line invariant vt100 replays against. `-S` is omitted so
/// the capture is the live visible screen, not scrollback.
///
/// `target` is passed through verbatim: tmux, not this function, owns target
/// syntax, so `$0` and `%12` both work and neither is rewritten.
fn styled_capture_args(target: &str) -> Vec<String> {
    ["capture-pane", "-p", "-e", "-N", "-t", target]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect()
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
    let command = crate::wake::poke_command(fifo);
    let poke = format!("run-shell -b {}", shell_quote(&command));
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
    let command = crate::wake::stream_command(fifo);
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

fn tmux_bytes(args: &[&str]) -> Option<Vec<u8>> {
    let base = tmux_base();
    let mut cmd = Command::new(base[0]);
    cmd.args(&base[1..]).args(args).stdin(Stdio::null());
    let out = no_window(&mut cmd).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout)
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
    use super::{
        Meta, PaneInfo, merge_window_activity, parse_panes, parse_sessions, parse_window_activity,
        shell_quote, styled_capture_args,
    };

    /// Same live server, [`super::PANES_FORMAT`]. Session `0` spans three
    /// windows; `%49` is a non-active split.
    const PANES_SAMPLE: &str = "%0\t0\t1\t1\t81\t51\t1\topencode\n\
                                %4\t0\t2\t1\t167\t51\t1\topencode\n\
                                %49\t0\t3\t2\t116\t10\t0\topencode\n\
                                %10\tgrok-ref\t1\t1\t81\t24\t1\tbash\n";

    #[test]
    fn parse_panes_reads_every_field() {
        let panes = parse_panes(PANES_SAMPLE);

        assert_eq!(panes.len(), 4);
        assert_eq!(
            panes[1],
            PaneInfo {
                pane_id: "%4".to_string(),
                session_name: "0".to_string(),
                window_index: 2,
                pane_index: 1,
                cols: 167,
                rows: 51,
                active: true,
                command: "opencode".to_string(),
            }
        );
        assert!(!panes[2].active, "%49 is a non-active split");
        assert_eq!(panes[3].session_name, "grok-ref");
    }

    #[test]
    fn parse_panes_keeps_every_pane_of_a_session() {
        let panes = parse_panes(PANES_SAMPLE);

        let session_0: Vec<_> = panes.iter().filter(|p| p.session_name == "0").collect();
        assert_eq!(session_0.len(), 3, "all three panes of session 0 survive");
    }

    #[test]
    fn parse_panes_handles_names_and_commands_with_spaces() {
        let raw = "%1\tmy project\t1\t1\t80\t24\t1\tvim foo.rs\n";

        let panes = parse_panes(raw);

        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].session_name, "my project");
        assert_eq!(panes[0].command, "vim foo.rs");
    }

    #[test]
    fn parse_panes_tolerates_extra_trailing_fields() {
        let raw = "%1\tsolo\t1\t1\t80\t24\t1\tbash\tfuture\tfields\n";

        let panes = parse_panes(raw);

        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].command, "bash");
    }

    #[test]
    fn parse_panes_skips_malformed() {
        let raw = "garbage\n\
                   %1\tshort\t1\n\
                   %2\tbad\tx\t1\t80\t24\t1\tbash\n\
                   %3\tgood\t1\t1\t80\t24\t0\tbash\n";

        let panes = parse_panes(raw);

        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].pane_id, "%3");
        assert!(!panes[0].active);
    }

    #[test]
    fn styled_capture_args_use_escape_and_trailing_spaces() {
        let args = styled_capture_args("$0");

        for expected in ["capture-pane", "-p", "-e", "-N", "-t", "$0"] {
            assert!(
                args.iter().any(|arg| arg == expected),
                "missing {expected:?} in {args:?}"
            );
        }
        for forbidden in ["-J", "-S"] {
            assert!(
                !args.iter().any(|arg| arg == forbidden),
                "unexpected {forbidden:?} in {args:?}"
            );
        }
    }

    #[test]
    fn styled_capture_targets_session_id_verbatim() {
        let args = styled_capture_args("$12");

        let target = args
            .iter()
            .position(|arg| arg == "-t")
            .expect("-t flag present");
        assert_eq!(args.get(target + 1).map(String::as_str), Some("$12"));
    }

    #[test]
    fn styled_capture_targets_pane_id_verbatim() {
        let args = styled_capture_args("%12");

        let target = args
            .iter()
            .position(|arg| arg == "-t")
            .expect("-t flag present");
        assert_eq!(args.get(target + 1).map(String::as_str), Some("%12"));
    }

    #[test]
    #[ignore = "needs a live tmux server with session $0"]
    fn live_styled_capture_returns_escape_bytes() {
        let bytes = super::capture_pane_styled("$0");

        assert!(!bytes.is_empty(), "no bytes from live tmux session $0");
        assert!(
            bytes.contains(&0x1b),
            "no ESC (0x1b) in {} captured bytes",
            bytes.len()
        );
    }

    #[test]
    #[ignore = "needs a live tmux server"]
    fn live_list_panes_captures_each_pane() {
        let panes = super::list_panes();

        assert!(!panes.is_empty(), "no panes from live tmux");
        for pane in &panes {
            assert!(
                pane.pane_id.starts_with('%'),
                "{} is not a %-prefixed pane id",
                pane.pane_id
            );
            assert!(pane.cols > 0 && pane.rows > 0, "{} empty", pane.pane_id);

            let bytes = super::capture_pane_styled(&pane.pane_id);
            assert!(!bytes.is_empty(), "{} captured nothing", pane.pane_id);
        }
    }

    #[test]
    fn parses_well_formed_lines() {
        let raw = "$0\tmain\t1\n$3\tscout\t0\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].id, "$0");
        assert_eq!(s[0].name, "main");
        assert_eq!(s[0].activity, None);
        assert!(s[0].attached);
        assert!(!s[1].attached);
    }

    #[test]
    fn skips_malformed_lines() {
        let raw = "garbage\n$1\tok\t0\n\ttoo\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "ok");
    }

    #[test]
    fn aggregation_picks_max_across_session_windows() {
        let mut sessions = parse_sessions("$1\tmain\t0\n$2\tscout\t0\n");
        let rows = parse_window_activity("$1\t100\n$2\t50\n$1\t120\n");

        merge_window_activity(&mut sessions, rows);

        assert_eq!(sessions[0].activity, Some(120));
        assert_eq!(sessions[1].activity, Some(50));
    }

    #[test]
    fn window_rows_for_unknown_sessions_are_ignored() {
        let mut sessions = parse_sessions("$1\tmain\t0\n");
        let rows = parse_window_activity("$99\t500\n$1\t100\n");

        merge_window_activity(&mut sessions, rows);

        assert_eq!(sessions[0].activity, Some(100));
    }

    #[test]
    fn session_without_window_rows_has_unknown_idle_label() {
        let mut sessions = parse_sessions("$1\tmain\t0\n");

        merge_window_activity(&mut sessions, Vec::new());

        assert_eq!(sessions[0].activity, None);
        assert_eq!(sessions[0].idle_label(1_000), "--:--");
    }

    #[test]
    fn malformed_window_lines_are_skipped() {
        let rows = parse_window_activity("garbage\n$1\tnot-a-number\n$2\t100\textra\n$3\t200\n");

        assert_eq!(rows, vec![("$3".to_string(), 200)]);
    }

    #[test]
    fn idle_label_counts_up_in_mm_ss() {
        let s = &Meta {
            id: "$0".to_string(),
            name: "main".to_string(),
            activity: Some(1000),
            attached: true,
        };
        assert_eq!(s.idle_label(1000), "00:00");
        assert_eq!(s.idle_label(1009), "00:09");
        assert_eq!(s.idle_label(1090), "01:30");
        assert_eq!(s.idle_label(1000 + 59 * 60 + 59), "59:59");
    }

    #[test]
    fn idle_label_minutes_are_not_capped() {
        let s = &Meta {
            id: "$0".to_string(),
            name: "main".to_string(),
            activity: Some(0),
            attached: true,
        };
        assert_eq!(s.idle_label(2 * 3600), "120:00");
    }

    #[test]
    fn idle_label_handles_unknown_and_clock_skew() {
        let unknown = &Meta {
            id: "$0".to_string(),
            name: "main".to_string(),
            activity: None,
            attached: true,
        };
        assert_eq!(unknown.activity, None);
        assert_eq!(unknown.idle_label(1000), "--:--");

        let future = &Meta {
            id: "$0".to_string(),
            name: "main".to_string(),
            activity: Some(2000),
            attached: true,
        };
        assert_eq!(future.idle_label(1000), "00:00");
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
