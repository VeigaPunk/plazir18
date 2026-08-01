# plazir18 agent-tui milestones (wwkd)

**Axes**: minimal clutter · auth completeness · agent workflows · preserve wall · titanium DNA · lean binary · ship velocity

| # | Milestone | Status | Owner |
|---|-----------|--------|-------|
| M0 | Branch + PLAN.md / AGENT_PLAN.md | **done** | team |
| M1 | `src/auth/` trait + Zen + Local + XDG store | **done** | Benjamin |
| M2 | Cargo feature `agent` + reqwest/tokio/toml | **done** | team |
| M3 | `provider::OpenAICompat` client (blocking) | **done** | Grok/team |
| M4 | Minimal agent TUI skeleton (`--agent`) | **done** | Grok/team |
| M5 | Wire `main.rs` `--agent`; keep `--tui` as wall | **done** | Grok/team |
| M6 | E2E local chat turn | pending | |
| M7 | `/connect` stub + Zen key path in TUI | partial (env) | |
| M8 | Session persistence + multi-session | pending | |
| M9 | Tools (bash, read) + plan/build toggle | partial (mode) | |
| M10 | ChatGPT device-code OAuth | pending | |
| M11 | Grok PKCE OAuth | pending | |
| M12 | `/init` AGENTS.md + `@file` + README polish | pending | |

## Acceptance

- M4/M5: `cargo run --features agent -- --agent` opens chat, `/help` works, `q` quits, `--tui` wall unchanged.
- M6: With Ollama, one message gets a reply.
- M12: Dual-mode documented; Zen/Local/ChatGPT/Grok reachable.

Godspeed.
