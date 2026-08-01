//! Minimal OpenCode-style agent TUI (feature = "agent").
//! Conversation + thin status + input. Slash commands. Zero chrome.

use crate::auth::{self, AuthProvider, ProviderId};
use crate::provider::{self, ChatMessage, OpenAICompat};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use std::io;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Build,
    Plan,
}

struct App {
    messages: Vec<ChatMessage>,
    input: String,
    status: String,
    mode: Mode,
    provider: Option<(ProviderId, OpenAICompat)>,
    scroll: u16,
    quit: bool,
}

impl App {
    fn new() -> Self {
        let creds = auth::load_all().unwrap_or_default();
        let provider = provider::pick_default(&creds).and_then(|(id, cred)| {
            let model = match id {
                ProviderId::Local => "llama3.2".to_string(),
                ProviderId::Zen => "gpt-5.1-codex".to_string(),
                _ => "gpt-4o".to_string(),
            };
            OpenAICompat::from_cred(&cred, model)
                .ok()
                .map(|c| (id, c))
        });
        let status = match &provider {
            Some((id, c)) => format!("{} · {} · {}", id.as_str(), c.model, mode_str(Mode::Build)),
            None => "no provider — /connect (set OPENCODE_ZEN_API_KEY or run local ollama)".into(),
        };
        Self {
            messages: vec![ChatMessage {
                role: "system".into(),
                content: "You are plazir18, a minimal open coding agent (OpenCode × titanium). Be concise. Prefer action.".into(),
            }],
            input: String::new(),
            status,
            mode: Mode::Build,
            provider,
            scroll: 0,
            quit: false,
        }
    }

    fn help_text() -> &'static str {
        "/help  /clear  /connect  /models  /mode  /q  — type and Enter to chat. Esc or q to quit."
    }

    fn handle_slash(&mut self, cmd: &str) {
        let cmd = cmd.trim().trim_start_matches('/');
        match cmd {
            "help" | "h" | "?" => {
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: Self::help_text().into(),
                });
            }
            "clear" => {
                self.messages.retain(|m| m.role == "system");
                self.status = "cleared".into();
            }
            "mode" | "plan" | "build" => {
                self.mode = if self.mode == Mode::Build {
                    Mode::Plan
                } else {
                    Mode::Build
                };
                self.refresh_status();
            }
            "connect" => {
                let providers = auth::builtin_providers();
                for p in providers {
                    if let Ok(cred) = p.login() {
                        let _ = auth::save(p.id(), cred.clone());
                        if let Ok(client) = OpenAICompat::from_cred(&cred, "llama3.2") {
                            self.provider = Some((p.id(), client));
                            self.refresh_status();
                            self.messages.push(ChatMessage {
                                role: "assistant".into(),
                                content: format!("connected {}", p.display_name()),
                            });
                            return;
                        }
                    }
                }
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: "no credentials. For Zen: export OPENCODE_ZEN_API_KEY=... (from opencode.ai/auth). For local: start ollama.".into(),
                });
            }
            "models" => {
                let msg = match &self.provider {
                    Some((id, c)) => format!("active: {} / {}", id.as_str(), c.model),
                    None => "no active model".into(),
                };
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: msg,
                });
            }
            "q" | "quit" => self.quit = true,
            _ => {
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: format!("unknown /{cmd} — try /help"),
                });
            }
        }
    }

    fn refresh_status(&mut self) {
        self.status = match &self.provider {
            Some((id, c)) => format!("{} · {} · {}", id.as_str(), c.model, mode_str(self.mode)),
            None => "no provider — /connect".into(),
        };
    }

    fn send(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();
        if text.starts_with('/') {
            self.handle_slash(&text);
            return;
        }
        self.messages.push(ChatMessage {
            role: "user".into(),
            content: text,
        });
        let Some((_, client)) = &self.provider else {
            self.messages.push(ChatMessage {
                role: "assistant".into(),
                content: "not connected. /connect first.".into(),
            });
            return;
        };
        match client.chat(&self.messages) {
            Ok(reply) => {
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: reply,
                });
            }
            Err(e) => {
                self.messages.push(ChatMessage {
                    role: "assistant".into(),
                    content: format!("error: {e}"),
                });
            }
        }
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Build => "build",
        Mode::Plan => "plan",
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    let lines: Vec<Line> = app
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let (style, prefix) = match m.role.as_str() {
                "user" => (Style::default().fg(Color::Cyan), "you"),
                "assistant" => (Style::default().fg(Color::Green), "plazir"),
                _ => (Style::default().fg(Color::DarkGray), "?"),
            };
            Line::from(vec![
                Span::styled(format!("{prefix}: "), style.add_modifier(Modifier::BOLD)),
                Span::raw(m.content.clone()),
            ])
        })
        .collect();
    let hist = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" plazir18 "))
        .wrap(Wrap { trim: false })
        .scroll((app.scroll, 0));
    f.render_widget(hist, chunks[0]);

    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL).title(" input "));
    f.render_widget(input, chunks[1]);

    let status = Paragraph::new(app.status.as_str()).style(Style::default().fg(Color::DarkGray));
    f.render_widget(status, chunks[2]);
}

pub fn run() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let res = loop {
        terminal.draw(|f| ui(f, &app))?;
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') if app.input.is_empty() => {
                        app.quit = true;
                    }
                    KeyCode::Enter => app.send(),
                    KeyCode::Char(c) => app.input.push(c),
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Up => app.scroll = app.scroll.saturating_add(1),
                    KeyCode::Down => app.scroll = app.scroll.saturating_sub(1),
                    _ => {}
                }
            }
        }
        if app.quit {
            break Ok(());
        }
    };
    ratatui::restore();
    res
}
