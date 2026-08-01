//! Minimal tool set: bash + read/write file. Plan mode is read-only.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Hard cap so a hung `!bash` cannot freeze the agent TUI event loop.
pub const BASH_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub enum Tool {
    Bash {
        cmd: String,
    },
    Read {
        path: String,
    },
    /// File write (scaffold; not yet wired from TUI).
    #[allow(dead_code)]
    Write {
        path: String,
        content: String,
    },
    List {
        path: String,
    },
}

#[derive(Debug)]
pub struct ToolResult {
    #[allow(dead_code)]
    pub ok: bool,
    pub output: String,
}

/// Run `bash -c` with a wall-clock timeout. On timeout, SIGTERM then SIGKILL the child.
fn run_bash_timed(cmd: &str, timeout: Duration) -> ToolResult {
    let child = match Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ToolResult {
                ok: false,
                output: e.to_string(),
            };
        }
    };

    let pid = child.id();
    let finished = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let fin_k = finished.clone();
    let to_k = timed_out.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if fin_k.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if fin_k.load(Ordering::SeqCst) {
            return;
        }
        to_k.store(true, Ordering::SeqCst);
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
        std::thread::sleep(Duration::from_millis(200));
        if !fin_k.load(Ordering::SeqCst) {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .output();
        }
    });

    let wait = child.wait_with_output();
    finished.store(true, Ordering::SeqCst);

    if timed_out.load(Ordering::SeqCst) {
        return ToolResult {
            ok: false,
            output: format!("bash timed out after {}s", timeout.as_secs()),
        };
    }

    match wait {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&err);
            }
            if s.len() > 8000 {
                let head = truncate_utf8(&s, 8000);
                s = format!("{head}\n\u{2026}truncated");
            }
            ToolResult {
                ok: o.status.success(),
                output: s,
            }
        }
        Err(e) => ToolResult {
            ok: false,
            output: e.to_string(),
        },
    }
}

/// `plan_mode`: only Read + List allowed.
pub fn run_tool(tool: Tool, plan_mode: bool) -> ToolResult {
    match tool {
        Tool::Bash { cmd } => {
            if plan_mode {
                return ToolResult {
                    ok: false,
                    output: "plan mode: bash disabled".into(),
                };
            }
            run_bash_timed(&cmd, Duration::from_secs(BASH_TIMEOUT_SECS))
        }
        Tool::Read { path } => match std::fs::read_to_string(&path) {
            Ok(s) => {
                let output = if s.len() > 8000 {
                    format!("{}\n\u{2026}truncated", truncate_utf8(&s, 8000))
                } else {
                    s
                };
                ToolResult { ok: true, output }
            }
            Err(e) => ToolResult {
                ok: false,
                output: e.to_string(),
            },
        },
        Tool::Write { path, content } => {
            if plan_mode {
                return ToolResult {
                    ok: false,
                    output: "plan mode: write disabled".into(),
                };
            }
            match std::fs::write(&path, content) {
                Ok(()) => ToolResult {
                    ok: true,
                    output: format!("wrote {path}"),
                },
                Err(e) => ToolResult {
                    ok: false,
                    output: e.to_string(),
                },
            }
        }
        Tool::List { path } => {
            let p = if path.is_empty() { "." } else { &path };
            match std::fs::read_dir(p) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .flatten()
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            if e.path().is_dir() {
                                format!("{name}/")
                            } else {
                                name
                            }
                        })
                        .collect();
                    names.sort();
                    ToolResult {
                        ok: true,
                        output: names.join("\n"),
                    }
                }
                Err(e) => ToolResult {
                    ok: false,
                    output: e.to_string(),
                },
            }
        }
    }
}

/// Truncate `s` to at most `max_bytes` on a char boundary (never panics on UTF-8).
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Resolve `@path` mentions in user text into file contents for context.
pub fn expand_at_files(text: &str) -> String {
    let mut out = text.to_string();
    for token in text.split_whitespace() {
        if let Some(path) = token.strip_prefix('@')
            && Path::new(path).is_file()
            && let Ok(content) = std::fs::read_to_string(path)
        {
            let snippet = if content.len() > 4000 {
                format!("{}\u{2026}", truncate_utf8(&content, 4000))
            } else {
                content
            };
            out.push_str(&format!("\n\n--- @{path} ---\n{snippet}\n"));
        }
    }
    out
}

/// Write a minimal AGENTS.md for /init.
pub fn write_agents_md(dir: &str) -> Result<String, String> {
    let path = Path::new(dir).join("AGENTS.md");
    let body = r#"# AGENTS.md

Project context for plazir18 / coding agents.

## Build
```bash
cargo build --features agent
cargo run --features agent -- --agent
```

## Conventions
- Minimal clutter. Prefer action over explanation.
- Titanium defaults available: unrestricted tools when mode=build.
"#;
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn plan_mode_blocks_bash() {
        let r = run_tool(
            Tool::Bash {
                cmd: "echo hi".into(),
            },
            true,
        );
        assert!(!r.ok);
        assert!(r.output.contains("plan mode"));
    }

    #[test]
    fn bash_runs_when_not_plan() {
        let r = run_tool(
            Tool::Bash {
                cmd: "printf 'ok'".into(),
            },
            false,
        );
        assert!(r.ok);
        assert_eq!(r.output, "ok");
    }

    #[test]
    fn bash_timeout_kills_hanging_command() {
        // Short timeout so the unit suite stays fast.
        let r = run_bash_timed("sleep 60", Duration::from_millis(400));
        assert!(!r.ok, "timeout must fail: {}", r.output);
        assert!(
            r.output.contains("timed out"),
            "expected timeout message, got: {}",
            r.output
        );
    }

    #[test]
    fn expand_at_files_injects_content() {
        let dir = std::env::temp_dir().join(format!("plazir18-at-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("snippet.txt");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "hello-at").unwrap();
        }
        let path_s = path.to_string_lossy();
        let out = expand_at_files(&format!("see @{path_s} please"));
        assert!(out.contains("hello-at"), "{out}");
        assert!(out.contains(&format!("--- @{path_s} ---")), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expand_at_files_multibyte_truncate_no_panic() {
        // Multi-byte UTF-8 (é = 2 bytes) so byte 4000 lands mid-char if sliced naively.
        let dir = std::env::temp_dir().join(format!("plazir18-at-mb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("big.txt");
        let body: String = "é".repeat(2500); // 5000 bytes
        assert!(body.len() > 4000);
        std::fs::write(&path, &body).unwrap();
        let path_s = path.to_string_lossy();
        let out = expand_at_files(&format!("@{path_s}"));
        assert!(out.contains('\u{2026}'), "{out}");
        assert!(out.contains("--- @"), "{out}");
        // Snippet before ellipsis must be valid UTF-8 (already is as String) and shorter.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncate_utf8_respects_char_boundary() {
        let s = "é".repeat(10); // 20 bytes
        let t = truncate_utf8(&s, 5); // would split mid-char at 5
        assert!(t.len() <= 5);
        assert!(s.is_char_boundary(t.len()));
        assert_eq!(t, "éé"); // 4 bytes
    }

    #[test]
    fn tool_output_cap_8000_is_utf8_safe() {
        // odd max would mid-split multi-byte if String::truncate were used
        let body: String = "é".repeat(5000); // 10000 bytes
        assert!(body.len() > 8000);
        let head = truncate_utf8(&body, 8000);
        assert!(head.len() <= 8000);
        assert!(body.is_char_boundary(head.len()));
        let capped = format!("{head}\n\u{2026}truncated");
        assert!(capped.contains('\u{2026}'));
        assert!(capped.is_char_boundary(capped.len()));
    }

    #[test]
    fn write_agents_md_creates_file() {
        let dir = std::env::temp_dir().join(format!("plazir18-agents-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let written = write_agents_md(dir.to_str().unwrap()).unwrap();
        assert!(Path::new(&written).is_file());
        let body = std::fs::read_to_string(&written).unwrap();
        assert!(body.contains("AGENTS.md"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
