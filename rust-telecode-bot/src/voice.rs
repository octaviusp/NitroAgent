use std::path::PathBuf;

use teloxide::net::Download;
use teloxide::prelude::*;
use tracing::info;

use crate::config::BotConfig;

/// Download a voice/audio file from Telegram to a local temp path.
pub async fn download_voice(
    tg: &Bot,
    file_id: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let file = tg.get_file(file_id).await?;

    let ext = file.path.rsplit('.').next().unwrap_or("ogg");
    let local_path = std::env::temp_dir().join(format!(
        "telecode_voice_{}.{ext}",
        file.meta.unique_id,
    ));

    let mut dst = tokio::fs::File::create(&local_path).await?;
    tg.download_file(&file.path, &mut dst).await?;

    info!(path = %local_path.display(), "Voice file downloaded");
    Ok(local_path)
}

/// Transcribe an audio file by calling sst.py as a subprocess.
pub async fn transcribe(
    config: &BotConfig,
    audio_path: &std::path::Path,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let output = tokio::process::Command::new(&config.sst_python)
        .arg(&config.sst_script)
        .arg(audio_path)
        .arg("--language")
        .arg(&config.sst_language)
        .arg("--arch")
        .arg(&config.sst_arch)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lines: Vec<&str> = stderr.lines().collect();
        let tail = if lines.len() > 5 {
            lines[lines.len() - 5..].join("\n")
        } else {
            stderr.into_owned()
        };
        return Err(format!("sst.py failed (exit {}): {tail}", output.status).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
