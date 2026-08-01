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

    /// Pretty JSON export (portable; includes full message history).
    pub fn to_json_pretty(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Human-readable markdown transcript (user/assistant only by default).
    pub fn to_markdown(&self) -> String {
        let mut out = format!("# {}\n\n`{}`\n\n", self.title, self.id);
        for m in &self.messages {
            if m.role == "system" {
                continue;
            }
            let label = match m.role.as_str() {
                "user" => "You",
                "assistant" => "Assistant",
                "tool" => "Tool",
                other => other,
            };
            out.push_str(&format!("## {label}\n\n{}\n\n", m.content.trim()));
        }
        out
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|e| e.to_string())
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

    /// Load a session by exact id.
    pub fn load(id: &str) -> Option<Session> {
        let path = sessions_dir().join(format!("{id}.json"));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Write session JSON to an arbitrary path (export).
    pub fn export_json(session: &Session, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let pretty = session.to_json_pretty()?;
        std::fs::write(path, pretty).map_err(|e| e.to_string())
    }

    /// Write markdown transcript to path.
    pub fn export_markdown(session: &Session, path: &std::path::Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(path, session.to_markdown()).map_err(|e| e.to_string())
    }

    /// Import a session JSON file; keeps file id or assigns new if empty/collision requested.
    pub fn import_json(path: &std::path::Path, new_id: bool) -> Result<Session, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut s = Session::from_json(&raw)?;
        if new_id || s.id.trim().is_empty() {
            s.id = new_session_id();
        }
        s.updated_at = now_secs();
        Self::save(&s)?;
        Ok(s)
    }

    /// Load by exact id, else unique id prefix (for `/open s-…` short forms).
    pub fn load_by_prefix(prefix: &str) -> Result<Session, String> {
        let p = prefix.trim();
        if p.is_empty() {
            return Err("usage: /open <session-id-or-prefix>".into());
        }
        if let Some(s) = Self::load(p) {
            return Ok(s);
        }
        let hits: Vec<Session> = Self::list()
            .into_iter()
            .filter(|s| s.id.starts_with(p))
            .collect();
        match hits.len() {
            0 => Err(format!("no session matching {p}")),
            1 => Ok(hits.into_iter().next().expect("len 1")),
            n => Err(format!("{n} sessions match {p} \u{2014} use a longer id")),
        }
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

    #[test]
    fn export_json_and_markdown_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("plazir18-exp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("PLAZIR18_SESSIONS_DIR", &dir);
        }
        let mut s = Session::new("export-me");
        s.messages
            .push(crate::provider::ChatMessage::text("user", "hello export"));
        s.messages
            .push(crate::provider::ChatMessage::text("assistant", "hi back"));
        let json_path = dir.join("out.json");
        let md_path = dir.join("out.md");
        SessionStore::export_json(&s, &json_path).unwrap();
        SessionStore::export_markdown(&s, &md_path).unwrap();
        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("hello export"));
        assert!(md.contains("hi back"));
        let imported = SessionStore::import_json(&json_path, true).unwrap();
        assert_ne!(imported.id, s.id);
        assert_eq!(imported.title, "export-me");
        assert_eq!(imported.messages.len(), 2);
        SessionStore::delete(&imported.id).unwrap();
        unsafe {
            std::env::remove_var("PLAZIR18_SESSIONS_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_by_prefix_exact_and_unique() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("plazir18-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("PLAZIR18_SESSIONS_DIR", &dir);
        }
        let s = Session::new("pfx");
        let id = s.id.clone();
        SessionStore::save(&s).unwrap();
        let exact = SessionStore::load_by_prefix(&id).unwrap();
        assert_eq!(exact.id, id);
        let short = &id[..id.len().min(10)];
        let by = SessionStore::load_by_prefix(short).unwrap();
        assert_eq!(by.id, id);
        assert!(SessionStore::load_by_prefix("no-such-prefix-zzz").is_err());
        SessionStore::delete(&id).unwrap();
        unsafe {
            std::env::remove_var("PLAZIR18_SESSIONS_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
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
    #[test]
    fn export_import_json_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("plazir18-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::set_var("PLAZIR18_SESSIONS_DIR", &dir);
        }
        let mut s = Session::new("exp");
        s.messages.push(crate::provider::ChatMessage {
            role: "user".into(),
            content: "hello-export".into(),
            ..Default::default()
        });
        let json_path = dir.join("out.json");
        let md_path = dir.join("out.md");
        SessionStore::export_json(&s, &json_path).unwrap();
        SessionStore::export_markdown(&s, &md_path).unwrap();
        assert!(json_path.is_file());
        assert!(md_path.is_file());
        let md = std::fs::read_to_string(&md_path).unwrap();
        assert!(md.contains("hello-export"));
        let imported = SessionStore::import_json(&json_path, true).unwrap();
        assert_ne!(imported.id, s.id);
        assert!(imported.messages.iter().any(|m| m.content == "hello-export"));
        SessionStore::delete(&imported.id).unwrap();
        unsafe {
            std::env::remove_var("PLAZIR18_SESSIONS_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

}
