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

/// Dual-env: first non-empty after trim wins (OPENCODE then PLAZIR).
/// Empty `OPENCODE_ZEN_API_KEY=""` must not shadow a real `PLAZIR_ZEN_KEY`.
pub fn resolve_zen_login_dual(
    opencode_zen: Option<&str>,
    plazir_zen: Option<&str>,
) -> Result<Credential, String> {
    let key = first_nonempty_trim(opencode_zen).or_else(|| first_nonempty_trim(plazir_zen));
    resolve_zen_login(key)
}

fn first_nonempty_trim(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

impl AuthProvider for ZenAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Zen
    }

    fn display_name(&self) -> &str {
        "OpenCode Zen"
    }

    fn login(&self) -> Result<Credential, String> {
        resolve_zen_login_dual(
            std::env::var("OPENCODE_ZEN_API_KEY").ok().as_deref(),
            std::env::var("PLAZIR_ZEN_KEY").ok().as_deref(),
        )
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

    #[test]
    fn dual_empty_opencode_does_not_shadow_plazir() {
        let c = resolve_zen_login_dual(Some("  "), Some("  plazir-key  ")).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("plazir-key"));
    }

    #[test]
    fn dual_opencode_wins_when_nonempty() {
        let c = resolve_zen_login_dual(Some("oc"), Some("plazir")).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("oc"));
    }

    #[test]
    fn dual_both_empty_errs() {
        assert!(resolve_zen_login_dual(Some(""), Some("  ")).is_err());
        assert!(resolve_zen_login_dual(None, None).is_err());
    }
}
