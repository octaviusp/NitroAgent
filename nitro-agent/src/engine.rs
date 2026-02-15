use crate::config::BotConfig;
use crate::types::EngineCommand;

/// Build the Claude CLI command for subprocess execution.
pub fn build_claude_command(
    config: &BotConfig,
    prompt: &str,
    session_id: Option<&str>,
    tool_mode: &str,
    seed_summary: Option<&str>,
) -> EngineCommand {
    let mut args = vec![
        config.claude_bin.clone(),
        "-p".to_string(),
        "-".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ];

    if let Some(sid) = session_id {
        args.push("--resume".to_string());
        args.push(sid.to_string());
    }

    if let Some(summary) = seed_summary {
        let trimmed: String = summary.chars().take(4000).collect();
        args.push("--append-system-prompt".to_string());
        args.push(format!(
            "Persisted thread summary:\n{trimmed}\nTreat this as trusted memory and continue from it."
        ));
    }

    let allowed_tools = if tool_mode == "full" {
        &config.claude_full_allowed_tools
    } else {
        &config.claude_safe_allowed_tools
    };

    let tools: Vec<&str> = allowed_tools
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    if !tools.is_empty() {
        args.push("--allowedTools".to_string());
        args.push(tools.join(","));
    }

    EngineCommand {
        args,
        effective_prompt: prompt.to_string(),
        pipe_stdin: true,
    }
}
