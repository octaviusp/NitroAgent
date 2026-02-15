use std::collections::HashMap;
use std::io::Write as _;
use std::time::Instant;

use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

use crate::commands::handle_command;
use crate::config::BotConfig;
use crate::db::ThreadStore;
use crate::engine::build_claude_command;
use crate::stream::{parse_stream_line, RollingBuffer};
use crate::types::*;

/// Core bot logic shared across workers.
pub struct BotCore {
    pub config: BotConfig,
    pub store: ThreadStore,
    pub tg: Bot,
    /// Cached Claude Code metadata per thread (populated from stream-json events).
    pub info_cache: RwLock<HashMap<String, CachedClaudeInfo>>,
}

impl BotCore {
    pub fn new(config: BotConfig, store: ThreadStore, tg: Bot) -> Self {
        Self {
            config,
            store,
            tg,
            info_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Process a single incoming task from a worker queue.
    pub async fn process_task(
        &self,
        thread_key: &ThreadKey,
        task: &IncomingTask,
        cancel_requested: &Mutex<bool>,
        current_process: &Mutex<Option<Child>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let thread_state = self.store.get_or_create_thread(thread_key.as_str()).await?;

        if let Some(ref command) = task.command {
            // Cancel — needs subprocess access
            if command.name == "cancel" {
                let mut proc_guard = current_process.lock().await;
                if let Some(ref mut child) = *proc_guard {
                    let _ = child.kill().await;
                    self.send_html(
                        task.message.chat_id,
                        task.message.thread_id,
                        "⏹ <b>Canceled</b>\nProcess terminated.",
                    )
                    .await?;
                } else {
                    self.send_html(
                        task.message.chat_id,
                        task.message.thread_id,
                        "Nothing running.",
                    )
                    .await?;
                }
                return Ok(());
            }

            // Restart — kill process + clear session + clear cache
            if command.name == "restart" {
                let mut proc_guard = current_process.lock().await;
                if let Some(ref mut child) = *proc_guard {
                    let _ = child.kill().await;
                }
                drop(proc_guard);

                self.store
                    .set_active_session(thread_key.as_str(), None)
                    .await?;
                self.store
                    .set_compact_summary(thread_key.as_str(), None)
                    .await?;
                {
                    let mut cache = self.info_cache.write().await;
                    cache.remove(thread_key.as_str());
                }

                self.send_html(
                    task.message.chat_id,
                    task.message.thread_id,
                    "♻️ <b>Restarted</b>\n\nSession cleared.\nReady for new prompts.",
                )
                .await?;
                return Ok(());
            }

            // Compact — requires engine subprocess
            if command.name == "compact" {
                return self
                    .handle_compact(
                        thread_key,
                        task,
                        &thread_state,
                        cancel_requested,
                        current_process,
                    )
                    .await;
            }

            // Delegate to command handlers (mcp, skills, context, tasks, etc.)
            let handled = handle_command(
                self,
                task.message.chat_id,
                task.message.thread_id,
                thread_key.as_str(),
                &thread_state,
                command,
            )
            .await?;

            if handled {
                return Ok(());
            }

            // Unknown command
            self.send_html(
                task.message.chat_id,
                task.message.thread_id,
                &format!(
                    "Unknown: <code>/{}</code>\nType /help for commands.",
                    html_escape(&command.name)
                ),
            )
            .await?;
            return Ok(());
        }

        // Plain text prompt
        self.run_user_prompt(
            thread_key,
            task,
            &thread_state,
            &task.message.text,
            cancel_requested,
            current_process,
        )
        .await
    }

    async fn run_user_prompt(
        &self,
        thread_key: &ThreadKey,
        task: &IncomingTask,
        thread_state: &ThreadState,
        prompt: &str,
        cancel_requested: &Mutex<bool>,
        current_process: &Mutex<Option<Child>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let status_msg = self
            .send_html(
                task.message.chat_id,
                task.message.thread_id,
                "⏳ <b>Running...</b>",
            )
            .await?;
        let status_msg_id = status_msg.id.0;

        let log_path = self.build_log_path(thread_key);
        let run_id = self
            .store
            .create_run(
                thread_key.as_str(),
                &thread_state.active_engine,
                status_msg_id,
                &log_path.to_string_lossy(),
            )
            .await?;

        let seed_summary = if thread_state.active_session_id.is_none() {
            thread_state.compact_summary.as_deref()
        } else {
            None
        };

        let result = self
            .execute_engine_stream(
                thread_key,
                thread_state,
                prompt,
                task.message.chat_id,
                status_msg_id,
                thread_state.active_session_id.as_deref(),
                seed_summary,
                cancel_requested,
                current_process,
                &log_path,
                run_id,
            )
            .await?;

        self.store
            .finish_run(run_id, result.status.as_str())
            .await?;

        if let Some(ref sid) = result.session_id {
            self.store
                .set_active_session(thread_key.as_str(), Some(sid))
                .await?;
            self.store
                .add_session_history(thread_key.as_str(), &thread_state.active_engine, sid)
                .await?;
        }

        let sid_display = result
            .session_id
            .as_deref()
            .or(thread_state.active_session_id.as_deref())
            .unwrap_or("—");

        // Phone-optimized result message
        let final_html = format!(
            "{icon} <b>{status}</b>  ·  #{run_id}\n\
             <code>{session}</code>\n\n\
             <pre>{body}</pre>",
            icon = result.status.icon(),
            status = result.status.label(),
            session = html_escape(sid_display),
            body = html_escape(&result.output_tail),
        );
        let truncated = truncate_for_telegram(&final_html);

        if let Err(e) = self
            .edit_html(task.message.chat_id, status_msg_id, &truncated)
            .await
        {
            warn!("Failed to edit final message: {e}");
            let _ = self
                .send_html(task.message.chat_id, task.message.thread_id, &truncated)
                .await;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_engine_stream(
        &self,
        thread_key: &ThreadKey,
        thread_state: &ThreadState,
        prompt: &str,
        chat_id: i64,
        status_msg_id: i32,
        session_id: Option<&str>,
        seed_summary: Option<&str>,
        cancel_requested: &Mutex<bool>,
        current_process: &Mutex<Option<Child>>,
        log_path: &std::path::Path,
        run_id: i64,
    ) -> Result<RunResult, Box<dyn std::error::Error + Send + Sync>> {
        let cmd = build_claude_command(
            &self.config,
            prompt,
            session_id,
            thread_state.tool_mode(),
            seed_summary,
        );

        // Build clean env (strip Claude Code nesting vars)
        let mut env: HashMap<String, String> = std::env::vars().collect();
        for key in &[
            "CLAUDECODE",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
        ] {
            env.remove(*key);
        }

        let workspace = &thread_state.workspace_path;
        std::fs::create_dir_all(workspace)?;

        let mut child = tokio::process::Command::new(&cmd.args[0])
            .args(&cmd.args[1..])
            .current_dir(workspace)
            .stdin(if cmd.pipe_stdin {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env_clear()
            .envs(&env)
            .spawn()?;

        // Pipe prompt via stdin
        if cmd.pipe_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(cmd.effective_prompt.as_bytes()).await?;
                stdin.shutdown().await?;
            }
        }

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

        {
            let mut proc_guard = current_process.lock().await;
            *proc_guard = Some(child);
        }

        // Drain stderr into buffer
        let stderr_buf = std::sync::Arc::new(Mutex::new(String::new()));
        let stderr_buf_clone = stderr_buf.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut buf = stderr_buf_clone.lock().await;
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(&line);
            }
        });

        let mut reader = BufReader::new(stdout).lines();
        let mut rolling = RollingBuffer::new(self.config.max_output_chars);
        let mut discovered_session_id: Option<String> = session_id.map(|s| s.to_string());
        let mut last_edit = Instant::now();
        let mut last_payload = String::new();
        let start_time = Instant::now();
        let max_runtime = std::time::Duration::from_secs(self.config.max_runtime_seconds);
        let edit_interval =
            std::time::Duration::from_secs_f64(self.config.stream_edit_interval_secs);

        // Captured metadata from stream events
        let mut captured_meta: Option<SessionMeta> = None;
        let mut captured_usage: Option<UsageInfo> = None;

        // Open log file
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;

        writeln!(
            log_file,
            "$ {}",
            cmd.args
                .iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" ")
        )?;

        let mut timed_out = false;

        loop {
            if start_time.elapsed() > max_runtime {
                timed_out = true;
                let mut proc_guard = current_process.lock().await;
                if let Some(ref mut child) = *proc_guard {
                    let _ = child.kill().await;
                }
                break;
            }

            {
                let cancelled = *cancel_requested.lock().await;
                if cancelled {
                    let mut proc_guard = current_process.lock().await;
                    if let Some(ref mut child) = *proc_guard {
                        let _ = child.kill().await;
                    }
                    break;
                }
            }

            let line =
                tokio::time::timeout(std::time::Duration::from_millis(500), reader.next_line())
                    .await;

            match line {
                Ok(Ok(Some(text))) => {
                    writeln!(log_file, "{text}").ok();
                    let event = parse_stream_line(&text);

                    if let Some(sid) = event.session_id {
                        discovered_session_id = Some(sid);
                    }
                    if let Some(meta) = event.init_meta {
                        captured_meta = Some(meta);
                    }
                    if let Some(usage) = event.usage {
                        captured_usage = Some(usage);
                    }
                    if !event.text.is_empty() {
                        rolling.append(&event.text);
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => {
                    warn!("Error reading stdout: {e}");
                    break;
                }
                Err(_) => {}
            }

            // Periodic Telegram edit
            if last_edit.elapsed() >= edit_interval {
                let raw = rolling.value().trim();
                let display = if raw.is_empty() {
                    "(waiting...)"
                } else {
                    raw
                };
                let body = html_escape(display);
                let elapsed = start_time.elapsed().as_secs();
                let payload = format!(
                    "⏳ <b>Running</b>  ·  #{run_id}  ·  {elapsed}s\n\n<pre>{body}</pre>",
                );
                let truncated = truncate_for_telegram(&payload);
                if truncated != last_payload
                    && self
                        .edit_html(chat_id, status_msg_id, &truncated)
                        .await
                        .is_ok()
                {
                    last_payload = truncated;
                }
                last_edit = Instant::now();
            }
        }

        // Drain stderr
        let _ = stderr_handle.await;
        {
            let stderr_text = stderr_buf.lock().await;
            if !stderr_text.is_empty() {
                writeln!(log_file, "[stderr] {stderr_text}").ok();
                rolling.append(&format!("\n[stderr] {stderr_text}\n"));
            }
        }

        // Wait for process exit
        let exit_code = {
            let mut proc_guard = current_process.lock().await;
            if let Some(ref mut child) = *proc_guard {
                match child.wait().await {
                    Ok(status) => status.code().unwrap_or(-1),
                    Err(_) => -1,
                }
            } else {
                -1
            }
        };

        let cancelled = *cancel_requested.lock().await;
        let status = if timed_out {
            rolling.append("\n[timeout] Max runtime exceeded.\n");
            RunStatus::Failed
        } else if cancelled {
            rolling.append("\n[canceled] Canceled by user.\n");
            RunStatus::Canceled
        } else if exit_code == 0 {
            RunStatus::Succeeded
        } else {
            rolling.append(&format!("\n[exit-code] {exit_code}\n"));
            RunStatus::Failed
        };

        // Update metadata cache
        if captured_meta.is_some() || captured_usage.is_some() {
            let mut cache = self.info_cache.write().await;
            let entry = cache
                .entry(thread_key.as_str().to_string())
                .or_default();
            if let Some(meta) = captured_meta {
                entry.meta = meta;
            }
            if let Some(usage) = captured_usage {
                entry.usage = usage;
            }
        }

        let output_tail = rolling.value().trim().to_string();
        let output_tail = if output_tail.is_empty() {
            "(no output)".to_string()
        } else {
            output_tail
        };

        Ok(RunResult {
            status,
            output_tail,
            session_id: discovered_session_id,
            exit_code,
        })
    }

    async fn handle_compact(
        &self,
        thread_key: &ThreadKey,
        task: &IncomingTask,
        thread_state: &ThreadState,
        cancel_requested: &Mutex<bool>,
        current_process: &Mutex<Option<Child>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let status_msg = self
            .send_html(
                task.message.chat_id,
                task.message.thread_id,
                "⏳ <b>Compacting memory...</b>",
            )
            .await?;
        let status_msg_id = status_msg.id.0;

        let log_path = self.build_log_path(thread_key);
        let run_id = self
            .store
            .create_run(
                thread_key.as_str(),
                &thread_state.active_engine,
                status_msg_id,
                &log_path.to_string_lossy(),
            )
            .await?;

        let prompt = "Create a strict JSON summary for session compaction.\n\
                      Return JSON object with keys: decisions, current_state, \
                      file_changes, next_steps, constraints.\n\
                      Keep it concise and factual.";

        let result = self
            .execute_engine_stream(
                thread_key,
                thread_state,
                prompt,
                task.message.chat_id,
                status_msg_id,
                thread_state.active_session_id.as_deref(),
                None,
                cancel_requested,
                current_process,
                &log_path,
                run_id,
            )
            .await?;

        self.store
            .finish_run(run_id, result.status.as_str())
            .await?;

        if result.status != RunStatus::Succeeded {
            let html = format!(
                "❌ <b>Compact failed</b>\n\n<pre>{}</pre>",
                html_escape(&result.output_tail)
            );
            self.edit_html(task.message.chat_id, status_msg_id, &truncate_for_telegram(&html))
                .await?;
            return Ok(());
        }

        let summary = result.output_tail.trim().to_string();
        if summary.is_empty() {
            self.edit_html(
                task.message.chat_id,
                status_msg_id,
                "❌ <b>Compact failed</b>\nNo summary produced.",
            )
            .await?;
            return Ok(());
        }

        self.store
            .set_compact_summary(thread_key.as_str(), Some(&summary))
            .await?;
        self.store
            .set_active_session(thread_key.as_str(), None)
            .await?;

        // Seed a fresh session with the summary
        let seed_log = self.build_log_path(thread_key);
        let seed_run_id = self
            .store
            .create_run(
                thread_key.as_str(),
                &thread_state.active_engine,
                status_msg_id,
                &seed_log.to_string_lossy(),
            )
            .await?;

        let seed_result = self
            .execute_engine_stream(
                thread_key,
                thread_state,
                "Memory loaded. Reply exactly MEMORY_READY.",
                task.message.chat_id,
                status_msg_id,
                None,
                Some(&summary),
                cancel_requested,
                current_process,
                &seed_log,
                seed_run_id,
            )
            .await?;

        self.store
            .finish_run(seed_run_id, seed_result.status.as_str())
            .await?;

        if let Some(ref sid) = seed_result.session_id {
            self.store
                .set_active_session(thread_key.as_str(), Some(sid))
                .await?;
            self.store
                .add_session_history(thread_key.as_str(), &thread_state.active_engine, sid)
                .await?;
        }

        let final_html = format!(
            "✅ <b>Compacted</b>\n\n\
             Memory: {} chars stored\n\
             Session: <code>{}</code>",
            summary.len(),
            html_escape(
                seed_result
                    .session_id
                    .as_deref()
                    .unwrap_or("next message")
            ),
        );
        self.edit_html(task.message.chat_id, status_msg_id, &final_html)
            .await?;

        Ok(())
    }

    fn build_log_path(&self, thread_key: &ThreadKey) -> std::path::PathBuf {
        let thread_dir = self.config.logs_root.join(thread_key.slug());
        std::fs::create_dir_all(&thread_dir).ok();
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        thread_dir.join(format!("run-{ts}.log"))
    }

    // ── Telegram messaging helpers ──

    pub async fn send_text(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        text: &str,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let truncated = truncate_for_telegram(text);
        let mut req = self.tg.send_message(ChatId(chat_id), &truncated);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(
                tid as i32,
            )));
        }
        Ok(req.await?)
    }

    pub async fn send_html(
        &self,
        chat_id: i64,
        thread_id: Option<i64>,
        html: &str,
    ) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
        let truncated = truncate_for_telegram(html);
        let mut req = self
            .tg
            .send_message(ChatId(chat_id), &truncated)
            .parse_mode(ParseMode::Html);
        if let Some(tid) = thread_id {
            req = req.message_thread_id(teloxide::types::ThreadId(teloxide::types::MessageId(
                tid as i32,
            )));
        }
        Ok(req.await?)
    }

    #[allow(dead_code)]
    pub async fn edit_text(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let truncated = truncate_for_telegram(text);
        let result = self
            .tg
            .edit_message_text(
                ChatId(chat_id),
                teloxide::types::MessageId(message_id),
                &truncated,
            )
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("message is not modified") {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub async fn edit_html(
        &self,
        chat_id: i64,
        message_id: i32,
        html: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let truncated = truncate_for_telegram(html);
        let result = self
            .tg
            .edit_message_text(
                ChatId(chat_id),
                teloxide::types::MessageId(message_id),
                &truncated,
            )
            .parse_mode(ParseMode::Html)
            .await;

        match result {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("message is not modified") {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }
}

/// Telegram message limit is 4096 chars. Leave room for safety.
pub fn truncate_for_telegram(text: &str) -> String {
    const MAX_LEN: usize = 4000;
    if text.len() <= MAX_LEN {
        text.to_string()
    } else {
        let boundary = text
            .char_indices()
            .take_while(|(i, _)| *i < MAX_LEN)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(MAX_LEN);
        format!("{}...(truncated)", &text[..boundary])
    }
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn shell_escape(s: &str) -> String {
    if s.contains(char::is_whitespace) || s.contains('\'') || s.contains('"') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}
