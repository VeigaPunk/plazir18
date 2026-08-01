//! Minimal OpenCode-style agent TUI (feature = "agent").
//! Conversation + thin status + input. Slash commands. Zero chrome.

use crate::agent::{Session, SessionStore, Tool, expand_at_files, run_tool, write_agents_md};
use crate::auth::{self, ProviderId};
use crate::provider::{self, ChatMessage, OpenAICompat, apply_chat_turn};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
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
    session: Session,
    scroll: u16,
    quit: bool,
}

impl App {
    fn new() -> Self {
        let creds = auth::load_all().unwrap_or_default();
        let provider = provider::pick_default(&creds)
            .and_then(|(id, cred)| provider::client_for(id, &cred).ok().map(|c| (id, c)));
        let session = Session::new("default");
        let status = match &provider {
            Some((id, c)) => format!(
                "{} \u{00b7} {} \u{00b7} {} \u{00b7} {}",
                id.as_str(),
                c.model,
                mode_str(Mode::Build),
                short_id(&session.id)
            ),
            None => "no provider \u{2014} /connect (Zen \u{00b7} local \u{00b7} OPENAI_API_KEY \u{00b7} XAI_API_KEY)".into(),
        };
        Self {
            messages: vec![ChatMessage {
                role: "system".into(),
                content: "You are plazir18, a minimal open coding agent (OpenCode \u{00d7} titanium). Be concise. Prefer action. When the user pastes @path, treat the file contents as context.".into(),
            }],
            input: String::new(),
            status,
            mode: Mode::Build,
            provider,
            session,
            scroll: 0,
            quit: false,
        }
    }

    fn help_text() -> &'static str {
        "/help /clear /connect /models /mode /session /sessions /open <id> /new /init /q  \u{00b7}  !bash !read !ls  \u{00b7}  @file"
    }

    fn handle_slash(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.trim().trim_start_matches('/').splitn(2, ' ').collect();
        let head = parts.first().copied().unwrap_or("");
        let arg = parts.get(1).copied().unwrap_or("").trim();
        match head {
            "help" | "h" | "?" => self.push_assistant(Self::help_text()),
            "clear" => {
                self.messages.retain(|m| m.role == "system");
                self.status = "cleared".into();
            }
            "mode" | "plan" | "build" => {
                self.mode = match head {
                    "plan" => Mode::Plan,
                    "build" => Mode::Build,
                    _ if self.mode == Mode::Build => Mode::Plan,
                    _ => Mode::Build,
                };
                self.refresh_status();
                self.push_assistant(format!("mode \u{2192} {}", mode_str(self.mode)));
            }
            "connect" => {
                let providers = auth::builtin_providers();
                let picked = auth::try_connect_providers(|id| {
                    providers
                        .iter()
                        .find(|p| p.id() == id)
                        .and_then(|p| p.login().ok())
                });
                if let Some((id, cred)) = picked {
                    let _ = auth::save(id, cred.clone());
                    if let Ok(client) = provider::client_for(id, &cred) {
                        let name = providers
                            .iter()
                            .find(|p| p.id() == id)
                            .map(|p| p.display_name().to_string())
                            .unwrap_or_else(|| id.as_str().to_string());
                        let model = client.model.clone();
                        self.provider = Some((id, client));
                        self.refresh_status();
                        self.push_assistant(format!("connected {name} \u{00b7} {model}"));
                        return;
                    }
                }
                self.push_assistant(
                    "no credentials.\n  Zen:   OPENCODE_ZEN_API_KEY / PLAZIR_ZEN_KEY  (opencode.ai/auth)\n  Local: PLAZIR_LOCAL_BASE / PLAZIR_LOCAL_KEY / PLAZIR_LOCAL_MODEL (default :11434 / llama3.2); PLAZIR_LOCAL=prefer; loopback OPENAI_BASE_URL handed to Local\n  OpenAI: OPENAI_API_KEY (+ remote OPENAI_BASE_URL)\n  Grok:  XAI_API_KEY / GROK_API_KEY",
                );
            }
            "models" => {
                let msg = match &self.provider {
                    Some((id, c)) => {
                        let mut line = format!("active: {} / {}", id.as_str(), c.model);
                        if *id == ProviderId::Local {
                            match c.list_model_ids() {
                                Ok(ids) if !ids.is_empty() => {
                                    let shown: Vec<&str> =
                                        ids.iter().take(12).map(String::as_str).collect();
                                    line.push_str(&format!("\ncatalog: {}", shown.join(", ")));
                                    if ids.len() > 12 {
                                        line.push_str(" \u{2026}");
                                    }
                                }
                                Ok(_) => line.push_str("\ncatalog: (empty)"),
                                Err(e) => line.push_str(&format!("\ncatalog: {e}")),
                            }
                        }
                        line
                    }
                    None => "no active model".into(),
                };
                self.push_assistant(msg);
            }
            "session" => {
                if arg.is_empty() {
                    self.push_assistant(format!(
                        "session {} \u{2014} {} msgs \u{2014} /open <id> to switch",
                        self.session.id,
                        self.messages.iter().filter(|m| m.role != "system").count()
                    ));
                } else {
                    self.open_session(arg);
                }
            }
            "open" => self.open_session(arg),
            "sessions" => {
                let list = SessionStore::list();
                if list.is_empty() {
                    self.push_assistant("no saved sessions");
                } else {
                    let lines: Vec<String> = list
                        .iter()
                        .take(12)
                        .map(|s| format!("{}  {}  ({})", s.id, s.title, s.messages.len()))
                        .collect();
                    self.push_assistant(lines.join("\n"));
                }
            }
            "new" => {
                let _ = self.persist();
                self.session = Session::new(if arg.is_empty() { "default" } else { arg });
                self.messages.retain(|m| m.role == "system");
                self.refresh_status();
                self.push_assistant(format!("new session {}", self.session.id));
            }
            "init" => match write_agents_md(".") {
                Ok(path) => self.push_assistant(format!("wrote {path}")),
                Err(e) => self.push_assistant(format!("init failed: {e}")),
            },
            "q" | "quit" => {
                let _ = self.persist();
                self.quit = true;
            }
            _ => self.push_assistant(format!("unknown /{head} \u{2014} try /help")),
        }
    }

    fn push_assistant(&mut self, content: impl Into<String>) {
        self.messages.push(ChatMessage {
            role: "assistant".into(),
            content: content.into(),
        });
    }

    fn open_session(&mut self, id_or_prefix: &str) {
        let _ = self.persist();
        match SessionStore::load_by_prefix(id_or_prefix) {
            Ok(s) => {
                self.messages = s.messages.clone();
                if !self.messages.iter().any(|m| m.role == "system") {
                    self.messages.insert(
                        0,
                        ChatMessage {
                            role: "system".into(),
                            content: "You are plazir18, a minimal open coding agent (OpenCode \u{00d7} titanium). Be concise. Prefer action. When the user pastes @path, treat the file contents as context.".into(),
                        },
                    );
                }
                self.session = s;
                self.scroll = 0;
                self.refresh_status();
                self.push_assistant(format!(
                    "opened {} \u{2014} {} msgs",
                    self.session.id,
                    self.messages.iter().filter(|m| m.role != "system").count()
                ));
            }
            Err(e) => self.push_assistant(e),
        }
    }

    fn persist(&mut self) -> Result<(), String> {
        self.session.messages = self.messages.clone();
        SessionStore::save(&self.session)
    }

    fn refresh_status(&mut self) {
        self.status = match &self.provider {
            Some((id, c)) => format!(
                "{} \u{00b7} {} \u{00b7} {} \u{00b7} {}",
                id.as_str(),
                c.model,
                mode_str(self.mode),
                short_id(&self.session.id)
            ),
            None => "no provider \u{2014} /connect".into(),
        };
    }

    fn maybe_run_inline_tool(&mut self, text: &str) -> bool {
        let plan = self.mode == Mode::Plan;
        if let Some(rest) = text.strip_prefix("!bash ") {
            let r = run_tool(Tool::Bash { cmd: rest.into() }, plan);
            self.push_assistant(format!("$ {rest}\n{}", r.output));
            return true;
        }
        if let Some(rest) = text.strip_prefix("!read ") {
            let r = run_tool(Tool::Read { path: rest.into() }, plan);
            self.push_assistant(format!("read {rest}:\n{}", r.output));
            return true;
        }
        if let Some(rest) = text.strip_prefix("!ls ") {
            let r = run_tool(Tool::List { path: rest.into() }, plan);
            self.push_assistant(r.output);
            return true;
        }
        if text == "!ls" {
            let r = run_tool(Tool::List { path: ".".into() }, plan);
            self.push_assistant(r.output);
            return true;
        }
        false
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
        if self.maybe_run_inline_tool(&text) {
            let _ = self.persist();
            return;
        }
        let expanded = expand_at_files(&text);
        let Some((_, client)) = &self.provider else {
            self.messages.push(ChatMessage {
                role: "user".into(),
                content: expanded,
            });
            self.push_assistant("not connected. /connect first.");
            let _ = self.persist();
            return;
        };
        // Build request including the pending user turn without mutating yet.
        let mut req = self.messages.clone();
        req.push(ChatMessage {
            role: "user".into(),
            content: expanded.clone(),
        });
        match client.chat(&req) {
            Ok(reply) => apply_chat_turn(&mut self.messages, expanded, reply),
            Err(e) => {
                self.messages.push(ChatMessage {
                    role: "user".into(),
                    content: expanded,
                });
                self.push_assistant(format!("error: {e}"));
            }
        }
        let _ = self.persist();
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Build => "build",
        Mode::Plan => "plan",
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(12)]
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

/// Restores the terminal on drop so `?` / panic paths leave the TTY usable.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub fn run() -> io::Result<()> {
    use std::io::IsTerminal;
    if !io::stdin().is_terminal() {
        return Err(io::Error::other(
            "agent TUI requires a TTY (stdin is not a terminal)",
        ));
    }
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    let mut app = App::new();
    loop {
        terminal.draw(|f| ui(f, &app))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') if app.input.is_empty() => {
                    let _ = app.persist();
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
        if app.quit {
            return Ok(());
        }
    }
}
