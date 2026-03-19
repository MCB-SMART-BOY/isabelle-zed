use crate::transport::BridgeEndpoint;
use shell_words::split as split_shell_words;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::process::Stdio;
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::time::Duration;
use tracing::{debug, error, info, warn};

pub(crate) fn parse_autostart_command(command: &str) -> Result<(String, Vec<String>), String> {
    let parts = split_shell_words(command).map_err(|err| err.to_string())?;
    if parts.is_empty() {
        return Err("autostart command is empty".to_string());
    }

    Ok((parts[0].clone(), parts[1..].to_vec()))
}

pub(crate) async fn autostart_bridge_if_needed(
    endpoint: &BridgeEndpoint,
    command_env: &str,
    timeout_env: &str,
    default_timeout_ms: u64,
) -> Option<Child> {
    if endpoint_is_healthy(endpoint).await {
        return None;
    }

    if let BridgeEndpoint::Unix(path) = endpoint
        && path.exists()
        && let Err(err) = remove_stale_socket(path)
    {
        error!(
            "failed to remove stale bridge socket {}: {err}",
            path.display()
        );
        return None;
    }

    let command = std::env::var(command_env).ok()?;
    if command.trim().is_empty() {
        return None;
    }

    let (program, args) = match parse_autostart_command(&command) {
        Ok(parsed) => parsed,
        Err(err) => {
            error!("invalid bridge autostart command: {err}");
            return None;
        }
    };

    info!("autostarting bridge via command: {} {:?}", program, args);
    let child = match Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            error!("failed to autostart bridge: {err}");
            return None;
        }
    };

    let timeout_ms = std::env::var(timeout_env)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default_timeout_ms);

    if !wait_for_endpoint(endpoint, Duration::from_millis(timeout_ms)).await {
        warn!(
            "bridge autostart command launched but endpoint {} was not ready within {}ms",
            endpoint.describe(),
            timeout_ms
        );
    }

    Some(child)
}

async fn endpoint_is_healthy(endpoint: &BridgeEndpoint) -> bool {
    match endpoint {
        BridgeEndpoint::Unix(path) => {
            #[cfg(unix)]
            {
                match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(path))
                    .await
                {
                    Ok(Ok(stream)) => {
                        drop(stream);
                        true
                    }
                    Ok(Err(err)) => {
                        debug!("bridge socket {} is not connectable: {err}", path.display());
                        false
                    }
                    Err(_) => false,
                }
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                false
            }
        }
        BridgeEndpoint::Tcp(address) => {
            match tokio::time::timeout(Duration::from_millis(200), TcpStream::connect(address))
                .await
            {
                Ok(Ok(stream)) => {
                    drop(stream);
                    true
                }
                Ok(Err(err)) => {
                    debug!("bridge tcp endpoint {} is not connectable: {err}", address);
                    false
                }
                Err(_) => false,
            }
        }
    }
}

fn remove_stale_socket(socket_path: &std::path::Path) -> Result<(), std::io::Error> {
    let metadata = std::fs::symlink_metadata(socket_path)?;
    #[cfg(unix)]
    {
        if metadata.file_type().is_socket() {
            return std::fs::remove_file(socket_path);
        }
        Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "refusing to remove non-socket path before autostart: {}",
                socket_path.display()
            ),
        ))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err(std::io::Error::new(
            ErrorKind::Unsupported,
            "non-unix platforms do not support unix socket cleanup",
        ))
    }
}

async fn wait_for_endpoint(endpoint: &BridgeEndpoint, timeout: Duration) -> bool {
    let start = tokio::time::Instant::now();
    while start.elapsed() <= timeout {
        if endpoint_is_healthy(endpoint).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
