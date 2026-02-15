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

- **Streaming output** with animated spinner and tool activity strip
- **Inline keyboards** — Cancel, Retry, New Session with one tap
- **Voice prompts** — speak your coding instructions (Whisper transcription)
- **Photo analysis** — send screenshots for Claude to analyze
- **Session memory** — resume conversations, compact memory across sessions
- **Shell access** — run commands remotely via `/bash`
- **Menu bar app** — start/stop/restart from your macOS menu bar

## Quick Start

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

### Menu Bar App (optional)

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

Or just **send any text** as a coding prompt.

## How It Works

```
You (Telegram) ──message──▶ NitroAgent (your Mac)
                                  │
                                  ▼
                            claude -p --stream-json
                                  │
                                  ▼
                            Streaming edits ──▶ You (Telegram)
                            ⠹ Running · #42 · 12s
                            📖→✏️→🔨→📖
```

1. Send a message from Telegram
2. NitroAgent pipes it to Claude Code CLI on your Mac
3. Streams output back with animated spinner + tool icons
4. Shows completion card with cost, duration, and action buttons

## Requirements

- macOS (Apple Silicon)
- Rust toolchain (`rustup`)
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed and authenticated
- Telegram bot token from [@BotFather](https://t.me/BotFather)

## Configuration

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `TELEGRAM_BOT_TOKEN` | yes | — | Bot token from BotFather |
| `ALLOWED_TELEGRAM_USER_IDS` | yes | — | Your Telegram numeric ID |
| `DEFAULT_TOOL_MODE` | no | `safe` | `safe` or `full` |
| `MAX_RUNTIME_SECONDS` | no | `1200` | Per-run timeout |
| `SST_LANGUAGE` | no | `es` | Whisper language for voice |

## License

MIT
