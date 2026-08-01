//! Multi-session persistence under XDG data dir.

use crate::provider::ChatMessage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

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
        let id = new_session_id();
        Self {
            id,
            title: title.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Unique id: `s-{secs}-{nanos}-{seq}` so same-second `Session::new` never collides.
fn new_session_id() -> String {
    let (secs, nanos) = now_parts();
    let seq = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("s-{secs}-{nanos}-{seq}")
}

fn now_parts() -> (u64, u32) {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0))
}

fn now_secs() -> u64 {
    now_parts().0
}

fn sessions_dir() -> PathBuf {
    // Test / override hook: isolated path without mutating XDG_DATA_HOME.
    if let Some(dir) = std::env::var_os("PLAZIR18_SESSIONS_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
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
            if let Ok(raw) = std::fs::read_to_string(&path)
                && let Ok(s) = serde_json::from_str::<Session>(&raw)
            {
                out.push(s);
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
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

    /// Load a session by id (API surface for multi-session UI).
    #[allow(dead_code)]
    pub fn load(id: &str) -> Option<Session> {
        let path = sessions_dir().join(format!("{id}.json"));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Delete a session file by id.
    #[allow(dead_code)]
    pub fn delete(id: &str) -> Result<(), String> {
        let path = sessions_dir().join(format!("{id}.json"));
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialise PLAZIR18_SESSIONS_DIR mutation (process-global env).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn save_list_load_delete_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("plazir18-sess-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Prefer dedicated override so we never touch XDG_DATA_HOME.
        // SAFETY: held under ENV_LOCK; cleaned up before release.
        unsafe {
            std::env::set_var("PLAZIR18_SESSIONS_DIR", &dir);
        }
        let s = Session::new("t1");
        let id = s.id.clone();
        SessionStore::save(&s).unwrap();
        let listed = SessionStore::list();
        assert!(listed.iter().any(|x| x.id == id));
        let loaded = SessionStore::load(&id).expect("load");
        assert_eq!(loaded.title, "t1");
        SessionStore::delete(&id).unwrap();
        assert!(SessionStore::load(&id).is_none());
        unsafe {
            std::env::remove_var("PLAZIR18_SESSIONS_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_ids_unique_same_second() {
        let a = Session::new("a");
        let b = Session::new("b");
        assert_ne!(a.id, b.id);
        assert!(a.id.starts_with("s-"));
        assert!(b.id.starts_with("s-"));
    }

    /// Offline M6: mock completion body → apply turn → persist/load session.
    #[test]
    fn m6_offline_one_chat_turn_persists() {
        let _g = ENV_LOCK.lock().unwrap();
        let reply = crate::provider::parse_chat_completion_body(
            r#"{"choices":[{"message":{"role":"assistant","content":"e2e-ok"}}]}"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("plazir18-m6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: held under ENV_LOCK; cleaned up before release.
        unsafe {
            std::env::set_var("PLAZIR18_SESSIONS_DIR", &dir);
        }
        let mut session = Session::new("m6");
        crate::provider::apply_chat_turn(&mut session.messages, "hi", reply);
        SessionStore::save(&session).unwrap();
        let loaded = SessionStore::load(&session.id).expect("load");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "hi");
        assert_eq!(loaded.messages[1].content, "e2e-ok");
        SessionStore::delete(&session.id).unwrap();
        unsafe {
            std::env::remove_var("PLAZIR18_SESSIONS_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
