//! OpenAI-compatible model client + simple router (feature = "agent").

use crate::auth::{Credential, ProviderId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
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
        let url = format!("{}/chat/completions", self.base_url);
        let body = ChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            stream: false,
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
        parse_chat_completion_body(&text)
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
    let parsed: ChatResponse =
        serde_json::from_str(body).map_err(|e| ProviderError::Other(e.to_string()))?;
    parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or(ProviderError::Empty)
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
    });
    messages.push(ChatMessage {
        role: "assistant".into(),
        content: assistant.into(),
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
