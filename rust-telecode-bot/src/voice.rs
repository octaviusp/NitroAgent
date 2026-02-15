use std::path::PathBuf;

use teloxide::net::Download;
use teloxide::prelude::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::config::BotConfig;

// ── Temp file RAII guard ──

/// Guard that auto-deletes a temporary file on drop (sync fs remove).
pub struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ── Telegram file download ──

/// Download a voice/audio file from Telegram to a local temp path.
/// Returns a guard that auto-deletes the file when dropped.
pub async fn download_voice(
    tg: &Bot,
    file_id: &str,
) -> Result<TempFileGuard, Box<dyn std::error::Error + Send + Sync>> {
    let file = tg.get_file(file_id).await?;

    let ext = file.path.rsplit('.').next().unwrap_or("ogg");
    let local_path = std::env::temp_dir().join(format!(
        "telecode_voice_{}.{ext}",
        file.meta.unique_id,
    ));

    let mut dst = tokio::fs::File::create(&local_path).await?;
    tg.download_file(&file.path, &mut dst).await?;

    info!(path = %local_path.display(), "Voice file downloaded");
    Ok(TempFileGuard { path: local_path })
}

// ── Photo download ──

/// Download a photo from Telegram into the workspace directory.
/// Returns the relative filename (e.g. `_photo_abc123.jpg`) so Claude Code
/// can access it with a relative path from its working directory.
pub async fn download_photo(
    tg: &Bot,
    file_id: &str,
    workspace: &std::path::Path,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let file = tg.get_file(file_id).await?;

    let ext = file.path.rsplit('.').next().unwrap_or("jpg");
    let filename = format!("_photo_{}.{ext}", file.meta.unique_id);
    let local_path = workspace.join(&filename);

    let mut dst = tokio::fs::File::create(&local_path).await?;
    tg.download_file(&file.path, &mut dst).await?;

    info!(path = %local_path.display(), "Photo downloaded to workspace");
    Ok(filename)
}

// ── Startup checks ──

/// Check if ffmpeg is available on PATH. Call at startup and warn if missing.
pub fn check_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Persistent sst.py server ──

/// Persistent voice transcriber that keeps the ML model loaded across calls.
///
/// On first `transcribe()`, spawns `sst.py --server` which loads the model once
/// and then processes file paths via stdin/stdout JSON protocol. Automatically
/// restarts the server if it dies.
pub struct VoiceTranscriber {
    python: String,
    script: PathBuf,
    language: String,
    arch: String,
    server: Mutex<Option<SstServer>>,
}

struct SstServer {
    child: tokio::process::Child,
    stdin: BufWriter<tokio::process::ChildStdin>,
    stdout: BufReader<tokio::process::ChildStdout>,
}

#[derive(serde::Deserialize)]
struct SstResponse {
    text: Option<String>,
    error: Option<String>,
}

impl VoiceTranscriber {
    pub fn new(config: &BotConfig) -> Self {
        Self {
            python: config.sst_python.clone(),
            script: config.sst_script.clone(),
            language: config.sst_language.clone(),
            arch: config.sst_arch.clone(),
            server: Mutex::new(None),
        }
    }

    /// Transcribe an audio file. Reuses or spawns a persistent sst.py server.
    pub async fn transcribe(
        &self,
        audio_path: &std::path::Path,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut guard = self.server.lock().await;

        // Spawn server if not running
        if guard.is_none() {
            *guard = Some(self.spawn_server().await?);
        }
        let server = guard.as_mut().unwrap();

        // Send file path
        let line = format!("{}\n", audio_path.display());
        if let Err(e) = server.stdin.write_all(line.as_bytes()).await {
            warn!("sst.py stdin write failed: {e}, will restart");
            Self::kill_server(&mut *guard).await;
            return Err(format!("sst.py write failed: {e}").into());
        }
        if let Err(e) = server.stdin.flush().await {
            warn!("sst.py stdin flush failed: {e}, will restart");
            Self::kill_server(&mut *guard).await;
            return Err(format!("sst.py flush failed: {e}").into());
        }

        // Read JSON response (120s timeout for long audio files)
        let mut response_line = String::new();
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            server.stdout.read_line(&mut response_line),
        )
        .await
        {
            Ok(Ok(0)) | Ok(Err(_)) => {
                warn!("sst.py server died, will restart on next call");
                Self::kill_server(&mut *guard).await;
                return Err("sst.py server process ended unexpectedly".into());
            }
            Err(_) => {
                warn!("sst.py transcription timed out (120s)");
                Self::kill_server(&mut *guard).await;
                return Err("sst.py transcription timed out".into());
            }
            Ok(Ok(_)) => {}
        }

        let resp: SstResponse = serde_json::from_str(response_line.trim())
            .map_err(|e| format!("sst.py invalid response: {e}: {response_line}"))?;

        if let Some(err) = resp.error {
            return Err(format!("sst.py: {err}").into());
        }

        Ok(resp.text.unwrap_or_default())
    }

    async fn spawn_server(
        &self,
    ) -> Result<SstServer, Box<dyn std::error::Error + Send + Sync>> {
        info!(
            python = %self.python,
            script = %self.script.display(),
            "Spawning sst.py server"
        );

        let mut child = tokio::process::Command::new(&self.python)
            .arg(&self.script)
            .arg("--server")
            .arg("--language")
            .arg(&self.language)
            .arg("--arch")
            .arg(&self.arch)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().ok_or("Failed to capture sst.py stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to capture sst.py stdout")?;

        // Drain stderr in background so it doesn't block the process
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    debug!(target: "sst_stderr", "{line}");
                }
            });
        }

        // Wait for ready signal (model loading can take 30-60s first time)
        let mut stdout = BufReader::new(stdout);
        let mut ready_line = String::new();
        match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            stdout.read_line(&mut ready_line),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => {
                let trimmed = ready_line.trim();
                if !trimmed.contains("ready") {
                    let _ = child.kill().await;
                    return Err(
                        format!("sst.py unexpected startup output: {trimmed}").into()
                    );
                }
                info!("sst.py server ready");
            }
            Ok(Ok(_)) => {
                // Read 0 bytes = process exited before sending ready
                let status = child.wait().await.ok();
                return Err(
                    format!("sst.py exited during startup (status: {status:?})").into(),
                );
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return Err(format!("sst.py stdout read error: {e}").into());
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err("sst.py startup timed out (120s) — model download may be required".into());
            }
        }

        Ok(SstServer {
            child,
            stdin: BufWriter::new(stdin),
            stdout,
        })
    }

    async fn kill_server(slot: &mut Option<SstServer>) {
        if let Some(mut server) = slot.take() {
            let _ = server.child.kill().await;
        }
    }
}
