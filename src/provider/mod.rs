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

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(String),
    #[error("no credential for provider")]
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
        let parsed: ChatResponse = resp
            .json()
            .map_err(|e| ProviderError::Other(e.to_string()))?;
        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or(ProviderError::Empty)
    }
}

/// Prefer local if present, then zen, then others.
pub fn pick_default(
    creds: &std::collections::HashMap<ProviderId, Credential>,
) -> Option<(ProviderId, Credential)> {
    for id in [
        ProviderId::Local,
        ProviderId::Zen,
        ProviderId::Openai,
        ProviderId::Xai,
    ] {
        if let Some(c) = creds.get(&id) {
            return Some((id, c.clone()));
        }
    }
    None
}
