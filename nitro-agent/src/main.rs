mod bot;
mod commands;
mod config;
mod db;
mod engine;
mod format;
mod multi;
mod stream;
#[allow(dead_code)]
mod types;
mod voice;
mod worker;

use std::collections::HashMap;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::{MaybeInaccessibleMessage, MessageEntityKind, UpdateKind};
use tracing::{debug, error, info, warn};

use crate::bot::BotCore;
use crate::commands::parse_command;
use crate::config::BotConfig;
use crate::db::ThreadStore;
use crate::types::{IncomingTask, MessageContext, ThreadKey};
use crate::worker::WorkerRegistry;

#[tokio::main]
async fn main() {
    // Load .env
    dotenvy::dotenv().ok();

    // Init structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .compact()
        .init();

    info!("NitroAgent starting");

    // Startup checks for voice transcription dependencies
    if !voice::check_ffmpeg() {
        warn!("ffmpeg not found on PATH — voice transcription will fail");
    }

    // Try multi-agent mode (agents.toml), fall back to legacy single-agent (.env)
    let agents: Vec<(String, BotConfig)> = if std::path::Path::new("agents.toml").exists() {
        info!("Found agents.toml — loading multi-agent configuration");
        match multi::load_agents_toml("agents.toml") {
            Ok(a) => {
                info!(count = a.len(), "Loaded agents from agents.toml");
                a
            }
            Err(e) => {
                error!("agents.toml error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        info!("No agents.toml found — legacy single-agent mode");
        let config = match BotConfig::from_env() {
            Ok(c) => c,
            Err(e) => {
                error!("Config error: {e}");
                std::process::exit(1);
            }
        };
        vec![("default".to_string(), config)]
    };

    // Spawn one tokio task per agent
    let mut handles = Vec::new();

    for (name, config) in agents {
        info!(agent = %name, claude_bin = %config.claude_bin, "Starting agent");

        if !config.sst_script.exists() {
            warn!(
                agent = %name,
                path = %config.sst_script.display(),
                "sst.py not found — voice transcription will fail"
            );
        }

        let store = match ThreadStore::new(&config).await {
            Ok(s) => s,
            Err(e) => {
                error!(agent = %name, "Database error: {e}");
                std::process::exit(1);
            }
        };

        // Build HTTP client with timeout > long-poll timeout
        let http_client = teloxide::net::default_reqwest_settings()
            .timeout(std::time::Duration::from_secs(
                config.poll_timeout_seconds + 30,
            ))
            .build()
            .expect("failed to build HTTP client");

        let tg = Bot::with_client(&config.telegram_bot_token, http_client);

        // Delete webhook and drop pending updates
        if let Err(e) = tg.delete_webhook().drop_pending_updates(true).await {
            info!(agent = %name, "deleteWebhook: {e} (continuing)");
        } else {
            info!(agent = %name, "Webhook cleared");
        }

        // Get bot's own identity for mention detection in groups
        let me = tg
            .get_me()
            .await
            .unwrap_or_else(|e| panic!("agent.{name}: Failed to call getMe: {e}"));
        let bot_id = me.id.0;
        let bot_username = me.username.clone().unwrap_or_default().to_lowercase();
        info!(agent = %name, bot_id, bot_username = %bot_username, "Bot identity resolved");

        let allowed_user_ids = config.allowed_user_ids.clone();
        let allowed_group_ids = config.allowed_group_ids.clone();
        let bot_to_bot_max_turns = config.bot_to_bot_max_turns;

        if !allowed_group_ids.is_empty() {
            info!(agent = %name, groups = ?allowed_group_ids, max_turns = bot_to_bot_max_turns, "Group chat enabled");
        }

        let bot_core = Arc::new(BotCore::new(config, store, tg.clone()));
        let registry = Arc::new(WorkerRegistry::new(bot_core.clone()));

        let handle = tokio::spawn(run_agent_loop(
            name,
            bot_core,
            registry,
            allowed_user_ids,
            allowed_group_ids,
            bot_to_bot_max_turns,
            bot_username,
            tg,
        ));
        handles.push(handle);
    }

    info!(
        agents = handles.len(),
        "All agents started — polling for updates"
    );

    // Block forever — if any agent task panics, exit
    for handle in handles {
        if let Err(e) = handle.await {
            error!("Agent task failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn run_agent_loop(
    name: String,
    bot_core: Arc<BotCore>,
    registry: Arc<WorkerRegistry>,
    allowed_user_ids: std::collections::HashSet<u64>,
    allowed_group_ids: std::collections::HashSet<i64>,
    bot_to_bot_max_turns: u32,
    bot_username: String,
    tg: Bot,
) {
    let poll_timeout = bot_core.config.poll_timeout_seconds as u32;
    let mut offset: Option<i32> = None;
    let mut bot_turns: HashMap<i64, u32> = HashMap::new();

    info!(agent = %name, "Polling loop started");

    loop {
        let mut req = tg.get_updates();
        if let Some(off) = offset {
            req = req.offset(off);
        }
        req = req.timeout(poll_timeout);

        match req.await {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.id.0 as i32 + 1);

                    // Handle callback queries (inline keyboard buttons)
                    if let UpdateKind::CallbackQuery(ref cq) = update.kind {
                        if !allowed_user_ids.contains(&cq.from.id.0) {
                            continue;
                        }

                        let data = match &cq.data {
                            Some(d) => d.clone(),
                            None => continue,
                        };

                        let (cb_chat_id, cb_thread_id) = match &cq.message {
                            Some(MaybeInaccessibleMessage::Regular(ref msg)) => {
                                let tid = msg.thread_id.map(|t| t.0 .0 as i64);
                                (msg.chat.id.0, tid)
                            }
                            _ => continue,
                        };

                        let _ = tg.answer_callback_query(&cq.id).await;
                        let thread_key = ThreadKey::new(cb_chat_id, cb_thread_id);

                        match data.as_str() {
                            "cancel" => {
                                let worker = registry.get_or_create(&thread_key).await;
                                let cancelled = worker.cancel_current().await;
                                if !cancelled {
                                    let _ = bot_core
                                        .send_html(cb_chat_id, cb_thread_id, "Nothing running.")
                                        .await;
                                }
                            }
                            d if d.starts_with("resume:") => {
                                let sid = &d[7..];
                                let _ = bot_core
                                    .store
                                    .set_active_session(thread_key.as_str(), Some(sid))
                                    .await;
                                let _ = bot_core
                                    .send_html(
                                        cb_chat_id,
                                        cb_thread_id,
                                        &format!(
                                            "✅ <b>Session resumed</b>\n<code>{}</code>",
                                            crate::bot::html_escape(sid)
                                        ),
                                    )
                                    .await;
                            }
                            _ => {}
                        }

                        continue;
                    }

                    let msg = match &update.kind {
                        UpdateKind::Message(m) => m,
                        _ => continue,
                    };

                    let user = match &msg.from {
                        Some(u) => u,
                        None => continue,
                    };

                    let chat_id = msg.chat.id.0;
                    let is_group = msg.chat.is_group() || msg.chat.is_supergroup();

                    // ── Access control & group routing ──
                    if is_group {
                        if !allowed_group_ids.contains(&chat_id) {
                            debug!(agent = %name, chat_id, "Group not in allowed list, skipping");
                            continue;
                        }

                        let sender_is_bot = user.is_bot;

                        // Parse @mentions from message entities
                        let mentioned_usernames: Vec<String> = msg
                            .entities()
                            .unwrap_or_default()
                            .iter()
                            .filter_map(|ent| {
                                if let MessageEntityKind::Mention = &ent.kind {
                                    msg.text().and_then(|txt| {
                                        let start = ent.offset;
                                        let end = start + ent.length;
                                        txt.get(start..end)
                                            .map(|s| s.trim_start_matches('@').to_lowercase())
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();

                        let we_are_mentioned = mentioned_usernames
                            .iter()
                            .any(|u| u == &bot_username);
                        let has_mentions = !mentioned_usernames.is_empty();

                        if sender_is_bot {
                            if !we_are_mentioned {
                                debug!(agent = %name, chat_id, from = user.id.0, "Bot msg without our mention, skipping");
                                continue;
                            }
                            let turns = bot_turns.entry(chat_id).or_insert(0);
                            if bot_to_bot_max_turns > 0 && *turns >= bot_to_bot_max_turns {
                                debug!(agent = %name, chat_id, turns = *turns, "Bot-to-bot turn limit reached");
                                continue;
                            }
                            *turns += 1;
                            debug!(agent = %name, chat_id, turn = *turns, "Bot-to-bot turn accepted");
                        } else {
                            bot_turns.remove(&chat_id);
                            if has_mentions && !we_are_mentioned {
                                debug!(agent = %name, chat_id, mentions = ?mentioned_usernames, "Human mentioned other bot, skipping");
                                continue;
                            }
                        }
                    } else {
                        // Private chat: original access control
                        if !allowed_user_ids.contains(&user.id.0) {
                            continue;
                        }
                    }

                    let thread_id = msg.thread_id.map(|tid| tid.0 .0 as i64);

                    // Extract text, voice file_id, or photo file_id
                    let (text, voice_file_id, photo_file_id) = if let Some(t) = msg.text() {
                        (t.to_string(), None, None)
                    } else if msg.voice().is_some() || msg.audio().is_some() {
                        let file_id = msg
                            .voice()
                            .map(|v| v.file.id.clone())
                            .or_else(|| msg.audio().map(|a| a.file.id.clone()));
                        match file_id {
                            Some(id) => ("[voice]".to_string(), Some(id), None),
                            None => continue,
                        }
                    } else if let Some(sizes) = msg.photo() {
                        let file_id = sizes.last().map(|p| p.file.id.clone());
                        match file_id {
                            Some(id) => {
                                let caption = msg
                                    .caption()
                                    .map(|c| c.to_string())
                                    .unwrap_or_else(|| "[photo]".to_string());
                                (caption, None, Some(id))
                            }
                            None => continue,
                        }
                    } else {
                        continue;
                    };

                    // Extract reply-to context (quoted message)
                    let reply_text = msg.reply_to_message().and_then(|r| {
                        r.text()
                            .map(|t| t.to_string())
                            .or_else(|| r.caption().map(|c| c.to_string()))
                    });

                    // Extract forwarded message text
                    let forwarded_text = if msg.forward_origin().is_some() {
                        msg.text()
                            .map(|t| t.to_string())
                            .or_else(|| msg.caption().map(|c| c.to_string()))
                    } else {
                        None
                    };

                    let thread_key = ThreadKey::new(chat_id, thread_id);
                    let command = parse_command(&text);

                    let task = IncomingTask {
                        message: MessageContext {
                            chat_id,
                            user_id: user.id.0,
                            text,
                            message_id: msg.id.0,
                            thread_id,
                            voice_file_id,
                            photo_file_id,
                            reply_text,
                            forwarded_text,
                        },
                        command,
                    };

                    // Cancel is handled inline for immediate response
                    if task
                        .command
                        .as_ref()
                        .map(|c| c.name == "cancel")
                        .unwrap_or(false)
                    {
                        let worker = registry.get_or_create(&thread_key).await;
                        let cancelled = worker.cancel_current().await;
                        let reply = if cancelled {
                            "<b>Run canceled</b>\nStopping current execution."
                        } else {
                            "<b>No active run</b>\nNothing to cancel."
                        };
                        if let Err(e) = bot_core
                            .send_html(chat_id, task.message.thread_id, reply)
                            .await
                        {
                            error!(agent = %name, "Failed to send cancel reply: {e}");
                        }
                        continue;
                    }

                    let worker = registry.get_or_create(&thread_key).await;
                    let was_busy = worker.is_running().await;

                    if let Err(e) = worker.enqueue(task.clone()) {
                        error!(agent = %name, "Failed to enqueue task: {e}");
                        continue;
                    }

                    if was_busy {
                        let _ = bot_core
                            .send_html(
                                chat_id,
                                thread_id,
                                "<b>Queued</b>\nA run is already in progress. Your message has been queued.",
                            )
                            .await;
                    }
                }
            }
            Err(e) => {
                error!(agent = %name, "Polling error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}
