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
    /// Telegram file_id for a voice/audio attachment (downloaded in worker, not poller).
    pub voice_file_id: Option<String>,
    /// Telegram file_id for a photo attachment (downloaded in worker, not poller).
    pub photo_file_id: Option<String>,
    /// Text from a quoted/replied-to message (reply_to_message).
    pub reply_text: Option<String>,
    /// Text from a forwarded message body.
    pub forwarded_text: Option<String>,
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
    pub elapsed_secs: u64,
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
            Self::Succeeded => "✅",
            Self::Failed => "❌",
            Self::Canceled => "⏹",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Succeeded => "Done",
            Self::Failed => "Failed",
            Self::Canceled => "Canceled",
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

// ── Claude Code metadata (extracted from stream-json) ──

/// Metadata from the Claude Code `system` init event.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    pub model: String,
    pub version: String,
    pub permission_mode: String,
    pub cwd: String,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<McpServerInfo>,
    pub skills: Vec<String>,
    pub agents: Vec<String>,
    pub slash_commands: Vec<String>,
    pub plugins: Vec<PluginInfo>,
    pub fast_mode_state: String,
    pub api_key_source: String,
}

#[derive(Debug, Clone, Default)]
pub struct McpServerInfo {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct PluginInfo {
    pub name: String,
}

/// Token usage from a Claude Code `result` event.
///
/// Per-run fields: `input_tokens`, `cache_read_tokens`, `cache_creation_tokens`
/// reflect the LATEST run (current context fill).
/// Accumulated fields: `output_tokens_total`, `cost_total`, `num_runs`
/// grow across runs in the same session.
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
    /// New input tokens from latest run (usually just the user message).
    pub input_tokens: u64,
    /// Cached tokens read from latest run (system prompt + conversation history).
    pub cache_read_tokens: u64,
    /// Newly cached tokens from latest run.
    pub cache_creation_tokens: u64,
    /// Accumulated output tokens across all runs in session.
    pub output_tokens_total: u64,
    /// Accumulated cost across all runs in session.
    pub cost_total: f64,
    /// Number of runs completed in this session.
    pub num_runs: u64,
    /// From modelUsage — real context window size (0 = unknown).
    pub context_window: u64,
    /// From modelUsage — real max output tokens (0 = unknown).
    pub max_output_tokens: u64,
}

/// Combined cached info per thread, populated from stream-json events.
#[derive(Debug, Clone, Default)]
pub struct CachedClaudeInfo {
    pub meta: SessionMeta,
    pub usage: UsageInfo,
    /// Last prompt text (for retry).
    pub last_prompt: Option<String>,
}

/// Summary of a historical run (for `/tasks` display).
#[derive(Debug, Clone)]
pub struct RunInfo {
    pub id: i64,
    pub engine: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
}
