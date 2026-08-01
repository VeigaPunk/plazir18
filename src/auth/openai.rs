//! ChatGPT / Codex OAuth (device-code + PKCE path). Feature = "oauth".

use super::{AuthProvider, Credential, ProviderId};

pub struct OpenAiAuth;

/// True when URL targets loopback (Ollama / local stack) — not remote OpenAI cloud.
pub fn is_loopback_base(base: &str) -> bool {
    let b = base.to_ascii_lowercase();
    b.contains("127.0.0.1") || b.contains("localhost") || b.contains("[::1]")
}

/// Pure OpenAI env → credential (no process env). Empty key after trim → Err.
/// Loopback OPENAI_BASE_URL → Err so `/connect` can fall through to Local.
pub fn resolve_openai_login(
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Credential, String> {
    let key = api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "set OPENAI_API_KEY, or run full OAuth (feature=oauth device-code) \u{2014} visit https://auth.openai.com"
                .to_string()
        })?;

    let base = match base_url.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) if is_loopback_base(b) => {
            return Err(
                "use PLAZIR_LOCAL_BASE for Ollama; OPENAI_BASE_URL is for remote OpenAI-compatible cloud"
                    .into(),
            );
        }
        Some(b) => b.trim_end_matches('/').to_string(),
        None => "https://api.openai.com/v1".into(),
    };

    Ok(Credential {
        api_key: Some(key.to_string()),
        base_url: Some(base),
        ..Default::default()
    })
}

impl AuthProvider for OpenAiAuth {
    fn id(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn display_name(&self) -> &str {
        "ChatGPT / Codex"
    }

    fn login(&self) -> Result<Credential, String> {
        // Key path first. Browser PKCE scaffold lives in `auth::pkce` (feature oauth);
        // full device-code exchange is still pending (M10).
        resolve_openai_login(
            std::env::var("OPENAI_API_KEY").ok().as_deref(),
            std::env::var("OPENAI_BASE_URL").ok().as_deref(),
        )
    }

    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.access_token
            .as_ref()
            .or(cred.api_key.as_ref())
            .map(|t| format!("Bearer {t}"))
    }
}

/// Default loopback redirect used by `/oauth` (must match token exchange).
#[cfg(feature = "oauth")]
pub const OPENAI_LOOPBACK_REDIRECT: &str = "http://127.0.0.1:1455/auth/callback";

/// Start browser OAuth PKCE: returns (authorize_url, code_verifier, state).
#[cfg(feature = "oauth")]
pub fn openai_browser_oauth_start(redirect_uri: &str) -> (String, String, String) {
    let verifier = super::pkce::generate_code_verifier();
    let challenge = super::pkce::code_challenge_s256(&verifier);
    let state = super::pkce::generate_state();
    let url = super::pkce::openai_authorize_url(
        redirect_uri,
        &state,
        &challenge,
        "openid profile email offline_access",
    );
    (url, verifier, state)
}

/// xAI authorize host constant (full URL when client_id is known).
#[cfg(feature = "oauth")]
pub fn xai_authorize_url_hint() -> &'static str {
    super::pkce::XAI_AUTHORIZE_URL
}

/// Start xAI browser PKCE when `PLAZIR_XAI_OAUTH_CLIENT_ID` is set.
/// Returns (authorize_url, code_verifier, state).
#[cfg(feature = "oauth")]
pub fn xai_browser_oauth_start(client_id: &str, redirect_uri: &str) -> (String, String, String) {
    let verifier = super::pkce::generate_code_verifier();
    let challenge = super::pkce::code_challenge_s256(&verifier);
    let state = super::pkce::generate_state();
    let url = super::pkce::xai_authorize_url(
        client_id,
        redirect_uri,
        &state,
        &challenge,
        "openid offline_access",
    );
    (url, verifier, state)
}

/// Generic authorization_code → tokens POST.
#[cfg(feature = "oauth")]
pub fn oauth_exchange_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
    api_base: &str,
) -> Result<super::Credential, String> {
    let body = super::pkce::oauth_token_exchange_body(client_id, code, redirect_uri, code_verifier);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(token_url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token exchange {status}: {text}"));
    }
    let tokens = super::pkce::parse_oauth_token_json(&text)?;
    Ok(super::pkce::credential_from_oauth_tokens(&tokens, api_base))
}

/// POST authorization_code → tokens (OpenAI). Used by `/oauth-code`.
#[cfg(feature = "oauth")]
pub fn openai_exchange_code(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<super::Credential, String> {
    oauth_exchange_code(
        super::pkce::OPENAI_TOKEN_URL,
        super::pkce::OPENAI_OAUTH_CLIENT_ID,
        code,
        redirect_uri,
        code_verifier,
        "https://api.openai.com/v1",
    )
}

/// POST authorization_code → tokens (xAI). Client id from env/start.
#[cfg(feature = "oauth")]
pub fn xai_exchange_code(
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<super::Credential, String> {
    let token_url = std::env::var("PLAZIR_XAI_TOKEN_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| super::pkce::XAI_TOKEN_URL.to_string());
    oauth_exchange_code(
        &token_url,
        client_id,
        code,
        redirect_uri,
        code_verifier,
        "https://api.x.ai/v1",
    )
}

/// POST refresh_token → tokens (network).
#[cfg(feature = "oauth")]
pub fn openai_refresh_access_token(refresh_token: &str) -> Result<super::Credential, String> {
    let body = super::pkce::openai_refresh_body(refresh_token);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(super::pkce::OPENAI_TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token refresh {status}: {text}"));
    }
    let tokens = super::pkce::parse_oauth_token_json(&text)?;
    Ok(super::pkce::credential_from_oauth_tokens(
        &tokens,
        "https://api.openai.com/v1",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cloud_base() {
        let c = resolve_openai_login(Some("sk-test"), None).unwrap();
        assert_eq!(c.api_key.as_deref(), Some("sk-test"));
        assert_eq!(c.base_url.as_deref(), Some("https://api.openai.com/v1"));
    }

    #[test]
    fn custom_remote_base_trims_slash() {
        let c = resolve_openai_login(Some("sk"), Some("https://example.com/v1/")).unwrap();
        assert_eq!(c.base_url.as_deref(), Some("https://example.com/v1"));
    }

    #[test]
    fn loopback_base_errors_for_local_path() {
        let e = resolve_openai_login(Some("sk"), Some("http://127.0.0.1:11434/v1")).unwrap_err();
        assert!(e.contains("PLAZIR_LOCAL_BASE"), "{e}");
    }

    #[test]
    fn localhost_base_errors() {
        let e = resolve_openai_login(Some("sk"), Some("http://localhost:8080/v1")).unwrap_err();
        assert!(e.contains("PLAZIR_LOCAL_BASE"), "{e}");
    }

    #[test]
    fn empty_key_errs() {
        assert!(resolve_openai_login(Some("   "), None).is_err());
        assert!(resolve_openai_login(None, None).is_err());
    }
}
