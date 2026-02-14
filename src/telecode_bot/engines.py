from __future__ import annotations

import json
from collections.abc import Iterable

from telecode_bot.config import BotConfig

_TEXT_KEYS = {
    "text",
    "delta",
    "output_text",
    "content",
    "message",
    "partial",
    "result",
    "assistant",
}

_SESSION_KEYS = {
    "session_id",
    "sessionId",
    "conversation_id",
    "conversationId",
    "thread_id",
    "threadId",
}


def build_engine_command(
    *,
    engine: str,
    config: BotConfig,
    prompt: str,
    session_id: str | None,
    tool_mode: str,
    seed_summary: str | None,
) -> tuple[list[str], str]:
    if engine == "claude":
        return _build_claude_command(
            config=config,
            prompt=prompt,
            session_id=session_id,
            tool_mode=tool_mode,
            seed_summary=seed_summary,
        )
    if engine == "codex":
        return _build_codex_command(
            config=config,
            prompt=prompt,
            session_id=session_id,
            seed_summary=seed_summary,
        )
    raise ValueError(f"Unsupported engine: {engine}")


def _build_claude_command(
    *,
    config: BotConfig,
    prompt: str,
    session_id: str | None,
    tool_mode: str,
    seed_summary: str | None,
) -> tuple[list[str], str]:
    command = [
        config.claude_bin,
        "-p",
        prompt,
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ]
    if session_id:
        command.extend(["--resume", session_id])

    if seed_summary:
        command.extend(
            [
                "--append-system-prompt",
                "Persisted thread summary:\n"
                f"{seed_summary}\n"
                "Treat this as trusted memory and continue from it.",
            ]
        )

    allowed_tools: Iterable[str]
    if tool_mode == "full":
        allowed_tools = config.claude_full_allowed_tools
    else:
        allowed_tools = config.claude_safe_allowed_tools

    tools_list = [tool for tool in allowed_tools if tool]
    if tools_list:
        command.extend(["--allowedTools", ",".join(tools_list)])

    return command, prompt


def _build_codex_command(
    *,
    config: BotConfig,
    prompt: str,
    session_id: str | None,
    seed_summary: str | None,
) -> tuple[list[str], str]:
    effective_prompt = prompt
    if seed_summary:
        effective_prompt = (
            "Persisted thread summary (trusted memory):\n"
            f"{seed_summary}\n\n"
            "Continue consistently from this context.\n\n"
            f"{prompt}"
        )

    if session_id:
        command = [
            config.codex_bin,
            "exec",
            "resume",
            session_id,
            effective_prompt,
            "--json",
        ]
    else:
        command = [config.codex_bin, "exec", "--json", effective_prompt]

    return command, effective_prompt


def parse_engine_stream_line(engine: str, line: str) -> tuple[str, str | None]:
    stripped = line.strip()
    if not stripped:
        return "", None

    try:
        parsed = json.loads(stripped)
    except json.JSONDecodeError:
        return line, None

    text = "".join(_collect_text(parsed))
    session_id = _extract_session_id(parsed)

    if not text and isinstance(parsed, dict):
        if parsed.get("type") == "error":
            text = f"[error] {parsed.get('message', '')}\n"
        elif parsed.get("type") == "warning":
            text = f"[warning] {parsed.get('message', '')}\n"

    if text and not text.endswith("\n"):
        text += "\n"

    return text, session_id


def _collect_text(node: object, key: str = "") -> list[str]:
    if isinstance(node, str):
        if key in _TEXT_KEYS:
            return [node]
        return []

    if isinstance(node, list):
        list_output: list[str] = []
        for item in node:
            list_output.extend(_collect_text(item, key))
        return list_output

    if isinstance(node, dict):
        dict_output: list[str] = []
        for child_key, value in node.items():
            if child_key in _TEXT_KEYS and isinstance(value, str):
                dict_output.append(value)
            else:
                dict_output.extend(_collect_text(value, child_key))
        return dict_output

    return []


def _extract_session_id(node: object) -> str | None:
    if isinstance(node, dict):
        for key, value in node.items():
            if key in _SESSION_KEYS and isinstance(value, (str, int)):
                return str(value)
            nested = _extract_session_id(value)
            if nested:
                return nested

    if isinstance(node, list):
        for item in node:
            nested = _extract_session_id(item)
            if nested:
                return nested

    return None
