CREATE TABLE IF NOT EXISTS threads (
    thread_key    TEXT PRIMARY KEY,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    active_engine TEXT NOT NULL,
    workspace_path TEXT NOT NULL,
    active_session_id TEXT,
    compact_summary TEXT,
    settings_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS runs (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_key              TEXT NOT NULL,
    engine                  TEXT NOT NULL,
    started_at              TEXT NOT NULL,
    ended_at                TEXT,
    status                  TEXT NOT NULL,
    telegram_msg_id         INTEGER,
    log_path                TEXT NOT NULL,
    FOREIGN KEY (thread_key) REFERENCES threads(thread_key)
);

CREATE TABLE IF NOT EXISTS session_history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_key  TEXT NOT NULL,
    engine      TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    FOREIGN KEY (thread_key) REFERENCES threads(thread_key)
);
