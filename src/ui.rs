//! Wall state, event draining, and the eframe::App implementation.
//! Row rendering lives in `tile.rs`; colours/sizes in `theme.rs`.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use eframe::egui::{self, Vec2};

use crate::AppMsg;
use crate::ipc::IpcCmd;
use crate::persist::WinState;
use crate::theme::{BG, GAP, MUTED, PAD, WIN_H};
use crate::tmux::Meta;

pub struct Wall {
    pub rx: std::sync::mpsc::Receiver<AppMsg>,
    #[cfg(unix)]
    pub ipc_rx: std::sync::mpsc::Receiver<IpcCmd>,
    /// Poke the poller to wake early (after a kill or a reveal).
    pub wake_tx: std::sync::mpsc::SyncSender<()>,
    /// Shared with the poller: when hidden it skips pane capture.
    pub visible: Arc<AtomicBool>,
    pub sessions: Vec<Meta>,
    pub panes: HashMap<String, String>,
    pub last_tp: HashMap<String, Instant>,
    pub last_tick: Option<Instant>,
    pub status: String,
    pub seen_first_list: bool,
    pub win_state: WinState,
}

impl Wall {
    pub fn new(
        rx: std::sync::mpsc::Receiver<AppMsg>,
        #[cfg(unix)] ipc_rx: std::sync::mpsc::Receiver<IpcCmd>,
        win_state: WinState,
        wake_tx: std::sync::mpsc::SyncSender<()>,
        visible: Arc<AtomicBool>,
    ) -> Self {
        Self {
            rx,
            #[cfg(unix)]
            ipc_rx,
            wake_tx,
            visible,
            sessions: Vec::new(),
            panes: HashMap::new(),
            last_tp: HashMap::new(),
            last_tick: None,
            status: "starting…".into(),
            seen_first_list: false,
            win_state,
        }
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                AppMsg::Sessions(list) => {
                    let alive: std::collections::HashSet<_> =
                        list.iter().map(|m| m.name.clone()).collect();
                    self.panes.retain(|name, _| alive.contains(name));
                    self.sessions = list;
                    self.seen_first_list = true;
                    self.last_tick = Some(Instant::now());
                }
                AppMsg::Pane(name, text) => {
                    self.panes.insert(name, text);
                }
            }
        }

        #[cfg(unix)]
        while let Ok(cmd) = self.ipc_rx.try_recv() {
            match cmd {
                IpcCmd::Toggle => self.toggle_visibility(ctx),
            }
        }

        let age = self.last_tick.map_or_else(
            || "never".to_string(),
            |t| format!("{}s ago", t.elapsed().as_secs()),
        );
        let n = self.sessions.len();
        self.status = if !self.seen_first_list {
            "starting…".to_string()
        } else if n == 0 {
            "no sessions".to_string()
        } else {
            format!("{n} sessions · {age}")
        };
    }

    pub(crate) fn toggle_visibility(&mut self, ctx: &egui::Context) {
        let now_visible = !self.visible.load(Ordering::Relaxed);
        self.visible.store(now_visible, Ordering::Relaxed);
        // ViewportCommand::Visible is a no-op on Wayland (winit 0.30 xdg_toplevel).
        // Minimized is the supported way to hide/show on Wayland compositors.
        if now_visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // Wake the poller so previews refresh immediately on reveal.
            let _ = self.wake_tx.try_send(());
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }
}

impl eframe::App for Wall {
    /// Logic runs even when the window is hidden (poller calls request_repaint).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain(ctx);

        // Track outer window position/size for persistence.
        if let Some(rect) = ctx.input(|i| i.viewport().outer_rect) {
            let x = rect.min.x as i32;
            let y = rect.min.y as i32;
            let w = rect.width() as u32;
            let h = rect.height() as u32;
            let s = &self.win_state;
            if (x - s.x).abs() > 2 || (y - s.y).abs() > 2 || w != s.w || h != s.h {
                self.win_state = WinState { x, y, w, h };
            }
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "Agent Wall — {} sessions",
            self.sessions.len()
        )));
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(PAD))
            .show(root, |ui| {
                let mut to_kill: Vec<String> = Vec::new();

                egui::ScrollArea::horizontal()
                    .max_height(WIN_H - PAD * 2.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(GAP, 0.0);
                            if self.sessions.is_empty() {
                                ui.label(
                                    egui::RichText::new(if self.seen_first_list {
                                        "no tmux sessions"
                                    } else {
                                        "starting…"
                                    })
                                    .color(MUTED)
                                    .size(9.0),
                                );
                            } else {
                                let sessions = self.sessions.clone();
                                for meta in &sessions {
                                    if let Some(id) = self.strip_tile(ui, meta) {
                                        to_kill.push(id);
                                    }
                                }
                            }
                            ui.label(egui::RichText::new(&self.status).color(MUTED).size(9.0));
                        });
                    });

                // Process deferred kills after iteration.
                if !to_kill.is_empty() {
                    for id in &to_kill {
                        crate::tmux::kill_session(id);
                        self.sessions.retain(|m| &m.id != id);
                    }
                    let alive: std::collections::HashSet<_> =
                        self.sessions.iter().map(|m| m.name.clone()).collect();
                    self.panes.retain(|n, _| alive.contains(n));
                    self.status = format!("killed {}", to_kill.join(", "));
                    // Wake the poller so it sees the kill immediately.
                    let _ = self.wake_tx.try_send(());
                }
            });
    }

    fn on_exit(&mut self) {
        self.win_state.save();
        crate::tmux::remove_hooks();
    }
}
