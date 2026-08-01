//! Local OpenAI-compatible endpoints (Ollama, vLLM, LM Studio, …).

use super::{AuthProvider, Credential, ProviderId};

#[derive(Debug, Clone)]
pub struct LocalAuth {
    pub default_base: String,
}

impl Default for LocalAuth {
    fn default() -> Self {
        Self {
            default_base: "http://127.0.0.1:11434/v1".into(),
        }
    }
}

impl AuthProvider for LocalAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Local
    }

    fn display_name(&self) -> &str {
        "Local (OpenAI-compatible)"
    }

    fn login(&self) -> Result<Credential, String> {
        let base = std::env::var("PLAZIR_LOCAL_BASE")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| self.default_base.clone());
        let api_key = std::env::var("PLAZIR_LOCAL_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok();
        Ok(Credential {
            api_key,
            base_url: Some(base),
            ..Default::default()
        })
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.api_key.as_ref().map(|k| format!("Bearer {k}"))
    }
}
