from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


def _load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        os.environ.setdefault(key, value)


def _parse_user_ids(value: str) -> set[int]:
    ids: set[int] = set()
    for part in value.split(","):
        chunk = part.strip()
        if not chunk:
            continue
        ids.add(int(chunk))
    return ids


def _parse_tools(value: str) -> list[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


@dataclass(frozen=True)
class BotConfig:
    telegram_bot_token: str
    allowed_user_ids: set[int]
    default_engine: str
    poll_timeout_seconds: int
    stream_edit_interval_seconds: float
    max_runtime_seconds: int
    max_output_chars: int
    db_path: Path
    workspace_root: Path
    logs_root: Path
    claude_bin: str
    codex_bin: str
    claude_safe_allowed_tools: list[str]
    claude_full_allowed_tools: list[str]
    default_tool_mode: str

    @classmethod
    def from_env(cls, cwd: Path) -> BotConfig:
        _load_dotenv(cwd / ".env")
        token = os.getenv("TELEGRAM_BOT_TOKEN", "").strip()
        if not token:
            raise ValueError("TELEGRAM_BOT_TOKEN is required")

        raw_ids = os.getenv("ALLOWED_TELEGRAM_USER_IDS", "").strip()
        if not raw_ids:
            raise ValueError("ALLOWED_TELEGRAM_USER_IDS is required")

        config = cls(
            telegram_bot_token=token,
            allowed_user_ids=_parse_user_ids(raw_ids),
            default_engine=os.getenv("DEFAULT_ENGINE", "claude").strip().lower(),
            poll_timeout_seconds=int(os.getenv("POLL_TIMEOUT_SECONDS", "25")),
            stream_edit_interval_seconds=float(os.getenv("STREAM_EDIT_INTERVAL_SECONDS", "0.7")),
            max_runtime_seconds=int(os.getenv("MAX_RUNTIME_SECONDS", "1200")),
            max_output_chars=int(os.getenv("MAX_OUTPUT_CHARS", "3500")),
            db_path=Path(os.getenv("DB_PATH", "data/telecode_bot.db")),
            workspace_root=Path(os.getenv("WORKSPACE_ROOT", "workspaces")),
            logs_root=Path(os.getenv("LOGS_ROOT", "logs")),
            claude_bin=os.getenv("CLAUDE_BIN", "claude").strip(),
            codex_bin=os.getenv("CODEX_BIN", "codex").strip(),
            claude_safe_allowed_tools=_parse_tools(
                os.getenv("CLAUDE_SAFE_ALLOWED_TOOLS", "Read,Edit,Bash")
            ),
            claude_full_allowed_tools=_parse_tools(os.getenv("CLAUDE_FULL_ALLOWED_TOOLS", "")),
            default_tool_mode=os.getenv("DEFAULT_TOOL_MODE", "safe").strip().lower(),
        )

        config.db_path.parent.mkdir(parents=True, exist_ok=True)
        config.workspace_root.mkdir(parents=True, exist_ok=True)
        config.logs_root.mkdir(parents=True, exist_ok=True)
        if config.default_engine not in {"claude", "codex"}:
            raise ValueError("DEFAULT_ENGINE must be claude or codex")
        if config.default_tool_mode not in {"safe", "full"}:
            raise ValueError("DEFAULT_TOOL_MODE must be safe or full")
        return config
