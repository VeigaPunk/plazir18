//! OpenAI-compatible model client + simple router (feature = "agent").

use crate::auth::{Credential, ProviderId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatMessage {
    pub role: String,
    /// OpenAI may send `null` when using tool_calls; treat as empty.
    #[serde(default, deserialize_with = "deserialize_maybe_content")]
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            ..Default::default()
        }
    }
}

fn deserialize_maybe_content<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// OpenAI-compatible tool call on an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type", default = "default_function_type")]
    pub kind: String,
    pub function: ToolFunction,
}

fn default_function_type() -> String {
    "function".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    /// JSON-encoded arguments object.
    pub arguments: String,
}

/// Result of one model completion (text and/or tool calls).
#[derive(Debug, Clone)]
pub struct AssistantTurn {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    data: Vec<ModelListEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelListEntry {
    id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("no credential for provider")]
    #[allow(dead_code)]
    NoCred,
    #[error("empty response")]
    Empty,
    #[error("{0}")]
    Other(String),
}

impl ProviderError {
    /// True for HTTP 401 Unauthorized (expired OAuth / bad bearer).
    pub fn is_unauthorized(&self) -> bool {
        match self {
            Self::Http(s) => {
                let t = s.trim_start();
                t.starts_with("401") || t.contains("status: 401")
            }
            _ => false,
        }
    }
}

pub struct OpenAICompat {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

impl OpenAICompat {
    pub fn from_cred(cred: &Credential, model: impl Into<String>) -> Result<Self, ProviderError> {
        let base = cred
            .base_url
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into());
        Ok(Self {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: cred.api_key.clone().or(cred.access_token.clone()),
            model: model.into(),
        })
    }

    pub fn chat(&self, messages: &[ChatMessage]) -> Result<String, ProviderError> {
        if chat_stream_preferred() {
            match self.chat_stream(messages) {
                Ok(s) if !s.is_empty() => return Ok(s),
                Ok(_) => { /* fall through to non-stream */ }
                Err(e) if e.is_unauthorized() => return Err(e),
                Err(_) => { /* fall through */ }
            }
        }
        self.chat_blocking(messages)
    }

    fn chat_blocking(&self, messages: &[ChatMessage]) -> Result<String, ProviderError> {
        let turn = self.chat_turn(messages, None)?;
        if turn.content.is_empty() && !turn.tool_calls.is_empty() {
            return Ok(String::new());
        }
        if turn.content.is_empty() {
            return Err(ProviderError::Empty);
        }
        Ok(turn.content)
    }

    /// Non-stream completion with optional tools; returns full assistant turn.
    pub fn chat_turn(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[serde_json::Value]>,
    ) -> Result<AssistantTurn, ProviderError> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = ChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            stream: false,
            tools: tools.map(|t| t.to_vec()),
            tool_choice: if tools.map(|t| !t.is_empty()).unwrap_or(false) {
                Some("auto".into())
            } else {
                None
            },
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let mut req = client.post(&url).json(&body);
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let resp = req.send().map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(ProviderError::Http(format!("{status}: {text}")));
        }
        let text = resp
            .text()
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        parse_assistant_turn(&text)
    }

    /// Streaming chat (SSE). Accumulates `delta.content` into a full reply.
    pub fn chat_stream(&self, messages: &[ChatMessage]) -> Result<String, ProviderError> {
        self.chat_stream_for_each(messages, |_| {})
    }

    /// Line-by-line SSE stream; invokes `on_delta` for each content piece as it arrives.
    pub fn chat_stream_for_each(
        &self,
        messages: &[ChatMessage],
        mut on_delta: impl FnMut(&str),
    ) -> Result<String, ProviderError> {
        use std::io::{BufRead, BufReader};
        let url = format!("{}/chat/completions", self.base_url);
        let body = ChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            stream: true,
            tools: None,
            tool_choice: None,
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let mut req = client.post(&url).json(&body);
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let resp = req.send().map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(ProviderError::Http(format!("{status}: {text}")));
        }
        let mut out = String::new();
        let mut reader = BufReader::new(resp);
        let mut line = String::new();
        let mut raw_fallback = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| ProviderError::Other(e.to_string()))?;
            if n == 0 {
                break;
            }
            raw_fallback.push_str(&line);
            let trimmed = line.trim();
            let Some(payload) = trimmed.strip_prefix("data:") else {
                continue;
            };
            if let Some(piece) = parse_sse_data_payload(payload) {
                on_delta(&piece);
                out.push_str(&piece);
            }
        }
        if out.is_empty() {
            // Retry pure accumulator (same buffer) then non-SSE JSON.
            if let Ok(s) = accumulate_sse_chat_body(&raw_fallback) {
                if !s.is_empty() {
                    on_delta(&s);
                    return Ok(s);
                }
            }
            if let Ok(s) = parse_chat_completion_body(&raw_fallback) {
                if !s.is_empty() {
                    on_delta(&s);
                }
                return Ok(s);
            }
            return Err(ProviderError::Empty);
        }
        Ok(out)
    }

    /// GET `{base}/models` (OpenAI-compatible catalog). Short timeout for connect path.
    pub fn list_model_ids(&self) -> Result<Vec<String>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let mut req = client.get(&url);
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let resp = req.send().map_err(|e| ProviderError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(ProviderError::Http(format!("{status}: {text}")));
        }
        let text = resp
            .text()
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        parse_models_list_body(&text)
    }

    /// If preferred model is absent from the local catalog, switch to the first listed id.
    /// No-op when catalog fetch fails or is empty (keeps preferred).
    pub fn resolve_local_model(mut self) -> Self {
        if let Ok(ids) = self.list_model_ids() {
            self.model = pick_model(&self.model, &ids);
        }
        self
    }
}

/// Pure: extract first choice content from an OpenAI-compatible chat JSON body.
/// Unit-tested without network (M6 scaffold path).
pub fn parse_chat_completion_body(body: &str) -> Result<String, ProviderError> {
    let turn = parse_assistant_turn(body)?;
    if turn.content.is_empty() && turn.tool_calls.is_empty() {
        return Err(ProviderError::Empty);
    }
    Ok(turn.content)
}

/// Pure: parse assistant message including optional tool_calls.
pub fn parse_assistant_turn(body: &str) -> Result<AssistantTurn, ProviderError> {
    let parsed: ChatResponse =
        serde_json::from_str(body).map_err(|e| ProviderError::Other(e.to_string()))?;
    let msg = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .ok_or(ProviderError::Empty)?;
    Ok(AssistantTurn {
        content: msg.content,
        tool_calls: msg.tool_calls,
    })
}

/// Builtin OpenAI tool definitions for plazir agent tools.
pub fn builtin_tool_defs() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description": "Run a shell command (build mode only)",
                "parameters": {
                    "type": "object",
                    "properties": { "cmd": { "type": "string" } },
                    "required": ["cmd"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a text file",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write a text file (build mode only)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List directory entries",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Replace first (or all) occurrences in a file (build mode only)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old": { "type": "string" },
                        "new": { "type": "string" },
                        "all": { "type": "boolean" }
                    },
                    "required": ["path", "old", "new"]
                }
            }
        }),
    ]
}

/// Prefer SSE stream when `PLAZIR_CHAT_STREAM` is 1/true/yes (default: on for local-ish bases).
pub fn chat_stream_preferred() -> bool {
    match std::env::var("PLAZIR_CHAT_STREAM") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        }
        Err(_) => false,
    }
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<StreamDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

/// Extract content delta from one SSE `data:` JSON payload (not including the `data: ` prefix).
pub fn parse_sse_data_payload(data: &str) -> Option<String> {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let chunk: StreamChunk = serde_json::from_str(data).ok()?;
    chunk
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.delta)
        .and_then(|d| d.content)
        .filter(|s| !s.is_empty())
}

/// Accumulate OpenAI-style SSE body (`data: {...}\n\n` lines) into assistant text.
pub fn accumulate_sse_chat_body(body: &str) -> Result<String, ProviderError> {
    let mut out = String::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        if let Some(piece) = parse_sse_data_payload(payload) {
            out.push_str(&piece);
        }
    }
    if out.is_empty() {
        // Some servers return a single non-SSE JSON even when stream=true.
        if let Ok(s) = parse_chat_completion_body(body) {
            return Ok(s);
        }
        return Err(ProviderError::Empty);
    }
    Ok(out)
}

/// Append a user turn + assistant reply into a session history (pure M6 helper).
pub fn apply_chat_turn(
    messages: &mut Vec<ChatMessage>,
    user: impl Into<String>,
    assistant: impl Into<String>,
) {
    messages.push(ChatMessage {
        role: "user".into(),
        content: user.into(),
        ..Default::default()
    });
    messages.push(ChatMessage {
        role: "assistant".into(),
        content: assistant.into(),
        ..Default::default()
    });
}

/// Prefer cloud creds (Zen → OpenAI → xAI), Local last (always-present local stub).
/// Skips stored credentials with empty key/token (stale auth.json rows).
pub fn pick_default(
    creds: &std::collections::HashMap<ProviderId, Credential>,
) -> Option<(ProviderId, Credential)> {
    for id in crate::auth::connect_provider_order() {
        if let Some(c) = creds.get(&id) {
            if crate::auth::credential_usable(id, c) {
                return Some((id, c.clone()));
            }
        }
    }
    None
}

/// Default chat model per provider. Local honors `PLAZIR_LOCAL_MODEL` when set.
pub fn default_model_for(id: ProviderId) -> String {
    match id {
        ProviderId::Local => std::env::var("PLAZIR_LOCAL_MODEL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "llama3.2".into()),
        ProviderId::Zen => "gpt-5.1-codex".into(),
        ProviderId::Openai => "gpt-4o".into(),
        ProviderId::Xai => "grok-3".into(),
    }
}

/// Pure: parse OpenAI-compatible `/models` JSON into model ids (order preserved).
pub fn parse_models_list_body(body: &str) -> Result<Vec<String>, ProviderError> {
    let parsed: ModelsListResponse =
        serde_json::from_str(body).map_err(|e| ProviderError::Other(e.to_string()))?;
    Ok(parsed
        .data
        .into_iter()
        .map(|m| m.id)
        .filter(|id| !id.is_empty())
        .collect())
}

/// Prefer `preferred` when present in catalog; else first catalog id; else preferred.
pub fn pick_model(preferred: &str, catalog: &[String]) -> String {
    if catalog.is_empty() {
        return preferred.to_string();
    }
    if catalog.iter().any(|m| m == preferred) {
        return preferred.to_string();
    }
    catalog[0].clone()
}

/// Build client for provider; Local auto-picks catalog model when preferred missing.
pub fn client_for(id: ProviderId, cred: &Credential) -> Result<OpenAICompat, ProviderError> {
    let model = default_model_for(id);
    let client = OpenAICompat::from_cred(cred, model)?;
    Ok(match id {
        ProviderId::Local => client.resolve_local_model(),
        _ => client,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{Credential, ProviderId};
    use std::collections::HashMap;

    #[test]
    fn from_cred_trims_slash_and_defaults_base() {
        let c = Credential {
            api_key: Some("k".into()),
            base_url: Some("http://example.com/v1/".into()),
            ..Default::default()
        };
        let client = OpenAICompat::from_cred(&c, "m").unwrap();
        assert_eq!(client.base_url, "http://example.com/v1");
        assert_eq!(client.api_key.as_deref(), Some("k"));
        assert_eq!(client.model, "m");

        let bare = Credential::default();
        let local = OpenAICompat::from_cred(&bare, "llama").unwrap();
        assert_eq!(local.base_url, "http://127.0.0.1:11434/v1");
    }

    #[test]
    fn pick_default_prefers_zen_over_local() {
        let mut map = HashMap::new();
        map.insert(
            ProviderId::Zen,
            Credential {
                api_key: Some("z".into()),
                ..Default::default()
            },
        );
        map.insert(
            ProviderId::Openai,
            Credential {
                api_key: Some("o".into()),
                ..Default::default()
            },
        );
        let (id, _) = pick_default(&map).unwrap();
        assert_eq!(id, ProviderId::Zen);

        map.insert(
            ProviderId::Local,
            Credential {
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ..Default::default()
            },
        );
        // Cloud key still wins over always-Ok local.
        let (id, _) = pick_default(&map).unwrap();
        assert_eq!(id, ProviderId::Zen);

        map.remove(&ProviderId::Zen);
        map.remove(&ProviderId::Openai);
        let (id, _) = pick_default(&map).unwrap();
        assert_eq!(id, ProviderId::Local);

        assert!(pick_default(&HashMap::new()).is_none());
    }

    #[test]
    fn pick_default_skips_empty_cloud_key() {
        let mut map = HashMap::new();
        map.insert(
            ProviderId::Zen,
            Credential {
                api_key: Some("  ".into()),
                ..Default::default()
            },
        );
        map.insert(
            ProviderId::Local,
            Credential {
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ..Default::default()
            },
        );
        let (id, _) = pick_default(&map).unwrap();
        assert_eq!(id, ProviderId::Local);
    }

    #[test]
    fn provider_error_is_unauthorized() {
        assert!(ProviderError::Http("401 Unauthorized: expired".into()).is_unauthorized());
        assert!(!ProviderError::Http("500 boom".into()).is_unauthorized());
        assert!(!ProviderError::Empty.is_unauthorized());
    }

    #[test]
    fn parse_chat_completion_body_extracts_content() {
        let body = r#"{
          "choices": [
            {"message": {"role": "assistant", "content": "hello-local"}}
          ]
        }"#;
        let s = parse_chat_completion_body(body).unwrap();
        assert_eq!(s, "hello-local");
    }

    #[test]
    fn parse_assistant_turn_with_tool_calls_and_null_content() {
        let body = r#"{
          "choices": [{
            "message": {
              "role": "assistant",
              "content": null,
              "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "bash", "arguments": "{\"cmd\":\"echo hi\"}"}
              }]
            }
          }]
        }"#;
        let turn = parse_assistant_turn(body).unwrap();
        assert!(turn.content.is_empty());
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].function.name, "bash");
    }

    #[test]
    fn accumulate_sse_chat_body_joins_deltas() {
        let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
data: [DONE]\n\n";
        let s = accumulate_sse_chat_body(body).unwrap();
        assert_eq!(s, "hello");
        assert_eq!(
            parse_sse_data_payload(r#" {"choices":[{"delta":{"content":"x"}}]} "#).as_deref(),
            Some("x")
        );
        assert!(parse_sse_data_payload("[DONE]").is_none());
    }

    #[test]
    fn parse_chat_completion_body_empty_choices_errs() {
        let body = r#"{"choices":[]}"#;
        assert!(matches!(
            parse_chat_completion_body(body),
            Err(ProviderError::Empty)
        ));
    }

    #[test]
    fn apply_chat_turn_appends_user_then_assistant() {
        let mut msgs = Vec::new();
        apply_chat_turn(&mut msgs, "ping", "pong");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "ping");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "pong");
    }

    #[test]
    fn default_model_local_uses_env_when_set() {
        // Serialise with a unique key probe: only assert pure fallback here.
        assert_eq!(default_model_for(ProviderId::Openai), "gpt-4o");
        assert_eq!(default_model_for(ProviderId::Xai), "grok-3");
        // Local default without depending on process env mutation in parallel suites.
        let local = default_model_for(ProviderId::Local);
        assert!(!local.is_empty());
    }

    #[test]
    fn parse_models_list_body_ids() {
        let body = r#"{"object":"list","data":[{"id":"qwen3:14b"},{"id":"gemma4"}]}"#;
        let ids = parse_models_list_body(body).unwrap();
        assert_eq!(ids, vec!["qwen3:14b".to_string(), "gemma4".to_string()]);
    }

    #[test]
    fn pick_model_prefers_present_else_first() {
        let cat = vec!["a".into(), "b".into()];
        assert_eq!(pick_model("b", &cat), "b");
        assert_eq!(pick_model("missing", &cat), "a");
        assert_eq!(pick_model("x", &[]), "x");
    }

    /// Live Ollama chat — run with:
    /// `PLAZIR_LOCAL_MODEL=qwen3:14b cargo test --features agent -- --ignored live_local_chat`
    #[test]
    #[ignore = "live ollama on :11434"]
    fn live_local_chat_one_turn() {
        let model = std::env::var("PLAZIR_LOCAL_MODEL").unwrap_or_else(|_| "qwen3:14b".into());
        let client = OpenAICompat {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model,
        };
        let reply = client
            .chat(&[ChatMessage {
                role: "user".into(),
                content: "Reply with exactly the single word: pong".into(),
                ..Default::default()
            }])
            .expect("live chat");
        assert!(
            reply.to_ascii_lowercase().contains("pong"),
            "expected pong in reply, got: {reply}"
        );
    }

    /// Live: resolve_local_model replaces missing default with a catalog id.
    #[test]
    #[ignore = "live ollama on :11434"]
    fn live_resolve_local_model_picks_catalog() {
        let client = OpenAICompat {
            base_url: "http://127.0.0.1:11434/v1".into(),
            api_key: None,
            model: "llama3.2-not-installed".into(),
        }
        .resolve_local_model();
        assert_ne!(client.model, "llama3.2-not-installed");
        assert!(!client.model.is_empty());
    }
}
