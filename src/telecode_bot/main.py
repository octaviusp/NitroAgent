from __future__ import annotations

import asyncio
from pathlib import Path

from telecode_bot.bot import TelegramBridgeBot
from telecode_bot.config import BotConfig


def main() -> None:
    config = BotConfig.from_env(Path.cwd())
    bot = TelegramBridgeBot(config)
    try:
        asyncio.run(bot.run_forever())
    except KeyboardInterrupt:
        print("Shutting down...")


if __name__ == "__main__":
    main()
