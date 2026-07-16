//! Rendering of a single session row.

use std::time::Instant;

use eframe::egui::{self, Align2, FontId, PointerButton, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::theme::{ATTACHED, BORDER, CARD, MUTED, PAD, STRIP_TILE_H, STRIP_W, TEXT, TP_DEBOUNCE};
use crate::tmux::Meta;
use crate::ui::Wall;

impl Wall {
    /// One compact 220×28 row for a session.
    /// Returns Some(id) if the session should be killed, or None otherwise.
    pub(crate) fn strip_tile(&mut self, ui: &mut egui::Ui, meta: &Meta) -> Option<String> {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(STRIP_W, STRIP_TILE_H), Sense::click());

        let painter = ui.painter().clone();
        painter.rect(
            rect,
            0.0,
            CARD,
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );

        let dot = rect.min + Vec2::new(PAD, STRIP_TILE_H / 2.0);
        if meta.attached {
            painter.circle_filled(dot, 4.0, ATTACHED);
        } else {
            painter.circle_stroke(dot, 4.0, Stroke::new(1.5, BORDER));
        }

        let name_origin = dot + Vec2::new(12.0, 0.0);
        let name_clip =
            Rect::from_min_size(name_origin - Vec2::new(0.0, 14.0), Vec2::new(90.0, 28.0));
        painter.with_clip_rect(name_clip).text(
            name_origin,
            Align2::LEFT_CENTER,
            &meta.name,
            FontId::proportional(9.0),
            TEXT,
        );

        // Pane tail: last 2 non-empty lines, right-aligned, clipped to right half.
        let pane = self.panes.get(&meta.name).map(String::as_str).unwrap_or("");
        let tail: Vec<&str> = pane
            .lines()
            .filter(|l| !l.trim().is_empty())
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let pane_clip = Rect::from_min_size(
            rect.min + Vec2::new(STRIP_W / 2.0, 0.0),
            Vec2::new(STRIP_W / 2.0 - 2.0, STRIP_TILE_H),
        );
        let pane_painter = painter.with_clip_rect(pane_clip);
        let pane_font = FontId::monospace(9.0);
        for (i, line) in tail.iter().enumerate() {
            let y = rect.min.y + 5.0 + i as f32 * 10.0;
            pane_painter.text(
                egui::pos2(rect.max.x - 4.0, y),
                Align2::RIGHT_TOP,
                *line,
                pane_font.clone(),
                MUTED,
            );
        }

        // Left-click → attach with debounce.
        if resp.clicked() {
            let now = Instant::now();
            let fresh = self
                .last_tp
                .get(&meta.id)
                .is_none_or(|t| now.duration_since(*t) >= TP_DEBOUNCE);
            if fresh {
                self.last_tp.insert(meta.id.clone(), now);
                self.status = crate::launch::alacritty(&meta.id);
            }
        }

        // Right-click → kill (deferred to caller to avoid borrow conflict).
        if resp.secondary_clicked() {
            return Some(meta.id.clone());
        }

        // Extra2 (MB5 forward button) when hovered → toggle visibility.
        if resp.clicked_by(PointerButton::Extra2) {
            let ctx = ui.ctx().clone();
            self.toggle_visibility(&ctx);
        }

        None
    }
}
