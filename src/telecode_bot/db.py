from __future__ import annotations

import json
import sqlite3
import threading
from pathlib import Path

from telecode_bot.types import ThreadState
from telecode_bot.utils import thread_workspace_slug, utc_now_iso


class ThreadStore:
    def __init__(
        self,
        db_path: Path,
        workspace_root: Path,
        default_engine: str,
        default_tool_mode: str,
    ) -> None:
        self.db_path = db_path
        self.workspace_root = workspace_root
        self.default_engine = default_engine
        self.default_tool_mode = default_tool_mode
        self._lock = threading.Lock()
        self._conn = sqlite3.connect(self.db_path, check_same_thread=False)
        self._conn.row_factory = sqlite3.Row
        self._init_schema()

    def _init_schema(self) -> None:
        with self._lock:
            self._conn.executescript(
                """
                CREATE TABLE IF NOT EXISTS threads (
                    thread_key TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    active_engine TEXT NOT NULL,
                    workspace_path TEXT NOT NULL,
                    active_session_id TEXT,
                    compact_summary TEXT,
                    settings_json TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS runs (
                    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    thread_key TEXT NOT NULL,
                    engine TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    ended_at TEXT,
                    status TEXT NOT NULL,
                    last_telegram_message_id INTEGER,
                    log_path TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS session_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    thread_key TEXT NOT NULL,
                    engine TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                """
            )
            self._conn.commit()

    def _default_workspace(self, thread_key: str) -> Path:
        path = self.workspace_root / thread_workspace_slug(thread_key)
        path.mkdir(parents=True, exist_ok=True)
        return path

    def create_fresh_workspace(self, thread_key: str) -> Path:
        timestamp = utc_now_iso().replace(":", "").replace("+00:00", "Z")
        path = self.workspace_root / f"{thread_workspace_slug(thread_key)}-{timestamp}"
        path.mkdir(parents=True, exist_ok=True)
        return path

    def get_or_create_thread(self, thread_key: str) -> ThreadState:
        with self._lock:
            row = self._conn.execute(
                "SELECT * FROM threads WHERE thread_key = ?", (thread_key,)
            ).fetchone()
            if row is None:
                now = utc_now_iso()
                workspace = str(self._default_workspace(thread_key))
                settings = json.dumps({"tool_mode": self.default_tool_mode})
                self._conn.execute(
                    """
                    INSERT INTO threads (
                        thread_key,
                        created_at,
                        updated_at,
                        active_engine,
                        workspace_path,
                        active_session_id,
                        compact_summary,
                        settings_json
                    ) VALUES (?, ?, ?, ?, ?, NULL, NULL, ?)
                    """,
                    (thread_key, now, now, self.default_engine, workspace, settings),
                )
                self._conn.commit()
                row = self._conn.execute(
                    "SELECT * FROM threads WHERE thread_key = ?", (thread_key,)
                ).fetchone()

        assert row is not None
        return self._row_to_thread_state(row)

    def _row_to_thread_state(self, row: sqlite3.Row) -> ThreadState:
        raw_settings = row["settings_json"] or "{}"
        parsed = json.loads(raw_settings)
        settings = {str(k): str(v) for k, v in parsed.items()}
        return ThreadState(
            thread_key=row["thread_key"],
            active_engine=row["active_engine"],
            workspace_path=Path(row["workspace_path"]),
            active_session_id=row["active_session_id"],
            compact_summary=row["compact_summary"],
            settings=settings,
        )

    def _update_timestamp(self, thread_key: str) -> None:
        self._conn.execute(
            "UPDATE threads SET updated_at = ? WHERE thread_key = ?", (utc_now_iso(), thread_key)
        )

    def save_thread_state(self, thread: ThreadState) -> None:
        with self._lock:
            settings_json = json.dumps(thread.settings)
            self._conn.execute(
                """
                UPDATE threads
                SET updated_at = ?,
                    active_engine = ?,
                    workspace_path = ?,
                    active_session_id = ?,
                    compact_summary = ?,
                    settings_json = ?
                WHERE thread_key = ?
                """,
                (
                    utc_now_iso(),
                    thread.active_engine,
                    str(thread.workspace_path),
                    thread.active_session_id,
                    thread.compact_summary,
                    settings_json,
                    thread.thread_key,
                ),
            )
            self._conn.commit()

    def set_active_engine(self, thread_key: str, engine: str) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE threads SET active_engine = ? WHERE thread_key = ?", (engine, thread_key)
            )
            self._update_timestamp(thread_key)
            self._conn.commit()

    def set_active_session(self, thread_key: str, session_id: str | None) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE threads SET active_session_id = ? WHERE thread_key = ?",
                (session_id, thread_key),
            )
            self._update_timestamp(thread_key)
            self._conn.commit()

    def set_compact_summary(self, thread_key: str, summary: str | None) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE threads SET compact_summary = ? WHERE thread_key = ?",
                (summary, thread_key),
            )
            self._update_timestamp(thread_key)
            self._conn.commit()

    def set_tool_mode(self, thread_key: str, mode: str) -> None:
        with self._lock:
            row = self._conn.execute(
                "SELECT settings_json FROM threads WHERE thread_key = ?", (thread_key,)
            ).fetchone()
            settings = json.loads(row["settings_json"] if row is not None else "{}")
            settings["tool_mode"] = mode
            self._conn.execute(
                "UPDATE threads SET settings_json = ? WHERE thread_key = ?",
                (json.dumps(settings), thread_key),
            )
            self._update_timestamp(thread_key)
            self._conn.commit()

    def clear_thread_state(self, thread_key: str) -> ThreadState:
        fresh_workspace = self.create_fresh_workspace(thread_key)
        with self._lock:
            self._conn.execute(
                """
                UPDATE threads
                SET workspace_path = ?,
                    active_session_id = NULL,
                    compact_summary = NULL,
                    settings_json = ?
                WHERE thread_key = ?
                """,
                (
                    str(fresh_workspace),
                    json.dumps({"tool_mode": self.default_tool_mode}),
                    thread_key,
                ),
            )
            self._update_timestamp(thread_key)
            self._conn.commit()
        return self.get_or_create_thread(thread_key)

    def create_run(
        self,
        thread_key: str,
        engine: str,
        last_telegram_message_id: int,
        log_path: Path,
    ) -> int:
        with self._lock:
            cursor = self._conn.execute(
                """
                INSERT INTO runs (
                    thread_key,
                    engine,
                    started_at,
                    status,
                    last_telegram_message_id,
                    log_path
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    thread_key,
                    engine,
                    utc_now_iso(),
                    "running",
                    last_telegram_message_id,
                    str(log_path),
                ),
            )
            self._conn.commit()
            return int(cursor.lastrowid)

    def finish_run(self, run_id: int, status: str) -> None:
        with self._lock:
            self._conn.execute(
                "UPDATE runs SET status = ?, ended_at = ? WHERE run_id = ?",
                (status, utc_now_iso(), run_id),
            )
            self._conn.commit()

    def add_session_history(self, thread_key: str, engine: str, session_id: str) -> None:
        with self._lock:
            self._conn.execute(
                """
                INSERT INTO session_history (thread_key, engine, session_id, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (thread_key, engine, session_id, utc_now_iso()),
            )
            self._conn.commit()

    def recent_sessions(self, thread_key: str, limit: int = 5) -> list[str]:
        with self._lock:
            rows = self._conn.execute(
                """
                SELECT session_id
                FROM session_history
                WHERE thread_key = ?
                ORDER BY id DESC
                LIMIT ?
                """,
                (thread_key, limit),
            ).fetchall()
        return [str(row["session_id"]) for row in rows]
