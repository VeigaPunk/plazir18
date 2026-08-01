//! Minimal tool set: bash + read/write file. Plan mode is read-only.

use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub enum Tool {
    Bash { cmd: String },
    Read { path: String },
    Write { path: String, content: String },
    List { path: String },
}

#[derive(Debug)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
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
            match Command::new("bash").arg("-c").arg(&cmd).output() {
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
                        s.truncate(8000);
                        s.push_str("\n\u2026truncated");
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
        Tool::Read { path } => match std::fs::read_to_string(&path) {
            Ok(mut s) => {
                if s.len() > 8000 {
                    s.truncate(8000);
                    s.push_str("\n\u2026truncated");
                }
                ToolResult { ok: true, output: s }
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

/// Resolve `@path` mentions in user text into file contents for context.
pub fn expand_at_files(text: &str) -> String {
    let mut out = text.to_string();
    for token in text.split_whitespace() {
        if let Some(path) = token.strip_prefix('@') {
            if Path::new(path).is_file() {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let snippet = if content.len() > 4000 {
                        format!("{}\u2026", &content[..4000])
                    } else {
                        content
                    };
                    out.push_str(&format!("\n\n--- @{path} ---\n{snippet}\n"));
                }
            }
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
