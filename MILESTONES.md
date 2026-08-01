# plazir18 agent-tui milestones (wwkd)

**Axes**: minimal clutter · auth completeness · agent workflows · preserve wall · titanium DNA · lean binary · ship velocity

| # | Milestone | Status | Owner |
|---|-----------|--------|-------|
| M0 | Branch + PLAN.md / AGENT_PLAN.md | **done** | team |
| M1 | `src/auth/` trait + Zen + Local + XDG store | **done** | Benjamin |
| M2 | Cargo feature `agent` + reqwest/tokio/toml | **done** (compiles under `--features agent/full`) | team |
| M3 | `provider::OpenAICompat` client (blocking) | **done** (scaffold; unit-light) | Grok/team |
| M4 | Minimal agent TUI skeleton (`--agent`) | **scaffolded** (compiles; needs real TTY) | Grok/team |
| M5 | Wire `main.rs` `--agent`; keep `--tui` as wall | **scaffolded** (flag + feature gate) | Grok/team |
| M6 | E2E local chat turn | pending | |
| M7 | `/connect` stub + Zen key path in TUI | partial (env; order Zen→OpenAI→xAI→Local) | |
| M8 | Session persistence + multi-session | partial (store wired; unique ids) | |
| M9 | Tools (bash, read) + plan/build toggle | partial (tools + mode; unit-tested) | |
| M10 | ChatGPT device-code OAuth | pending (feature flag deps only) | |
| M11 | Grok PKCE OAuth | pending (feature flag deps only) | |
| M12 | `/init` AGENTS.md + `@file` + README polish | partial (helpers + README dual-mode) | |

## Acceptance

- M4/M5: `cargo run --features agent -- --agent` opens chat, `/help` works, `q` quits, `--tui` wall unchanged.
- M6: With Ollama, one message gets a reply.
- M12: Dual-mode documented; Zen/Local/ChatGPT/Grok reachable.

Godspeed.
