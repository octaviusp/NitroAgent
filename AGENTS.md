# NitroAgent — Agent Guide

## 1. What This Repository Is

A Rust Telegram bot that acts as a private execution bridge to Claude Code CLI (`claude -p`). Designed for a single trusted operator (or small allowlist), runs on the host machine, persists thread memory in SQLite, and streams model output back into Telegram with animated UI feedback.

## 2. Architecture

```
Telegram  ──getUpdates──▶  main.rs (poller + callback handler)
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
              → rolling buffer → animated spinner
              → tool strip → Telegram edits
              → inline keyboards
```

## 3. Source Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point: long-polling, message extraction, access control, callback query handler |
| `src/bot.rs` | Core: `process_task()`, `run_user_prompt()`, `execute_engine_stream()`, keyboards, Telegram helpers |
| `src/commands.rs` | Slash command handlers: `/start`, `/help`, `/new`, `/resume`, `/clear`, `/cd`, `/status`, `/context`, `/mcp`, `/skills`, `/tasks`, `/mode` |
| `src/config.rs` | `BotConfig` from env vars, claude/python binary resolution, tool list parsing |
| `src/db.rs` | SQLite: `threads`, `runs`, `session_history` tables via sqlx |
| `src/engine.rs` | `build_claude_command()`: args, stdin piping, session resume, tool allowlist |
| `src/format.rs` | Markdown-to-Telegram-HTML converter (pulldown-cmark), message splitting |
| `src/stream.rs` | `parse_stream_line()`: text deltas, session IDs, init metadata, usage/cost, tool names |
| `src/types.rs` | `ThreadKey`, `MessageContext`, `IncomingTask`, `ThreadState`, `RunResult`, `SessionMeta`, `UsageInfo` |
| `src/voice.rs` | `download_voice()`, `download_photo()`, `VoiceTranscriber` (persistent sst.py server) |
| `src/worker.rs` | `ThreadWorker` (per-thread task queue), `WorkerRegistry` (HashMap of workers) |
| `menubar/NitroBar.swift` | macOS menu bar app: start/stop/restart daemon, auto-start toggle, log viewer |

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
    voice_file_id: Option<String>,
    photo_file_id: Option<String>,
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
- **Callback query handler** for inline keyboard button presses
- **Access control** via `ALLOWED_TELEGRAM_USER_IDS` allowlist
- **Per-thread isolation** — one async worker queue per `chat:topic` key
- **Sequential execution** — one active run per thread, additional messages queued
- **Claude Code execution** — `claude -p -` with `stream-json` output, stdin prompt piping
- **Session resume** — `--resume <session_id>` across messages

### UI/UX
- **Animated braille spinner** during streaming (rotates every 0.7s)
- **Tool activity strip** — real-time emoji strip showing Claude's tool usage (📖→✏️→🔨)
- **Typing indicator** — "typing..." bubble, re-sent every 4s during execution
- **Reply-to threading** — bot responses linked to user messages
- **Inline keyboards** — Cancel during streaming, New/Retry/Restart on completion
- **Completion summary card** — cost, duration, context fill bar
- **Markdown rendering** — pulldown-cmark conversion to Telegram HTML
- **Message splitting** — automatic splitting at paragraph boundaries for >4000 chars
- **Silent streaming** — no notification during execution, audible on completion

### Input Handling
- **Text** — direct prompt to Claude
- **Voice/Audio** — download → persistent Whisper server → transcript as prompt
- **Photo** — download to workspace → prompt Claude with Read tool instruction
- **Captions** — photo captions used as prompt text

### Memory & Sessions
- **Session persistence** — active session IDs in SQLite, resume across messages
- **Compact** (`/compact`) — structured summary → store → seed fresh session
- **Session buttons** — `/resume` shows tappable inline buttons instead of text list

## 6. Persistence Model

SQLite at `data/nitro_agent.db`:

| Table | Purpose |
|-------|---------|
| `threads` | One row per thread key. Engine, workspace, session ID, compact summary, settings JSON |
| `runs` | One row per execution. Engine, timestamps, status, log path |
| `session_history` | Append-only session IDs per thread+engine (for `/resume`) |

## 7. Dependencies

```toml
teloxide = "0.13"        # Telegram Bot API
tokio = "1"              # Async runtime
sqlx = "0.8"             # SQLite (async)
serde = "1"              # Serialization
serde_json = "1"         # JSON parsing
pulldown-cmark = "0.12"  # Markdown parsing
tracing = "0.1"          # Structured logging
chrono = "0.4"           # Timestamps
dotenvy = "0.15"         # .env loading
```
