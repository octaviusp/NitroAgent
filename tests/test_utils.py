from telecode_bot.utils import RollingBuffer, build_thread_key, parse_command, truncate_for_telegram


def test_build_thread_key_topic() -> None:
    assert build_thread_key(10, 55) == "chat:10:topic:55"


def test_build_thread_key_plain_chat() -> None:
    assert build_thread_key(10, None) == "chat:10"


def test_parse_command_with_bot_suffix() -> None:
    parsed = parse_command("/clear@MyBot")
    assert parsed is not None
    assert parsed.name == "clear"
    assert parsed.args == ""


def test_parse_command_with_args() -> None:
    parsed = parse_command("/resume abc-123")
    assert parsed is not None
    assert parsed.name == "resume"
    assert parsed.args == "abc-123"


def test_rolling_buffer_keeps_tail() -> None:
    buff = RollingBuffer(max_chars=5)
    buff.append("hello")
    buff.append("world")
    assert buff.value == "world"


def test_truncate_for_telegram() -> None:
    text = "x" * 5000
    out = truncate_for_telegram(text)
    assert len(out) <= 4096
    assert out.endswith("...<truncated>")
