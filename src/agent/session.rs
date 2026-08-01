//! Multi-session persistence under XDG data dir.

use crate::provider::ChatMessage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Session {
    pub fn new(title: impl Into<String>) -> Self {
        let now = now_secs();
        let id = format!("s-{now}");
        Self {
            id,
            title: title.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sessions_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("plazir18").join("sessions")
}

pub struct SessionStore;

impl SessionStore {
    pub fn list() -> Vec<Session> {
        let dir = sessions_dir();
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return out;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<Session>(&raw) {
                    out.push(s);
                }
            }
        }
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        out
    }

    pub fn save(session: &Session) -> Result<(), String> {
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{}.json", session.id));
        let mut s = session.clone();
        s.updated_at = now_secs();
        let pretty = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        std::fs::write(path, pretty).map_err(|e| e.to_string())
    }

    pub fn load(id: &str) -> Option<Session> {
        let path = sessions_dir().join(format!("{id}.json"));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub fn delete(id: &str) -> Result<(), String> {
        let path = sessions_dir().join(format!("{id}.json"));
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
