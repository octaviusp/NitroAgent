# TeleCode Bot

Telegram bridge for Claude Code (`claude -p`) and Codex CLI (`codex exec`) with per-thread memory,
streaming message edits, resumable sessions, and local persistence.

## Features

- Telegram long polling (`getUpdates`) no inbound ports required
- Per-thread/topic memory keyed by `chat_id` + optional `message_thread_id`
- SQLite-backed thread/session/run store
- Streaming output via rolling `editMessageText` updates
- Engine resume support:
  - Claude: `--resume <session_id>`
  - Codex: `codex exec resume <session_id> ...`
- Commands:
  - `/new_thread`
  - `/resume <session_id>`
  - `/clear`
  - `/compact`
  - `/cancel`
  - `/engine <claude|codex>`
  - `/toolmode <safe|full>`
  - `/status`
  - `/publish <repo-name> [private|public]`

## Requirements

- macOS or Linux
- Python 3.11+
- Claude Code CLI installed and authenticated
- Codex CLI installed and authenticated
- GitHub CLI (`gh`) installed and authenticated for `/publish`
- Telegram bot token from BotFather

## Quick Start

```bash
cp .env.example .env
# edit .env with TELEGRAM_BOT_TOKEN and ALLOWED_TELEGRAM_USER_IDS

python3 -m venv .venv
source .venv/bin/activate
pip install -e .

telecode-bot
```

## Fast Start Checklist

- Add Telegram token and your numeric user ID into `.env`.
- Ensure `claude`, `codex`, and `gh` are authenticated in your shell.
- Start with `source .venv/bin/activate && telecode-bot`.
- In Telegram, run `/status`, then send plain text to begin coding.

## Configuration

Set values in `.env`:

- `TELEGRAM_BOT_TOKEN`: bot token
- `ALLOWED_TELEGRAM_USER_IDS`: comma-separated allowed Telegram numeric user IDs
- `DEFAULT_ENGINE`: `claude` or `codex`
- `CLAUDE_SAFE_ALLOWED_TOOLS`: comma list passed into `--allowedTools` in safe mode
- `DEFAULT_TOOL_MODE`: `safe` or `full`
- `WORKSPACE_ROOT`, `DB_PATH`, `LOGS_ROOT`
- `GH_TOKEN` or `GITHUB_TOKEN` for non-interactive GitHub operations

## Operational Notes

- One run at a time per thread key; later messages queue automatically.
- `/cancel` terminates the currently running subprocess.
- `/compact` is bot-level compaction:
  1. asks current engine for compact structured memory
  2. stores compact summary in DB
  3. resets session and seeds a fresh one
- Telegram output is kept under 4096 chars and continuously edited in place.

## launchd (macOS)

1. Copy plist:

```bash
mkdir -p ~/Library/LaunchAgents
cp deploy/launchd/com.octaviusp.telecode-bot.plist ~/Library/LaunchAgents/
```

2. Edit plist paths/env values if needed.

3. Load service:

```bash
launchctl unload ~/Library/LaunchAgents/com.octaviusp.telecode-bot.plist 2>/dev/null || true
launchctl load ~/Library/LaunchAgents/com.octaviusp.telecode-bot.plist
launchctl start com.octaviusp.telecode-bot
```

4. Logs:

- `~/Library/Logs/telecode-bot/stdout.log`
- `~/Library/Logs/telecode-bot/stderr.log`

## Security Defaults

- User allowlist enforced (`ALLOWED_TELEGRAM_USER_IDS`)
- Safe tool mode default for Claude
- Per-run timeout (`MAX_RUNTIME_SECONDS`)
- Run logs persisted for auditing
