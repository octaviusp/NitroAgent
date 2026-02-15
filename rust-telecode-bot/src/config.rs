use std::collections::HashSet;
use std::path::PathBuf;

/// Typed configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct BotConfig {
    pub telegram_bot_token: String,
    pub allowed_user_ids: HashSet<u64>,
    pub default_engine: String,
    pub default_tool_mode: String,
    pub poll_timeout_seconds: u64,
    pub stream_edit_interval_secs: f64,
    pub max_runtime_seconds: u64,
    pub max_output_chars: usize,
    pub db_path: PathBuf,
    pub workspace_root: PathBuf,
    pub logs_root: PathBuf,
    pub claude_bin: String,
    pub claude_safe_allowed_tools: Vec<String>,
    pub claude_full_allowed_tools: Vec<String>,
    // ── Speech-to-text (sst.py) ──
    pub sst_python: String,
    pub sst_script: PathBuf,
    pub sst_language: String,
    pub sst_arch: String,
}

impl BotConfig {
    /// Load config from environment. Call `dotenvy::dotenv()` before this.
    pub fn from_env() -> Result<Self, String> {
        let token = require_env("TELEGRAM_BOT_TOKEN")?;
        let raw_ids = require_env("ALLOWED_TELEGRAM_USER_IDS")?;

        let allowed_user_ids: HashSet<u64> = raw_ids
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    trimmed.parse::<u64>().ok()
                }
            })
            .collect();

        if allowed_user_ids.is_empty() {
            return Err("ALLOWED_TELEGRAM_USER_IDS must contain at least one valid ID".into());
        }

        let default_engine = env_or("DEFAULT_ENGINE", "claude").to_lowercase();
        if default_engine != "claude" {
            return Err("DEFAULT_ENGINE must be claude (only engine supported in Rust bot)".into());
        }

        let default_tool_mode = env_or("DEFAULT_TOOL_MODE", "safe").to_lowercase();
        if default_tool_mode != "safe" && default_tool_mode != "full" {
            return Err("DEFAULT_TOOL_MODE must be safe or full".into());
        }

        let db_path = PathBuf::from(env_or("DB_PATH", "data/telecode_bot.db"));
        let workspace_root = PathBuf::from(env_or("WORKSPACE_ROOT", "workspaces"));
        let logs_root = PathBuf::from(env_or("LOGS_ROOT", "logs"));

        // Ensure directories exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create db dir: {e}"))?;
        }
        std::fs::create_dir_all(&workspace_root)
            .map_err(|e| format!("create workspace dir: {e}"))?;
        std::fs::create_dir_all(&logs_root).map_err(|e| format!("create logs dir: {e}"))?;

        Ok(Self {
            telegram_bot_token: token,
            allowed_user_ids,
            default_engine,
            default_tool_mode,
            poll_timeout_seconds: env_or("POLL_TIMEOUT_SECONDS", "25").parse().unwrap_or(25),
            stream_edit_interval_secs: env_or("STREAM_EDIT_INTERVAL_SECONDS", "0.7")
                .parse()
                .unwrap_or(0.7),
            max_runtime_seconds: env_or("MAX_RUNTIME_SECONDS", "1200")
                .parse()
                .unwrap_or(1200),
            max_output_chars: env_or("MAX_OUTPUT_CHARS", "3500").parse().unwrap_or(3500),
            db_path,
            workspace_root,
            logs_root,
            claude_bin: env_or("CLAUDE_BIN", "claude"),
            claude_safe_allowed_tools: parse_tool_list(&env_or(
                "CLAUDE_SAFE_ALLOWED_TOOLS",
                "Read,Edit,Bash",
            )),
            claude_full_allowed_tools: parse_tool_list(&env_or("CLAUDE_FULL_ALLOWED_TOOLS", "")),
            sst_python: resolve_sst_python(),
            sst_script: resolve_sst_script(),
            sst_language: env_or("SST_LANGUAGE", "es"),
            sst_arch: env_or("SST_ARCH", "base"),
        })
    }
}

fn resolve_sst_python() -> String {
    let explicit = env_or("SST_PYTHON", "");
    if !explicit.is_empty() {
        return explicit;
    }
    // Auto-detect project venv
    let venv = PathBuf::from("../.venv/bin/python3");
    if venv.exists() {
        venv.canonicalize()
            .unwrap_or(venv)
            .to_string_lossy()
            .to_string()
    } else {
        "python3".to_string()
    }
}

fn resolve_sst_script() -> PathBuf {
    let explicit = env_or("SST_SCRIPT", "");
    if !explicit.is_empty() {
        return PathBuf::from(explicit);
    }
    let relative = PathBuf::from("../sst.py");
    if relative.exists() {
        relative
            .canonicalize()
            .unwrap_or(relative)
    } else {
        relative
    }
}

fn require_env(key: &str) -> Result<String, String> {
    std::env::var(key)
        .map(|v| v.trim().to_string())
        .map_err(|_| format!("{key} is required"))
        .and_then(|v| {
            if v.is_empty() {
                Err(format!("{key} is required"))
            } else {
                Ok(v)
            }
        })
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .map(|v| {
            let trimmed = v.trim().to_string();
            if trimmed.is_empty() {
                default.to_string()
            } else {
                trimmed
            }
        })
        .unwrap_or_else(|_| default.to_string())
}

fn parse_tool_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
