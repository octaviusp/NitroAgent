use crate::bot::BotCore;
use crate::types::{ParsedCommand, ThreadState};

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
            handle_start(bot, chat_id, thread_id, thread_state).await?;
            Ok(true)
        }
        "help" => {
            handle_help(bot, chat_id, thread_id).await?;
            Ok(true)
        }
        "new_thread" => {
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
        "toolmode" => {
            handle_toolmode(bot, chat_id, thread_id, thread_key, &command.args).await?;
            Ok(true)
        }
        "status" => {
            handle_status(bot, chat_id, thread_id, thread_key, thread_state).await?;
            Ok(true)
        }
        "context" => {
            handle_context(bot, chat_id, thread_id, thread_state).await?;
            Ok(true)
        }
        "compact" | "cancel" | "restart" | "mcps" | "skills" | "tasks" => {
            // These commands require subprocess interaction, handled in bot.rs
            Ok(false)
        }
        _ => {
            bot.send_html(
                chat_id,
                thread_id,
                "<b>Unknown command</b>\nType /help to see available commands.",
            )
            .await?;
            Ok(true)
        }
    }
}

async fn handle_start(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    state: &ThreadState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let html = format!(
        "<b>TeleCode Bot (Rust)</b>\n\n\
         High-performance Claude Code execution bridge.\n\n\
         Engine: <code>{engine}</code>\n\
         Tool mode: <code>{mode}</code>\n\n\
         Type /help for available commands.",
        engine = html_escape(&state.active_engine),
        mode = html_escape(state.tool_mode()),
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

async fn handle_help(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let html = "\
<b>Commands</b>\n\n\
/start - Welcome message\n\
/help - This command list\n\
/new_thread - Clear session, start fresh\n\
/resume [id] - Resume a session or list recent\n\
/clear - Full reset (session + workspace)\n\
/compact - Summarize and start fresh with memory\n\
/cancel - Kill running subprocess\n\
/restart - Restart Claude Code\n\
/status - Thread state info\n\
/context - Session context details\n\
/mcps - List active MCP servers\n\
/skills - List Claude Code skills\n\
/tasks - List current tasks\n\
/toolmode &lt;safe|full&gt; - Set tool permission mode\n\n\
Send any text to execute as a prompt.";

    bot.send_html(chat_id, thread_id, html).await?;
    Ok(())
}

async fn handle_new_thread(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    bot.store.set_active_session(thread_key, None).await?;
    bot.store.set_compact_summary(thread_key, None).await?;
    bot.send_html(
        chat_id,
        thread_id,
        "<b>Fresh session ready</b>\nPrevious active session and compact memory were cleared.",
    )
    .await?;
    Ok(())
}

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
            "<b>Session updated</b>\nActive session: <code>{}</code>",
            html_escape(trimmed)
        );
        bot.send_html(chat_id, thread_id, &html).await?;
    } else {
        let sessions = bot.store.recent_sessions(thread_key, 5).await?;
        if sessions.is_empty() {
            bot.send_html(
                chat_id,
                thread_id,
                "<b>No recent sessions</b>\nNo session history found for this thread.",
            )
            .await?;
        } else {
            let list: String = sessions
                .iter()
                .enumerate()
                .map(|(i, sid)| format!("{}. <code>{}</code>", i + 1, html_escape(sid)))
                .collect::<Vec<_>>()
                .join("\n");
            let html =
                format!("<b>Recent sessions</b>\n\n{list}\n\nUse /resume &lt;id&gt; to resume.");
            bot.send_html(chat_id, thread_id, &html).await?;
        }
    }
    Ok(())
}

async fn handle_clear(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let fresh = bot.store.clear_thread_state(thread_key).await?;
    let html = format!(
        "<b>Thread state cleared</b>\n\
         Session and compact memory removed.\n\
         Workspace: <code>{}</code>",
        html_escape(&fresh.workspace_path.to_string_lossy())
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

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
            "<b>Usage</b>\n<code>/toolmode &lt;safe|full&gt;</code>",
        )
        .await?;
        return Ok(());
    }

    bot.store.set_tool_mode(thread_key, &selected).await?;
    let html = format!(
        "<b>Tool mode updated</b>\nNow using <code>{}</code>.",
        html_escape(&selected)
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

async fn handle_status(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    thread_key: &str,
    state: &ThreadState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sessions = bot.store.recent_sessions(thread_key, 1).await?;
    let latest = sessions.first().map(|s| s.as_str()).unwrap_or("none");

    let html = format!(
        "<b>Thread Status</b>\n\n\
         Thread: <code>{thread_key}</code>\n\
         Engine: <code>{engine}</code>\n\
         Workspace: <code>{workspace}</code>\n\
         Active session: <code>{session}</code>\n\
         Compact memory: {compact}\n\
         Tool mode: <code>{tool_mode}</code>\n\
         Latest session: <code>{latest}</code>",
        thread_key = html_escape(thread_key),
        engine = html_escape(&state.active_engine),
        workspace = html_escape(&state.workspace_path.to_string_lossy()),
        session = html_escape(state.active_session_id.as_deref().unwrap_or("none")),
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

async fn handle_context(
    bot: &BotCore,
    chat_id: i64,
    thread_id: Option<i64>,
    state: &ThreadState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let summary_preview = state
        .compact_summary
        .as_deref()
        .map(|s| {
            let preview: String = s.chars().take(500).collect();
            format!("<pre>{}</pre>", html_escape(&preview))
        })
        .unwrap_or_else(|| "none".to_string());

    let html = format!(
        "<b>Session Context</b>\n\n\
         Active session: <code>{session}</code>\n\
         Engine: <code>{engine}</code>\n\n\
         <b>Compact summary:</b>\n{summary}",
        session = html_escape(state.active_session_id.as_deref().unwrap_or("none")),
        engine = html_escape(&state.active_engine),
        summary = summary_preview,
    );
    bot.send_html(chat_id, thread_id, &html).await?;
    Ok(())
}

/// Parse a message text into a command, if it starts with `/`.
pub fn parse_command(text: &str) -> Option<ParsedCommand> {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    // Strip bot mention suffix (e.g. /start@mybotname)
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
        let cmd = parse_command("/toolmode safe").unwrap();
        assert_eq!(cmd.name, "toolmode");
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
    fn html_escape_special_chars() {
        assert_eq!(
            html_escape("<b>&\"test\"</b>"),
            "&lt;b&gt;&amp;&quot;test&quot;&lt;/b&gt;"
        );
    }
}
