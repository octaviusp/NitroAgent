from __future__ import annotations

import re
from datetime import UTC, datetime

from telecode_bot.types import ParsedCommand

_THREAD_PATTERN = re.compile(r"[^a-zA-Z0-9._-]+")


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat(timespec="seconds")


def build_thread_key(chat_id: int, thread_id: int | None) -> str:
    if thread_id is None:
        return f"chat:{chat_id}"
    return f"chat:{chat_id}:topic:{thread_id}"


def thread_workspace_slug(thread_key: str) -> str:
    return _THREAD_PATTERN.sub("_", thread_key)


def parse_command(text: str) -> ParsedCommand | None:
    if not text.startswith("/"):
        return None
    parts = text.split(maxsplit=1)
    command_token = parts[0][1:]
    if not command_token:
        return None
    command = command_token.split("@", maxsplit=1)[0].lower()
    args = parts[1].strip() if len(parts) > 1 else ""
    return ParsedCommand(name=command, args=args)


def compact_whitespace(value: str) -> str:
    return re.sub(r"\s+", " ", value).strip()


def truncate_for_telegram(text: str, max_len: int = 4096) -> str:
    if len(text) <= max_len:
        return text
    suffix = "\n...<truncated>"
    keep = max_len - len(suffix)
    if keep <= 0:
        return text[:max_len]
    return text[:keep] + suffix


class RollingBuffer:
    def __init__(self, max_chars: int) -> None:
        self.max_chars = max_chars
        self._value = ""

    @property
    def value(self) -> str:
        return self._value

    def append(self, text: str) -> None:
        if not text:
            return
        self._value += text
        if len(self._value) > self.max_chars:
            self._value = self._value[-self.max_chars :]
