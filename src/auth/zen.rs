//! OpenCode Zen gateway auth (API key after https://opencode.ai/auth).

use super::{AuthProvider, Credential, ProviderId};

pub struct ZenAuth;

impl AuthProvider for ZenAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Zen
    }

    fn display_name(&self) -> &str {
        "OpenCode Zen"
    }

    fn login(&self) -> Result<Credential, String> {
        let key = std::env::var("OPENCODE_ZEN_API_KEY")
            .or_else(|_| std::env::var("PLAZIR_ZEN_KEY"))
            .map_err(|_| {
                "set OPENCODE_ZEN_API_KEY or use /connect (visit https://opencode.ai/auth)"
                    .to_string()
            })?;
        Ok(Credential {
            api_key: Some(key),
            base_url: Some("https://opencode.ai/zen/v1".into()),
            ..Default::default()
        })
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.api_key.as_ref().map(|k| format!("Bearer {k}"))
    }
}
