# TeleCode Bot — Project Instructions

## What This Is

Rust-powered Telegram bot that bridges to Claude Code CLI (`claude -p`) for headless AI execution. Single-operator, local-first, SQLite-backed. The original Python bot was removed — this is Rust-only.

## Project Structure

```
rust-telecode-bot/
├── Cargo.toml              # Rust crate: teloxide, tokio, sqlx, serde, chrono
├── src/
│   ├── main.rs             # Entry: long-polling loop, message extraction, access control
│   ├── bot.rs              # Core logic: process_task, engine streaming, Telegram messaging
│   ├── commands.rs         # Slash command handlers (/start, /help, /status, /mcp, etc.)
│   ├── config.rs           # BotConfig from env, claude/python binary resolution
│   ├── db.rs               # SQLite: threads, runs, session_history tables
│   ├── engine.rs           # Claude CLI command builder (stream-json, stdin piping)
│   ├── stream.rs           # stream-json parser: text deltas, session IDs, metadata, usage
│   ├── types.rs            # Shared types: MessageContext, ThreadKey, RunResult, etc.
│   ├── voice.rs            # Voice download, photo download, VoiceTranscriber (sst.py server)
│   └── worker.rs           # Per-thread worker queues, subprocess lifecycle
├── menubar/
│   └── TeleCodeBar.swift   # macOS menu bar app for daemon start/stop/restart
├── docs/
│   └── CLAUDE_CODE_SPEC.md # Claude Code CLI stream-json format reference
├── data/                   # SQLite DB (telecode_bot.db)
├── logs/                   # Per-thread run logs
└── workspaces/             # Per-thread working directories for Claude execution
```

## Build & Run

```bash
cd rust-telecode-bot
cargo build --release
# Binary: target/release/rust-telecode-bot

# Requires .env with TELEGRAM_BOT_TOKEN and ALLOWED_TELEGRAM_USER_IDS
cp .env.example .env && $EDITOR .env
./target/release/rust-telecode-bot
```

## Architecture

### Message Flow
1. `main.rs` polls Telegram via `getUpdates` with offset tracking
2. Extracts text, voice `file_id`, or photo `file_id` from messages
3. Access control via `ALLOWED_TELEGRAM_USER_IDS`
4. Creates `IncomingTask` and dispatches to per-thread `WorkerRegistry`
5. Worker calls `BotCore::process_task()` which routes to:
   - Command handlers (commands.rs)
   - Voice transcription pipeline (download -> sst.py -> prompt)
   - Photo analysis pipeline (download to workspace -> instruct Claude to Read)
   - Plain text prompt execution

### Engine Execution
- `engine.rs` builds: `claude -p - --output-format stream-json --verbose --dangerously-skip-permissions`
- Prompt piped via stdin to avoid argument length limits
- `--resume <session_id>` for session continuity
- `--allowedTools` controlled by tool mode (safe/full)
- `--append-system-prompt` for compact memory injection

### Streaming
- `stream.rs` parses line-by-line JSON: text deltas, session IDs, init metadata, usage/cost
- `RollingBuffer` keeps last N chars for Telegram display
- `bot.rs` periodically edits the status message with latest output

### Persistence (SQLite)
- `threads`: thread_key, engine, workspace_path, active_session_id, compact_summary, settings
- `runs`: per-execution records (status, timestamps, log path)
- `session_history`: append-only session IDs for `/resume`

### Input Types
- **Text**: direct prompt to Claude
- **Voice/Audio**: download -> transcribe via persistent `sst.py` server -> use transcript as prompt
- **Photo**: download to workspace dir -> construct prompt with `[Attached image: ./{filename} — use the Read tool to view it]` -> Claude reads image via multimodal Read tool

## Telegram Commands

| Command | Description |
|---------|-------------|
| `/start` | Show bot info (model, version, permission mode) |
| `/help` | List all commands |
| `/new` | Fresh session (clear session + memory) |
| `/resume [id]` | Resume session or list recent sessions |
| `/clear` | Full reset (session + memory + new workspace) |
| `/compact` | Compress memory via structured summary |
| `/cancel` | Kill running subprocess |
| `/restart` | Kill process + clear session + clear cache |
| `/bash <cmd>` | Direct shell execution in workspace |
| `/cd [path]` | Show or change workspace directory |
| `/status` | Thread state overview |
| `/context` | Token usage, cost, context fill bar |
| `/mcp` | List MCP servers |
| `/skills` | List skills, slash commands, agents, plugins |
| `/tasks` | Recent run history |
| `/mode <safe\|full>` | Set Claude tool permissions |

## Key Config (env vars)

| Variable | Default | Purpose |
|----------|---------|---------|
| `TELEGRAM_BOT_TOKEN` | required | Telegram bot token |
| `ALLOWED_TELEGRAM_USER_IDS` | required | Comma-separated numeric IDs |
| `DEFAULT_TOOL_MODE` | `safe` | `safe` or `full` |
| `CLAUDE_SAFE_ALLOWED_TOOLS` | `Read,Edit,Bash` | Tools in safe mode |
| `MAX_RUNTIME_SECONDS` | `1200` | Per-run timeout |
| `MAX_OUTPUT_CHARS` | `3500` | Rolling buffer size |
| `STREAM_EDIT_INTERVAL_SECONDS` | `0.7` | Telegram edit frequency |
| `SST_LANGUAGE` | `es` | Whisper transcription language |
| `SST_ARCH` | `base` | Whisper model architecture |

## Dev Notes

- Only `claude` engine supported (Codex removed in Rust rewrite)
- Claude binary resolved to absolute path at startup (checks `which`, well-known paths)
- `sst.py` runs as persistent server (model stays loaded across transcriptions)
- Photo files persist in workspace as `_photo_{unique_id}.{ext}` — not auto-cleaned
- All subprocess env vars strip `CLAUDECODE` / `CLAUDE_CODE_ENTRYPOINT` to avoid nesting detection
- Telegram messages capped at 4000 chars with truncation
- Pre-commit: `cargo build --release` (no test suite yet in Rust crate)
