use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Unique key identifying a Telegram thread context.
/// Format: `chat:{chat_id}` or `chat:{chat_id}:topic:{thread_id}`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThreadKey(pub String);

impl ThreadKey {
    pub fn new(chat_id: i64, topic_id: Option<i64>) -> Self {
        match topic_id {
            Some(tid) => Self(format!("chat:{chat_id}:topic:{tid}")),
            None => Self(format!("chat:{chat_id}")),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Filesystem-safe slug for workspace/log directories.
    pub fn slug(&self) -> String {
        self.0.replace(':', "_")
    }
}

impl std::fmt::Display for ThreadKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parsed Telegram command (e.g. `/start`, `/toolmode safe`).
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    pub name: String,
    pub args: String,
}

/// Context extracted from an incoming Telegram message.
#[derive(Debug, Clone)]
pub struct MessageContext {
    pub chat_id: i64,
    pub user_id: u64,
    pub text: String,
    pub message_id: i32,
    pub thread_id: Option<i64>,
}

/// A task enqueued for a thread worker.
#[derive(Debug, Clone)]
pub struct IncomingTask {
    pub message: MessageContext,
    pub command: Option<ParsedCommand>,
}

/// Persisted thread state from SQLite.
#[derive(Debug, Clone)]
pub struct ThreadState {
    pub thread_key: String,
    pub active_engine: String,
    pub workspace_path: PathBuf,
    pub active_session_id: Option<String>,
    pub compact_summary: Option<String>,
    pub settings: ThreadSettings,
}

impl ThreadState {
    pub fn tool_mode(&self) -> &str {
        &self.settings.tool_mode
    }
}

/// Per-thread settings stored as JSON in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSettings {
    #[serde(default = "default_tool_mode")]
    pub tool_mode: String,
}

fn default_tool_mode() -> String {
    "safe".to_string()
}

impl Default for ThreadSettings {
    fn default() -> Self {
        Self {
            tool_mode: default_tool_mode(),
        }
    }
}

/// Tracking context for a single engine run.
#[derive(Debug, Clone)]
pub struct RunContext {
    pub run_id: i64,
    pub status_message_id: i32,
    pub log_path: PathBuf,
}

/// Result of an engine execution.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub status: RunStatus,
    pub output_tail: String,
    pub session_id: Option<String>,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Succeeded,
    Failed,
    Canceled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Succeeded => "OK",
            Self::Failed => "FAIL",
            Self::Canceled => "STOP",
        }
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Engine command built for subprocess execution.
#[derive(Debug)]
pub struct EngineCommand {
    pub args: Vec<String>,
    pub effective_prompt: String,
    pub pipe_stdin: bool,
}
