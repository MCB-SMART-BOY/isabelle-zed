use crate::protocol::{
    Message, MessageType, Position, diagnostics_message_from_request, markup_message_from_request,
    parse_message, to_ndjson,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
enum AdapterMode {
    SpawnReal { isabelle_path: String },
    MockSubprocess,
    Socket { address: String },
}

enum AdapterWriter {
    Child(ChildStdin),
    Socket(OwnedWriteHalf),
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn adapter process: {0}")]
    Spawn(String),
    #[error("failed to connect adapter socket: {0}")]
    Connect(String),
    #[error("adapter process is not running")]
    NotRunning,
    #[error("adapter process exited: {0}")]
    ProcessExited(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("max retries exceeded after {retries} attempts; last error: {last_error}")]
    MaxRetriesExceeded { retries: u32, last_error: String },
    #[error("adapter output receiver has already been taken")]
    OutputReceiverTaken,
}

pub struct ProcessManager {
    mode: AdapterMode,
    max_retries: u32,
    base_backoff_ms: u64,
    child: Option<Child>,
    writer: Option<AdapterWriter>,
    output_tx: mpsc::Sender<String>,
    output_rx: Option<mpsc::Receiver<String>>,
}

impl ProcessManager {
    pub fn new(isabelle_path: String, mock_mode: bool, adapter_socket: Option<String>) -> Self {
        let mode = if mock_mode {
            AdapterMode::MockSubprocess
        } else if let Some(address) = adapter_socket {
            AdapterMode::Socket { address }
        } else {
            AdapterMode::SpawnReal { isabelle_path }
        };

        let (output_tx, output_rx) = mpsc::channel(256);

        Self {
            mode,
            max_retries: 3,
            base_backoff_ms: 250,
            child: None,
            writer: None,
            output_tx,
            output_rx: Some(output_rx),
        }
    }

    pub fn take_output_receiver(&mut self) -> Result<mpsc::Receiver<String>, ProcessError> {
        self.output_rx
            .take()
            .ok_or(ProcessError::OutputReceiverTaken)
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        self.stop().await?;

        match self.mode.clone() {
            AdapterMode::SpawnReal { isabelle_path } => {
                let mut command = Command::new(isabelle_path);
                command.arg("scala");
                self.start_spawn(command).await
            }
            AdapterMode::MockSubprocess => {
                let current_exe: PathBuf =
                    std::env::current_exe().map_err(|err| ProcessError::Spawn(err.to_string()))?;
                let mut command = Command::new(current_exe);
                command.arg("--mock-adapter");
                self.start_spawn(command).await
            }
            AdapterMode::Socket { address } => self.start_socket(&address).await,
        }
    }

    pub async fn send(&mut self, message: &Message) -> Result<(), ProcessError> {
        let line = to_ndjson(message).map_err(|err| ProcessError::Protocol(err.to_string()))?;

        let mut attempts = 0;
        loop {
            match self.write_line(&line).await {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if attempts >= self.max_retries {
                        return Err(ProcessError::MaxRetriesExceeded {
                            retries: self.max_retries,
                            last_error: err.to_string(),
                        });
                    }

                    warn!(
                        "adapter write failed (attempt {}/{}): {}",
                        attempts + 1,
                        self.max_retries,
                        err
                    );
                    self.restart_with_backoff(attempts).await?;
                    attempts += 1;
                }
            }
        }
    }

    pub async fn stop(&mut self) -> Result<(), ProcessError> {
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    async fn start_spawn(&mut self, mut command: Command) -> Result<(), ProcessError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        info!("starting adapter process: {:?}", command);
        let mut child = command
            .spawn()
            .map_err(|err| ProcessError::Spawn(err.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ProcessError::Spawn("failed to acquire child stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProcessError::Spawn("failed to acquire child stdout".to_string()))?;

        self.writer = Some(AdapterWriter::Child(stdin));
        self.child = Some(child);
        self.spawn_output_reader(stdout);

        Ok(())
    }

    async fn start_socket(&mut self, address: &str) -> Result<(), ProcessError> {
        info!("connecting to adapter socket: {address}");
        let stream = TcpStream::connect(address)
            .await
            .map_err(|err| ProcessError::Connect(err.to_string()))?;
        let (reader, writer) = stream.into_split();

        self.writer = Some(AdapterWriter::Socket(writer));
        self.child = None;
        self.spawn_socket_output_reader(reader);
        Ok(())
    }

    fn spawn_output_reader(&self, stdout: tokio::process::ChildStdout) {
        let output_tx = self.output_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if output_tx.send(line).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        error!("adapter stdout read error: {err}");
                        break;
                    }
                }
            }
        });
    }

    fn spawn_socket_output_reader(&self, reader: OwnedReadHalf) {
        let output_tx = self.output_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if output_tx.send(line).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        error!("adapter socket read error: {err}");
                        break;
                    }
                }
            }
        });
    }

    async fn write_line(&mut self, line: &str) -> Result<(), ProcessError> {
        if let Some(child) = self.child.as_mut()
            && let Some(status) = child.try_wait()?
        {
            self.writer = None;
            return Err(ProcessError::ProcessExited(status.to_string()));
        }

        match self.writer.as_mut() {
            Some(AdapterWriter::Child(stdin)) => {
                debug!("forwarding to adapter process: {}", line.trim_end());
                stdin.write_all(line.as_bytes()).await?;
                stdin.flush().await?;
                Ok(())
            }
            Some(AdapterWriter::Socket(writer)) => {
                debug!("forwarding to adapter socket: {}", line.trim_end());
                writer.write_all(line.as_bytes()).await?;
                writer.flush().await?;
                Ok(())
            }
            None => Err(ProcessError::NotRunning),
        }
    }

    async fn restart_with_backoff(&mut self, attempt: u32) -> Result<(), ProcessError> {
        let delay = Duration::from_millis(self.base_backoff_ms.saturating_mul(1_u64 << attempt));
        self.stop().await?;
        tokio::time::sleep(delay).await;
        self.start().await
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkupRequest {
    uri: String,
    offset: Position,
    #[serde(default)]
    info: Option<String>,
}

pub async fn run_mock_adapter() -> Result<(), ProcessError> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut input = BufReader::new(stdin).lines();
    let mut output = tokio::io::BufWriter::new(stdout);

    while let Some(line) = input.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let request = match parse_message(&line) {
            Ok(message) => message,
            Err(err) => {
                warn!("mock adapter received invalid JSON: {err}");
                continue;
            }
        };

        let response = match request.msg_type {
            MessageType::DocumentPush => {
                let payload = match request.push_payload() {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!("mock adapter invalid document.push payload: {err}");
                        continue;
                    }
                };
                diagnostics_message_from_request(
                    &request,
                    &payload.uri,
                    crate::protocol::Severity::Error,
                    "Parse error",
                )
                .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::DocumentCheck => {
                let payload = match request.check_payload() {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!("mock adapter invalid document.check payload: {err}");
                        continue;
                    }
                };
                diagnostics_message_from_request(
                    &request,
                    &payload.uri,
                    crate::protocol::Severity::Error,
                    "Parse error",
                )
                .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Markup => {
                let payload: MarkupRequest = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let info = payload
                    .info
                    .unwrap_or_else(|| "Mock hover information".to_string());
                markup_message_from_request(&request, &payload.uri, payload.offset, &info)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Diagnostics => {
                continue;
            }
        };

        let line = to_ndjson(&response).map_err(|err| ProcessError::Protocol(err.to_string()))?;
        debug!("mock adapter -> bridge: {}", line.trim_end());
        output.write_all(line.as_bytes()).await?;
        output.flush().await?;
    }

    Ok(())
}
