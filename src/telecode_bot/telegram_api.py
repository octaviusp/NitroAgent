from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


class TelegramApiError(RuntimeError):
    pass


@dataclass(frozen=True)
class TelegramMessageRef:
    chat_id: int
    message_id: int


class TelegramAPI:
    def __init__(self, token: str) -> None:
        self.base_url = f"https://api.telegram.org/bot{token}"

    def _call(self, method: str, payload: dict[str, Any], timeout: float) -> dict[str, Any]:
        body = json.dumps(payload).encode("utf-8")
        request = Request(
            f"{self.base_url}/{method}",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urlopen(request, timeout=timeout) as response:
                raw = response.read().decode("utf-8")
        except HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            raise TelegramApiError(f"HTTP {exc.code}: {detail}") from exc
        except URLError as exc:
            raise TelegramApiError(f"Network error: {exc.reason}") from exc

        parsed = json.loads(raw)
        if not parsed.get("ok"):
            raise TelegramApiError(str(parsed.get("description", "Unknown Telegram error")))
        return parsed["result"]

    def get_updates(self, offset: int | None, timeout_seconds: int) -> list[dict[str, Any]]:
        payload: dict[str, Any] = {
            "timeout": timeout_seconds,
            "allowed_updates": ["message"],
        }
        if offset is not None:
            payload["offset"] = offset
        return self._call("getUpdates", payload, timeout=timeout_seconds + 10.0)

    def send_message(
        self,
        chat_id: int,
        text: str,
        thread_id: int | None,
    ) -> TelegramMessageRef:
        payload: dict[str, Any] = {
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": True,
        }
        if thread_id is not None:
            payload["message_thread_id"] = thread_id
        result = self._call("sendMessage", payload, timeout=20.0)
        return TelegramMessageRef(chat_id=chat_id, message_id=int(result["message_id"]))

    def edit_message_text(self, chat_id: int, message_id: int, text: str) -> None:
        payload = {
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "disable_web_page_preview": True,
        }
        try:
            self._call("editMessageText", payload, timeout=20.0)
        except TelegramApiError as exc:
            if "message is not modified" in str(exc).lower():
                return
            raise
