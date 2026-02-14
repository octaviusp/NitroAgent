from pathlib import Path

from telecode_bot.config import BotConfig
from telecode_bot.engines import build_engine_command, parse_engine_stream_line


def make_config() -> BotConfig:
    return BotConfig(
        telegram_bot_token="t",
        allowed_user_ids={1},
        default_engine="claude",
        poll_timeout_seconds=25,
        stream_edit_interval_seconds=0.5,
        max_runtime_seconds=100,
        max_output_chars=3000,
        db_path=Path("data/test.db"),
        workspace_root=Path("workspaces"),
        logs_root=Path("logs"),
        claude_bin="claude",
        codex_bin="codex",
        claude_safe_allowed_tools=["Read", "Edit"],
        claude_full_allowed_tools=["Read", "Edit", "Bash"],
        default_tool_mode="safe",
    )


def test_build_claude_command_resume_and_seed() -> None:
    command, effective_prompt = build_engine_command(
        engine="claude",
        config=make_config(),
        prompt="hello",
        session_id="sid-1",
        tool_mode="safe",
        seed_summary="sum",
    )
    assert command[0] == "claude"
    assert "--resume" in command
    assert "sid-1" in command
    assert "--append-system-prompt" in command
    assert effective_prompt == "hello"


def test_build_codex_command_resume() -> None:
    command, effective_prompt = build_engine_command(
        engine="codex",
        config=make_config(),
        prompt="hello",
        session_id="sid-1",
        tool_mode="safe",
        seed_summary=None,
    )
    assert command[:3] == ["codex", "exec", "resume"]
    assert "sid-1" in command
    assert effective_prompt == "hello"


def test_parse_engine_stream_line_extracts_text_and_session() -> None:
    line = '{"session_id":"abc","message":{"content":[{"text":"hello"}]}}'
    text, session_id = parse_engine_stream_line("claude", line)
    assert "hello" in text
    assert session_id == "abc"


def test_parse_engine_stream_line_plain_text_fallback() -> None:
    text, session_id = parse_engine_stream_line("codex", "plain output")
    assert text == "plain output"
    assert session_id is None
