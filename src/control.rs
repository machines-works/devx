use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::events::DevxEvent;

#[derive(Debug, Deserialize)]
struct ControlCommand {
    cmd: String,
    service: Option<String>,
}

/// Returns the socket path for a given project name.
pub fn socket_path(project_name: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/devx-{}.sock", project_name))
}

/// Start the control socket server. Listens for JSON commands and emits
/// DevxEvent variants. Runs until the sender is dropped or the task is
/// cancelled.
pub async fn serve(project_name: &str, event_tx: mpsc::Sender<DevxEvent>) -> Result<()> {
    let path = socket_path(project_name);

    // Remove stale socket file if it exists
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        if let Err(e) = handle_connection(stream, &event_tx).await {
            let _ = event_tx.try_send(DevxEvent::LogLine {
                service: "devx".to_string(),
                line: format!("[control] connection error: {}", e),
                is_stderr: true,
            });
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    event_tx: &mpsc::Sender<DevxEvent>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let resp = match serde_json::from_str::<ControlCommand>(line.trim()) {
        Ok(cmd) => match cmd.cmd.as_str() {
            "shutdown" => {
                let _ = event_tx.send(DevxEvent::ControlShutdown).await;
                r#"{"ok":true}"#
            }
            "restart" => {
                if let Some(service) = cmd.service {
                    let _ = event_tx
                        .send(DevxEvent::ControlRestart {
                            service: service.clone(),
                        })
                        .await;
                    r#"{"ok":true}"#
                } else {
                    r#"{"error":"restart requires a 'service' field"}"#
                }
            }
            _ => r#"{"error":"unknown command"}"#,
        },
        Err(e) => {
            // Write error inline since we can't use a formatted &str
            let msg = format!(r#"{{"error":"invalid json: {}"}}"#, e);
            writer.write_all(msg.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.shutdown().await?;
            return Ok(());
        }
    };

    writer.write_all(resp.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;
    Ok(())
}

/// Remove the socket file on shutdown.
pub fn cleanup(project_name: &str) {
    let path = socket_path(project_name);
    let _ = std::fs::remove_file(path);
}

/// Send a command to a running devx instance via the control socket.
pub async fn send_command(project_name: &str, command: &str) -> Result<String> {
    let path = socket_path(project_name);
    if !Path::new(&path).exists() {
        anyhow::bail!(
            "no running devx instance found for project '{}' (socket {} not found)",
            project_name,
            path.display()
        );
    }

    let stream = UnixStream::connect(&path).await?;
    let (reader, mut writer) = stream.into_split();

    writer.write_all(command.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;

    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    Ok(response)
}
