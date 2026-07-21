//! Palette and layout constants for the wall.

use eframe::egui::Color32;
use std::time::Duration;

pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0e, 0x14);
pub const CARD: Color32 = Color32::from_rgb(0x16, 0x1b, 0x26);
pub const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd6, 0xe6);
pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x95, 0xab);
pub const ATTACHED: Color32 = Color32::from_rgb(0x3e, 0xe0, 0x8b);
pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x2a, 0x38);
pub const TITLE_BAR: Color32 = Color32::from_rgb(0x12, 0x16, 0x1f);

/// Hard concurrent agent / terminal panel ceiling. No soft nudge below this.
pub const MAX_CONCURRENT: usize = 24;

/// Compact strip (legacy top-bar) tile size.
pub const WIN_H: f32 = 58.0;
pub const STRIP_TILE_H: f32 = 28.0;
pub const STRIP_W: f32 = 220.0;

/// Multi-panel terminal tile (dashboard).
pub const PANEL_W: f32 = 320.0;
pub const PANEL_H: f32 = 160.0;
pub const PANEL_TITLE_H: f32 = 18.0;
/// Lines of pane tail shown inside each panel body.
pub const PANEL_TAIL_LINES: usize = 10;

pub const GAP: f32 = 6.0;
pub const PAD: f32 = 10.0;

/// Ignore repeat left-clicks on a tile within this window (double-launch guard).
pub const TP_DEBOUNCE: Duration = Duration::from_millis(500);

/// Column count for a multi-panel grid given how many tiles are visible.
pub fn grid_cols(n: usize) -> usize {
    match n {
        0 => 1,
        1 => 1,
        2 => 2,
        3..=4 => 2,
        5..=9 => 3,
        10..=16 => 4,
        17..=24 => 6,
        _ => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_cols_covers_full_24_ceiling() {
        assert_eq!(grid_cols(0), 1);
        assert_eq!(grid_cols(1), 1);
        assert_eq!(grid_cols(4), 2);
        assert_eq!(grid_cols(9), 3);
        assert_eq!(grid_cols(16), 4);
        assert_eq!(grid_cols(24), 6);
        assert_eq!(MAX_CONCURRENT, 24);
    }
}
