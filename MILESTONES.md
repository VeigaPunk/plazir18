# plazir18 agent-tui milestones (wwkd)

**Axes**: minimal clutter · auth completeness · agent workflows · preserve wall · titanium DNA · lean binary · ship velocity

| # | Milestone | Status | Owner |
|---|-----------|--------|-------|
| M0 | Branch + PLAN.md / AGENT_PLAN.md | **done** | team |
| M1 | `src/auth/` trait + Zen + Local + XDG store | **this commit** | Benjamin |
| M2 | Cargo feature `agent` + reqwest/tokio/toml | pending | |
| M3 | `provider::OpenAICompat` client (sync or async) | pending | |
| M4 | Minimal agent TUI skeleton (`--agent`) | pending | |
| M5 | Wire `main.rs` `--agent`; keep `--tui` as wall | pending | |
| M6 | E2E local chat turn | pending | |
| M7 | `/connect` stub + Zen key path in TUI | pending | |
| M8 | Session persistence + multi-session | pending | |
| M9 | Tools (bash, read) + plan/build toggle | pending | |
| M10 | ChatGPT device-code OAuth | pending | |
| M11 | Grok PKCE OAuth | pending | |
| M12 | `/init` AGENTS.md + `@file` + README polish | pending | |

## Acceptance per milestone

- M1: `cargo test` passes on auth module; no change to wall binary behaviour.
- M4: `plazir18 --agent` opens a blank chat, `/help` works, `q` quits, wall still works.
- M6: With Ollama running, one user message yields a streamed or blocking reply.
- M12: Documented dual-mode install; dual OAuth + local + Zen all reachable from `/connect`.

Godspeed.
