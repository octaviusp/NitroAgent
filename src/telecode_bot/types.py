from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class MessageContext:
    chat_id: int
    user_id: int
    text: str
    message_id: int
    thread_id: int | None


@dataclass(frozen=True)
class ParsedCommand:
    name: str
    args: str


@dataclass(frozen=True)
class IncomingTask:
    message: MessageContext
    command: ParsedCommand | None


@dataclass
class ThreadState:
    thread_key: str
    active_engine: str
    workspace_path: Path
    active_session_id: str | None
    compact_summary: str | None
    settings: dict[str, str]

    @property
    def tool_mode(self) -> str:
        return self.settings.get("tool_mode", "safe")


@dataclass(frozen=True)
class RunContext:
    run_id: int
    status_message_id: int
    log_path: Path


@dataclass(frozen=True)
class RunResult:
    status: str
    output_tail: str
    session_id: str | None
    exit_code: int
