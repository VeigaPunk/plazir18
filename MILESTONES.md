# plazir18 agent-tui milestones (wwkd)

**Axes**: minimal clutter · auth completeness · agent workflows · preserve wall · titanium DNA · lean binary · ship velocity

| # | Milestone | Status | Owner |
|---|-----------|--------|-------|
| M0 | Branch + PLAN.md / AGENT_PLAN.md | **done** | team |
| M1 | `src/auth/` trait + Zen + Local + XDG store | **done** | Benjamin |
| M2 | Cargo feature `agent` + reqwest/tokio/toml | **done** (compiles under `--features agent/full`) | team |
| M3 | `provider::OpenAICompat` client (blocking) | **done** (scaffold; unit-light) | Grok/team |
| M4 | Minimal agent TUI skeleton (`--agent`) | **scaffolded** (compiles; stdin TTY guard; non-TTY clean fail) | Grok/team |
| M5 | Wire `main.rs` `--agent`; keep `--tui` as wall | **scaffolded** (flag + feature gate; `--tui` TTY guard) | Grok/team |
| M6 | E2E local chat turn | partial (`--agent-once`; SSE stream parse; 401 refresh; live Ollama ignore probes) | |
| M7 | `/connect` + env providers | partial (Zen→OpenAI→xAI→Local; empty-key fallthrough; loopback→Local handoff; compose unit) | |
| M8 | Session persistence + multi-session | partial (store; unique ids; `/open` `/session <id>`; `/delete <id>`) | |
| M9 | Tools (bash, read) + plan/build toggle | partial (`!bash`/`!read`/`!write`/`!edit path:old=>new`; plan blocks writes) | |
| M10 | ChatGPT device-code OAuth | partial (PKCE; loopback; `/oauth-device`; soft refresh on start; skip empty store rows) | |
| M11 | Grok PKCE OAuth | partial (`/oauth-xai`; device; `xai_refresh_access_token`; 401 mid-chat refresh; URL overrides) | |
| M12 | `/init` AGENTS.md + `@file` + README polish | partial (helpers + README dual-mode env table) | |

## Acceptance

- M4/M5: `cargo run --features agent -- --agent` opens chat, `/help` works, `q` quits, `--tui` wall unchanged.
- M6: With Ollama, one message gets a reply.
- M12: Dual-mode documented; Zen/Local/ChatGPT/Grok reachable.

Godspeed.
