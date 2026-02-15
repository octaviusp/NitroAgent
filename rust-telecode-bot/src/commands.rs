use std::path::PathBuf;

use crate::bot::{html_escape, truncate_for_telegram, BotCore};
use crate::types::{CachedClaudeInfo, ParsedCommand, ThreadState};

/// Handle all slash commands. Returns true if the command was handled.
pub async fn handle_command(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    thread_state: &ThreadState,
    command: &ParsedCommand,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    match command.name.as_str() {
        "start" => {
            handle_start(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "help" => {
            handle_help(bot, chat_id, thread_id).await?;
            Ok(true)
        }
        "new_thread" | "new" => {
            handle_new_thread(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "resume" => {
            handle_resume(bot, chat_id, thread_id, thread_key, &command.args).await?;
            Ok(true)
        }
        "clear" => {
            handle_clear(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "toolmode" | "mode" => {
            handle_toolmode(bot, chat_id, thread_id, thread_key, &command.args).await?;
            Ok(true)
        }
        "status" => {
            handle_status(bot, chat_id, thread_id, thread_key, thread_state).await?;
            Ok(true)
        }
        "context" => {
            handle_context(bot, chat_id, thread_id, thread_key, thread_state).await?;
            Ok(true)
        }
        "mcp" | "mcps" => {
            handle_mcp(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "skills" => {
            handle_skills(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "tasks" => {
            handle_tasks(bot, chat_id, thread_id, thread_key).await?;
            Ok(true)
        }
        "cd" | "pwd" => {
            handle_cd(bot, chat_id, thread_id, thread_key, thread_state, &command.args).await?;
            Ok(true)
        }
        // cancel, restart, compact, bash → handled in bot.rs (need subprocess)
        "compact" | "cancel" | "restart" | "bash" => Ok(false),
        _ => Ok(false),
    }
}

// ── /start ──

async fn handle_start(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache = bot.info_cache.read().await;
    let info = cache.get(thread_key);

    let model = info
        .map(|i| i.meta.model.as_str())
        .unwrap_or("claude");
    let version = info
        .map(|i| i.meta.version.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("—");
    let perm = info
        .map(|i| i.meta.permission_mode.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("bypassPermissions");

    let html = format!(
        "\
<b>TeleCode Bot</b>
High-performance Claude Code bridge

<b>Model</b>    <code>{model}</code>
<b>Version</b>  <code>{version}</code>
<b>Mode</b>     <code>{perm}</code>

Type /help for commands.",
        model = html_escape(model),
        version = html_escape(version),
        perm = html_escape(perm),
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /help ──

async fn handle_help(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let html = "\
<b>Commands</b>

<b>Session</b>
/new — Fresh session
/resume [id] — Resume or list sessions
/clear — Full reset (session + workspace)
/compact — Compress memory

<b>Execution</b>
/cancel — Kill running process
/restart — Reset Claude Code
/bash &lt;cmd&gt; — Run shell command

<b>Navigation</b>
/cd [path] — Show or change workspace

<b>Info</b>
/status — Thread state
/context — Token usage + session
/mcp — MCP servers
/skills — Available skills
/tasks — Recent runs

<b>Config</b>
/mode &lt;safe|full&gt; — Tool permissions

Send any text as a prompt.";

    bot.send_html(chat_id, thread_id, html).await?;
    Ok(())
}

// ── /new ──

async fn handle_new_thread(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    bot.store.set_active_session(thread_key, None).await?;
    bot.store.set_compact_summary(thread_key, None).await?;
    // Reset accumulated usage for fresh session
    {
        let mut cache = bot.info_cache.write().await;
        cache.remove(thread_key);
    }
    bot.send_html(
        chat_id,
        thread_id,
        "✅ <b>Fresh session</b>\nMemory and session cleared.",
    )
    .await?;
    Ok(())
}

// ── /resume ──

async fn handle_resume(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    args: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let trimmed = args.trim();
    if !trimmed.is_empty() {
        bot.store
            .set_active_session(thread_key, Some(trimmed))
            .await?;
        let html = format!(
            "✅ <b>Session set</b>\n<code>{}</code>",
            html_escape(trimmed)
        );
        bot.send_html(chat_id, thread_id, &html).await?;
    } else {
        let sessions = bot.store.recent_sessions(thread_key, 5).await?;
        if sessions.is_empty() {
            bot.send_html(
                chat_id,
                thread_id,
                "<b>No sessions</b>\nNo history for this thread.",
            )
            .await?;
        } else {
            let list: String = sessions
                .iter()
                .enumerate()
                .map(|(i, sid)| format!("{}. <code>{}</code>", i + 1, html_escape(sid)))
                .collect::<Vec<_>>()
                .join("\n");
            let html = format!(
                "<b>Recent Sessions</b>\n\n{list}\n\n/resume &lt;id&gt; to resume"
            );
            bot.send_html(chat_id, thread_id, &html).await?;
        }
    }
    Ok(())
}

// ── /clear ──

async fn handle_clear(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let fresh = bot.store.clear_thread_state(thread_key).await?;
    {
        let mut cache = bot.info_cache.write().await;
        cache.remove(thread_key);
    }
    let html = format!(
        "✅ <b>Cleared</b>\n\n\
         Session and memory removed.\n\
         Workspace: <code>{}</code>",
        html_escape(&fresh.workspace_path.to_string_lossy())
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /cd ──

async fn handle_cd(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    state: &ThreadState,
    args: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path_str = args.trim();

    if path_str.is_empty() {
        let html = format!(
            "<b>Workspace</b>\n<code>{}</code>",
            html_escape(&state.workspace_path.to_string_lossy()),
        );
        bot.send_html(chat_id, thread_id, &html).await?;
        return Ok(());
    }

    // Expand ~ to HOME
    let expanded = if path_str.starts_with("~/") {
        match std::env::var("HOME") {
            Ok(home) => format!("{}/{}", home, &path_str[2..]),
            Err(_) => path_str.to_string(),
        }
    } else if path_str == "~" {
        std::env::var("HOME").unwrap_or_else(|_| path_str.to_string())
    } else {
        path_str.to_string()
    };

    let path = PathBuf::from(&expanded);

    // Resolve relative paths against current workspace
    let resolved = if path.is_absolute() {
        path
    } else {
        state.workspace_path.join(&path)
    };

    // Canonicalize (resolves symlinks, .., etc.)
    let canonical = match resolved.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            bot.send_html(
                chat_id,
                thread_id,
                &format!(
                    "❌ <b>Path not found</b>\n<code>{}</code>",
                    html_escape(&resolved.to_string_lossy()),
                ),
            )
            .await?;
            return Ok(());
        }
    };

    if !canonical.is_dir() {
        bot.send_html(
            chat_id,
            thread_id,
            &format!(
                "❌ <b>Not a directory</b>\n<code>{}</code>",
                html_escape(&canonical.to_string_lossy()),
            ),
        )
        .await?;
        return Ok(());
    }

    bot.store
        .set_workspace_path(thread_key, &canonical)
        .await?;

    let html = format!(
        "✅ <b>Workspace</b>\n<code>{}</code>",
        html_escape(&canonical.to_string_lossy()),
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /mode ──

async fn handle_toolmode(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    args: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let selected = args.trim().to_lowercase();
    if selected != "safe" && selected != "full" {
        bot.send_html(
            chat_id,
            thread_id,
            "<b>Usage</b>\n/mode &lt;safe|full&gt;",
        )
        .await?;
        return Ok(());
    }

    bot.store.set_tool_mode(thread_key, &selected).await?;
    let html = format!(
        "✅ <b>Tool mode</b>: <code>{}</code>",
        html_escape(&selected)
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /status ──

async fn handle_status(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    state: &ThreadState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sessions = bot.store.recent_sessions(thread_key, 1).await?;
    let latest = sessions.first().map(|s| s.as_str()).unwrap_or("—");

    let cache = bot.info_cache.read().await;
    let info = cache.get(thread_key);

    let model = info
        .map(|i| i.meta.model.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("—");
    let version = info
        .map(|i| i.meta.version.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("—");

    let html = format!(
        "\
<b>Status</b>

<b>Thread</b>    <code>{thread_key}</code>
<b>Engine</b>    <code>{engine}</code>
<b>Model</b>     <code>{model}</code>
<b>Version</b>   <code>{version}</code>
<b>Session</b>   <code>{session}</code>
<b>Memory</b>    {compact}
<b>Mode</b>      <code>{tool_mode}</code>
<b>Latest</b>    <code>{latest}</code>",
        thread_key = html_escape(thread_key),
        engine = html_escape(&state.active_engine),
        model = html_escape(model),
        version = html_escape(version),
        session = html_escape(state.active_session_id.as_deref().unwrap_or("—")),
        compact = if state.compact_summary.is_some() {
            "yes"
        } else {
            "no"
        },
        tool_mode = html_escape(state.tool_mode()),
        latest = html_escape(latest),
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /context ──

async fn handle_context(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    state: &ThreadState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache = bot.info_cache.read().await;
    let info = cache.get(thread_key);

    let html = match info {
        Some(info) => format_context_with_data(info, state),
        None => format_context_no_data(state),
    };

    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

fn format_context_with_data(info: &CachedClaudeInfo, state: &ThreadState) -> String {
    let u = &info.usage;

    // Context fill = cache_read + cache_creation + input (the full input for latest turn)
    // This represents how much of the window the current conversation occupies.
    let context_used = u.cache_read_tokens + u.cache_creation_tokens + u.input_tokens;

    // Real context window from modelUsage, fallback 200k
    let context_window = if u.context_window > 0 {
        u.context_window
    } else {
        200_000
    };
    let pct = ((context_used as f64 / context_window as f64) * 100.0).min(100.0);

    // Visual bar (10 blocks)
    let filled = ((pct / 10.0).round() as usize).min(10);
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);

    let model = if info.meta.model.is_empty() {
        "—"
    } else {
        &info.meta.model
    };

    let mut html = format!(
        "\
<b>Context</b>

<code>{bar}</code>  {used}k / {window}k  ({pct:.0}%)

<b>Cached</b>     {cache_r}k read  ·  {cache_w}k write
<b>Input</b>      {input} tokens (last turn)
<b>Output</b>     {output}k tokens (total)
<b>Cost</b>       ${cost:.4} ({runs} run{s})

<b>Model</b>      <code>{model}</code>
<b>Session</b>    <code>{session}</code>",
        bar = bar,
        used = context_used / 1000,
        window = context_window / 1000,
        pct = pct,
        cache_r = format_k(u.cache_read_tokens),
        cache_w = format_k(u.cache_creation_tokens),
        input = u.input_tokens,
        output = format_k(u.output_tokens_total),
        cost = u.cost_total,
        runs = u.num_runs,
        s = if u.num_runs == 1 { "" } else { "s" },
        model = html_escape(model),
        session = html_escape(state.active_session_id.as_deref().unwrap_or("—")),
    );

    if u.max_output_tokens > 0 {
        html.push_str(&format!(
            "\n<b>Max output</b> {}k tokens",
            u.max_output_tokens / 1000
        ));
    }
    if !info.meta.mcp_servers.is_empty() {
        html.push_str(&format!(
            "\n<b>MCPs</b>       {}",
            info.meta.mcp_servers.len()
        ));
    }
    if !info.meta.tools.is_empty() {
        html.push_str(&format!(
            "\n<b>Tools</b>      {}",
            info.meta.tools.len()
        ));
    }
    if !info.meta.plugins.is_empty() {
        html.push_str(&format!(
            "\n<b>Plugins</b>    {}",
            info.meta.plugins.len()
        ));
    }
    if !info.meta.fast_mode_state.is_empty() {
        html.push_str(&format!(
            "\n<b>Fast mode</b>  <code>{}</code>",
            html_escape(&info.meta.fast_mode_state)
        ));
    }

    html
}

fn format_context_no_data(state: &ThreadState) -> String {
    let summary_preview = state
        .compact_summary
        .as_deref()
        .map(|s| {
            let preview: String = s.chars().take(300).collect();
            format!("\n\n<b>Compact memory</b>\n<pre>{}</pre>", html_escape(&preview))
        })
        .unwrap_or_default();

    format!(
        "\
<b>Context</b>

<i>No usage data yet — send a message first.</i>

<b>Session</b>  <code>{session}</code>
<b>Engine</b>   <code>{engine}</code>{summary}",
        session = html_escape(state.active_session_id.as_deref().unwrap_or("—")),
        engine = html_escape(&state.active_engine),
        summary = summary_preview,
    )
}

// ── /mcp ──

async fn handle_mcp(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache = bot.info_cache.read().await;
    let info = cache.get(thread_key);

    let html = match info {
        Some(info) if !info.meta.mcp_servers.is_empty() => {
            let count = info.meta.mcp_servers.len();
            let list: String = info
                .meta
                .mcp_servers
                .iter()
                .map(|s| {
                    let icon = if s.status == "connected" {
                        "✅"
                    } else {
                        "❌"
                    };
                    format!(
                        "{icon} <b>{name}</b>  ·  {status}",
                        name = html_escape(&s.name),
                        status = html_escape(&s.status),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "<b>MCP Servers</b>  ·  {count}\n\n{list}"
            )
        }
        Some(_) => "<b>MCP Servers</b>\n\nNo servers configured.".to_string(),
        None => {
            "<b>MCP Servers</b>\n\n<i>No data yet — send a message first.</i>".to_string()
        }
    };

    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

// ── /skills ──

async fn handle_skills(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cache = bot.info_cache.read().await;
    let info = cache.get(thread_key);

    let html = match info {
        Some(info) => {
            let mut result = String::new();
            let has_any = !info.meta.skills.is_empty()
                || !info.meta.slash_commands.is_empty()
                || !info.meta.agents.is_empty()
                || !info.meta.plugins.is_empty();

            if !has_any {
                return bot
                    .send_html(
                        chat_id,
                        thread_id,
                        "<b>Skills</b>\n\nNo skills available.",
                    )
                    .await
                    .map(|_| ());
            }

            // Skills (user-defined)
            if !info.meta.skills.is_empty() {
                let count = info.meta.skills.len();
                let list: String = info
                    .meta
                    .skills
                    .iter()
                    .map(|s| format!("  <code>{}</code>", html_escape(s)))
                    .collect::<Vec<_>>()
                    .join("\n");
                result.push_str(&format!("<b>Skills</b>  ·  {count}\n\n{list}"));
            }

            // Slash commands (all available)
            if !info.meta.slash_commands.is_empty() {
                let count = info.meta.slash_commands.len();
                let list: String = info
                    .meta
                    .slash_commands
                    .iter()
                    .map(|s| format!("  <code>/{}</code>", html_escape(s)))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                result.push_str(&format!("<b>Slash Commands</b>  ·  {count}\n\n{list}"));
            }

            // Agents
            if !info.meta.agents.is_empty() {
                let count = info.meta.agents.len();
                let list: String = info
                    .meta
                    .agents
                    .iter()
                    .map(|a| format!("  <code>{}</code>", html_escape(a)))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                result.push_str(&format!("<b>Agents</b>  ·  {count}\n\n{list}"));
            }

            // Plugins
            if !info.meta.plugins.is_empty() {
                let count = info.meta.plugins.len();
                let list: String = info
                    .meta
                    .plugins
                    .iter()
                    .map(|p| format!("  <code>{}</code>", html_escape(&p.name)))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                result.push_str(&format!("<b>Plugins</b>  ·  {count}\n\n{list}"));
            }

            result
        }
        None => "<b>Skills</b>\n\n<i>No data yet — send a message first.</i>".to_string(),
    };

    bot.send_html(chat_id, thread_id, &truncate_for_telegram(&html)).await?;
    Ok(())
}

// ── /tasks ──

async fn handle_tasks(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runs = bot.store.recent_runs(thread_key, 10).await?;

    if runs.is_empty() {
        bot.send_html(
            chat_id,
            thread_id,
            "<b>Recent Runs</b>\n\nNo runs yet.",
        )
        .await?;
        return Ok(());
    }

    let list: String = runs
        .iter()
        .map(|r| {
            let icon = match r.status.as_str() {
                "succeeded" => "✅",
                "failed" => "❌",
                "canceled" => "⏹",
                "running" => "⏳",
                _ => "·",
            };
            let duration = match &r.ended_at {
                Some(end) => format_duration_between(&r.started_at, end),
                None => "running".to_string(),
            };
            let ago = format_time_ago(&r.started_at);
            format!(
                "{icon} #{id}  ·  {status}  ·  {duration}  ·  {ago}",
                id = r.id,
                status = html_escape(&r.status),
                duration = duration,
                ago = ago,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = format!(
        "<b>Recent Runs</b>  ·  {count}\n\n{list}",
        count = runs.len(),
    );

    bot.send_html(chat_id, thread_id, &truncate_for_telegram(&html))
        .await?;
    Ok(())
}

// ── Command parser ──

/// Parse a message text into a command, if it starts with `/`.
pub fn parse_command(text: &str) -> Option<ParsedCommand> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let first_space = trimmed.find(char::is_whitespace);
    let (cmd_part, args) = match first_space {
        Some(idx) => (&trimmed[..idx], trimmed[idx..].trim()),
        None => (trimmed, ""),
    };

    let cmd_name = cmd_part
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or("")
        .to_lowercase();

    if cmd_name.is_empty() {
        return None;
    }

    Some(ParsedCommand {
        name: cmd_name,
        args: args.to_string(),
    })
}

// ── Helpers ──

fn format_k(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

fn format_time_ago(iso: &str) -> String {
    let Ok(ts) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return "—".to_string();
    };
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(ts);

    let secs = diff.num_seconds();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn format_duration_between(start: &str, end: &str) -> String {
    let Ok(s) = chrono::DateTime::parse_from_rfc3339(start) else {
        return "—".to_string();
    };
    let Ok(e) = chrono::DateTime::parse_from_rfc3339(end) else {
        return "—".to_string();
    };
    let diff = e.signed_duration_since(s);
    let secs = diff.num_seconds();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_command() {
        let cmd = parse_command("/start").unwrap();
        assert_eq!(cmd.name, "start");
        assert_eq!(cmd.args, "");
    }

    #[test]
    fn parse_command_with_args() {
        let cmd = parse_command("/mode safe").unwrap();
        assert_eq!(cmd.name, "mode");
        assert_eq!(cmd.args, "safe");
    }

    #[test]
    fn parse_command_with_bot_mention() {
        let cmd = parse_command("/start@mybot").unwrap();
        assert_eq!(cmd.name, "start");
    }

    #[test]
    fn parse_no_command() {
        assert!(parse_command("hello world").is_none());
    }

    #[test]
    fn parse_new_alias() {
        let cmd = parse_command("/new").unwrap();
        assert_eq!(cmd.name, "new");
    }

    #[test]
    fn format_k_small() {
        assert_eq!(format_k(500), "500");
    }

    #[test]
    fn format_k_large() {
        assert_eq!(format_k(24923), "24.9");
    }

    #[test]
    fn time_ago_format() {
        let now = chrono::Utc::now();
        let ago = (now - chrono::Duration::seconds(120)).to_rfc3339();
        let result = format_time_ago(&ago);
        assert!(result.contains("m ago"));
    }
}
