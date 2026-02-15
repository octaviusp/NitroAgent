mod bot;
mod commands;
mod config;
mod db;
mod engine;
mod format;
mod stream;
#[allow(dead_code)]
mod types;
mod voice;
mod worker;

use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::{MaybeInaccessibleMessage, UpdateKind};
use tracing::{error, info, warn};

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

    let config = match BotConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!("Config error: {e}");
            std::process::exit(1);
        }
    };

    info!(claude_bin = %config.claude_bin, "Resolved claude binary");

    // Startup checks for voice transcription dependencies
    if !voice::check_ffmpeg() {
        warn!("ffmpeg not found on PATH — voice transcription will fail");
    }
    if !config.sst_script.exists() {
        warn!(
            path = %config.sst_script.display(),
            "sst.py not found — voice transcription will fail"
        );
    }

    let store = match ThreadStore::new(&config).await {
        Ok(s) => s,
        Err(e) => {
            error!("Database error: {e}");
            std::process::exit(1);
        }
    };

    // Build HTTP client with timeout > long-poll timeout to avoid
    // premature request cancellation during getUpdates
    let http_client = teloxide::net::default_reqwest_settings()
        .timeout(std::time::Duration::from_secs(
            config.poll_timeout_seconds as u64 + 30,
        ))
        .build()
        .expect("failed to build HTTP client");

    let tg = Bot::with_client(&config.telegram_bot_token, http_client);

    // Delete webhook and drop pending updates
    if let Err(e) = tg.delete_webhook().drop_pending_updates(true).await {
        info!("deleteWebhook: {e} (continuing)");
    } else {
        info!("Webhook cleared");
    }

    let allowed_user_ids = config.allowed_user_ids.clone();
    let bot_core = Arc::new(BotCore::new(config, store, tg.clone()));
    let registry = Arc::new(WorkerRegistry::new(bot_core.clone()));

    info!("Polling for updates...");

    // Long-polling loop
    let mut offset: Option<i32> = None;
    let poll_timeout = bot_core.config.poll_timeout_seconds as u32;

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
                        // Access control
                        if !allowed_user_ids.contains(&cq.from.id.0) {
                            continue;
                        }

                        let data = match &cq.data {
                            Some(d) => d.clone(),
                            None => continue,
                        };

                        // Extract chat_id and thread_id from the message
                        let (cb_chat_id, cb_thread_id) = match &cq.message {
                            Some(MaybeInaccessibleMessage::Regular(ref msg)) => {
                                let tid = msg.thread_id.map(|t| t.0 .0 as i64);
                                (msg.chat.id.0, tid)
                            }
                            _ => continue,
                        };

                        // Answer callback query to dismiss loading spinner
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

                    // Access control
                    if !allowed_user_ids.contains(&user.id.0) {
                        continue;
                    }

                    let chat_id = msg.chat.id.0;
                    let thread_id = msg.thread_id.map(|tid| tid.0 .0 as i64);

                    // Extract text, voice file_id, or photo file_id (download happens in worker)
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
                        // photo() returns &[PhotoSize] sorted by size — last is highest resolution
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
                            error!("Failed to send cancel reply: {e}");
                        }
                        continue;
                    }

                    let worker = registry.get_or_create(&thread_key).await;
                    let was_busy = worker.is_running().await;

                    if let Err(e) = worker.enqueue(task.clone()) {
                        error!("Failed to enqueue task: {e}");
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
                error!("Polling error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
}
