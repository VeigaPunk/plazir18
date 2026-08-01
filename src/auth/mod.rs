//! Pluggable authentication for plazir18 agent mode.
//!
//! Credentials: `$XDG_DATA_HOME/plazir18/auth.json` (fallback `~/.local/share`).
//! Providers implement [`AuthProvider`]. See PLAN.md / AGENT_PLAN.md.

mod local;
mod zen;

pub use local::LocalAuth;
pub use zen::ZenAuth;

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
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
        })
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
    fn refresh(&self, cred: &Credential) -> Result<Credential, String> {
        Ok(cred.clone())
    }
    fn authorization_header(&self, cred: &Credential) -> Option<String> {
        cred.access_token
            .as_ref()
            .or(cred.api_key.as_ref())
            .map(|t| format!("Bearer {t}"))
    }
}

pub fn builtin_providers() -> Vec<Box<dyn AuthProvider>> {
    vec![Box::new(ZenAuth), Box::new(LocalAuth::default())]
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
}
