//! External players. MPV is driven over its JSON IPC (a named pipe on Windows, a Unix socket
//! elsewhere) so B-Anime knows how far you got and can mark episodes watched.

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{Error, Result};

/// A fresh IPC endpoint name for one MPV instance.
pub fn ipc_endpoint() -> String {
    let id = format!("{:016x}", rand::random::<u64>());
    if cfg!(windows) {
        format!(r"\\.\pipe\banime-mpv-{id}")
    } else {
        std::env::temp_dir().join(format!("banime-mpv-{id}.sock")).to_string_lossy().into_owned()
    }
}

/// Connects to a starting MPV (retrying while it opens the endpoint) and reports
/// `(position, duration)` in seconds whenever either changes, until MPV exits.
pub async fn watch_mpv(endpoint: &str, mut on_progress: impl FnMut(f64, f64)) -> Result<()> {
    #[cfg(windows)]
    let stream = {
        use tokio::net::windows::named_pipe::ClientOptions;
        let mut tries = 0;
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(c) => break c,
                Err(_) if tries < 60 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => return Err(Error::Config(format!("couldn't connect to MPV: {e}"))),
            }
        }
    };
    #[cfg(unix)]
    let stream = {
        let mut tries = 0;
        loop {
            match tokio::net::UnixStream::connect(endpoint).await {
                Ok(c) => break c,
                Err(_) if tries < 60 => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => return Err(Error::Config(format!("couldn't connect to MPV: {e}"))),
            }
        }
    };

    let (read, mut write) = tokio::io::split(stream);
    for (id, prop) in [(1, "time-pos"), (2, "duration")] {
        let cmd = format!("{{\"command\":[\"observe_property\",{id},\"{prop}\"]}}\n");
        write.write_all(cmd.as_bytes()).await?;
    }
    let mut lines = BufReader::new(read).lines();
    let (mut position, mut duration) = (0.0_f64, 0.0_f64);
    while let Some(line) = lines.next_line().await? {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        match (msg["event"].as_str(), msg["name"].as_str(), msg["data"].as_f64()) {
            (Some("property-change"), Some("time-pos"), Some(v)) => position = v,
            (Some("property-change"), Some("duration"), Some(v)) => duration = v,
            (Some("shutdown"), _, _) => break,
            _ => continue,
        }
        on_progress(position, duration);
    }
    Ok(())
}
