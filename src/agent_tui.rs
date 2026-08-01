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

/// Which IdP was used for the pending PKCE flow.
#[cfg(feature = "oauth")]
#[derive(Debug, Clone)]
enum OAuthPendingKind {
    Openai,
    /// client_id required for token exchange
    Xai {
        client_id: String,
    },
}

#[cfg(feature = "oauth")]
#[derive(Clone)]
struct OAuthPending {
    verifier: String,
    state: String,
    kind: OAuthPendingKind,
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
    /// Pending browser PKCE (feature oauth).
    #[cfg(feature = "oauth")]
    oauth_pending: Option<OAuthPending>,
    /// Background loopback listener result channel.
    #[cfg(feature = "oauth")]
    oauth_listen_rx: Option<std::sync::mpsc::Receiver<Result<(String, String), String>>>,
    /// Background device-code poll result: (provider, credential).
    #[cfg(feature = "oauth")]
    oauth_device_rx:
        Option<std::sync::mpsc::Receiver<Result<(ProviderId, auth::Credential), String>>>,
}

impl App {
    fn new() -> Self {
        let mut creds = auth::load_all().unwrap_or_default();
        #[cfg(feature = "oauth")]
        {
            // Soft-refresh expired OpenAI tokens when a refresh_token is present.
            if let Some(c) = creds.get(&ProviderId::Openai).cloned()
                && auth::credential_expired(&c)
                && let Some(rt) = c.refresh_token.as_deref().filter(|s| !s.is_empty())
                && let Ok(fresh) = auth::openai_refresh_access_token(rt)
            {
                let _ = auth::save(ProviderId::Openai, fresh.clone());
                creds.insert(ProviderId::Openai, fresh);
            }
        }
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
            #[cfg(feature = "oauth")]
            oauth_pending: None,
            #[cfg(feature = "oauth")]
            oauth_listen_rx: None,
            #[cfg(feature = "oauth")]
            oauth_device_rx: None,
        }
    }

    fn help_text() -> &'static str {
        #[cfg(feature = "oauth")]
        {
            "/help /clear /connect /oauth /oauth-device [xai] /oauth-xai /oauth-wait /oauth-code /oauth-refresh /models /model <id> /mode /session /sessions /open <id> /delete <id> /new /init /q  \u{00b7}  !bash !read !write !ls  \u{00b7}  @file"
        }
        #[cfg(not(feature = "oauth"))]
        {
            "/help /clear /connect /models /model <id> /mode /session /sessions /open <id> /delete <id> /new /init /q  \u{00b7}  !bash !read !write !ls  \u{00b7}  @file"
        }
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
            #[cfg(feature = "oauth")]
            "oauth" => {
                let (url, verifier, state) =
                    auth::openai_browser_oauth_start(auth::OPENAI_LOOPBACK_REDIRECT);
                self.oauth_pending = Some(OAuthPending {
                    verifier,
                    state: state.clone(),
                    kind: OAuthPendingKind::Openai,
                });
                let open_note = auth::open_url_best_effort(&url)
                    .map(|_| "browser launch attempted".to_string())
                    .unwrap_or_else(|e| format!("browser open skipped ({e})"));
                self.push_assistant(format!(
                    "OpenAI browser PKCE started ({open_note}).\n1) open:\n{url}\n2a) /oauth-wait  (background listen :1455, 180s; TUI stays live)\n2b) or paste: /oauth-code <callback-url-or-code> [state]\nstate={state}\n3) headless: /oauth-device\nxAI: /oauth-xai if PLAZIR_XAI_OAUTH_CLIENT_ID set (host {})",
                    auth::xai_authorize_url_hint()
                ));
            }
            #[cfg(feature = "oauth")]
            "oauth-device" => {
                if self.oauth_device_rx.is_some() {
                    self.push_assistant("device poll already running");
                    return;
                }
                let want_xai = arg.eq_ignore_ascii_case("xai") || arg.eq_ignore_ascii_case("grok");
                if want_xai {
                    let Some(client_id) = std::env::var("PLAZIR_XAI_OAUTH_CLIENT_ID")
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                    else {
                        self.push_assistant(
                            "usage: /oauth-device xai  requires PLAZIR_XAI_OAUTH_CLIENT_ID",
                        );
                        return;
                    };
                    match auth::xai_request_device_code(&client_id) {
                        Ok(dc) => {
                            let verify = dc
                                .verification_uri_complete
                                .clone()
                                .unwrap_or_else(|| dc.verification_uri.clone());
                            let _ = auth::open_url_best_effort(&verify);
                            let interval = dc.interval.unwrap_or(5).max(1);
                            let expires = dc.expires_in.unwrap_or(600);
                            let device_code = dc.device_code.clone();
                            let cid = client_id.clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            std::thread::spawn(move || {
                                let r = auth::xai_poll_device_token(
                                    &cid,
                                    &device_code,
                                    interval,
                                    std::time::Duration::from_secs(expires.min(900)),
                                )
                                .map(|c| (ProviderId::Xai, c));
                                let _ = tx.send(r);
                            });
                            self.oauth_device_rx = Some(rx);
                            self.push_assistant(format!(
                                "xAI device code started.\nuser_code: {}\nvisit: {verify}\npolling every {interval}s (background)",
                                dc.user_code
                            ));
                        }
                        Err(e) => self.push_assistant(format!(
                            "xAI device code failed: {e}\n(override PLAZIR_XAI_DEVICE_URL)"
                        )),
                    }
                } else {
                    match auth::openai_request_device_code() {
                        Ok(dc) => {
                            let verify = dc
                                .verification_uri_complete
                                .clone()
                                .unwrap_or_else(|| dc.verification_uri.clone());
                            let _ = auth::open_url_best_effort(&verify);
                            let interval = dc.interval.unwrap_or(5).max(1);
                            let expires = dc.expires_in.unwrap_or(600);
                            let device_code = dc.device_code.clone();
                            let (tx, rx) = std::sync::mpsc::channel();
                            std::thread::spawn(move || {
                                let r = auth::openai_poll_device_token(
                                    &device_code,
                                    interval,
                                    std::time::Duration::from_secs(expires.min(900)),
                                )
                                .map(|c| (ProviderId::Openai, c));
                                let _ = tx.send(r);
                            });
                            self.oauth_device_rx = Some(rx);
                            self.push_assistant(format!(
                                "OpenAI device code started.\nuser_code: {}\nvisit: {verify}\npolling every {interval}s (background, max {expires}s)\n(tip: /oauth-device xai for Grok)",
                                dc.user_code
                            ));
                        }
                        Err(e) => self.push_assistant(format!(
                            "device code request failed: {e}\n(override PLAZIR_OPENAI_DEVICE_URL if IdP path differs)"
                        )),
                    }
                }
            }
            #[cfg(feature = "oauth")]
            "oauth-xai" => {
                let Some(client_id) = std::env::var("PLAZIR_XAI_OAUTH_CLIENT_ID")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                else {
                    self.push_assistant(
                        "set PLAZIR_XAI_OAUTH_CLIENT_ID for Grok browser PKCE (M11 partial)",
                    );
                    return;
                };
                let (url, verifier, state) =
                    auth::xai_browser_oauth_start(&client_id, auth::OPENAI_LOOPBACK_REDIRECT);
                self.oauth_pending = Some(OAuthPending {
                    verifier,
                    state: state.clone(),
                    kind: OAuthPendingKind::Xai {
                        client_id: client_id.clone(),
                    },
                });
                let open_note = auth::open_url_best_effort(&url)
                    .map(|_| "browser launch attempted".to_string())
                    .unwrap_or_else(|e| format!("browser open skipped ({e})"));
                self.push_assistant(format!(
                    "xAI browser PKCE started ({open_note}; token host auth.x.ai; override PLAZIR_XAI_TOKEN_URL).\n1) open:\n{url}\n2) /oauth-wait or /oauth-code\nstate={state}"
                ));
            }
            #[cfg(feature = "oauth")]
            "oauth-wait" => {
                if self.oauth_pending.is_none() {
                    self.push_assistant("no pending /oauth \u{2014} run /oauth first");
                    return;
                }
                if self.oauth_listen_rx.is_some() {
                    self.push_assistant("already listening \u{2014} finish browser login or wait");
                    return;
                }
                let bind = auth::bind_addr_from_redirect(auth::OPENAI_LOOPBACK_REDIRECT)
                    .unwrap_or_else(|_| auth::LOOPBACK_ADDR.to_string());
                let (tx, rx) = std::sync::mpsc::channel();
                let bind_t = bind.clone();
                std::thread::spawn(move || {
                    let r =
                        auth::wait_for_oauth_callback(&bind_t, std::time::Duration::from_secs(180));
                    let _ = tx.send(r);
                });
                self.oauth_listen_rx = Some(rx);
                self.push_assistant(format!(
                    "listening on http://{bind}/auth/callback (180s, background) \u{2014} TUI stays interactive"
                ));
            }
            #[cfg(feature = "oauth")]
            "oauth-code" => {
                if arg.is_empty() {
                    self.push_assistant(
                        "usage: /oauth-code <callback-url|code> [state]  (run /oauth first)",
                    );
                } else {
                    let Some(pending) = self.oauth_pending.clone() else {
                        self.push_assistant("no pending /oauth \u{2014} run /oauth first");
                        return;
                    };
                    let (code, state) = if arg.contains('=') || arg.contains('?') {
                        match auth::pkce::parse_oauth_callback_query(arg) {
                            Ok(pair) => pair,
                            Err(e) => {
                                self.push_assistant(e);
                                return;
                            }
                        }
                    } else {
                        let mut parts = arg.splitn(2, ' ');
                        let code = parts.next().unwrap_or("").trim().to_string();
                        let state = parts
                            .next()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .unwrap_or(pending.state.as_str())
                            .to_string();
                        if code.is_empty() {
                            self.push_assistant("missing code");
                            return;
                        }
                        (code, state)
                    };
                    if state != pending.state {
                        self.push_assistant(format!(
                            "state mismatch (got {state}, expected {})",
                            pending.state
                        ));
                        return;
                    }
                    self.finish_oauth_exchange(&code, &pending);
                }
            }
            #[cfg(feature = "oauth")]
            "oauth-refresh" => {
                let stored = auth::load_all().unwrap_or_default();
                let Some(cred) = stored.get(&ProviderId::Openai) else {
                    self.push_assistant("no saved openai credential \u{2014} /oauth first");
                    return;
                };
                let Some(rt) = cred.refresh_token.as_deref().filter(|s| !s.is_empty()) else {
                    self.push_assistant("openai credential has no refresh_token");
                    return;
                };
                match auth::openai_refresh_access_token(rt) {
                    Ok(new_cred) => {
                        let _ = auth::save(ProviderId::Openai, new_cred.clone());
                        match provider::client_for(ProviderId::Openai, &new_cred) {
                            Ok(client) => {
                                self.provider = Some((ProviderId::Openai, client));
                                self.refresh_status();
                                self.push_assistant("OpenAI token refreshed");
                            }
                            Err(e) => self.push_assistant(format!("client build failed: {e}")),
                        }
                    }
                    Err(e) => self.push_assistant(format!("refresh failed: {e}")),
                }
            }
            "model" => {
                if arg.is_empty() {
                    self.push_assistant("usage: /model <id>");
                } else if let Some((_, c)) = &mut self.provider {
                    c.model = arg.to_string();
                    self.refresh_status();
                    self.push_assistant(format!("model \u{2192} {arg}"));
                } else {
                    self.push_assistant("not connected. /connect first.");
                }
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
            "delete" => {
                if arg.is_empty() {
                    self.push_assistant("usage: /delete <session-id-or-prefix>");
                } else {
                    match SessionStore::load_by_prefix(arg) {
                        Ok(s) => {
                            let id = s.id.clone();
                            if id == self.session.id {
                                self.push_assistant(
                                    "cannot delete the active session \u{2014} /new first",
                                );
                            } else {
                                match SessionStore::delete(&id) {
                                    Ok(()) => self.push_assistant(format!("deleted {id}")),
                                    Err(e) => self.push_assistant(format!("delete failed: {e}")),
                                }
                            }
                        }
                        Err(e) => self.push_assistant(e),
                    }
                }
            }
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

    /// Non-blocking: complete background `/oauth-wait` when the listener reports.
    #[cfg(feature = "oauth")]
    fn poll_oauth_listen(&mut self) {
        let Some(rx) = &self.oauth_listen_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((code, state))) => {
                self.oauth_listen_rx = None;
                let Some(pending) = self.oauth_pending.clone() else {
                    self.push_assistant("callback received but no pending /oauth");
                    return;
                };
                if state != pending.state {
                    self.push_assistant(format!(
                        "state mismatch (got {state}, expected {})",
                        pending.state
                    ));
                    return;
                }
                self.finish_oauth_exchange(&code, &pending);
            }
            Ok(Err(e)) => {
                self.oauth_listen_rx = None;
                self.push_assistant(format!("oauth-wait: {e}"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.oauth_listen_rx = None;
                self.push_assistant("oauth-wait: listener dropped");
            }
        }
    }

    #[cfg(feature = "oauth")]
    fn poll_oauth_device(&mut self) {
        let Some(rx) = &self.oauth_device_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((id, cred))) => {
                self.oauth_device_rx = None;
                let _ = auth::save(id, cred.clone());
                match provider::client_for(id, &cred) {
                    Ok(client) => {
                        self.provider = Some((id, client));
                        self.refresh_status();
                        self.push_assistant(format!(
                            "{} device OAuth connected (tokens saved)",
                            id.as_str()
                        ));
                    }
                    Err(e) => self.push_assistant(format!("client build failed: {e}")),
                }
            }
            Ok(Err(e)) => {
                self.oauth_device_rx = None;
                self.push_assistant(format!("device OAuth failed: {e}"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.oauth_device_rx = None;
                self.push_assistant("device OAuth: poller dropped");
            }
        }
    }

    #[cfg(feature = "oauth")]
    fn finish_oauth_exchange(&mut self, code: &str, pending: &OAuthPending) {
        let result = match &pending.kind {
            OAuthPendingKind::Openai => {
                auth::openai_exchange_code(code, auth::OPENAI_LOOPBACK_REDIRECT, &pending.verifier)
            }
            OAuthPendingKind::Xai { client_id } => auth::xai_exchange_code(
                client_id,
                code,
                auth::OPENAI_LOOPBACK_REDIRECT,
                &pending.verifier,
            ),
        };
        match result {
            Ok(cred) => {
                let id = match &pending.kind {
                    OAuthPendingKind::Openai => ProviderId::Openai,
                    OAuthPendingKind::Xai { .. } => ProviderId::Xai,
                };
                let _ = auth::save(id, cred.clone());
                match provider::client_for(id, &cred) {
                    Ok(client) => {
                        self.provider = Some((id, client));
                        self.oauth_pending = None;
                        self.refresh_status();
                        self.push_assistant(format!(
                            "{} OAuth connected (tokens saved)",
                            id.as_str()
                        ));
                    }
                    Err(e) => self.push_assistant(format!("client build failed: {e}")),
                }
            }
            Err(e) => self.push_assistant(format!("token exchange failed: {e}")),
        }
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
        // !write path <<EOF body or !write path:content (single line)
        if let Some(rest) = text.strip_prefix("!write ") {
            let rest = rest.trim();
            if let Some((path, content)) = rest.split_once(':') {
                let r = run_tool(
                    Tool::Write {
                        path: path.trim().into(),
                        content: content.to_string(),
                    },
                    plan,
                );
                self.push_assistant(format!("write {}:\n{}", path.trim(), r.output));
                return true;
            }
            self.push_assistant("usage: !write path:content");
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
        #[cfg(feature = "oauth")]
        {
            app.poll_oauth_listen();
            app.poll_oauth_device();
        }
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
