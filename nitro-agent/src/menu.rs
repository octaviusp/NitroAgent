use teloxide::types::{BotCommand, KeyboardButton, KeyboardMarkup};

// ── Persistent Reply Keyboard ──────────────────────────────────────
//
// Always visible at the bottom of the chat. Each button sends its text
// as a regular message, which gets intercepted and mapped to a command.
// No callback queries, no inline keyboards, no sub-menus — just simple
// button → command mapping.

/// Build the persistent reply keyboard shown at the bottom of the chat.
pub fn persistent_keyboard() -> KeyboardMarkup {
    KeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new("🆕 New"),
            KeyboardButton::new("⏹ Cancel"),
            KeyboardButton::new("📦 Compact"),
        ],
        vec![
            KeyboardButton::new("📊 Status"),
            KeyboardButton::new("💰 Context"),
            KeyboardButton::new("🔄 Resume"),
        ],
        vec![
            KeyboardButton::new("🔒 Safe"),
            KeyboardButton::new("🔓 Full"),
            KeyboardButton::new("❓ Help"),
        ],
    ])
    .resize_keyboard()
    .persistent()
}

/// Bot commands registered with Telegram's command menu (the "/" button).
pub fn bot_commands() -> Vec<BotCommand> {
    vec![
        BotCommand::new("new", "Fresh session"),
        BotCommand::new("resume", "Resume previous session"),
        BotCommand::new("compact", "Compress memory"),
        BotCommand::new("cancel", "Kill running process"),
        BotCommand::new("restart", "Full reset"),
        BotCommand::new("bash", "Run shell command"),
        BotCommand::new("cd", "Change working directory"),
        BotCommand::new("context", "Token usage and cost"),
        BotCommand::new("status", "Thread state"),
        BotCommand::new("mode", "Toggle tool permissions"),
        BotCommand::new("mcp", "MCP servers"),
        BotCommand::new("skills", "Available skills"),
        BotCommand::new("tasks", "Recent runs"),
        BotCommand::new("help", "List all commands"),
    ]
}

/// Match a persistent keyboard button press to its equivalent slash command.
/// Returns None if the text doesn't match any button.
pub fn match_keyboard_button(text: &str) -> Option<&'static str> {
    match text.trim() {
        "🆕 New" => Some("/new"),
        "⏹ Cancel" => Some("/cancel"),
        "📦 Compact" => Some("/compact"),
        "📊 Status" => Some("/status"),
        "💰 Context" => Some("/context"),
        "🔄 Resume" => Some("/resume"),
        "🔒 Safe" => Some("/mode safe"),
        "🔓 Full" => Some("/mode full"),
        "❓ Help" => Some("/help"),
        _ => None,
    }
}
