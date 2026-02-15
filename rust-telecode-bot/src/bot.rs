use std::io::Write as _;
use std::time::Instant;

use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;
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
}

impl BotCore {
    pub fn new(config: BotConfig, store: ThreadStore, tg: Bot) -> Self {
        Self { config, store, tg }
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
            // Handle cancel specially -- it needs subprocess access
            if command.name == "cancel" {
                let mut proc_guard = current_process.lock().await;
                if let Some(ref mut child) = *proc_guard {
                    let _ = child.kill().await;
                    self.send_html(
                        task.message.chat_id,
                        task.message.thread_id,
                        "<b>Run canceled</b>\nStopping current execution.",
                    )
                    .await?;
                } else {
                    self.send_html(
                        task.message.chat_id,
                        task.message.thread_id,
                        "<b>No active run</b>\nNothing to cancel.",
                    )
                    .await?;
                }
                return Ok(());
            }

            // Handle compact -- requires engine subprocess
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

            // Delegate to simple command handlers
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

            // Unhandled subprocess commands: restart, mcps, skills, tasks
            self.send_html(
                task.message.chat_id,
                task.message.thread_id,
                &format!(
                    "<b>/{}</b> is not yet implemented in the Rust bot.",
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
            .send_text(
                task.message.chat_id,
                task.message.thread_id,
                &format!("Running {}...", thread_state.active_engine),
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
            .unwrap_or("unknown");

        let header = format!(
            "{icon} <b>{status}</b>\n\
             Engine: {engine}\n\
             Run: {run_id}\n\
             Session: <code>{session}</code>",
            icon = result.status.icon(),
            status = result.status.as_str().to_uppercase(),
            engine = html_escape(&thread_state.active_engine),
            session = html_escape(sid_display),
        );

        let body = html_escape(&result.output_tail);
        let final_html = format!("{header}\n\n<pre>{body}</pre>");
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
        let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
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

        // Spawn a task to drain stderr into a shared buffer
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
            // Check timeout
            if start_time.elapsed() > max_runtime {
                timed_out = true;
                let mut proc_guard = current_process.lock().await;
                if let Some(ref mut child) = *proc_guard {
                    let _ = child.kill().await;
                }
                break;
            }

            // Check cancel
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

            // Read next line with short timeout
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
                    if !event.text.is_empty() {
                        rolling.append(&event.text);
                    }
                }
                Ok(Ok(None)) => break, // EOF
                Ok(Err(e)) => {
                    warn!("Error reading stdout: {e}");
                    break;
                }
                Err(_) => {} // Timeout, continue loop
            }

            // Periodic Telegram message edit
            if last_edit.elapsed() >= edit_interval {
                let raw = rolling.value().trim();
                let display = if raw.is_empty() {
                    "(waiting for output)"
                } else {
                    raw
                };
                let body = html_escape(display);
                let payload = format!(
                    "Running {engine} | run {run_id}\n\n<pre>{body}</pre>",
                    engine = html_escape(&thread_state.active_engine),
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

        // Wait for stderr drain and append to log/buffer
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
            rolling.append("\n[timeout] Run exceeded configured max runtime.\n");
            RunStatus::Failed
        } else if cancelled {
            rolling.append("\n[canceled] Run canceled by user.\n");
            RunStatus::Canceled
        } else if exit_code == 0 {
            RunStatus::Succeeded
        } else {
            rolling.append(&format!("\n[exit-code] {exit_code}\n"));
            RunStatus::Failed
        };

        let output_tail = rolling.value().trim().to_string();
        let output_tail = if output_tail.is_empty() {
            "(no streamed output)".to_string()
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
            .send_text(
                task.message.chat_id,
                task.message.thread_id,
                "Compacting memory...",
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
            self.edit_text(
                task.message.chat_id,
                status_msg_id,
                &format!(
                    "Compact failed ({}).\n\n{}",
                    result.status, result.output_tail
                ),
            )
            .await?;
            return Ok(());
        }

        let summary = result.output_tail.trim().to_string();
        if summary.is_empty() {
            self.edit_text(
                task.message.chat_id,
                status_msg_id,
                "Compact failed: no summary produced.",
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

        let final_text = format!(
            "Compact complete.\nSummary stored ({} chars).\nSeed session: {}",
            summary.len(),
            seed_result
                .session_id
                .as_deref()
                .unwrap_or("created on next message")
        );
        self.edit_text(task.message.chat_id, status_msg_id, &final_text)
            .await?;

        Ok(())
    }

    fn build_log_path(&self, thread_key: &ThreadKey) -> std::path::PathBuf {
        let thread_dir = self.config.logs_root.join(thread_key.slug());
        std::fs::create_dir_all(&thread_dir).ok();
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        thread_dir.join(format!("run-{ts}.log"))
    }

    // -- Telegram messaging helpers --

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
fn truncate_for_telegram(text: &str) -> String {
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

fn html_escape(s: &str) -> String {
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
