use crate::protocol::{
    Diagnostic, Message, MessageType, Position, Range, Severity, diagnostics_message_from_request,
    markup_message_from_request, parse_message, to_ndjson,
};
use regex::Regex;
use serde::Deserialize;
use shell_words::split as split_shell_words;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tempfile::tempdir;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(Debug, Clone)]
enum AdapterMode {
    SpawnReal {
        isabelle_path: String,
        logic: String,
        session_dirs: Vec<PathBuf>,
        adapter_command: Option<String>,
    },
    MockSubprocess,
    Socket {
        address: String,
    },
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
    pub fn new(
        isabelle_path: String,
        logic: String,
        session_dirs: Vec<PathBuf>,
        mock_mode: bool,
        adapter_socket: Option<String>,
        adapter_command: Option<String>,
    ) -> Self {
        let adapter_command = adapter_command.and_then(|command| {
            let trimmed = command.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });

        let mode = if mock_mode {
            AdapterMode::MockSubprocess
        } else if let Some(address) = adapter_socket {
            AdapterMode::Socket { address }
        } else {
            AdapterMode::SpawnReal {
                isabelle_path,
                logic,
                session_dirs,
                adapter_command,
            }
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
            AdapterMode::SpawnReal {
                isabelle_path,
                logic,
                session_dirs,
                adapter_command,
            } => match adapter_command {
                Some(command_line) => {
                    let (program, args) = parse_adapter_command(&command_line)?;
                    let mut command = Command::new(program);
                    command.args(args);
                    self.start_spawn(command).await
                }
                None => {
                    self.start_default_real_adapter(&isabelle_path, &logic, &session_dirs)
                        .await
                }
            },
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
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProcessError::Spawn("failed to acquire child stderr".to_string()))?;

        self.writer = Some(AdapterWriter::Child(stdin));
        self.child = Some(child);
        self.spawn_output_reader(stdout);
        self.spawn_stderr_reader(stderr);

        Ok(())
    }

    async fn start_default_real_adapter(
        &mut self,
        isabelle_path: &str,
        logic: &str,
        session_dirs: &[PathBuf],
    ) -> Result<(), ProcessError> {
        let current_exe: PathBuf =
            std::env::current_exe().map_err(|err| ProcessError::Spawn(err.to_string()))?;
        let mut command = Command::new(current_exe);
        command.arg("--real-adapter");
        command.arg("--isabelle-path");
        command.arg(isabelle_path);
        command.arg("--logic");
        command.arg(logic);
        for session_dir in session_dirs {
            command.arg("--session-dir");
            command.arg(session_dir);
        }
        self.start_spawn(command).await
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

    fn spawn_stderr_reader(&self, stderr: tokio::process::ChildStderr) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        warn!("adapter stderr: {line}");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        error!("adapter stderr read error: {err}");
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

fn parse_adapter_command(command_line: &str) -> Result<(String, Vec<String>), ProcessError> {
    let parts = split_shell_words(command_line).map_err(|err| {
        ProcessError::Spawn(format!(
            "invalid --adapter-command (shell parse error): {err}"
        ))
    })?;

    if parts.is_empty() {
        return Err(ProcessError::Spawn(
            "invalid --adapter-command: command is empty".to_string(),
        ));
    }

    Ok((parts[0].clone(), parts[1..].to_vec()))
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

struct RealAdapterState {
    isabelle_path: String,
    logic: String,
    session_dirs: Vec<PathBuf>,
    latest_documents: HashMap<String, (String, i64)>,
}

impl RealAdapterState {
    fn new(isabelle_path: String, logic: String, session_dirs: Vec<PathBuf>) -> Self {
        Self {
            isabelle_path,
            logic,
            session_dirs,
            latest_documents: HashMap::new(),
        }
    }

    async fn update_document(&mut self, uri: &str, text: String, version: i64) -> Vec<Diagnostic> {
        self.latest_documents
            .insert(uri.to_string(), (text.clone(), version));
        self.run_isabelle_check(uri, &text).await
    }

    async fn check_document(&self, uri: &str) -> Vec<Diagnostic> {
        let text = if let Some((text, _)) = self.latest_documents.get(uri) {
            Some(text.clone())
        } else {
            read_document_from_uri(uri).await
        };

        let Some(text) = text else {
            return vec![Diagnostic {
                uri: uri.to_string(),
                range: Range {
                    start: Position { line: 1, col: 1 },
                    end: Position { line: 1, col: 2 },
                },
                severity: Severity::Warning,
                message: "No theory text available for check".to_string(),
            }];
        };

        if text.trim().is_empty() {
            return vec![Diagnostic {
                uri: uri.to_string(),
                range: Range {
                    start: Position { line: 1, col: 1 },
                    end: Position { line: 1, col: 2 },
                },
                severity: Severity::Warning,
                message: "No theory text available for check".to_string(),
            }];
        }

        self.run_isabelle_check(uri, &text).await
    }

    async fn run_isabelle_check(&self, uri: &str, text: &str) -> Vec<Diagnostic> {
        let theory_name = resolve_theory_name(uri, text);
        let temp_dir = match tempdir() {
            Ok(dir) => dir,
            Err(err) => {
                return vec![Diagnostic {
                    uri: uri.to_string(),
                    range: Range {
                        start: Position { line: 1, col: 1 },
                        end: Position { line: 1, col: 2 },
                    },
                    severity: Severity::Error,
                    message: format!("Failed to create temp dir: {err}"),
                }];
            }
        };
        let theory_path = temp_dir.path().join(format!("{theory_name}.thy"));

        if let Err(err) = std::fs::write(&theory_path, text) {
            return vec![Diagnostic {
                uri: uri.to_string(),
                range: Range {
                    start: Position { line: 1, col: 1 },
                    end: Position { line: 1, col: 2 },
                },
                severity: Severity::Error,
                message: format!("Failed to write theory file: {err}"),
            }];
        }

        let mut command = Command::new(&self.isabelle_path);
        command.arg("process_theories");
        command.arg("-l");
        command.arg(&self.logic);
        for session_dir in self.session_dirs_for_uri(uri) {
            command.arg("-d");
            command.arg(session_dir);
        }
        command.arg("-D");
        command.arg(temp_dir.path());
        command.arg("-O");
        command.arg(&theory_name);
        let output = command.output().await;

        match output {
            Ok(result) => {
                let mut combined = String::new();
                combined.push_str(&String::from_utf8_lossy(&result.stdout));
                if !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&result.stderr));
                let exit_code = result.status.code().unwrap_or(1);
                parse_process_theories_diagnostics(uri, &combined, exit_code)
            }
            Err(err) => vec![Diagnostic {
                uri: uri.to_string(),
                range: Range {
                    start: Position { line: 1, col: 1 },
                    end: Position { line: 1, col: 2 },
                },
                severity: Severity::Error,
                message: format!("Failed to run isabelle process_theories: {err}"),
            }],
        }
    }

    async fn hover_markup(&self, uri: &str, offset: Position) -> String {
        let text = if let Some((text, _)) = self.latest_documents.get(uri) {
            Some(text.clone())
        } else {
            read_document_from_uri(uri).await
        };

        let line = normalize_one_based(offset.line);
        let col = normalize_one_based(offset.col);

        match text {
            Some(text) => build_hover_info(&text, line, col),
            None => format!("No theory text available for hover at {line}:{col}"),
        }
    }

    fn session_dirs_for_uri(&self, uri: &str) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for session_dir in &self.session_dirs {
            if !dirs.iter().any(|dir| dir == session_dir) {
                dirs.push(session_dir.clone());
            }
        }

        if let Some(parent) = parent_dir_from_file_uri(uri)
            && !dirs.iter().any(|dir| dir == &parent)
        {
            dirs.push(parent);
        }

        dirs
    }
}

pub async fn run_real_adapter(
    isabelle_path: String,
    logic: String,
    session_dirs: Vec<PathBuf>,
) -> Result<(), ProcessError> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut input = BufReader::new(stdin).lines();
    let mut output = tokio::io::BufWriter::new(stdout);
    let mut state = RealAdapterState::new(isabelle_path, logic, session_dirs);

    while let Some(line) = input.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let request = match parse_message(&line) {
            Ok(message) => message,
            Err(err) => {
                warn!("real adapter received invalid JSON: {err}");
                continue;
            }
        };

        let response = match request.msg_type {
            MessageType::DocumentPush => {
                let payload = match request.push_payload() {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!("real adapter invalid document.push payload: {err}");
                        continue;
                    }
                };
                let diagnostics = state
                    .update_document(&payload.uri, payload.text, request.version)
                    .await;
                diagnostics_response_from_request(&request, diagnostics)?
            }
            MessageType::DocumentCheck => {
                let payload = match request.check_payload() {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!("real adapter invalid document.check payload: {err}");
                        continue;
                    }
                };
                let diagnostics = state.check_document(&payload.uri).await;
                diagnostics_response_from_request(&request, diagnostics)?
            }
            MessageType::Markup => {
                let payload: MarkupRequest = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let info = state
                    .hover_markup(&payload.uri, payload.offset.clone())
                    .await;
                markup_message_from_request(&request, &payload.uri, payload.offset, &info)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Diagnostics => {
                continue;
            }
        };

        let line = to_ndjson(&response).map_err(|err| ProcessError::Protocol(err.to_string()))?;
        debug!("real adapter -> bridge: {}", line.trim_end());
        output.write_all(line.as_bytes()).await?;
        output.flush().await?;
    }

    Ok(())
}

fn diagnostics_response_from_request(
    request: &Message,
    diagnostics: Vec<Diagnostic>,
) -> Result<Message, ProcessError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::Diagnostics,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(diagnostics)
            .map_err(|err| ProcessError::Protocol(err.to_string()))?,
    })
}

fn parse_process_theories_diagnostics(uri: &str, output: &str, exit_code: i32) -> Vec<Diagnostic> {
    if exit_code == 0 {
        return Vec::new();
    }

    let error_line_regex =
        Regex::new(r#"^\*\*\* .* \(line ([0-9]+) of "([^"]+)"\):\s*(.*)$"#).expect("valid regex");
    let mut parsed = Vec::new();

    for line in output.lines() {
        let Some(captures) = error_line_regex.captures(line) else {
            continue;
        };

        let line_number = captures
            .get(1)
            .and_then(|raw| raw.as_str().parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let file_path = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or_default();
        let message = captures
            .get(3)
            .map(|value| value.as_str().trim().to_string())
            .unwrap_or_else(|| "Isabelle check failed".to_string());

        let diagnostic_uri = path_to_uri(file_path).unwrap_or_else(|| uri.to_string());
        parsed.push(Diagnostic {
            uri: diagnostic_uri,
            range: Range {
                start: Position {
                    line: line_number,
                    col: 1,
                },
                end: Position {
                    line: line_number,
                    col: 2,
                },
            },
            severity: Severity::Error,
            message,
        });
    }

    if !parsed.is_empty() {
        return parsed;
    }

    let fallback_message = output
        .lines()
        .find_map(|line| {
            line.strip_prefix("*** ")
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_else(|| format!("Isabelle check failed (exit code {exit_code})"));

    vec![Diagnostic {
        uri: uri.to_string(),
        range: Range {
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 2 },
        },
        severity: Severity::Error,
        message: fallback_message,
    }]
}

fn path_to_uri(path: &str) -> Option<String> {
    Url::from_file_path(path).ok().map(|uri| uri.to_string())
}

fn resolve_theory_name(uri: &str, text: &str) -> String {
    if let Some(extracted) = extract_theory_name(text) {
        let sanitized = sanitize_theory_name(&extracted);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    if let Some(from_uri) = theory_name_from_uri(uri) {
        let sanitized = sanitize_theory_name(&from_uri);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    "Scratch".to_string()
}

fn extract_theory_name(text: &str) -> Option<String> {
    let theory_regex = Regex::new(r"(?m)^\s*theory\s+([A-Za-z0-9_'.-]+)\b").expect("valid regex");
    theory_regex
        .captures(text)
        .and_then(|captures| captures.get(1).map(|value| value.as_str().to_string()))
}

fn theory_name_from_uri(uri: &str) -> Option<String> {
    let parsed = Url::parse(uri).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }

    let path = parsed.to_file_path().ok()?;
    let file_name = path.file_name()?.to_str()?;
    file_name
        .strip_suffix(".thy")
        .map(std::string::ToString::to_string)
}

fn parent_dir_from_file_uri(uri: &str) -> Option<PathBuf> {
    let parsed = Url::parse(uri).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }

    let path = parsed.to_file_path().ok()?;
    path.parent().map(|parent| parent.to_path_buf())
}

fn sanitize_theory_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\'' | '.' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn normalize_one_based(value: i64) -> usize {
    if value <= 1 {
        1
    } else {
        usize::try_from(value).unwrap_or(usize::MAX)
    }
}

fn build_hover_info(text: &str, line: usize, col: usize) -> String {
    if text.is_empty() {
        return format!("Document is empty at {line}:{col}");
    }

    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return format!("Document is empty at {line}:{col}");
    }

    if line > lines.len() {
        return format!(
            "Position {line}:{col} is outside document ({} lines)",
            lines.len()
        );
    }

    let line_text = lines[line - 1];
    if let Some((identifier, start_col, end_col)) = extract_identifier_at(line_text, col) {
        return format!(
            "Identifier: {identifier}\nRange: {line}:{start_col}-{line}:{end_col}\n{line_text}"
        );
    }

    format!("Position: {line}:{col}\n{line_text}")
}

fn extract_identifier_at(line_text: &str, col: usize) -> Option<(String, usize, usize)> {
    let chars = line_text.char_indices().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }

    let mut index = col.saturating_sub(1);
    if index >= chars.len() {
        index = chars.len() - 1;
    }

    if !is_identifier_char(chars[index].1) && index > 0 && is_identifier_char(chars[index - 1].1) {
        index -= 1;
    }
    if !is_identifier_char(chars[index].1) {
        return None;
    }

    let mut start = index;
    while start > 0 && is_identifier_char(chars[start - 1].1) {
        start -= 1;
    }

    let mut end = index;
    while end + 1 < chars.len() && is_identifier_char(chars[end + 1].1) {
        end += 1;
    }

    let start_byte = chars[start].0;
    let end_byte = if end + 1 < chars.len() {
        chars[end + 1].0
    } else {
        line_text.len()
    };

    Some((
        line_text[start_byte..end_byte].to_string(),
        start + 1,
        end + 1,
    ))
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\'' | '.' | '-')
}

async fn read_document_from_uri(uri: &str) -> Option<String> {
    let parsed = Url::parse(uri).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    let path = parsed.to_file_path().ok()?;
    tokio::fs::read_to_string(path).await.ok()
}

#[cfg(test)]
mod real_adapter_tests {
    use super::*;

    #[test]
    fn process_theories_parser_extracts_line_diagnostics() {
        let output = r#"Running Draft ...
Draft FAILED (see also "isabelle build_log -H Error Draft")
*** Outer syntax error (line 5 of "/tmp/Broken.thy"): proposition expected,
*** but end-of-input (line 5 of "/tmp/Broken.thy") was found
*** At command "<malformed>" (line 5 of "/tmp/Broken.thy")
Unfinished session(s): Draft
"#;

        let diagnostics = parse_process_theories_diagnostics("file:///tmp/Broken.thy", output, 1);

        assert!(!diagnostics.is_empty());
        assert_eq!(diagnostics[0].uri, "file:///tmp/Broken.thy");
        assert_eq!(diagnostics[0].range.start.line, 5);
        assert_eq!(diagnostics[0].severity, Severity::Error);
    }

    #[test]
    fn resolve_theory_name_prefers_theory_header() {
        let name = resolve_theory_name(
            "file:///tmp/Fallback.thy",
            "theory Demo imports Main begin\nend\n",
        );
        assert_eq!(name, "Demo");
    }

    #[test]
    fn parse_adapter_command_parses_and_validates() {
        let parsed =
            parse_adapter_command("bridge --mock-adapter").expect("adapter command should parse");
        assert_eq!(parsed.0, "bridge");
        assert_eq!(parsed.1, vec!["--mock-adapter".to_string()]);

        let quoted = parse_adapter_command("\"/tmp/my bridge\" --mock-adapter")
            .expect("quoted adapter command should parse");
        assert_eq!(quoted.0, "/tmp/my bridge");
        assert_eq!(quoted.1, vec!["--mock-adapter".to_string()]);

        assert!(parse_adapter_command("   ").is_err());
    }

    #[test]
    fn build_hover_info_extracts_identifier_and_range() {
        let text = "theory Demo imports Main begin\nlemma foo_bar: True by simp\nend\n";
        let info = build_hover_info(text, 2, 8);
        assert!(info.contains("Identifier: foo_bar"));
        assert!(info.contains("Range: 2:7-2:13"));
    }

    #[test]
    fn build_hover_info_reports_out_of_range_position() {
        let text = "theory Demo imports Main begin\nend\n";
        let info = build_hover_info(text, 10, 1);
        assert!(info.contains("outside document"));
    }

    #[test]
    fn session_dirs_for_uri_merges_cli_and_uri_parent_without_duplicates() {
        let mut state = RealAdapterState::new(
            "isabelle".to_string(),
            "HOL".to_string(),
            vec![PathBuf::from("/tmp/work"), PathBuf::from("/tmp/work")],
        );
        state.latest_documents.insert(
            "file:///tmp/work/Example.thy".to_string(),
            ("theory Example imports Main begin\nend\n".to_string(), 1),
        );

        let dirs = state.session_dirs_for_uri("file:///tmp/work/Example.thy");
        assert_eq!(dirs, vec![PathBuf::from("/tmp/work")]);
    }
}
