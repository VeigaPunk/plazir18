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
}
