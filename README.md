<p align="center">
  <img src="icon.png" width="120" alt="NitroAgent">
</p>

<h1 align="center">NitroAgent</h1>

<p align="center">
  <b>Ultra-fast Telegram coding agent</b><br>
  Bridge Claude Code CLI to your phone. Send a message, get code back.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/rust-stable-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/platform-macOS-blue?logo=apple" alt="macOS">
  <img src="https://img.shields.io/badge/telegram-bot-26A5E4?logo=telegram" alt="Telegram">
</p>

---

## What is this?

NitroAgent turns your Telegram chat into a full coding environment. Send a text prompt, voice message, or photo — Claude Code executes it on your Mac and streams the result back in real-time with animated feedback.

**No servers. No cloud. Runs on your Mac, talks to your Telegram.**

## Features

- **Multi-agent** — run multiple bots from a single process, each with its own workspace and database
- **Group chat** — add bots to a Telegram group, @mention to route, bot-to-bot communication
- **Streaming output** with animated spinner and tool activity strip
- **Quick command menu** — persistent reply keyboard for one-tap access to common commands
- **Inline keyboards** — Cancel, Retry, New Session with one tap
- **Voice prompts** — speak your coding instructions (Whisper transcription)
- **Photo analysis** — send screenshots for Claude to analyze
- **Session memory** — resume conversations, compact memory across sessions
- **Shell access** — run commands remotely via `/bash`
- **Menu bar app** — start/stop/restart from your macOS menu bar

## Quick Start

### Single bot (simple)

```bash
# 1. Clone and build
git clone https://github.com/octaviusp/NitroAgent.git
cd NitroAgent/nitro-agent
cargo build --release

# 2. Configure
cp .env.example .env
# Edit .env — add your TELEGRAM_BOT_TOKEN and ALLOWED_TELEGRAM_USER_IDS

# 3. Run
./target/release/nitro-agent
```

### Multi-agent (multiple bots)

Run a swarm of specialized bots from a single process. Each bot gets its own workspace, database, and Telegram token.

```bash
# 1. Create bots via @BotFather on Telegram

# 2. Add tokens to .env
echo 'BOT_A_TOKEN=your_token_here' >> .env
echo 'BOT_B_TOKEN=your_token_here' >> .env

# 3. Create agents.toml
cp agents.toml.example agents.toml
```

```toml
# agents.toml
[defaults]
allowed_user_ids = "YOUR_TELEGRAM_USER_ID"
default_engine = "claude"
default_tool_mode = "safe"
bot_to_bot_max_turns = 5

[agent.assistant]
telegram_bot_token = "${BOT_A_TOKEN}"
workspace_root = "/path/to/project-a"

[agent.devops]
telegram_bot_token = "${BOT_B_TOKEN}"
workspace_root = "/path/to/project-b"
default_tool_mode = "full"
```

```bash
# 4. Run — both bots start as independent tokio tasks
./target/release/nitro-agent
```

Each agent auto-derives its database (`data/{name}.db`) and logs (`logs/{name}/`) from the agent name. Tokens use `${ENV_VAR}` syntax so secrets stay in `.env`, not in the TOML file.

If no `agents.toml` exists, NitroAgent falls back to single-bot mode using `.env` only — **zero breaking changes**.

### Group chat

Add multiple bots to a Telegram group for a swarm of specialized agents:

1. Add bots to a group
2. Set `allowed_group_ids` in `agents.toml` (or `ALLOWED_GROUP_IDS` in `.env`)
3. @mention a specific bot to route to it, or send an untagged message for any bot to respond
4. `bot_to_bot_max_turns` limits how many times bots can chain responses to each other (0 = unlimited)

### Run as background service (recommended)

```bash
cd nitro-agent
bash install-service.sh
```

Installs NitroAgent as a macOS `launchd` service. The bot starts automatically on login and restarts on crash — no GUI, no menu bar icon, zero overhead.

```bash
bash install-service.sh --status   # check if running
bash uninstall-service.sh          # stop and remove service
```

### Menu bar app (optional)

```bash
cd nitro-agent/menubar
bash install.sh
```

Red lightning bolt appears in your menu bar. Start/stop the bot, toggle auto-start on login, view logs.

## Commands

| Command | What it does |
|---------|-------------|
| `/new` | Fresh session |
| `/resume` | Resume previous session (tap to select) |
| `/compact` | Compress memory for long conversations |
| `/cancel` | Kill running process |
| `/restart` | Full reset |
| `/bash <cmd>` | Run shell command on your Mac |
| `/cd <path>` | Change working directory |
| `/context` | See token usage, cost, context fill |
| `/mode <safe\|full>` | Toggle tool permissions |

Or just **send any text** as a coding prompt. A persistent reply keyboard provides one-tap access to the most common commands.

## How It Works

```
You (Telegram) ──message──▶ NitroAgent (your Mac)
                                  │
                            ┌─────┴─────┐
                            ▼           ▼
                       Single bot   Multi-agent
                       (.env only)  (agents.toml)
                            │           │
                            ▼           ▼
                      1 polling    N polling loops
                        loop       (1 per agent)
                            │           │
                            └─────┬─────┘
                                  ▼
                            claude -p --stream-json
                                  │
                                  ▼
                            Streaming edits ──▶ You (Telegram)
                            ⠹ Running · #42 · 12s
                            📖→✏️→🔨→📖
```

1. Send a message from Telegram (text, voice, or photo)
2. NitroAgent pipes it to Claude Code CLI on your Mac
3. Streams output back with animated spinner + tool icons
4. Shows completion card with cost, duration, and action buttons

## Requirements

- macOS (Apple Silicon)
- Rust toolchain (`rustup`)
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and authenticated
- Telegram bot token from [@BotFather](https://t.me/BotFather)

## Configuration

### Single bot (`.env`)

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `TELEGRAM_BOT_TOKEN` | yes | — | Bot token from BotFather |
| `ALLOWED_TELEGRAM_USER_IDS` | yes | — | Comma-separated numeric IDs |
| `DEFAULT_TOOL_MODE` | no | `safe` | `safe` or `full` |
| `MAX_RUNTIME_SECONDS` | no | `1200` | Per-run timeout |
| `MAX_OUTPUT_CHARS` | no | `3500` | Max chars in streaming buffer |
| `ALLOWED_GROUP_IDS` | no | — | Comma-separated group chat IDs |
| `BOT_TO_BOT_MAX_TURNS` | no | `0` | Max bot-to-bot turns (0 = unlimited) |
| `SST_LANGUAGE` | no | `es` | Whisper language for voice |

### Multi-agent (`agents.toml`)

All `.env` variables can be set per-agent in `agents.toml`. See [`agents.toml.example`](nitro-agent/agents.toml.example) for the full reference.

## License

MIT
