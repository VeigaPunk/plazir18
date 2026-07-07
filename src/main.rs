//! Agent Wall: one tile per live tmux session, native Windows binary.
//! A fixed POLL_SECONDS heartbeat advances a round-robin index by one and
//! refreshes just that tile each tick. Mouse-only: left-click attaches a
//! terminal, right-click kills the session. Cross-compiled from WSL; tmux
//! calls route through wsl.exe -e when tmux isn't on PATH.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Vec2};

const POLL_SECONDS: u64 = 60;
const PANE_LINES: i32 = 30;

const TILE_W: f32 = 220.0;
const TILE_H: f32 = 150.0;
const GAP: f32 = 10.0;
const PAD: f32 = 12.0;
const STATUS_H: f32 = 20.0;
const TP_DEBOUNCE: Duration = Duration::from_millis(500);

const BG: Color32 = Color32::from_rgb(0x0b, 0x0e, 0x14);
const CARD: Color32 = Color32::from_rgb(0x16, 0x1b, 0x26);
const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd6, 0xe6);
const MUTED: Color32 = Color32::from_rgb(0x8b, 0x95, 0xab);
const ATTACHED: Color32 = Color32::from_rgb(0x3e, 0xe0, 0x8b);
const BORDER: Color32 = Color32::from_rgb(0x23, 0x2a, 0x38);

// wsl.exe -e (exec), not --: `--` pipes the command through the default
// shell, which expands a literal "$0" session-id argument to its own $0.
fn tmux_base() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["wsl.exe", "-e", "tmux"]
    } else {
        vec!["tmux"]
    }
}

fn tmux(args: &[&str]) -> Option<String> {
    let base = tmux_base();
    let out = Command::new(base[0])
        .args(&base[1..])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Clone)]
struct Meta {
    id: String,
    name: String,
    attached: bool,
}

fn parse_sessions(raw: &str) -> Vec<Meta> {
    raw.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() != 4 {
                return None;
            }
            Some(Meta {
                id: parts[0].to_string(),
                name: parts[1].to_string(),
                attached: parts[3] != "0",
            })
        })
        .collect()
}

fn list_sessions() -> Vec<Meta> {
    tmux(&[
        "list-sessions",
        "-F",
        "#{session_id}\t#{session_name}\t#{session_created}\t#{session_attached}",
    ])
    .map(|raw| parse_sessions(&raw))
    .unwrap_or_default()
}

// Target by $-prefixed session id, not name: -t takes a target-pane, and a
// bare numeric name like "0" resolves as a pane index first.
fn capture_pane(session_id: &str) -> String {
    tmux(&["capture-pane", "-p", "-t", session_id, "-S", &format!("-{PANE_LINES}")])
        .unwrap_or_default()
}

fn launch_tp(name: &str) -> String {
    let spawn = Command::new("wt.exe")
        .args(["-w", "_new", "wsl.exe", "--", "tmux", "attach", "-t", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawn {
        Ok(_) => format!("launched terminal for {name}"),
        Err(e) => format!("failed to launch wt.exe: {e}"),
    }
}

enum Msg {
    Sessions(Vec<Meta>),
    Pane(String, String), // session name, pane text
}

// Runs for the life of the process; dies with it when the window closes.
fn poller(tx: Sender<Msg>, ctx: egui::Context) {
    let mut rotation = 0usize;
    let mut known: HashSet<String> = HashSet::new();
    loop {
        let sessions = list_sessions();
        let _ = tx.send(Msg::Sessions(sessions.clone()));
        // new sessions get a pane immediately, off-turn
        for s in sessions.iter().filter(|s| !known.contains(&s.name)) {
            let _ = tx.send(Msg::Pane(s.name.clone(), capture_pane(&s.id)));
        }
        known = sessions.iter().map(|s| s.name.clone()).collect();
        if !sessions.is_empty() {
            rotation %= sessions.len();
            let s = &sessions[rotation];
            rotation += 1;
            let _ = tx.send(Msg::Pane(s.name.clone(), capture_pane(&s.id)));
        }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_secs(POLL_SECONDS));
    }
}

struct Wall {
    rx: Receiver<Msg>,
    sessions: Vec<Meta>,
    panes: HashMap<String, String>,
    last_tp: HashMap<String, Instant>,
    last_tick: Option<Instant>,
    status: String,
    seen_first_list: bool,
}

impl Wall {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        std::thread::spawn(move || poller(tx, ctx));
        Self {
            rx,
            sessions: Vec::new(),
            panes: HashMap::new(),
            last_tp: HashMap::new(),
            last_tick: None,
            status: "starting…".into(),
            seen_first_list: false,
        }
    }

    fn drain(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Sessions(list) => {
                    let alive: HashSet<&String> = list.iter().map(|m| &m.name).collect();
                    self.panes.retain(|name, _| alive.contains(name));
                    self.status = if list.is_empty() {
                        "no tmux sessions".into()
                    } else {
                        format!("{} sessions", list.len())
                    };
                    self.sessions = list;
                    self.last_tick = Some(Instant::now());
                    self.seen_first_list = true;
                }
                Msg::Pane(name, text) => {
                    self.panes.insert(name, text);
                }
            }
        }
    }

    fn tile(&mut self, ui: &mut egui::Ui, meta: &Meta) {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(TILE_W, TILE_H), Sense::click());
        let border = if meta.attached { ATTACHED } else { BORDER };
        let painter = ui.painter().clone();
        painter.rect(rect, 0.0, CARD, Stroke::new(1.0, border), StrokeKind::Inside);

        // pane text: tail of the capture, clipped to the card
        let inner = rect.shrink(8.0);
        let font = FontId::monospace(9.0);
        let line_h = ui.fonts_mut(|f| f.row_height(&font));
        let fit = (inner.height() / line_h).floor() as usize;
        let pane = self.panes.get(&meta.name).map(String::as_str).unwrap_or("");
        let trimmed = pane.trim_end_matches('\n');
        let lines: Vec<&str> = trimmed.lines().collect();
        let tail = lines[lines.len().saturating_sub(fit)..].join("\n");
        let clip = painter.with_clip_rect(inner);
        clip.text(inner.left_top(), Align2::LEFT_TOP, tail, font, TEXT);

        // nameplate, bottom-left
        let np_font = FontId::proportional(9.0);
        let galley = painter.layout_no_wrap(meta.name.clone(), np_font, Color32::WHITE);
        let np = Rect::from_min_size(
            rect.left_bottom() + Vec2::new(6.0, -(galley.size().y + 8.0)),
            galley.size() + Vec2::new(14.0, 4.0),
        );
        painter.rect_filled(np, 0.0, Color32::BLACK);
        painter.galley(np.min + Vec2::new(7.0, 2.0), galley, Color32::WHITE);

        if resp.clicked() {
            let now = Instant::now();
            let fresh = self
                .last_tp
                .get(&meta.name)
                .is_none_or(|t| now.duration_since(*t) >= TP_DEBOUNCE);
            if fresh {
                self.last_tp.insert(meta.name.clone(), now);
                self.status = launch_tp(&meta.name);
            }
        }
        if resp.secondary_clicked() {
            let base = tmux_base();
            let _ = Command::new(base[0])
                .args(&base[1..])
                .args(["kill-session", "-t", &meta.name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            self.status = format!("killed {}", meta.name);
            self.panes.remove(&meta.name);
            self.sessions.retain(|m| m.name != meta.name);
        }
    }
}

impl eframe::App for Wall {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.drain();

        let rows = self.sessions.len().max(1);
        let h = rows as f32 * TILE_H + (rows - 1) as f32 * GAP + PAD * 2.0 + STATUS_H;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
            TILE_W + PAD * 2.0,
            h,
        )));

        let age = match self.last_tick {
            None => "never".into(),
            Some(t) => format!("{}s ago", t.elapsed().as_secs()),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "Agent Wall — {} sessions — last tick {age}",
            self.sessions.len()
        )));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(PAD))
            .show(root, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(0.0, GAP);
                if self.sessions.is_empty() {
                    ui.label(
                        egui::RichText::new(if self.seen_first_list {
                            "no tmux sessions — F7 tmux ON, then F6"
                        } else {
                            "starting…"
                        })
                        .color(MUTED)
                        .size(9.0),
                    );
                } else {
                    for meta in self.sessions.clone() {
                        self.tile(ui, &meta);
                    }
                }
                ui.add_space(2.0);
                ui.label(egui::RichText::new(&self.status).color(MUTED).size(8.0));
            });

        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

fn main() -> eframe::Result<()> {
    // single-instance guard: a second launch fails the bind and exits silently
    let _guard = match std::net::TcpListener::bind("127.0.0.1:47819") {
        Ok(l) => l,
        Err(_) => return Ok(()),
    };

    let on_top = !std::env::args().any(|a| a == "--no-top");
    // plain orange square, generated in code — no icon assets
    let icon = egui::IconData {
        rgba: [0xff, 0x8c, 0x00, 0xff].repeat(32 * 32),
        width: 32,
        height: 32,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([TILE_W + PAD * 2.0, TILE_H + PAD * 2.0 + STATUS_H])
        .with_resizable(false)
        .with_icon(icon);
    if on_top {
        viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnTop);
    }
    eframe::run_native(
        "Agent Wall",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Wall::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_sessions;

    #[test]
    fn parses_well_formed_lines() {
        let raw = "$0\tmain\t1751800000\t1\n$3\tscout\t1751800100\t0\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].id, "$0");
        assert_eq!(s[0].name, "main");
        assert!(s[0].attached);
        assert!(!s[1].attached);
    }

    #[test]
    fn skips_malformed_lines() {
        let raw = "garbage\n$1\tok\t123\t0\n\ttoo\tfew\n";
        let s = parse_sessions(raw);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "ok");
    }

    #[test]
    fn empty_input() {
        assert!(parse_sessions("").is_empty());
    }
}
