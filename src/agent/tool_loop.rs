//! Model-driven tool loop: chat → tool_calls → execute → continue (max rounds).

use crate::agent::tools::{Tool, ToolResult, run_tool};
use crate::provider::{
    AssistantTurn, ChatMessage, OpenAICompat, ProviderError, ToolCall, builtin_tool_defs,
};

/// Max model↔tool rounds per user turn (prevents infinite loops).
pub const MAX_TOOL_ROUNDS: usize = 6;

/// Tool-loop policy from env + provider.
/// - `PLAZIR_TOOL_LOOP=1|true|yes|on` → always on  
/// - `PLAZIR_TOOL_LOOP=0|false|no|off` → always off  
/// - unset → **on for Local only** (safe default for Ollama tool-capable models)
pub fn tool_loop_enabled_for(provider: Option<crate::auth::ProviderId>) -> bool {
    match std::env::var("PLAZIR_TOOL_LOOP") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            if matches!(v.as_str(), "0" | "false" | "no" | "off") {
                return false;
            }
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => matches!(provider, Some(crate::auth::ProviderId::Local)),
    }
}

/// Dispatch a named tool from model JSON arguments.
pub fn dispatch_tool_call(name: &str, arguments_json: &str, plan_mode: bool) -> ToolResult {
    let args: serde_json::Value = match serde_json::from_str(arguments_json) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                ok: false,
                output: format!("invalid tool args JSON: {e}"),
            };
        }
    };
    let str_field = |k: &str| {
        args.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let tool = match name {
        "bash" | "shell" => Tool::Bash {
            cmd: str_field("cmd"),
        },
        "read_file" | "read" => Tool::Read {
            path: str_field("path"),
        },
        "write_file" | "write" => Tool::Write {
            path: str_field("path"),
            content: str_field("content"),
        },
        "list_dir" | "list" | "ls" => Tool::List {
            path: {
                let p = str_field("path");
                if p.is_empty() { ".".into() } else { p }
            },
        },
        "edit_file" | "edit" => Tool::Edit {
            path: str_field("path"),
            old: str_field("old"),
            new: str_field("new"),
            all: args.get("all").and_then(|v| v.as_bool()).unwrap_or(false),
        },
        other => {
            return ToolResult {
                ok: false,
                output: format!("unknown tool: {other}"),
            };
        }
    };
    run_tool(tool, plan_mode)
}

/// Append assistant tool_calls message + tool results to history.
pub fn append_tool_round(
    messages: &mut Vec<ChatMessage>,
    turn: &AssistantTurn,
    plan_mode: bool,
) -> Vec<String> {
    let mut notes = Vec::new();
    messages.push(ChatMessage {
        role: "assistant".into(),
        content: turn.content.clone(),
        tool_calls: turn.tool_calls.clone(),
        tool_call_id: None,
        name: None,
    });
    for tc in &turn.tool_calls {
        let result = dispatch_tool_call(&tc.function.name, &tc.function.arguments, plan_mode);
        let note = format!(
            "{} → {}",
            tc.function.name,
            if result.ok { "ok" } else { "err" }
        );
        notes.push(note);
        let mut out = result.output;
        if out.len() > 8000 {
            out.truncate(8000);
            out.push_str("\n…truncated");
        }
        messages.push(tool_result_message(tc, out));
    }
    notes
}

/// Pure helper: build a tool role message for a completed call.
pub fn tool_result_message(call: &ToolCall, output: impl Into<String>) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: output.into(),
        tool_calls: Vec::new(),
        tool_call_id: Some(call.id.clone()),
        name: Some(call.function.name.clone()),
    }
}

/// Run up to `MAX_TOOL_ROUNDS` of tool-using chat. Returns final assistant text.
pub fn run_tool_loop(
    client: &OpenAICompat,
    messages: &mut Vec<ChatMessage>,
    plan_mode: bool,
) -> Result<(String, Vec<String>), ProviderError> {
    let tools = builtin_tool_defs();
    let mut log = Vec::new();
    for _ in 0..MAX_TOOL_ROUNDS {
        let turn = client.chat_turn(messages, Some(&tools))?;
        if turn.tool_calls.is_empty() {
            if turn.content.is_empty() {
                return Err(ProviderError::Empty);
            }
            return Ok((turn.content, log));
        }
        let notes = append_tool_round(messages, &turn, plan_mode);
        log.extend(notes);
    }
    // Cap: last try without tools for a final answer.
    let final_turn = client.chat_turn(messages, None)?;
    if final_turn.content.is_empty() {
        Ok((
            format!(
                "(tool loop hit {MAX_TOOL_ROUNDS} rounds; no final text)\n{}",
                log.join("\n")
            ),
            log,
        ))
    } else {
        Ok((final_turn.content, log))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolFunction;

    #[test]
    fn tool_loop_default_local_when_env_unset() {
        // Cannot safely mutate process env under parallel tests; assert policy pure form:
        // with explicit env the helper is deterministic; unset path covered by match arms.
        assert!(!tool_loop_enabled_for(None));
        // Local default-on only when PLAZIR_TOOL_LOOP unset — if suite sets it, skip soft assert.
        if std::env::var("PLAZIR_TOOL_LOOP").is_err() {
            assert!(tool_loop_enabled_for(Some(crate::auth::ProviderId::Local)));
            assert!(!tool_loop_enabled_for(Some(
                crate::auth::ProviderId::Openai
            )));
        }
    }

    #[test]
    fn dispatch_bash_plan_blocked() {
        let r = dispatch_tool_call("bash", r#"{"cmd":"echo hi"}"#, true);
        assert!(!r.ok);
        assert!(r.output.contains("plan mode"));
    }

    #[test]
    fn dispatch_read_and_list() {
        let dir = std::env::temp_dir().join(format!("plazir18-tloop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.txt");
        std::fs::write(&path, "hello-tool").unwrap();
        let path_s = path.to_string_lossy();
        let r = dispatch_tool_call("read_file", &format!(r#"{{"path":"{path_s}"}}"#), false);
        assert!(r.ok, "{}", r.output);
        assert!(r.output.contains("hello-tool"));
        let list = dispatch_tool_call(
            "list_dir",
            &format!(r#"{{"path":"{}"}}"#, dir.to_string_lossy()),
            false,
        );
        assert!(list.ok, "{}", list.output);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_tool_round_shapes_messages() {
        let mut msgs = vec![ChatMessage::text("user", "list .")];
        let turn = AssistantTurn {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                kind: "function".into(),
                function: ToolFunction {
                    name: "list_dir".into(),
                    arguments: r#"{"path":"."}"#.into(),
                },
            }],
        };
        let notes = append_tool_round(&mut msgs, &turn, false);
        assert_eq!(notes.len(), 1);
        assert_eq!(msgs.len(), 3); // user + assistant + tool
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("call_1"));
    }
}
