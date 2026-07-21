//! Palette and layout constants for the wall.

use eframe::egui::Color32;
use std::time::Duration;

pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0e, 0x14);
pub const CARD: Color32 = Color32::from_rgb(0x16, 0x1b, 0x26);
pub const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd6, 0xe6);
pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x95, 0xab);
pub const ATTACHED: Color32 = Color32::from_rgb(0x3e, 0xe0, 0x8b);
pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x2a, 0x38);

pub const WIN_H: f32 = 58.0;
pub const STRIP_TILE_H: f32 = 28.0;
pub const STRIP_W: f32 = 220.0;
pub const GAP: f32 = 4.0;
pub const PAD: f32 = 10.0;

/// Ignore repeat left-clicks on a tile within this window (double-launch guard).
pub const TP_DEBOUNCE: Duration = Duration::from_millis(500);
