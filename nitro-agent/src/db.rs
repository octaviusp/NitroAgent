use std::path::PathBuf;

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use tracing::info;

use crate::config::BotConfig;
use crate::types::{RunInfo, ThreadSettings, ThreadState};

/// SQLite persistence layer for threads, runs, and session history.
#[derive(Clone)]
pub struct ThreadStore {
    pool: SqlitePool,
    workspace_root: PathBuf,
    default_engine: String,
    default_tool_mode: String,
}

impl ThreadStore {
    pub async fn new(config: &BotConfig) -> Result<Self, sqlx::Error> {
        let db_url = format!("sqlite:{}?mode=rwc", config.db_path.display());
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        let store = Self {
            pool,
            workspace_root: config.workspace_root.clone(),
            default_engine: config.default_engine.clone(),
            default_tool_mode: config.default_tool_mode.clone(),
        };
        store.init_schema().await?;
        info!("SQLite store initialized at {}", config.db_path.display());
        Ok(store)
    }

    async fn init_schema(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS threads (
                thread_key    TEXT PRIMARY KEY,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL,
                active_engine TEXT NOT NULL,
                workspace_path TEXT NOT NULL,
                active_session_id TEXT,
                compact_summary TEXT,
                settings_json TEXT NOT NULL DEFAULT '{}'
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS runs (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_key          TEXT NOT NULL,
                engine              TEXT NOT NULL,
                started_at          TEXT NOT NULL,
                ended_at            TEXT,
                status              TEXT NOT NULL,
                telegram_msg_id     INTEGER,
                log_path            TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS session_history (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_key  TEXT NOT NULL,
                engine      TEXT NOT NULL,
                session_id  TEXT NOT NULL,
                created_at  TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    fn default_workspace(&self, thread_key: &str) -> PathBuf {
        let slug = thread_key.replace(':', "_");
        let path = self.workspace_root.join(slug);
        std::fs::create_dir_all(&path).ok();
        path
    }

    fn fresh_workspace(&self, thread_key: &str) -> PathBuf {
        let slug = thread_key.replace(':', "_");
        let ts = utc_now_compact();
        let path = self.workspace_root.join(format!("{slug}-{ts}"));
        std::fs::create_dir_all(&path).ok();
        path
    }

    pub async fn get_or_create_thread(&self, thread_key: &str) -> Result<ThreadState, sqlx::Error> {
        let row: Option<ThreadRow> = sqlx::query_as(
            "SELECT thread_key, active_engine, workspace_path, active_session_id, compact_summary, settings_json FROM threads WHERE thread_key = ?"
        )
        .bind(thread_key)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            return Ok(row.into_thread_state());
        }

        let now = utc_now();
        let workspace = self.default_workspace(thread_key);
        let settings = ThreadSettings {
            tool_mode: self.default_tool_mode.clone(),
        };
        let settings_json = serde_json::to_string(&settings).unwrap_or_default();

        sqlx::query(
            "INSERT INTO threads (thread_key, created_at, updated_at, active_engine, workspace_path, active_session_id, compact_summary, settings_json) VALUES (?, ?, ?, ?, ?, NULL, NULL, ?)"
        )
        .bind(thread_key)
        .bind(&now)
        .bind(&now)
        .bind(&self.default_engine)
        .bind(workspace.to_string_lossy().as_ref())
        .bind(&settings_json)
        .execute(&self.pool)
        .await?;

        Ok(ThreadState {
            thread_key: thread_key.to_string(),
            active_engine: self.default_engine.clone(),
            workspace_path: workspace,
            active_session_id: None,
            compact_summary: None,
            settings,
        })
    }

    pub async fn set_active_session(
        &self,
        thread_key: &str,
        session_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE threads SET active_session_id = ?, updated_at = ? WHERE thread_key = ?",
        )
        .bind(session_id)
        .bind(utc_now())
        .bind(thread_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_compact_summary(
        &self,
        thread_key: &str,
        summary: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE threads SET compact_summary = ?, updated_at = ? WHERE thread_key = ?")
            .bind(summary)
            .bind(utc_now())
            .bind(thread_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_tool_mode(&self, thread_key: &str, mode: &str) -> Result<(), sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT settings_json FROM threads WHERE thread_key = ?")
                .bind(thread_key)
                .fetch_optional(&self.pool)
                .await?;

        let mut settings: ThreadSettings = row
            .map(|(json,)| serde_json::from_str(&json).unwrap_or_default())
            .unwrap_or_default();
        settings.tool_mode = mode.to_string();

        let settings_json = serde_json::to_string(&settings).unwrap_or_default();
        sqlx::query("UPDATE threads SET settings_json = ?, updated_at = ? WHERE thread_key = ?")
            .bind(&settings_json)
            .bind(utc_now())
            .bind(thread_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_workspace_path(
        &self,
        thread_key: &str,
        path: &std::path::Path,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE threads SET workspace_path = ?, updated_at = ? WHERE thread_key = ?",
        )
        .bind(path.to_string_lossy().as_ref())
        .bind(utc_now())
        .bind(thread_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_thread_state(&self, thread_key: &str) -> Result<ThreadState, sqlx::Error> {
        let fresh = self.fresh_workspace(thread_key);
        let settings = ThreadSettings {
            tool_mode: self.default_tool_mode.clone(),
        };
        let settings_json = serde_json::to_string(&settings).unwrap_or_default();

        sqlx::query(
            "UPDATE threads SET workspace_path = ?, active_session_id = NULL, compact_summary = NULL, settings_json = ?, updated_at = ? WHERE thread_key = ?"
        )
        .bind(fresh.to_string_lossy().as_ref())
        .bind(&settings_json)
        .bind(utc_now())
        .bind(thread_key)
        .execute(&self.pool)
        .await?;

        self.get_or_create_thread(thread_key).await
    }

    pub async fn create_run(
        &self,
        thread_key: &str,
        engine: &str,
        telegram_msg_id: i32,
        log_path: &str,
    ) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO runs (thread_key, engine, started_at, status, telegram_msg_id, log_path) VALUES (?, ?, ?, 'running', ?, ?)"
        )
        .bind(thread_key)
        .bind(engine)
        .bind(utc_now())
        .bind(telegram_msg_id)
        .bind(log_path)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn finish_run(&self, run_id: i64, status: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE runs SET status = ?, ended_at = ? WHERE id = ?")
            .bind(status)
            .bind(utc_now())
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn add_session_history(
        &self,
        thread_key: &str,
        engine: &str,
        session_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO session_history (thread_key, engine, session_id, created_at) VALUES (?, ?, ?, ?)"
        )
        .bind(thread_key)
        .bind(engine)
        .bind(session_id)
        .bind(utc_now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn recent_sessions(
        &self,
        thread_key: &str,
        limit: i64,
    ) -> Result<Vec<String>, sqlx::Error> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT session_id FROM session_history WHERE thread_key = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(thread_key)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|(sid,)| sid).collect())
    }

    pub async fn recent_runs(
        &self,
        thread_key: &str,
        limit: i64,
    ) -> Result<Vec<RunInfo>, sqlx::Error> {
        let rows: Vec<RunInfoRow> = sqlx::query_as(
            "SELECT id, engine, started_at, ended_at, status FROM runs WHERE thread_key = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(thread_key)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| RunInfo {
                id: r.id,
                engine: r.engine,
                started_at: r.started_at,
                ended_at: r.ended_at,
                status: r.status,
            })
            .collect())
    }
}

// Internal query row mapping
#[derive(sqlx::FromRow)]
struct ThreadRow {
    thread_key: String,
    active_engine: String,
    workspace_path: String,
    active_session_id: Option<String>,
    compact_summary: Option<String>,
    settings_json: String,
}

impl ThreadRow {
    fn into_thread_state(self) -> ThreadState {
        let settings: ThreadSettings =
            serde_json::from_str(&self.settings_json).unwrap_or_default();
        ThreadState {
            thread_key: self.thread_key,
            active_engine: self.active_engine,
            workspace_path: PathBuf::from(self.workspace_path),
            active_session_id: self.active_session_id,
            compact_summary: self.compact_summary,
            settings,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RunInfoRow {
    id: i64,
    engine: String,
    started_at: String,
    ended_at: Option<String>,
    status: String,
}

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn utc_now_compact() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}
