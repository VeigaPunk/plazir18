//! OpenCode Zen gateway auth (API key after https://opencode.ai/auth).

use super::{AuthProvider, Credential, ProviderId};

pub struct ZenAuth;

/// Pure Zen env key → credential. Empty/whitespace after trim → Err so `/connect`
/// falls through (same class as OpenAI empty-key).
pub fn resolve_zen_login(api_key: Option<&str>) -> Result<Credential, String> {
    let key = api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "set OPENCODE_ZEN_API_KEY or use /connect (visit https://opencode.ai/auth)".to_string()
        })?;
    Ok(Credential {
        api_key: Some(key.to_string()),
        base_url: Some("https://opencode.ai/zen/v1".into()),
        ..Default::default()
    })
}

impl AuthProvider for ZenAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Zen
    }

    fn display_name(&self) -> &str {
        "OpenCode Zen"
    }

    fn login(&self) -> Result<Credential, String> {
        let from_env = std::env::var("OPENCODE_ZEN_API_KEY")
            .or_else(|_| std::env::var("PLAZIR_ZEN_KEY"))
            .ok();
        resolve_zen_login(from_env.as_deref())
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.api_key.as_ref().map(|k| format!("Bearer {k}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_key_ok() {
        let c = resolve_zen_login(Some("  zen-key  ")).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("zen-key"));
        assert_eq!(c.base_url.as_deref(), Some("https://opencode.ai/zen/v1"));
    }

    #[test]
    fn empty_key_errs() {
        assert!(resolve_zen_login(Some("   ")).is_err());
        assert!(resolve_zen_login(None).is_err());
        assert!(resolve_zen_login(Some("")).is_err());
    }
}
