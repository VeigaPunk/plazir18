# plazir18 Agent-TUI Plan

**Goal**: Evolve the Ratatui surface of plazir18 (agent-wall) into the best *minimal* open coding-agent TUI — a deliberate breed of [OpenCode](https://opencode.ai) UX + the unrestricted Godspeed / multi-agent philosophy of [codex-titanium](https://github.com/VeigaPunk/codex-titanium).

Keep clutter at OpenCode level: primary conversation, thin status, slash commands, almost no chrome.

## Dual-mode product

| Mode | Flag / entry | Purpose |
|------|--------------|---------|
| **Wall** (existing) | `--tui` or `--wall` | High-quality multi-pane live tmux monitor (vt100 / tui-term). Zero-CPU event-driven. Keep 100 % of current excellence. |
| **Agent** (new) | default or `--agent` | Minimal full-screen coding agent. Chat + tools + sessions. |

Wall remains the best way to watch many parallel agents (including ones launched by this binary or by titanium / xbrd). Agent mode is the place you *drive* work.

## Must-have capabilities

### Auth (pluggable)

- **ChatGPT / Codex OAuth** — official OpenAI device-code + browser PKCE flow used by Codex CLI and titanium. Store tokens securely (keyring preferred, fallback `~/.local/share/plazir18/auth.json`).
- **xAI Grok / SuperGrok OAuth** — PKCE against `auth.x.ai` / `accounts.x.ai` (same family as OpenCode Grok plugins and Musketeer adapter). Device-code for headless.
- **OpenCode Zen** — browser login at https://opencode.ai/auth (or console), paste API key; route to `https://opencode.ai/zen/v1/*`.
- **Local / OpenAI-compatible** — any base URL (Ollama `:11434`, vLLM, LM Studio, custom). Optional key. First-class routing preference for local.

All providers implement a common `AuthProvider` trait: `login()`, `refresh()`, `credentials()`, `models()`.

### Model router

Simple config (`~/.config/plazir18/config.toml` or project-local):

```toml
[default]
provider = "local"          # or "chatgpt" | "grok" | "zen"
model = "qwen2.5-coder:14b"

[providers.local]
base_url = "http://127.0.0.1:11434/v1"

[providers.zen]
# key loaded from auth store
```

Router picks endpoint + headers; falls back on rate-limit / 5xx if multiple configured.

### Agent workflows (OpenCode parity, minimal)

- Multi-session (list / switch / new) — lightweight, not full multi-agent orchestration yet.
- Plan vs Build modes (read-only exploration vs full tools). Toggle via Tab or `/mode`.
- Slash commands: `/connect`, `/models`, `/init` (write / update `AGENTS.md`), `/compact`, `/help`, `/quit`.
- `@file` fuzzy reference (inject file content).
- Inline tool results (bash, read, write, edit, grep, glob). Approval policy configurable (default inspired by titanium: `never` + full sandbox for power users; safer default available).
- Godspeed-style system prompt injection optional (Pareto iteration, no goal-aiming).
- Project context from `AGENTS.md` / `CLAUDE.md` / `.plazir/AGENTS.md`.

Later: spawn titanium / xbrd multi-agent workers and surface them on the Wall.

## Architecture (Rust-only, Ratatui)

```
src/
  auth/
    mod.rs          # AuthProvider trait + store
    chatgpt.rs      # OpenAI Codex OAuth (device + browser)
    grok.rs         # xAI SuperGrok PKCE
    zen.rs          # API-key after web login
    local.rs        # no-op / env key
  provider/
    mod.rs          # ChatClient trait (OpenAI-compatible)
    openai_compat.rs
    router.rs
  agent/
    mod.rs
    session.rs
    tools.rs        # bash, fs, etc. with policy
    loop.rs         # ReAct-style or tool-calling loop
  tui/
    wall.rs         # existing dashboard (moved)
    agent.rs        # new minimal chat UI
    widgets.rs      # shared status, input, message list
  ...
```

Dependencies to add (minimal):
- `reqwest` (or `ureq` for smaller) + `tokio`
- `serde` / `toml` already partial
- `keyring` (optional feature)
- `oauth2` or hand-rolled PKCE (prefer small)
- Keep `ratatui` + `crossterm` + `tui-term` / `vt100`

No heavy agent frameworks. Immediate-mode Ratatui state machine.

## First-mile commits (this branch)

1. **This PLAN.md** + branch `feat/agent-tui`.
2. Scaffold empty `src/auth/` + `src/provider/` + `src/agent/` modules and `Cargo.toml` feature flags (`agent`, `oauth`).
3. Minimal `agent_tui` that can open, show status bar (“no provider — /connect”), accept `/help` and quit.
4. Implement local OpenAI-compatible client + one working chat turn.
5. Add Zen key flow (simplest OAuth-adjacent).
6. ChatGPT device-code OAuth.
7. Grok PKCE.
8. Tool surface + plan/build.
9. Wire Wall as side panel or separate binary mode.
10. Config, AGENTS.md, polish, tests.

## Non-goals (for v0)

- Full MCP server surface (titanium deliberately stripped it).
- Desktop/IDE extensions.
- Cloud sync / shareable session links (OpenCode has them; we can add later).
- Heavy retained-mode UI or React-like frameworks on Ratatui.

## Success metric

A developer can:

```bash
cargo install --path . --features agent
plazir18          # or plazir18 --agent
/connect          # pick local / zen / chatgpt / grok
/models
> fix the flaky test in @src/auth/chatgpt.rs
```

…and get a clean, fast, minimal TUI experience that feels like OpenCode while running unrestricted titanium-style defaults when desired, with the world’s best live agent wall one flag away.

Homepage brand remains https://ds4cc.com/.
