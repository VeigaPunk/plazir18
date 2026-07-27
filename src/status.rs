//! Waybar status fold: one-line JSON with session glyphs, tooltip, and class.

use crate::theme;
use eframe::egui::Color32;

const MAX_GLYPHS: usize = theme::MAX_CONCURRENT;

const ATTACHED_GLYPH: char = '●';
const DETACHED_GLYPH: char = '○';

/// Pango markup wants `#rrggbb`, but the palette of record is the wall's own
/// `Color32` — deriving the string here keeps the bar and the GUI in lockstep.
fn hex(color: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// Escape the three characters Pango reads as markup. `&` must be replaced
/// FIRST: doing it last would re-escape the ampersands that `<`/`>` introduce.
fn escape_pango(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Format a list of tmux sessions into a single Waybar JSON line.
///
/// Returns a JSON object with keys:
/// - `text`: one glyph per session (●/○), capped at 24, with +N overflow indicator
/// - `tooltip`: one line per shown session with name, idle time, and attach state
/// - `class`: "active" if sessions non-empty, "empty" if empty
pub fn format_status(sessions: &[crate::tmux::Meta], now: i64) -> String {
    let shown = &sessions[..sessions.len().min(MAX_GLYPHS)];
    let overflow = sessions.len() - shown.len();

    let mut text = String::new();
    for session in shown {
        let (color, glyph) = if session.attached {
            (theme::ATTACHED, ATTACHED_GLYPH)
        } else {
            (theme::BORDER, DETACHED_GLYPH)
        };
        text.push_str(&format!("<span foreground='{}'>{glyph}</span>", hex(color)));
    }
    if overflow > 0 {
        text.push_str(&format!(
            " <span foreground='{}'>+{overflow}</span>",
            hex(theme::MUTED)
        ));
    }

    let mut lines: Vec<String> = shown
        .iter()
        .map(|session| {
            let state = if session.attached {
                "attached"
            } else {
                "detached"
            };
            format!(
                "{}  {}  {state}",
                escape_pango(&session.name),
                session.idle_label(now)
            )
        })
        .collect();
    if overflow > 0 {
        lines.push(format!("+{overflow} more"));
    }

    let class = if sessions.is_empty() {
        "empty"
    } else {
        "active"
    };

    serde_json::json!({
        "text": text,
        "tooltip": lines.join("\n"),
        "class": class,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn format_status_empty_is_empty_class() {
        let result = format_status(&[], 1000);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        assert_eq!(parsed["text"], "");
        assert_eq!(parsed["tooltip"], "");
        assert_eq!(parsed["class"], "empty");
    }

    #[test]
    fn format_status_one_attached_one_filled_pip() {
        let sessions = vec![crate::tmux::Meta {
            id: "$0".to_string(),
            name: "main".to_string(),
            activity: Some(1000 - 65), // 65 seconds ago = 01:05
            attached: true,
        }];
        let now = 1000;

        let result = format_status(&sessions, now);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        let text = parsed["text"].as_str().expect("text must be string");
        assert!(text.contains("●"), "text must contain filled dot");
        assert!(text.contains("#3ee08b"), "text must contain attached color");

        assert_eq!(parsed["class"], "active");

        let tooltip = parsed["tooltip"].as_str().expect("tooltip must be string");
        assert!(tooltip.contains("01:05"), "tooltip must contain idle time");
        assert!(
            tooltip.contains("attached"),
            "tooltip must contain 'attached'"
        );
    }

    #[test]
    fn format_status_detached_hollow_pip() {
        let sessions = vec![crate::tmux::Meta {
            id: "$1".to_string(),
            name: "scout".to_string(),
            activity: Some(1000 - 120),
            attached: false,
        }];
        let now = 1000;

        let result = format_status(&sessions, now);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        let text = parsed["text"].as_str().expect("text must be string");
        assert!(text.contains("○"), "text must contain hollow dot");
        assert!(text.contains("#232a38"), "text must contain border color");

        let tooltip = parsed["tooltip"].as_str().expect("tooltip must be string");
        assert!(
            tooltip.contains("detached"),
            "tooltip must contain 'detached'"
        );
    }

    #[test]
    fn format_status_none_activity_dashes() {
        let sessions = vec![crate::tmux::Meta {
            id: "$2".to_string(),
            name: "idle".to_string(),
            activity: None,
            attached: true,
        }];
        let now = 1000;

        let result = format_status(&sessions, now);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        let tooltip = parsed["tooltip"].as_str().expect("tooltip must be string");
        assert!(
            tooltip.contains("--:--"),
            "tooltip must contain dashes for unknown activity"
        );
    }

    #[test]
    fn format_status_caps_at_24_with_overflow() {
        let mut sessions = Vec::new();
        for i in 0..26 {
            sessions.push(crate::tmux::Meta {
                id: format!("${}", i),
                name: format!("s{}", i),
                activity: Some(1000 - (i as i64) * 10),
                attached: i % 2 == 0,
            });
        }
        let now = 1000;

        let result = format_status(&sessions, now);
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        let text = parsed["text"].as_str().expect("text must be string");
        let dot_count = text.matches('●').count() + text.matches('○').count();
        assert_eq!(dot_count, 24, "text must contain exactly 24 glyphs");
        assert!(
            text.contains("+2"),
            "text must contain +2 overflow indicator"
        );

        let tooltip = parsed["tooltip"].as_str().expect("tooltip must be string");
        assert!(
            tooltip.contains("+2 more"),
            "tooltip must contain '+2 more' line"
        );
    }

    #[test]
    fn format_status_escapes_session_names() {
        let sessions = vec![crate::tmux::Meta {
            id: "$0".to_string(),
            name: "a\"<b>&".to_string(),
            activity: Some(1000 - 30),
            attached: true,
        }];
        let now = 1000;

        let result = format_status(&sessions, now);

        // Must parse as valid JSON
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("result must be valid JSON");

        let tooltip = parsed["tooltip"].as_str().expect("tooltip must be string");

        // Must contain escaped entities
        assert!(
            tooltip.contains("&lt;b&gt;"),
            "tooltip must contain escaped <b>"
        );
        assert!(tooltip.contains("&amp;"), "tooltip must contain escaped &");

        // Must NOT contain raw unescaped substring
        assert!(!tooltip.contains("<b>"), "tooltip must not contain raw <b>");
    }
}
