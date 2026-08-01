//! Pure PKCE helpers (feature = "oauth"). No network.
//!
//! Scaffold until the agent TUI wires a full browser OAuth loop; keep helpers
//! public and clippy-clean under `--features full` without test-only linkage.

#![allow(dead_code)] // pure helpers are unit-tested; production connect still key-path first

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Unreserved charset for code_verifier (RFC 7636).
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Generate a high-entropy code_verifier (43–128 chars). Default length 64.
pub fn generate_code_verifier() -> String {
    generate_code_verifier_len(64)
}

pub fn generate_code_verifier_len(len: usize) -> String {
    let n = len.clamp(43, 128);
    let mut raw = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut raw);
    raw.into_iter()
        .map(|b| UNRESERVED[(b as usize) % UNRESERVED.len()] as char)
        .collect()
}

/// S256 code_challenge = BASE64URL-ENCODE(SHA256(ASCII(code_verifier))) without padding.
pub fn code_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Opaque OAuth `state` (32 url-safe chars).
pub fn generate_state() -> String {
    generate_code_verifier_len(32)
}

/// OpenAI / Codex public client id used by Codex CLI family (AGENT_PLAN).
pub const OPENAI_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Default authorize endpoint for ChatGPT browser OAuth.
pub const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";

/// Build an authorize URL with PKCE (query only; no secrets in log-friendly form).
pub fn openai_authorize_url(
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
    scope: &str,
) -> String {
    // Manual query to avoid pulling url builder complexity into pure path tests.
    let mut q = String::new();
    push_q(&mut q, "response_type", "code");
    push_q(&mut q, "client_id", OPENAI_OAUTH_CLIENT_ID);
    push_q(&mut q, "redirect_uri", redirect_uri);
    push_q(&mut q, "scope", scope);
    push_q(&mut q, "state", state);
    push_q(&mut q, "code_challenge", code_challenge);
    push_q(&mut q, "code_challenge_method", "S256");
    format!("{OPENAI_AUTHORIZE_URL}?{q}")
}

fn push_q(buf: &mut String, k: &str, v: &str) {
    if !buf.is_empty() {
        buf.push('&');
    }
    buf.push_str(&urlencoding_encode(k));
    buf.push('=');
    buf.push_str(&urlencoding_encode(v));
}

/// Minimal application/x-www-form-urlencoded encode (OAuth query).
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// xAI / SuperGrok authorize host (scaffold; full browser flow later).
pub const XAI_AUTHORIZE_URL: &str = "https://accounts.x.ai/oauth/authorize";

/// OpenAI token endpoint for authorization-code + PKCE exchange.
pub const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Default xAI token endpoint (override with `PLAZIR_XAI_TOKEN_URL`).
pub const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth/token";

/// Build a PKCE authorize URL against the xAI host (client_id supplied by caller).
pub fn xai_authorize_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
    scope: &str,
) -> String {
    let mut q = String::new();
    push_q(&mut q, "response_type", "code");
    push_q(&mut q, "client_id", client_id);
    push_q(&mut q, "redirect_uri", redirect_uri);
    push_q(&mut q, "scope", scope);
    push_q(&mut q, "state", state);
    push_q(&mut q, "code_challenge", code_challenge);
    push_q(&mut q, "code_challenge_method", "S256");
    format!("{XAI_AUTHORIZE_URL}?{q}")
}

/// application/x-www-form-urlencoded body for code→token exchange (PKCE).
pub fn oauth_token_exchange_body(
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> String {
    let mut body = String::new();
    push_q(&mut body, "grant_type", "authorization_code");
    push_q(&mut body, "client_id", client_id);
    push_q(&mut body, "code", code);
    push_q(&mut body, "redirect_uri", redirect_uri);
    push_q(&mut body, "code_verifier", code_verifier);
    body
}

/// OpenAI client_id shorthand.
pub fn openai_token_exchange_body(code: &str, redirect_uri: &str, code_verifier: &str) -> String {
    oauth_token_exchange_body(OPENAI_OAUTH_CLIENT_ID, code, redirect_uri, code_verifier)
}

/// Refresh-token grant body (no browser).
pub fn oauth_refresh_body(client_id: &str, refresh_token: &str) -> String {
    let mut body = String::new();
    push_q(&mut body, "grant_type", "refresh_token");
    push_q(&mut body, "client_id", client_id);
    push_q(&mut body, "refresh_token", refresh_token);
    body
}

pub fn openai_refresh_body(refresh_token: &str) -> String {
    oauth_refresh_body(OPENAI_OAUTH_CLIENT_ID, refresh_token)
}

/// Parse `code` + `state` from a callback URL or raw query string.
pub fn parse_oauth_callback_query(input: &str) -> Result<(String, String), String> {
    let q = if let Some(idx) = input.find('?') {
        &input[idx + 1..]
    } else {
        input
    };
    let mut code = None;
    let mut state = None;
    for pair in q.split('&') {
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("");
        let v = it.next().unwrap_or("");
        let v = urlencoding_decode(v);
        match k {
            "code" if !v.is_empty() => code = Some(v),
            "state" if !v.is_empty() => state = Some(v),
            _ => {}
        }
    }
    match (code, state) {
        (Some(c), Some(s)) => Ok((c, s)),
        (None, _) => Err("callback missing code".into()),
        (_, None) => Err("callback missing state".into()),
    }
}

fn urlencoding_decode(s: &str) -> String {
    // Minimal decode for OAuth callback values (percent + '+').
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h = |c: u8| -> Option<u8> {
                    match c {
                        b'0'..=b'9' => Some(c - b'0'),
                        b'a'..=b'f' => Some(c - b'a' + 10),
                        b'A'..=b'F' => Some(c - b'A' + 10),
                        _ => None,
                    }
                };
                if let (Some(a), Some(b)) = (h(bytes[i + 1]), h(bytes[i + 2])) {
                    out.push((a << 4) | b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parsed token endpoint JSON (OpenAI-compatible OAuth).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OAuthTokenJson {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub token_type: Option<String>,
}

pub fn parse_oauth_token_json(body: &str) -> Result<OAuthTokenJson, String> {
    serde_json::from_str(body).map_err(|e| e.to_string())
}

/// Map token JSON → [`crate::auth::Credential`] with provider API base.
pub fn credential_from_oauth_tokens(
    tokens: &OAuthTokenJson,
    api_base: &str,
) -> crate::auth::Credential {
    let expires_at = tokens.expires_in.map(|secs| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().saturating_add(secs))
            .unwrap_or(secs)
    });
    crate::auth::Credential {
        access_token: Some(tokens.access_token.clone()),
        refresh_token: tokens.refresh_token.clone(),
        expires_at,
        base_url: Some(api_base.trim_end_matches('/').to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_and_charset() {
        let v = generate_code_verifier();
        assert!(v.len() >= 43 && v.len() <= 128, "len={}", v.len());
        assert!(v.bytes().all(|b| UNRESERVED.contains(&b)));
    }

    #[test]
    fn s256_challenge_stable_for_known_vector() {
        // RFC 7636 appendix B style: challenge is base64url of sha256.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = code_challenge_s256(v);
        assert_eq!(c, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn openai_authorize_url_contains_pkce() {
        let url = openai_authorize_url(
            "http://127.0.0.1:1455/auth/callback",
            "st",
            "ch",
            "openid profile email offline_access",
        );
        assert!(url.starts_with(OPENAI_AUTHORIZE_URL));
        assert!(url.contains("code_challenge=ch"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("state=st"));
    }

    #[test]
    fn xai_authorize_url_contains_pkce() {
        let url = xai_authorize_url(
            "xai_client",
            "http://127.0.0.1:1455/auth/callback",
            "st",
            "ch",
            "openid",
        );
        assert!(url.starts_with(XAI_AUTHORIZE_URL));
        assert!(url.contains("code_challenge=ch"));
        assert!(url.contains("client_id=xai_client"));
    }

    #[test]
    fn state_is_opaque_nonempty() {
        let a = generate_state();
        let b = generate_state();
        assert_ne!(a, b);
        assert!(a.len() >= 32);
    }

    #[test]
    fn urlencoding_encodes_spaces() {
        assert_eq!(urlencoding_encode("a b"), "a%20b");
    }

    #[test]
    fn token_exchange_body_has_grant_and_verifier() {
        let b = openai_token_exchange_body("c0de", "http://127.0.0.1:1455/cb", "verif");
        assert!(b.contains("grant_type=authorization_code"));
        assert!(b.contains("code=c0de"));
        assert!(b.contains("code_verifier=verif"));
        assert!(b.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
    }

    #[test]
    fn refresh_body_has_grant() {
        let b = openai_refresh_body("rtok");
        assert!(b.contains("grant_type=refresh_token"));
        assert!(b.contains("refresh_token=rtok"));
    }

    #[test]
    fn parse_callback_from_url_and_query() {
        let (c, s) = parse_oauth_callback_query(
            "http://127.0.0.1:1455/auth/callback?code=abc%2F1&state=xyz",
        )
        .unwrap();
        assert_eq!(c, "abc/1");
        assert_eq!(s, "xyz");
        let (c2, s2) = parse_oauth_callback_query("code=z&state=s").unwrap();
        assert_eq!(c2, "z");
        assert_eq!(s2, "s");
        assert!(parse_oauth_callback_query("state=only").is_err());
    }

    #[test]
    fn parse_token_json_to_credential() {
        let raw =
            r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"token_type":"Bearer"}"#;
        let t = parse_oauth_token_json(raw).unwrap();
        let cred = credential_from_oauth_tokens(&t, "https://api.openai.com/v1");
        assert_eq!(cred.access_token.as_deref(), Some("at"));
        assert_eq!(cred.refresh_token.as_deref(), Some("rt"));
        assert!(cred.expires_at.is_some());
        assert_eq!(cred.base_url.as_deref(), Some("https://api.openai.com/v1"));
    }
}
