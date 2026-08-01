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

/// Pure: resolve Local base.
/// 1. `PLAZIR_LOCAL_BASE` if non-empty
/// 2. Else `OPENAI_BASE_URL` if set and loopback (OpenAI rejects those — dual-env Ollama)
/// 3. Else `default_base` (typically `http://127.0.0.1:11434/v1`)
///
/// Never uses `OPENAI_API_KEY` for Local key — see [`resolve_local_key`].
pub fn resolve_local_base(
    plazir_local_base: Option<&str>,
    openai_base_url: Option<&str>,
    default_base: &str,
) -> String {
    if let Some(b) = plazir_local_base.map(str::trim).filter(|s| !s.is_empty()) {
        return b.to_string();
    }
    if let Some(b) = openai_base_url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| super::openai::is_loopback_base(s))
    {
        return b.to_string();
    }
    default_base.to_string()
}

/// Pure: resolve Local key from PLAZIR_LOCAL_KEY only (no OPENAI_API_KEY).
pub fn resolve_local_key(plazir_local_key: Option<&str>) -> Option<String> {
    plazir_local_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

impl AuthProvider for LocalAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Local
    }

    fn display_name(&self) -> &str {
        "Local (OpenAI-compatible)"
    }

    fn login(&self) -> Result<Credential, String> {
        let base = resolve_local_base(
            std::env::var("PLAZIR_LOCAL_BASE").ok().as_deref(),
            std::env::var("OPENAI_BASE_URL").ok().as_deref(),
            &self.default_base,
        );
        let api_key = resolve_local_key(std::env::var("PLAZIR_LOCAL_KEY").ok().as_deref());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_defaults_when_unset() {
        assert_eq!(
            resolve_local_base(None, None, "http://127.0.0.1:11434/v1"),
            "http://127.0.0.1:11434/v1"
        );
    }

    #[test]
    fn base_from_plazir_only() {
        assert_eq!(
            resolve_local_base(
                Some("http://127.0.0.1:8080/v1"),
                Some("http://127.0.0.1:1234/v1"),
                "http://default"
            ),
            "http://127.0.0.1:8080/v1"
        );
    }

    #[test]
    fn empty_base_falls_to_default() {
        assert_eq!(
            resolve_local_base(Some("  "), None, "http://def"),
            "http://def"
        );
    }

    #[test]
    fn loopback_openai_base_handed_to_local() {
        assert_eq!(
            resolve_local_base(None, Some("http://127.0.0.1:1234/v1"), "http://def"),
            "http://127.0.0.1:1234/v1"
        );
    }

    #[test]
    fn remote_openai_base_ignored() {
        assert_eq!(
            resolve_local_base(
                None,
                Some("https://api.openai.com/v1"),
                "http://127.0.0.1:11434/v1"
            ),
            "http://127.0.0.1:11434/v1"
        );
    }

    #[test]
    fn key_none_when_unset() {
        assert_eq!(resolve_local_key(None), None);
    }

    #[test]
    fn key_trimmed() {
        assert_eq!(
            resolve_local_key(Some("  sk-local  ")).as_deref(),
            Some("sk-local")
        );
    }
}
