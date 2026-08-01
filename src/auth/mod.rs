//! Pluggable authentication for plazir18 agent mode.
//!
//! Credentials: `$XDG_DATA_HOME/plazir18/auth.json` (fallback `~/.local/share`).
//! Providers implement [`AuthProvider`]. See PLAN.md / AGENT_PLAN.md.

mod local;
mod openai;
mod xai;
mod zen;

#[cfg(feature = "oauth")]
pub mod pkce;

pub use local::LocalAuth;
pub use openai::OpenAiAuth;
pub use xai::XaiAuth;
pub use zen::ZenAuth;

#[cfg(feature = "oauth")]
pub use openai::{
    OPENAI_LOOPBACK_REDIRECT, openai_browser_oauth_start, openai_exchange_code,
    openai_refresh_access_token, xai_authorize_url_hint,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    Zen,
    Xai,
    Openai,
    Local,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zen => "zen",
            Self::Xai => "xai",
            Self::Openai => "openai",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credential {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    providers: HashMap<String, Credential>,
}

fn auth_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("plazir18").join("auth.json")
}

pub fn load_all() -> Result<HashMap<ProviderId, Credential>, String> {
    let path = auth_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let file: AuthFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let mut out = HashMap::new();
    for (k, v) in file.providers {
        let id = match k.as_str() {
            "zen" => ProviderId::Zen,
            "xai" => ProviderId::Xai,
            "openai" => ProviderId::Openai,
            "local" => ProviderId::Local,
            _ => continue,
        };
        out.insert(id, v);
    }
    Ok(out)
}

pub fn save(id: ProviderId, cred: Credential) -> Result<(), String> {
    let path = auth_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file: AuthFile = if path.exists() {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())?
    } else {
        AuthFile::default()
    };
    file.providers.insert(id.as_str().to_string(), cred);
    let pretty = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(path, pretty).map_err(|e| e.to_string())?;
    Ok(())
}

pub trait AuthProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn display_name(&self) -> &str;
    fn login(&self) -> Result<Credential, String>;
    #[allow(dead_code)]
    fn refresh(&self, cred: &Credential) -> Result<Credential, String> {
        Ok(cred.clone())
    }
    #[allow(dead_code)]
    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.access_token
            .as_ref()
            .or(cred.api_key.as_ref())
            .map(|t| format!("Bearer {t}"))
    }
}

/// `/connect` attempt order: cloud keyed providers first, Local (always-Ok) last.
/// If `PLAZIR_LOCAL` is `1`, `true`, or `prefer`, Local is tried first.
pub fn connect_provider_order() -> [ProviderId; 4] {
    connect_provider_order_with_prefer_local(plazir_local_prefer())
}

fn plazir_local_prefer() -> bool {
    match std::env::var("PLAZIR_LOCAL") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "prefer"
        }
        Err(_) => false,
    }
}

/// Pure order builder (testable without env).
pub fn connect_provider_order_with_prefer_local(prefer_local: bool) -> [ProviderId; 4] {
    if prefer_local {
        [
            ProviderId::Local,
            ProviderId::Zen,
            ProviderId::Openai,
            ProviderId::Xai,
        ]
    } else {
        [
            ProviderId::Zen,
            ProviderId::Openai,
            ProviderId::Xai,
            ProviderId::Local,
        ]
    }
}

pub fn builtin_providers() -> Vec<Box<dyn AuthProvider>> {
    connect_provider_order()
        .into_iter()
        .map(|id| -> Box<dyn AuthProvider> {
            match id {
                ProviderId::Zen => Box::new(ZenAuth),
                ProviderId::Openai => Box::new(OpenAiAuth),
                ProviderId::Xai => Box::new(XaiAuth),
                ProviderId::Local => Box::new(LocalAuth::default()),
            }
        })
        .collect()
}

/// Pure connect selection: first provider whose `try_login` returns `Some`.
/// Used by TUI `/connect` and unit-tested without env or network.
pub fn try_connect_providers(
    mut try_login: impl FnMut(ProviderId) -> Option<Credential>,
) -> Option<(ProviderId, Credential)> {
    for id in connect_provider_order() {
        if let Some(cred) = try_login(id) {
            return Some((id, cred));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_id_strings() {
        assert_eq!(ProviderId::Zen.as_str(), "zen");
        assert_eq!(ProviderId::Local.as_str(), "local");
    }

    #[test]
    fn credential_roundtrip() {
        let c = Credential {
            api_key: Some("test-key".into()),
            base_url: Some("http://127.0.0.1:11434/v1".into()),
            ..Default::default()
        };
        let j = serde_json::to_string(&c).unwrap();
        let back: Credential = serde_json::from_str(&j).unwrap();
        assert_eq!(back.api_key.as_deref(), Some("test-key"));
    }

    #[test]
    fn connect_order_local_is_last_by_default() {
        let order = connect_provider_order_with_prefer_local(false);
        assert_eq!(order[0], ProviderId::Zen);
        assert_eq!(*order.last().unwrap(), ProviderId::Local);
    }

    #[test]
    fn connect_order_local_first_when_prefer() {
        let order = connect_provider_order_with_prefer_local(true);
        assert_eq!(order[0], ProviderId::Local);
        assert_eq!(order[1], ProviderId::Zen);
    }

    #[test]
    fn try_connect_prefers_zen_when_key_present_over_local() {
        // Simulate OPENCODE_ZEN_API_KEY set: Zen login succeeds; Local also would.
        let (id, cred) = try_connect_providers(|id| match id {
            ProviderId::Zen => Some(Credential {
                api_key: Some("zen-test-key".into()),
                base_url: Some("https://opencode.ai/zen/v1".into()),
                ..Default::default()
            }),
            ProviderId::Local => Some(Credential {
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ..Default::default()
            }),
            _ => None,
        })
        .expect("should connect");
        assert_eq!(id, ProviderId::Zen);
        assert_eq!(cred.api_key.as_deref(), Some("zen-test-key"));
    }

    #[test]
    fn try_connect_falls_back_to_local_when_no_cloud_keys() {
        let (id, _) = try_connect_providers(|id| match id {
            ProviderId::Local => Some(Credential {
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ..Default::default()
            }),
            _ => None,
        })
        .expect("local always ok");
        assert_eq!(id, ProviderId::Local);
    }

    /// Dual-env composition: OpenAI rejects loopback `OPENAI_BASE_URL`; Local
    /// inherits that base via `resolve_local_base` — one `try_connect` walk.
    #[test]
    fn try_connect_openai_loopback_hands_base_to_local() {
        let loopback = "http://127.0.0.1:11434/v1";
        let (id, cred) = try_connect_providers(|id| match id {
            ProviderId::Openai => {
                openai::resolve_openai_login(Some("sk-ollama"), Some(loopback)).ok()
            }
            ProviderId::Local => {
                let base =
                    local::resolve_local_base(None, Some(loopback), "http://127.0.0.1:9999/v1");
                Some(Credential {
                    base_url: Some(base),
                    ..Default::default()
                })
            }
            _ => None,
        })
        .expect("Local must accept after OpenAI loopback Err");
        assert_eq!(id, ProviderId::Local);
        assert_eq!(cred.base_url.as_deref(), Some(loopback));
    }
}
