# plazir18 (agent-wall)

Multi-panel agent terminal dashboard in Rust (eframe/egui): one **panel per live
tmux session**, up to **18 concurrent** tiles in a grid. Capacity layout is
**6 columns × 3 rows**. Live pane tails, DPI-aware rendering, always-on-top
optional.

**tmux is a must** — sessions only get a tile if they run inside tmux
(`capture-pane` is the feed). Anything launched outside tmux is invisible
to the wall.

## Layout

**Default: multi-panel grid** (dashboard of terminals)

- Up to **18** concurrent panels (hard ceiling; no soft nudge below that).
- Adaptive columns: 1 / 2 / 3 / 4 / 6 cols by count.
- At capacity: 6 columns × 3 rows.
- Each panel: title + attach dot + last ~10 non-empty pane lines.
- Left-click → attach in terminal · right-click → kill.

**Legacy strip** (top bar of compact rows):

```bash
cargo run -- --strip
```

## Linux: clone, build, and install

```bash
git clone https://github.com/VeigaPunk/plazir18.git
cd plazir18
cargo build --release --locked
install -Dm755 target/release/agent-wall ~/.local/bin/plazir18
```

Left-click attach launches `alacritty`.

## Omarchy 3.8 launch

Launch the installed 18-panel dashboard through UWSM:

```bash
uwsm-app -- plazir18
```

## Flags

| Flag | Effect |
|------|--------|
| *(none)* | Multi-panel dashboard, max 18 tiles |
| `--strip` | Legacy horizontal strip |
| `--no-top` | Disable always-on-top |
| `--toggle` | Reveal/hide running instance (Hyprland special workspace or IPC) |
| `--tui` | Launch ratatui dashboard (terminal-based grid, one panel per pane) |
| `--status-json` | Print Waybar status line and exit |
| `--status-pango` | Print the multiline colored pane grid as Waybar JSON and exit |
| `--agent` | Minimal coding-agent TUI (requires Cargo feature `agent`) |

## Dual mode (wall + agent)

Default build is the **wall** only. Optional features:

| Feature | Enables |
|---------|---------|
| `agent` | `--agent` TUI, OpenAI-compatible providers, tools, sessions |
| `oauth` | OAuth helpers (PKCE + token exchange; `/oauth` `/oauth-code`; depends on `agent`) |
| `full` | `wall` + `agent` + `oauth` |

```bash
cargo build --release --locked --features agent
cargo run --features agent -- --agent
cargo test --features full --locked
```

Agent needs a provider key or local endpoint. `/connect` tries cloud keys first (Zen → OpenAI → XAI), then Local:

| Env | Role |
|-----|------|
| `OPENCODE_ZEN_API_KEY` / `PLAZIR_ZEN_KEY` | OpenCode Zen |
| `OPENAI_API_KEY` (+ optional remote `OPENAI_BASE_URL`) | OpenAI cloud |
| `XAI_API_KEY` / `GROK_API_KEY` | Grok |
| `PLAZIR_LOCAL_BASE` / `PLAZIR_LOCAL_KEY` | Local (Ollama, etc.) |
| `PLAZIR_LOCAL_MODEL` | Local chat model (default `llama3.2`; if missing from `/models`, first catalog id is used) |
| `PLAZIR_LOCAL=1\|true\|prefer` | Force Local-first |
| Loopback `OPENAI_BASE_URL` | Rejected by OpenAI path; handed to Local base |

Offline M6 units cover parse + session turn; live Ollama: `PLAZIR_LOCAL_MODEL=… cargo test --features agent -- --ignored live_local_chat`.

## Build & deploy (Windows)

```bash
# inside WSL — cross-compiles to Windows and installs into shell:startup
./deploy.sh
```

## Dev

Local gate matrix (matches CI + labrat):

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                              # 71 pass, 2 ignored (live tmux)
cargo clippy --features full --locked --all-targets -- -D warnings
cargo test --features full --locked              # 129 pass, 4 ignored (live tmux + ollama)
# rust-version = "1.85" (edition 2024); agent !bash tools timeout at 30s
cargo run --locked -- --status-json
cargo run --locked -- --status-pango
cargo run --locked              # multi-panel wall
cargo run --locked -- --strip   # strip mode
```

Homepage / marketplace brand: https://ds4cc.com/
