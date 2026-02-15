# TeleCode Bot — Agent Guide

## 1. What This Repository Is

A Rust Telegram bot that acts as a private execution bridge to Claude Code CLI (`claude -p`). Designed for a single trusted operator (or small allowlist), runs on the host machine, persists thread memory in SQLite, and streams model output back into Telegram by continuously editing one status message.

The original Python implementation was removed (commit `97c2517`). This is a **Rust-only codebase**.

## 2. Architecture

```
Telegram  ──getUpdates──▶  main.rs (poller)
                              │
                    ┌─────────┼──────────┐
                    ▼         ▼          ▼
                  text      voice      photo
                    │         │          │
                    │    download +   download to
                    │    transcribe   workspace
                    │    (sst.py)       │
                    │         │     construct Read
                    │         ▼     tool prompt
                    │      prompt      │
                    ▼         ▼        ▼
              ┌──────────────────────────┐
              │  WorkerRegistry          │
              │  (per-thread queues)     │
              └──────────┬───────────────┘
                         ▼
              ┌──────────────────────────┐
              │  BotCore::process_task() │
              │  ├── command handlers    │
              │  └── engine execution    │
              └──────────┬───────────────┘
                         ▼
              ┌──────────────────────────┐
              │  claude -p - (subprocess)│
              │  --output-format         │
              │    stream-json           │
              │  --resume <session_id>   │
              └──────────┬───────────────┘
                         ▼
              stream.rs parses JSON lines
              → rolling buffer → Telegram edits
```

## 3. Source Files

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | ~210 | Entry point: long-polling, message extraction (text/voice/photo), access control, task dispatch |
| `src/bot.rs` | ~1050 | Core: `process_task()`, `run_user_prompt()`, `execute_engine_stream()`, `/compact`, `/bash`, Telegram helpers |
| `src/commands.rs` | ~870 | All slash command handlers: `/start`, `/help`, `/new`, `/resume`, `/clear`, `/cd`, `/status`, `/context`, `/mcp`, `/skills`, `/tasks`, `/mode` |
| `src/config.rs` | ~230 | `BotConfig` from env vars, claude/python binary resolution, tool list parsing |
| `src/db.rs` | ~370 | SQLite: `threads`, `runs`, `session_history` tables via sqlx |
| `src/engine.rs` | ~60 | `build_claude_command()`: args, stdin piping, session resume, tool allowlist |
| `src/stream.rs` | ~380 | `parse_stream_line()`: text deltas, session IDs, init metadata, usage/cost extraction, `RollingBuffer` |
| `src/types.rs` | ~160 | `ThreadKey`, `MessageContext`, `IncomingTask`, `ThreadState`, `RunResult`, `SessionMeta`, `UsageInfo` |
| `src/voice.rs` | ~270 | `download_voice()`, `download_photo()`, `VoiceTranscriber` (persistent sst.py server), `check_ffmpeg()` |
| `src/worker.rs` | ~145 | `ThreadWorker` (per-thread task queue), `WorkerRegistry` (HashMap of workers) |
| `menubar/TeleCodeBar.swift` | ~240 | macOS menu bar app: start/stop/restart daemon, auto-start toggle, log viewer |

## 4. Key Types

```rust
// Thread isolation key: "chat:{id}" or "chat:{id}:topic:{tid}"
struct ThreadKey(String);

// Extracted from Telegram message in poller
struct MessageContext {
    chat_id: i64,
    user_id: u64,
    text: String,
    message_id: i32,
    thread_id: Option<i64>,
    voice_file_id: Option<String>,   // voice/audio attachment
    photo_file_id: Option<String>,   // photo attachment
}

// Queued for worker
struct IncomingTask {
    message: MessageContext,
    command: Option<ParsedCommand>,
}

// Persisted in SQLite
struct ThreadState {
    thread_key: String,
    active_engine: String,
    workspace_path: PathBuf,
    active_session_id: Option<String>,
    compact_summary: Option<String>,
    settings: ThreadSettings,       // { tool_mode: "safe"|"full" }
}
```

## 5. Features

### Core
- **Telegram long-poll** (`getUpdates` with offset tracking, webhook cleared at startup)
- **Access control** via `ALLOWED_TELEGRAM_USER_IDS` allowlist
- **Per-thread isolation** — one async worker queue per `chat:topic` key
- **Sequential execution** — one active run per thread, additional messages queued
- **Claude Code execution** — `claude -p -` with `stream-json` output, stdin prompt piping
- **Session resume** — `--resume <session_id>` across messages
- **Streaming response edits** — rolling buffer, periodic Telegram `editMessageText`

### Input Handling
- **Text** — direct prompt to Claude
- **Voice/Audio** — download via Telegram API → persistent `sst.py` Whisper server → transcript as prompt
- **Photo** — download to workspace → prompt Claude with Read tool instruction for multimodal analysis
- **Captions** — photo captions used as prompt text; defaults to "Describe and analyze this image"

### Memory & Sessions
- **Session persistence** — active session IDs in SQLite, resume across messages
- **Compact** (`/compact`) — ask Claude for structured summary → store → clear session → seed fresh session
- **Seed injection** — compact summary injected via `--append-system-prompt` on first turn of new session

### Introspection
- **`/context`** — token usage, cost, context fill bar (from `stream-json` metadata)
- **`/mcp`** — list connected MCP servers with status
- **`/skills`** — skills, slash commands, agents, plugins from Claude Code init event
- **`/tasks`** — recent run history with status, duration, time ago

### Shell Access
- **`/bash <cmd>`** — direct shell execution in thread workspace (120s timeout, 100KB output limit)
- **`/cd [path]`** — show or change workspace directory (persisted in SQLite)

### macOS Integration
- **TeleCodeBar.swift** — menu bar app for daemon management (start/stop/restart, auto-start via launchd, log viewer)

## 6. Persistence Model

SQLite at `data/telecode_bot.db`:

| Table | Purpose |
|-------|---------|
| `threads` | One row per thread key. Engine, workspace, session ID, compact summary, settings JSON |
| `runs` | One row per execution. Engine, timestamps, status (`running`/`succeeded`/`failed`/`canceled`), log path |
| `session_history` | Append-only session IDs per thread+engine (for `/resume` suggestions) |

## 7. Engine Details

Claude CLI invocation:
```
claude -p - --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions
  [--resume <session_id>]
  [--append-system-prompt "Persisted thread summary: ..."]
  [--allowedTools Read,Edit,Bash]
```

- Prompt piped via stdin (`-p -`) to avoid argument length limits
- Tool allowlist depends on mode: `safe` → `CLAUDE_SAFE_ALLOWED_TOOLS`, `full` → `CLAUDE_FULL_ALLOWED_TOOLS`
- Environment stripped of `CLAUDECODE`, `CLAUDE_CODE_ENTRYPOINT` to avoid nesting detection
- Working directory set to thread workspace

## 8. Voice Transcription

- `VoiceTranscriber` spawns `sst.py --server` as a persistent child process
- Model loaded once, processes files via stdin/stdout JSON protocol
- Voice files downloaded to temp dir (auto-cleaned via `TempFileGuard` RAII)
- Requires: `ffmpeg` on PATH, `sst.py` script, Python with whisper

## 9. Photo Analysis

- Photos downloaded to workspace directory (not temp) so Claude Code can access them
- Filename: `_photo_{unique_id}.{ext}` (extension inferred from Telegram file path)
- Prompt constructed as: `[Attached image: ./{filename} — use the Read tool to view it]\n\n{caption}`
- Claude Code's Read tool natively handles images (multimodal)
- Files persist in workspace — not auto-cleaned

## 10. Environment Variables

### Required
| Variable | Purpose |
|----------|---------|
| `TELEGRAM_BOT_TOKEN` | Telegram bot token from BotFather |
| `ALLOWED_TELEGRAM_USER_IDS` | Comma-separated numeric Telegram user IDs |

### Optional
| Variable | Default | Purpose |
|----------|---------|---------|
| `DEFAULT_ENGINE` | `claude` | Only `claude` supported |
| `DEFAULT_TOOL_MODE` | `safe` | `safe` or `full` |
| `CLAUDE_BIN` | `claude` | Path to claude binary |
| `CLAUDE_SAFE_ALLOWED_TOOLS` | `Read,Edit,Bash` | Tools in safe mode |
| `CLAUDE_FULL_ALLOWED_TOOLS` | (empty = all) | Tools in full mode |
| `POLL_TIMEOUT_SECONDS` | `25` | Long-poll timeout |
| `STREAM_EDIT_INTERVAL_SECONDS` | `0.7` | Telegram edit interval |
| `MAX_RUNTIME_SECONDS` | `1200` | Per-run hard timeout |
| `MAX_OUTPUT_CHARS` | `3500` | Rolling buffer size |
| `DB_PATH` | `data/telecode_bot.db` | SQLite path |
| `WORKSPACE_ROOT` | `workspaces` | Thread workspace root |
| `LOGS_ROOT` | `logs` | Run log root |
| `SST_PYTHON` | auto-detect | Python binary for sst.py |
| `SST_SCRIPT` | `../sst.py` | Path to transcription script |
| `SST_LANGUAGE` | `es` | Whisper language |
| `SST_ARCH` | `base` | Whisper model size |

## 11. Commit History (feature branch)

| Commit | Type | Description |
|--------|------|-------------|
| `c58ffbf` | FEAT | Photo support via Claude Code Read tool |
| `ffd3fa3` | FIX | Resolve claude binary to absolute path at startup |
| `9ae888c` | FEAT | macOS menu bar app for daemon management |
| `12e9b17` | FEAT | `/bash` and `/cd` commands for remote shell control |
| `48a7980` | FIX | Harden voice transcription for production |
| `8a09f1e` | FEAT | Voice message transcription via sst.py |
| `998658e` | FIX | Accumulate usage across runs, fix context fill bar |
| `01c9f63` | FEAT | Capture all Claude CLI stream-json metadata |
| `e6d9263` | FEAT | `/mcp`, `/skills`, `/context`, `/tasks`, `/restart` commands |
| `97c2517` | REFACTOR | Remove Python bot, keep Rust-only |
| `ca89b3b` | FEAT | Rust-powered Telegram bot for Claude Code |
| `35a3778` | FEAT | Initial telegram claude-codex bridge bot |

## 12. Dependencies

```toml
teloxide = "0.13"    # Telegram Bot API
tokio = "1"          # Async runtime
sqlx = "0.8"         # SQLite (async)
serde = "1"          # Serialization
serde_json = "1"     # JSON parsing
tracing = "0.1"      # Structured logging
chrono = "0.4"       # Timestamps
dotenvy = "0.15"     # .env loading
```

## 13. Known Constraints

- No test suite in Rust crate yet (Python tests were removed with the Python bot)
- Photo files accumulate in workspace dirs — no cleanup mechanism
- No webhook mode — polling only
- No migration framework for SQLite schema changes
- Claude binary must be installed and authenticated on host
- `sst.py` + ffmpeg required for voice (graceful degradation: warns at startup if missing)
