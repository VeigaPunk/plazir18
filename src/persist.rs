//! Window state persistence — JSON at ~/.local/share/agent-wall/state.json.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct WinState {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

fn state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("agent-wall")
        .join("state.json")
}

impl WinState {
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(state_path()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) {
        let path = state_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WinState;

    #[test]
    fn roundtrip_win_state() {
        let path =
            std::env::temp_dir().join(format!("agent-wall-test-{}.json", std::process::id()));
        let state = WinState {
            x: 100,
            y: 200,
            w: 300,
            h: 400,
        };
        let text = serde_json::to_string(&state).unwrap();
        std::fs::write(&path, &text).unwrap();
        let loaded: WinState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.x, 100);
        assert_eq!(loaded.y, 200);
        assert_eq!(loaded.w, 300);
        assert_eq!(loaded.h, 400);
        let _ = std::fs::remove_file(path);
    }
}
