//! Grok / xAI SuperGrok auth. Feature = "oauth".

use super::{AuthProvider, Credential, ProviderId};

pub struct XaiAuth;

impl AuthProvider for XaiAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Xai
    }

    fn display_name(&self) -> &str {
        "Grok / xAI"
    }

    fn login(&self) -> Result<Credential, String> {
        if let Ok(key) = std::env::var("XAI_API_KEY") {
            return Ok(Credential {
                api_key: Some(key),
                base_url: Some("https://api.x.ai/v1".into()),
                ..Default::default()
            });
        }
        if let Ok(key) = std::env::var("GROK_API_KEY") {
            return Ok(Credential {
                api_key: Some(key),
                base_url: Some("https://api.x.ai/v1".into()),
                ..Default::default()
            });
        }
        Err(
            "set XAI_API_KEY (or GROK_API_KEY), or complete browser PKCE at accounts.x.ai"
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
