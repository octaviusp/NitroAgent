use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::Deserialize;
use tracing::info;

use crate::config::{
    parse_tool_list, resolve_claude_bin, resolve_sst_python, resolve_sst_script,
    resolve_to_absolute, BotConfig,
};

/// Shared defaults — every agent inherits these unless overridden.
#[derive(Deserialize, Default, Debug)]
#[serde(default)]
struct DefaultsConfig {
    allowed_user_ids: Option<String>,
    allowed_group_ids: Option<String>,
    default_engine: Option<String>,
    default_tool_mode: Option<String>,
    poll_timeout_seconds: Option<u64>,
    stream_edit_interval_secs: Option<f64>,
    max_runtime_seconds: Option<u64>,
    max_output_chars: Option<usize>,
    bot_to_bot_max_turns: Option<u32>,
    claude_bin: Option<String>,
    claude_safe_allowed_tools: Option<String>,
    claude_full_allowed_tools: Option<String>,
    sst_python: Option<String>,
    sst_script: Option<String>,
    sst_language: Option<String>,
    sst_arch: Option<String>,
}

/// Per-agent entry in [agent.NAME].
#[derive(Deserialize, Debug)]
struct AgentEntry {
    telegram_bot_token: String,
    workspace_root: Option<String>,
    db_path: Option<String>,
    logs_root: Option<String>,
    allowed_user_ids: Option<String>,
    allowed_group_ids: Option<String>,
    default_engine: Option<String>,
    default_tool_mode: Option<String>,
    poll_timeout_seconds: Option<u64>,
    stream_edit_interval_secs: Option<f64>,
    max_runtime_seconds: Option<u64>,
    max_output_chars: Option<usize>,
    bot_to_bot_max_turns: Option<u32>,
    claude_bin: Option<String>,
    claude_safe_allowed_tools: Option<String>,
    claude_full_allowed_tools: Option<String>,
    sst_python: Option<String>,
    sst_script: Option<String>,
    sst_language: Option<String>,
    sst_arch: Option<String>,
}

/// Top-level agents.toml structure.
#[derive(Deserialize, Debug)]
struct AgentsFile {
    defaults: Option<DefaultsConfig>,
    agent: HashMap<String, AgentEntry>,
}

/// Replace `${VAR}` references with their environment variable values.
fn expand_env(val: &str) -> String {
    let mut result = val.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 2..start + end];
            let replacement = std::env::var(var_name).unwrap_or_default();
            result = format!("{}{}{}", &result[..start], replacement, &result[start + end + 1..]);
        } else {
            break;
        }
    }
    result
}

fn parse_id_list_u64(raw: &str) -> HashSet<u64> {
    raw.split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<u64>().ok()
            }
        })
        .collect()
}

fn parse_id_list_i64(raw: &str) -> HashSet<i64> {
    raw.split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<i64>().ok()
            }
        })
        .collect()
}

/// Resolve a value with fallback: agent override > defaults > hardcoded default.
macro_rules! resolve {
    ($agent:expr, $defaults:expr, $field:ident, $default:expr) => {
        $agent
            .$field
            .as_ref()
            .or($defaults.as_ref().and_then(|d| d.$field.as_ref()))
            .map(|v| expand_env(&v.to_string()))
            .unwrap_or_else(|| $default.to_string())
    };
}

macro_rules! resolve_num {
    ($agent:expr, $defaults:expr, $field:ident, $default:expr, $ty:ty) => {
        $agent
            .$field
            .or($defaults.as_ref().and_then(|d| d.$field))
            .unwrap_or($default)
    };
}

/// Load agents.toml and return a list of (agent_name, BotConfig) pairs.
pub fn load_agents_toml(path: &str) -> Result<Vec<(String, BotConfig)>, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read {path}: {e}"))?;
    let file: AgentsFile =
        toml::from_str(&content).map_err(|e| format!("Failed to parse {path}: {e}"))?;

    if file.agent.is_empty() {
        return Err("agents.toml must contain at least one [agent.NAME] section".into());
    }

    let defaults = file.defaults;
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let base_dir = format!("{home}/.nitro-agent");

    let mut agents = Vec::new();

    for (name, entry) in &file.agent {
        let token = expand_env(&entry.telegram_bot_token);
        if token.is_empty() {
            return Err(format!(
                "agent.{name}: telegram_bot_token is empty (check env var reference)"
            ));
        }

        let raw_user_ids = resolve!(entry, defaults, allowed_user_ids, "");
        let allowed_user_ids = parse_id_list_u64(&raw_user_ids);
        if allowed_user_ids.is_empty() {
            return Err(format!(
                "agent.{name}: allowed_user_ids must contain at least one valid ID"
            ));
        }

        let raw_group_ids = resolve!(entry, defaults, allowed_group_ids, "");
        let allowed_group_ids = parse_id_list_i64(&raw_group_ids);

        let default_engine = resolve!(entry, defaults, default_engine, "claude").to_lowercase();
        let default_tool_mode =
            resolve!(entry, defaults, default_tool_mode, "safe").to_lowercase();

        // Auto-derive paths from agent name unless explicitly set
        let db_path = entry
            .db_path
            .as_ref()
            .map(|p| resolve_to_absolute(PathBuf::from(expand_env(p))))
            .unwrap_or_else(|| {
                resolve_to_absolute(PathBuf::from(format!("{base_dir}/data/{name}.db")))
            });

        let workspace_root = entry
            .workspace_root
            .as_ref()
            .map(|p| resolve_to_absolute(PathBuf::from(expand_env(p))))
            .unwrap_or_else(|| {
                resolve_to_absolute(PathBuf::from(format!("{base_dir}/workspaces/{name}")))
            });

        let logs_root = entry
            .logs_root
            .as_ref()
            .map(|p| resolve_to_absolute(PathBuf::from(expand_env(p))))
            .unwrap_or_else(|| {
                resolve_to_absolute(PathBuf::from(format!("{base_dir}/logs/{name}")))
            });

        // Ensure directories exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("agent.{name}: create db dir: {e}"))?;
        }
        std::fs::create_dir_all(&workspace_root)
            .map_err(|e| format!("agent.{name}: create workspace dir: {e}"))?;
        std::fs::create_dir_all(&logs_root)
            .map_err(|e| format!("agent.{name}: create logs dir: {e}"))?;

        let claude_bin_val = resolve!(entry, defaults, claude_bin, "");
        let claude_bin = if claude_bin_val.is_empty() {
            resolve_claude_bin()
        } else {
            claude_bin_val
        };

        let safe_tools = resolve!(entry, defaults, claude_safe_allowed_tools, "Read,Edit,Bash");
        let full_tools = resolve!(entry, defaults, claude_full_allowed_tools, "");

        let sst_python_val = resolve!(entry, defaults, sst_python, "");
        let sst_python = if sst_python_val.is_empty() {
            resolve_sst_python()
        } else {
            sst_python_val
        };

        let sst_script_val = resolve!(entry, defaults, sst_script, "");
        let sst_script = if sst_script_val.is_empty() {
            resolve_sst_script()
        } else {
            resolve_to_absolute(PathBuf::from(sst_script_val))
        };

        let config = BotConfig {
            agent_name: name.clone(),
            telegram_bot_token: token,
            allowed_user_ids,
            allowed_group_ids,
            bot_to_bot_max_turns: resolve_num!(entry, defaults, bot_to_bot_max_turns, 5, u32),
            default_engine,
            default_tool_mode,
            poll_timeout_seconds: resolve_num!(entry, defaults, poll_timeout_seconds, 25, u64),
            stream_edit_interval_secs: resolve_num!(
                entry,
                defaults,
                stream_edit_interval_secs,
                0.7,
                f64
            ),
            max_runtime_seconds: resolve_num!(entry, defaults, max_runtime_seconds, 1200, u64),
            max_output_chars: resolve_num!(entry, defaults, max_output_chars, 3500, usize),
            db_path,
            workspace_root,
            logs_root,
            claude_bin,
            claude_safe_allowed_tools: parse_tool_list(&safe_tools),
            claude_full_allowed_tools: parse_tool_list(&full_tools),
            sst_python,
            sst_script,
            sst_language: resolve!(entry, defaults, sst_language, "es"),
            sst_arch: resolve!(entry, defaults, sst_arch, "base"),
        };

        info!(agent = %name, token_prefix = &config.telegram_bot_token[..8], "Loaded agent config");
        agents.push((name.clone(), config));
    }

    Ok(agents)
}
