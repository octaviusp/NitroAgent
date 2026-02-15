# NitroAgent

Ultra-fast optimized macOS remote-agent to code with Telegram. Rust-powered bridge to Claude Code CLI with streaming output, inline keyboards, and rich UI.

## Features

- Telegram long polling (`getUpdates`) — no inbound ports required
- Per-thread/topic isolation keyed by `chat_id` + optional `topic_id`
- SQLite-backed thread/session/run persistence
- Streaming output via animated spinner + rolling `editMessageText` updates
- Real-time tool activity strip (📖→✏️→🔨) shows what Claude is doing
- Inline keyboards: Cancel during execution, New/Retry/Restart on completion
- Session resume with tappable inline buttons
- Markdown-to-HTML rendering for final output
- Message splitting for long responses (>4000 chars)
- Voice/audio transcription via Whisper (sst.py)
- Photo analysis via Claude Code multimodal Read tool
- macOS menu bar app for daemon management

## Requirements

- macOS (Apple Silicon)
- Rust toolchain
- Claude Code CLI installed and authenticated
- Telegram bot token from BotFather

## Quick Start

```bash
cd nitro-agent
cargo build --release

cp .env.example .env
# edit .env with TELEGRAM_BOT_TOKEN and ALLOWED_TELEGRAM_USER_IDS

./target/release/nitro-agent
```

## Menu Bar App

```bash
cd nitro-agent/menubar
bash install.sh
```

From the menu bar you can start/stop/restart the bot, toggle auto-start on login, and view logs.

## Commands

| Command | Description |
|---------|-------------|
| `/start` | Show bot info |
| `/help` | List all commands |
| `/new` | Fresh session |
| `/resume [id]` | Resume session (inline buttons) |
| `/clear` | Full reset |
| `/compact` | Compress memory |
| `/cancel` | Kill running process |
| `/restart` | Reset Claude Code |
| `/bash <cmd>` | Run shell command |
| `/cd [path]` | Show or change workspace |
| `/status` | Thread state |
| `/context` | Token usage + cost |
| `/mcp` | MCP servers |
| `/skills` | Available skills |
| `/tasks` | Recent runs |
| `/mode <safe\|full>` | Tool permissions |

## Configuration

Set values in `.env`:

| Variable | Default | Purpose |
|----------|---------|---------|
| `TELEGRAM_BOT_TOKEN` | required | Telegram bot token |
| `ALLOWED_TELEGRAM_USER_IDS` | required | Comma-separated numeric IDs |
| `DEFAULT_TOOL_MODE` | `safe` | `safe` or `full` |
| `MAX_RUNTIME_SECONDS` | `1200` | Per-run timeout |
| `STREAM_EDIT_INTERVAL_SECONDS` | `0.7` | Telegram edit frequency |

## Security

- User allowlist enforced (`ALLOWED_TELEGRAM_USER_IDS`)
- Safe tool mode by default (restricted Claude tool access)
- Per-run timeout with automatic process termination
- Run logs persisted for auditing
