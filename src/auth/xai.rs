//! Grok / xAI SuperGrok auth. Feature = "oauth".

use super::{AuthProvider, Credential, ProviderId};

pub struct XaiAuth;

/// Pure xAI/Grok key → credential. Empty/whitespace after trim → Err so `/connect`
/// falls through (same class as OpenAI empty-key).
pub fn resolve_xai_login(
    xai_key: Option<&str>,
    grok_key: Option<&str>,
) -> Result<Credential, String> {
    let key = xai_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| grok_key.map(str::trim).filter(|s| !s.is_empty()))
        .ok_or_else(|| {
            "set XAI_API_KEY (or GROK_API_KEY), or complete browser PKCE at accounts.x.ai"
                .to_string()
        })?;
    Ok(Credential {
        api_key: Some(key.to_string()),
        base_url: Some("https://api.x.ai/v1".into()),
        ..Default::default()
    })
}

impl AuthProvider for XaiAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Xai
    }

    fn display_name(&self) -> &str {
        "Grok / xAI"
    }

    fn login(&self) -> Result<Credential, String> {
        resolve_xai_login(
            std::env::var("XAI_API_KEY").ok().as_deref(),
            std::env::var("GROK_API_KEY").ok().as_deref(),
        )
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.access_token
            .as_ref()
            .or(cred.api_key.as_ref())
            .map(|t| format!("Bearer {t}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xai_key_preferred() {
        let c = resolve_xai_login(Some("  xai-k  "), Some("grok-k")).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("xai-k"));
        assert_eq!(c.base_url.as_deref(), Some("https://api.x.ai/v1"));
    }

    #[test]
    fn grok_key_fallback() {
        let c = resolve_xai_login(None, Some("grok-k")).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("grok-k"));
    }

    #[test]
    fn empty_keys_err() {
        assert!(resolve_xai_login(Some("  "), Some("")).is_err());
        assert!(resolve_xai_login(None, None).is_err());
    }
}
