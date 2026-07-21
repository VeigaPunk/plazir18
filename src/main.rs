//! Agent Wall — tmux session dashboard. Entry point + channel wiring.
//! The poll loop lives in `poller.rs`; its event source in `wake.rs`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

mod ipc;
mod launch;
mod persist;
mod poller;
mod theme;
mod tile;
mod tmux;
mod ui;
mod wake;

use persist::WinState;
use theme::{GAP, MAX_CONCURRENT, PAD, PANEL_H, PANEL_W};
use ui::{LayoutMode, Wall};

/// Legacy full-width strip defaults.
const STRIP_WIN_W: f32 = 3440.0;
const STRIP_WIN_H: f32 = 58.0;
/// Multi-panel dashboard default height (width is computed from 6-col grid).
const PANEL_WIN_H: f32 = 720.0;

pub enum AppMsg {
    Sessions(Vec<tmux::Meta>),
    Pane(String, String), // (session_name, pane_text)
}

fn main() -> eframe::Result<()> {
    // --toggle: use Omarchy's named special workspace on Hyprland, otherwise
    // fall back to the app's portable IPC visibility command.
    if std::env::args().any(|a| a == "--toggle") {
        #[cfg(target_os = "linux")]
        if toggle_hyprland_special_workspace() {
            return Ok(());
        }
        #[cfg(unix)]
        ipc::send_toggle();
        return Ok(());
    }

    // Single-instance guard: second launch silently exits.
    let _guard = match std::net::TcpListener::bind("127.0.0.1:47819") {
        Ok(l) => l,
        Err(_) => return Ok(()),
    };

    let win_state = WinState::load().unwrap_or_default();

    // Default: multi-panel terminal grid (up to MAX_CONCURRENT).
    // `--strip` restores the legacy top-bar strip layout.
    let layout = if std::env::args().any(|a| a == "--strip") {
        LayoutMode::Strip
    } else {
        LayoutMode::Panel
    };

    // Bounded channel: poller drops updates when the UI is behind.
    // Sized for up to MAX_CONCURRENT pane updates + session list.
    let (tx, rx) = mpsc::sync_channel::<AppMsg>(MAX_CONCURRENT * 2 + 8);

    // Wake channel: one sender in the FIFO reader (real tmux activity), one in
    // the UI (kill / reveal). The poller owns the receiver.
    let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(8);

    // Shared visibility: the poller skips pane capture while hidden.
    let visible = Arc::new(AtomicBool::new(true));

    // IPC server (unix only): listens for toggle commands.
    #[cfg(unix)]
    let (ipc_tx, ipc_rx) = mpsc::sync_channel::<ipc::IpcCmd>(4);

    let on_top = !std::env::args().any(|a| a == "--no-top");

    let icon = egui::IconData {
        rgba: [0xff, 0x8c, 0x00, 0xff].repeat(32 * 32),
        width: 32,
        height: 32,
    };

    let default_w = match layout {
        LayoutMode::Strip => STRIP_WIN_W,
        LayoutMode::Panel => {
            // 6 columns × panel + gaps + pad, enough for 24 tiles in a 6×4 grid.
            let cols = 6.0_f32;
            PAD * 2.0 + cols * PANEL_W + (cols - 1.0) * GAP
        }
    };
    let default_h = match layout {
        LayoutMode::Strip => STRIP_WIN_H,
        LayoutMode::Panel => PANEL_WIN_H,
    };
    let _ = PANEL_H; // used by layout sizing docs / future auto-fit

    let w = if win_state.w > 0 {
        win_state.w as f32
    } else {
        default_w
    };
    let h = if win_state.h > 0 {
        win_state.h as f32
    } else {
        default_h
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([w, h])
        .with_resizable(true)
        .with_icon(icon);

    if win_state.w > 0 {
        viewport = viewport.with_position(egui::pos2(win_state.x as f32, win_state.y as f32));
    }
    if on_top {
        viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnTop);
    }

    eframe::run_native(
        "Agent Wall",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();

            // Event source: FIFO poked by tmux hooks + pipe-pane streams.
            let fifo = wake::ensure_fifo();
            wake::spawn_reader(wake::fifo_path(), wake_tx.clone());

            let poller_visible = visible.clone();
            std::thread::spawn(move || poller::run(tx, ctx, wake_rx, poller_visible, fifo));

            #[cfg(unix)]
            {
                let ctx = cc.egui_ctx.clone();
                ipc::serve(ipc_tx, ctx);
            }

            Ok(Box::new(Wall::new(
                rx,
                #[cfg(unix)]
                ipc_rx,
                win_state,
                wake_tx,
                visible,
                layout,
            )))
        }),
    )
}

#[cfg(target_os = "linux")]
fn toggle_hyprland_special_workspace() -> bool {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return false;
    }
    std::process::Command::new("hyprctl")
        .args([
            "dispatch",
            r#"hl.dsp.workspace.toggle_special("agent-wall")"#,
        ])
        .status()
        .is_ok_and(|s| s.success())
}
