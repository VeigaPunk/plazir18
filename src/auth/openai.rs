//! ChatGPT / Codex OAuth (device-code + PKCE path). Feature = "oauth".

use super::{AuthProvider, Credential, ProviderId};

pub struct OpenAiAuth;

impl AuthProvider for OpenAiAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn display_name(&self) -> &str {
        "ChatGPT / Codex"
    }

    fn login(&self) -> Result<Credential, String> {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            return Ok(Credential {
                api_key: Some(key),
                base_url: Some("https://api.openai.com/v1".into()),
                ..Default::default()
            });
        }
        Err(
            "set OPENAI_API_KEY, or run full OAuth (feature=oauth device-code) \u2014 visit https://auth.openai.com"
                .into(),
        )
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.access_token
            .as_ref()
            .or(cred.api_key.as_ref())
            .map(|t| format!("Bearer {t}"))
    }
}
