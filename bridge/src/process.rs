use crate::protocol::{
    CodeActionPayload, CompletionItemPayload, Diagnostic, DocumentUriPayload, LocationPayload,
    Message, MessageType, Position, QueryPayload, Range, RenamePayload, SemanticTokenPayload,
    Severity, SymbolPayload, TextEditPayload, WorkspaceSymbolQueryPayload,
    code_actions_message_from_request, completion_message_from_request,
    diagnostics_message_from_request, location_message_from_request, markup_message_from_request,
    parse_message, semantic_tokens_message_from_request, symbols_message_from_request,
    text_edits_message_from_request, to_ndjson, workspace_symbols_message_from_request,
};
use regex::Regex;
use serde::Deserialize;
use shell_words::split as split_shell_words;
use std::collections::{HashMap, HashSet};
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
            MessageType::Definition => location_message_from_request(
                &request,
                MessageType::Definition,
                Vec::<LocationPayload>::new(),
            )
            .map_err(|err| ProcessError::Protocol(err.to_string()))?,
            MessageType::References => location_message_from_request(
                &request,
                MessageType::References,
                Vec::<LocationPayload>::new(),
            )
            .map_err(|err| ProcessError::Protocol(err.to_string()))?,
            MessageType::Completion => {
                completion_message_from_request(&request, Vec::<CompletionItemPayload>::new())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::DocumentSymbols => {
                symbols_message_from_request(&request, Vec::<SymbolPayload>::new())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Rename => {
                text_edits_message_from_request(&request, MessageType::Rename, Vec::new())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::CodeAction => {
                code_actions_message_from_request(&request, Vec::<CodeActionPayload>::new())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::SemanticTokens => {
                semantic_tokens_message_from_request(&request, Vec::<SemanticTokenPayload>::new())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::WorkspaceSymbols => {
                workspace_symbols_message_from_request(&request, Vec::<SymbolPayload>::new())
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
    latest_documents: HashMap<String, CachedDocument>,
}

#[derive(Clone)]
struct CachedDocument {
    text: String,
    version: i64,
    diagnostics: Vec<Diagnostic>,
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
        if let Some(cached) = self.latest_documents.get_mut(uri)
            && cached.text == text
        {
            cached.version = version;
            return cached.diagnostics.clone();
        }

        let diagnostics = self.run_isabelle_check(uri, &text).await;
        self.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text,
                version,
                diagnostics: diagnostics.clone(),
            },
        );
        diagnostics
    }

    async fn check_document(&mut self, uri: &str, version: i64) -> Vec<Diagnostic> {
        if let Some(cached) = self.latest_documents.get(uri)
            && cached.version == version
        {
            return cached.diagnostics.clone();
        }

        let text = if let Some(cached) = self.latest_documents.get(uri) {
            Some(cached.text.clone())
        } else {
            read_document_from_uri(uri).await
        };

        let Some(text) = text else {
            return vec![no_theory_text_diagnostic(uri)];
        };

        if text.trim().is_empty() {
            return vec![no_theory_text_diagnostic(uri)];
        }

        let diagnostics = self.run_isabelle_check(uri, &text).await;
        self.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text,
                version,
                diagnostics: diagnostics.clone(),
            },
        );
        diagnostics
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
        let text = self.resolve_text(uri).await;

        let line = normalize_one_based(offset.line);
        let col = normalize_one_based(offset.col);

        match text {
            Some(text) => build_hover_info(&text, line, col),
            None => format!("No theory text available for hover at {line}:{col}"),
        }
    }

    async fn definition_locations(&self, uri: &str, offset: Position) -> Vec<LocationPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        let Some((identifier, _, _)) = identifier_at_position(
            &text,
            normalize_one_based(offset.line),
            normalize_one_based(offset.col),
        ) else {
            return Vec::new();
        };

        let focus_line = i64::try_from(normalize_one_based(offset.line)).unwrap_or(i64::MAX);
        let mut locations = self
            .indexed_documents(uri)
            .await
            .into_iter()
            .flat_map(|(doc_uri, doc_text)| {
                declaration_symbols(&doc_text)
                    .into_iter()
                    .filter(|symbol| symbol.name == identifier)
                    .map(move |symbol| LocationPayload {
                        uri: doc_uri.clone(),
                        range: symbol.range,
                    })
            })
            .collect::<Vec<_>>();

        locations.sort_by(|a, b| {
            location_priority_key(a, uri, focus_line)
                .cmp(&location_priority_key(b, uri, focus_line))
        });
        locations.dedup_by(|a, b| location_identity(a) == location_identity(b));
        locations
    }

    async fn reference_locations(&self, uri: &str, offset: Position) -> Vec<LocationPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        let Some((identifier, _, _)) = identifier_at_position(
            &text,
            normalize_one_based(offset.line),
            normalize_one_based(offset.col),
        ) else {
            return Vec::new();
        };

        let docs = self.indexed_documents(uri).await;
        let declaration_locations = docs
            .iter()
            .flat_map(|(doc_uri, doc_text)| {
                declaration_symbols(doc_text)
                    .into_iter()
                    .filter(|symbol| symbol.name == identifier)
                    .map(|symbol| LocationPayload {
                        uri: doc_uri.clone(),
                        range: symbol.range,
                    })
            })
            .map(|location| location_identity(&location))
            .collect::<HashSet<_>>();

        let focus_line = i64::try_from(normalize_one_based(offset.line)).unwrap_or(i64::MAX);
        let mut locations = docs
            .into_iter()
            .flat_map(|(doc_uri, doc_text)| {
                identifier_ranges(&doc_text, &identifier)
                    .into_iter()
                    .map(move |range| LocationPayload {
                        uri: doc_uri.clone(),
                        range,
                    })
            })
            .collect::<Vec<_>>();

        locations.sort_by(|a, b| {
            reference_priority_key(a, uri, focus_line, &declaration_locations).cmp(
                &reference_priority_key(b, uri, focus_line, &declaration_locations),
            )
        });
        locations.dedup_by(|a, b| location_identity(a) == location_identity(b));
        locations
    }

    async fn completion_items(&self, uri: &str, offset: Position) -> Vec<CompletionItemPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return keyword_completion_items("");
        };

        let prefix = identifier_prefix_at_position(
            &text,
            normalize_one_based(offset.line),
            normalize_one_based(offset.col),
        )
        .unwrap_or_default();
        completion_items_from_documents(&self.indexed_documents(uri).await, &prefix)
    }

    async fn document_symbols(&self, uri: &str) -> Vec<SymbolPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        declaration_symbols(&text)
            .into_iter()
            .map(|symbol| SymbolPayload {
                uri: uri.to_string(),
                name: symbol.name,
                kind: symbol.kind,
                range: symbol.range,
            })
            .collect()
    }

    async fn workspace_symbols(&self, query: &str) -> Vec<SymbolPayload> {
        let normalized_query = query.trim().to_ascii_lowercase();
        let mut symbols = self
            .latest_documents
            .iter()
            .flat_map(|(uri, cached)| {
                declaration_symbols(&cached.text)
                    .into_iter()
                    .map(|symbol| SymbolPayload {
                        uri: uri.clone(),
                        name: symbol.name,
                        kind: symbol.kind,
                        range: symbol.range,
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|symbol| {
                normalized_query.is_empty()
                    || symbol.name.to_ascii_lowercase().contains(&normalized_query)
            })
            .collect::<Vec<_>>();

        symbols.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| a.uri.cmp(&b.uri))
                .then_with(|| a.range.start.line.cmp(&b.range.start.line))
                .then_with(|| a.range.start.col.cmp(&b.range.start.col))
        });
        symbols
    }

    async fn semantic_tokens(&self, uri: &str) -> Vec<SemanticTokenPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        let declaration_kinds = declaration_symbols(&text)
            .into_iter()
            .map(|symbol| (symbol.name, symbol.kind))
            .collect::<HashMap<_, _>>();

        let mut tokens = Vec::new();
        for (line_index, line_tokens) in identifier_tokens_by_line(&text).into_iter().enumerate() {
            for (token, start_col, end_col) in line_tokens {
                let token_type = if COMPLETION_KEYWORDS.contains(&token.as_str()) {
                    "keyword"
                } else if let Some(kind) = declaration_kinds.get(&token) {
                    semantic_token_type_from_declaration_kind(kind)
                } else {
                    "variable"
                };

                let length = end_col.saturating_sub(start_col).saturating_add(1);
                tokens.push(SemanticTokenPayload {
                    uri: uri.to_string(),
                    line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                    col: i64::try_from(start_col).unwrap_or(i64::MAX),
                    length: i64::try_from(length).unwrap_or(i64::MAX),
                    token_type: token_type.to_string(),
                });
            }
        }

        tokens
    }

    async fn rename_edits(
        &self,
        uri: &str,
        offset: Position,
        new_name: String,
    ) -> Vec<TextEditPayload> {
        if new_name.trim().is_empty() {
            return Vec::new();
        }

        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        let Some((identifier, _, _)) = identifier_at_position(
            &text,
            normalize_one_based(offset.line),
            normalize_one_based(offset.col),
        ) else {
            return Vec::new();
        };

        let focus_line = i64::try_from(normalize_one_based(offset.line)).unwrap_or(i64::MAX);
        let mut edits = self
            .indexed_documents(uri)
            .await
            .into_iter()
            .flat_map(|(doc_uri, doc_text)| {
                let replacement = new_name.clone();
                identifier_ranges(&doc_text, &identifier)
                    .into_iter()
                    .map(move |range| TextEditPayload {
                        uri: doc_uri.clone(),
                        range,
                        new_text: replacement.clone(),
                    })
            })
            .collect::<Vec<_>>();

        edits.sort_by(|a, b| {
            text_edit_priority_key(a, uri, focus_line)
                .cmp(&text_edit_priority_key(b, uri, focus_line))
        });
        edits.dedup_by(|a, b| {
            text_edit_identity(a) == text_edit_identity(b) && a.new_text == b.new_text
        });
        edits
    }

    async fn code_actions(&self, uri: &str) -> Vec<CodeActionPayload> {
        let Some(text) = self.resolve_text(uri).await else {
            return Vec::new();
        };

        let mut actions = Vec::new();
        for (line_index, line_tokens) in identifier_tokens_by_line(&text).into_iter().enumerate() {
            for (token, start_col, end_col) in line_tokens {
                if token != "sorry" {
                    continue;
                }

                actions.push(CodeActionPayload {
                    title: format!("Replace sorry with by simp (line {})", line_index + 1),
                    kind: "quickfix".to_string(),
                    edits: vec![TextEditPayload {
                        uri: uri.to_string(),
                        range: Range {
                            start: Position {
                                line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                                col: i64::try_from(start_col).unwrap_or(i64::MAX),
                            },
                            end: Position {
                                line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                                col: i64::try_from(end_col).unwrap_or(i64::MAX),
                            },
                        },
                        new_text: "by simp".to_string(),
                    }],
                });
            }
        }

        let diagnostics = self
            .latest_documents
            .get(uri)
            .map(|cached| cached.diagnostics.clone())
            .unwrap_or_default();
        if diagnostics_indicate_unfinished_theory(&diagnostics) && !contains_end_keyword(&text) {
            let end_pos = document_end_position(&text);
            actions.push(CodeActionPayload {
                title: "Append missing end".to_string(),
                kind: "quickfix".to_string(),
                edits: vec![TextEditPayload {
                    uri: uri.to_string(),
                    range: Range {
                        start: end_pos.clone(),
                        end: end_pos,
                    },
                    new_text: "\nend\n".to_string(),
                }],
            });
        }
        if !has_theory_header(&text) {
            actions.push(CodeActionPayload {
                title: "Insert minimal theory header".to_string(),
                kind: "quickfix".to_string(),
                edits: vec![TextEditPayload {
                    uri: uri.to_string(),
                    range: Range {
                        start: Position { line: 1, col: 1 },
                        end: Position { line: 1, col: 1 },
                    },
                    new_text: "theory Scratch imports Main begin\n\n".to_string(),
                }],
            });
        }

        actions
    }

    async fn resolve_text(&self, uri: &str) -> Option<String> {
        if let Some(cached) = self.latest_documents.get(uri) {
            return Some(cached.text.clone());
        }

        read_document_from_uri(uri).await
    }

    async fn indexed_documents(&self, focus_uri: &str) -> Vec<(String, String)> {
        let mut docs = self
            .latest_documents
            .iter()
            .map(|(uri, cached)| (uri.clone(), cached.text.clone()))
            .collect::<Vec<_>>();
        if !docs.iter().any(|(uri, _)| uri == focus_uri)
            && let Some(text) = read_document_from_uri(focus_uri).await
        {
            docs.push((focus_uri.to_string(), text));
        }

        if let Some(related_uris) = related_document_uris(&docs, focus_uri) {
            docs.retain(|(uri, _)| related_uris.contains(uri));
        }
        docs
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

fn no_theory_text_diagnostic(uri: &str) -> Diagnostic {
    Diagnostic {
        uri: uri.to_string(),
        range: Range {
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 2 },
        },
        severity: Severity::Warning,
        message: "No theory text available for check".to_string(),
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
                let diagnostics = state.check_document(&payload.uri, payload.version).await;
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
            MessageType::Definition => {
                let payload: QueryPayload = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let locations = state
                    .definition_locations(&payload.uri, payload.offset)
                    .await;
                location_message_from_request(&request, MessageType::Definition, locations)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::References => {
                let payload: QueryPayload = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let locations = state
                    .reference_locations(&payload.uri, payload.offset)
                    .await;
                location_message_from_request(&request, MessageType::References, locations)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Completion => {
                let payload: QueryPayload = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let items = state.completion_items(&payload.uri, payload.offset).await;
                completion_message_from_request(&request, items)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::DocumentSymbols => {
                let payload: DocumentUriPayload =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let symbols = state.document_symbols(&payload.uri).await;
                symbols_message_from_request(&request, symbols)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::Rename => {
                let payload: RenamePayload = serde_json::from_value(request.payload.clone())
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let edits = state
                    .rename_edits(&payload.uri, payload.offset, payload.new_name)
                    .await;
                text_edits_message_from_request(&request, MessageType::Rename, edits)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::CodeAction => {
                let payload: DocumentUriPayload =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let actions = state.code_actions(&payload.uri).await;
                code_actions_message_from_request(&request, actions)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::SemanticTokens => {
                let payload: DocumentUriPayload =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let tokens = state.semantic_tokens(&payload.uri).await;
                semantic_tokens_message_from_request(&request, tokens)
                    .map_err(|err| ProcessError::Protocol(err.to_string()))?
            }
            MessageType::WorkspaceSymbols => {
                let payload: WorkspaceSymbolQueryPayload =
                    serde_json::from_value(request.payload.clone())
                        .map_err(|err| ProcessError::Protocol(err.to_string()))?;
                let symbols = state.workspace_symbols(&payload.query).await;
                workspace_symbols_message_from_request(&request, symbols)
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

#[derive(Debug, Clone)]
struct TheoryHeader {
    name: String,
    imports: Vec<String>,
}

fn theory_header(text: &str) -> Option<TheoryHeader> {
    let mut theory_name: Option<String> = None;
    let mut imports = Vec::new();
    let mut collecting_imports = false;

    for line_tokens in identifier_tokens_by_line(text) {
        let tokens = line_tokens
            .into_iter()
            .map(|(token, _, _)| token)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            continue;
        }

        if theory_name.is_none() {
            if tokens.first().map(String::as_str) != Some("theory") {
                continue;
            }

            if let Some(candidate) = tokens.get(1) {
                let sanitized = sanitize_theory_name(candidate);
                if !sanitized.is_empty() {
                    theory_name = Some(sanitized);
                }
            }

            if let Some(imports_index) = tokens.iter().position(|token| token == "imports") {
                push_import_tokens(&tokens[imports_index + 1..], &mut imports);
                collecting_imports = !tokens.iter().any(|token| token == "begin");
            } else if tokens.iter().any(|token| token == "begin") {
                break;
            }
            continue;
        }

        if collecting_imports {
            if let Some(begin_index) = tokens.iter().position(|token| token == "begin") {
                push_import_tokens(&tokens[..begin_index], &mut imports);
                break;
            }
            push_import_tokens(&tokens, &mut imports);
            continue;
        }

        if tokens.first().map(String::as_str) == Some("imports") {
            if let Some(begin_index) = tokens.iter().position(|token| token == "begin") {
                push_import_tokens(&tokens[1..begin_index], &mut imports);
                break;
            }
            push_import_tokens(&tokens[1..], &mut imports);
            collecting_imports = true;
            continue;
        }

        if tokens.first().map(String::as_str) == Some("begin") {
            break;
        }
    }

    let theory_name = theory_name.or_else(|| {
        extract_theory_name(text).and_then(|candidate| {
            let sanitized = sanitize_theory_name(&candidate);
            if sanitized.is_empty() {
                None
            } else {
                Some(sanitized)
            }
        })
    })?;

    imports.sort();
    imports.dedup();
    Some(TheoryHeader {
        name: theory_name,
        imports,
    })
}

fn push_import_tokens(tokens: &[String], imports: &mut Vec<String>) {
    for token in tokens {
        if token == "begin" {
            break;
        }
        if token == "imports" {
            continue;
        }

        let sanitized = sanitize_theory_name(token);
        if sanitized.is_empty() {
            continue;
        }
        imports.push(sanitized);
    }
}

fn related_document_uris(docs: &[(String, String)], focus_uri: &str) -> Option<HashSet<String>> {
    let mut theory_to_uri = HashMap::new();
    let mut imports_by_theory = HashMap::new();
    let mut theory_for_uri = HashMap::new();

    for (uri, text) in docs {
        let Some(header) = theory_header(text) else {
            continue;
        };
        theory_for_uri.insert(uri.clone(), header.name.clone());
        theory_to_uri
            .entry(header.name.clone())
            .or_insert_with(|| uri.clone());
        imports_by_theory.insert(header.name, header.imports);
    }

    let focus_theory = theory_for_uri.get(focus_uri).cloned().or_else(|| {
        theory_name_from_uri(focus_uri).and_then(|name| {
            let sanitized = sanitize_theory_name(&name);
            if sanitized.is_empty() {
                None
            } else {
                Some(sanitized)
            }
        })
    })?;
    if !theory_to_uri.contains_key(&focus_theory) {
        return None;
    }

    let mut adjacency = HashMap::<String, HashSet<String>>::new();
    for (theory, imports) in &imports_by_theory {
        for imported in imports {
            if !theory_to_uri.contains_key(imported) {
                continue;
            }
            adjacency
                .entry(theory.clone())
                .or_default()
                .insert(imported.clone());
            adjacency
                .entry(imported.clone())
                .or_default()
                .insert(theory.clone());
        }
    }

    let mut reachable_theories = HashSet::new();
    let mut stack = vec![focus_theory];
    while let Some(current) = stack.pop() {
        if !reachable_theories.insert(current.clone()) {
            continue;
        }
        if let Some(neighbors) = adjacency.get(&current) {
            for neighbor in neighbors {
                if !reachable_theories.contains(neighbor) {
                    stack.push(neighbor.clone());
                }
            }
        }
    }

    let mut uris = HashSet::from([focus_uri.to_string()]);
    for theory in reachable_theories {
        if let Some(uri) = theory_to_uri.get(&theory) {
            uris.insert(uri.clone());
        }
    }
    Some(uris)
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
    if let Some((identifier, start_col, end_col)) = identifier_at_position(text, line, col) {
        return format!(
            "Identifier: {identifier}\nRange: {line}:{start_col}-{line}:{end_col}\n{line_text}"
        );
    }

    format!("Position: {line}:{col}\n{line_text}")
}

const COMPLETION_KEYWORDS: &[&str] = &[
    "theory",
    "imports",
    "begin",
    "end",
    "lemma",
    "theorem",
    "corollary",
    "proposition",
    "definition",
    "abbreviation",
    "fun",
    "function",
    "primrec",
    "datatype",
    "record",
    "type_synonym",
    "locale",
    "class",
    "instantiation",
    "context",
    "assumes",
    "shows",
    "fixes",
    "defines",
    "where",
    "proof",
    "qed",
    "by",
    "sorry",
    "oops",
    "have",
    "show",
    "thus",
    "hence",
    "from",
    "then",
    "using",
    "unfolding",
    "apply",
];

#[derive(Clone)]
struct DeclarationSymbol {
    name: String,
    kind: String,
    range: Range,
}

fn declaration_kind(keyword: &str) -> Option<&'static str> {
    match keyword {
        "lemma" | "theorem" | "corollary" | "proposition" => Some("theorem"),
        "definition" | "abbreviation" | "fun" | "function" | "primrec" => Some("function"),
        "datatype" | "record" | "type_synonym" => Some("type"),
        "locale" | "class" | "instantiation" => Some("module"),
        _ => None,
    }
}

fn semantic_token_type_from_declaration_kind(kind: &str) -> &'static str {
    match kind {
        "type" => "type",
        "module" => "namespace",
        "function" | "theorem" => "function",
        _ => "variable",
    }
}

fn declaration_symbols(text: &str) -> Vec<DeclarationSymbol> {
    let mut symbols = Vec::new();
    for (line_index, tokens) in identifier_tokens_by_line(text).into_iter().enumerate() {
        if tokens.len() < 2 {
            continue;
        }

        let keyword = tokens[0].0.as_str();
        let Some(kind) = declaration_kind(keyword) else {
            continue;
        };

        let name = tokens[1].0.clone();
        symbols.push(DeclarationSymbol {
            name,
            kind: kind.to_string(),
            range: Range {
                start: Position {
                    line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                    col: i64::try_from(tokens[1].1).unwrap_or(i64::MAX),
                },
                end: Position {
                    line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                    col: i64::try_from(tokens[1].2).unwrap_or(i64::MAX),
                },
            },
        });
    }

    symbols
}

fn identifier_at_position(text: &str, line: usize, col: usize) -> Option<(String, usize, usize)> {
    let tokens_by_line = identifier_tokens_by_line(text);
    if tokens_by_line.is_empty() || line == 0 || line > tokens_by_line.len() {
        return None;
    }
    for (token, start_col, end_col) in &tokens_by_line[line - 1] {
        if col >= *start_col && col <= end_col.saturating_add(1) {
            return Some((token.clone(), *start_col, *end_col));
        }
    }
    None
}

fn identifier_prefix_at_position(text: &str, line: usize, col: usize) -> Option<String> {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() || line == 0 || line > lines.len() {
        return None;
    }

    let line_text = lines[line - 1];
    if line_text.is_empty() {
        return None;
    }

    let chars = line_text.char_indices().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }

    let mut end = col.saturating_sub(1);
    if end > chars.len() {
        end = chars.len();
    }

    let mut start = end;
    while start > 0 && is_identifier_char(chars[start - 1].1) {
        start -= 1;
    }

    if start == end {
        return None;
    }

    let start_byte = chars[start].0;
    let end_byte = if end < chars.len() {
        chars[end].0
    } else {
        line_text.len()
    };

    Some(line_text[start_byte..end_byte].to_string())
}

fn identifier_ranges(text: &str, identifier: &str) -> Vec<Range> {
    let mut ranges = Vec::new();
    for (line_index, tokens) in identifier_tokens_by_line(text).into_iter().enumerate() {
        for (token, start_col, end_col) in tokens {
            if token != identifier {
                continue;
            }

            ranges.push(Range {
                start: Position {
                    line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                    col: i64::try_from(start_col).unwrap_or(i64::MAX),
                },
                end: Position {
                    line: i64::try_from(line_index + 1).unwrap_or(i64::MAX),
                    col: i64::try_from(end_col).unwrap_or(i64::MAX),
                },
            });
        }
    }

    ranges
}

fn keyword_completion_items(prefix: &str) -> Vec<CompletionItemPayload> {
    COMPLETION_KEYWORDS
        .iter()
        .filter(|keyword| keyword.starts_with(prefix))
        .map(|keyword| CompletionItemPayload {
            label: (*keyword).to_string(),
            detail: Some("keyword".to_string()),
        })
        .collect()
}

#[cfg(test)]
fn completion_items_from_text(text: &str, prefix: &str) -> Vec<CompletionItemPayload> {
    completion_items_from_documents(&[(String::new(), text.to_string())], prefix)
}

fn completion_items_from_documents(
    docs: &[(String, String)],
    prefix: &str,
) -> Vec<CompletionItemPayload> {
    let mut labels = std::collections::BTreeSet::new();

    for keyword in COMPLETION_KEYWORDS {
        if keyword.starts_with(prefix) {
            labels.insert((*keyword).to_string());
        }
    }

    for (_, text) in docs {
        for tokens in identifier_tokens_by_line(text) {
            for (token, _, _) in tokens {
                if prefix.is_empty() || token.starts_with(prefix) {
                    labels.insert(token);
                }
            }
        }
    }

    labels
        .into_iter()
        .take(200)
        .map(|label| CompletionItemPayload {
            label,
            detail: None,
        })
        .collect()
}

fn has_theory_header(text: &str) -> bool {
    identifier_tokens_by_line(text).into_iter().any(|tokens| {
        tokens
            .first()
            .map(|(token, _, _)| token == "theory")
            .unwrap_or(false)
    })
}

fn contains_end_keyword(text: &str) -> bool {
    identifier_tokens_by_line(text)
        .into_iter()
        .any(|tokens| tokens.iter().any(|(token, _, _)| token == "end"))
}

fn diagnostics_indicate_unfinished_theory(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(|diagnostic| {
        let message = diagnostic.message.to_ascii_lowercase();
        message.contains("end-of-input")
            || message.contains("unfinished")
            || message.contains("unexpected end")
    })
}

fn document_end_position(text: &str) -> Position {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return Position { line: 1, col: 1 };
    }

    let last_line = lines.len();
    let last_col = lines[last_line - 1].chars().count().saturating_add(1);
    Position {
        line: i64::try_from(last_line).unwrap_or(i64::MAX),
        col: i64::try_from(last_col).unwrap_or(i64::MAX),
    }
}

fn abs_i64_diff(lhs: i64, rhs: i64) -> i64 {
    if lhs >= rhs {
        lhs.saturating_sub(rhs)
    } else {
        rhs.saturating_sub(lhs)
    }
}

fn location_identity(location: &LocationPayload) -> (String, i64, i64, i64, i64) {
    (
        location.uri.clone(),
        location.range.start.line,
        location.range.start.col,
        location.range.end.line,
        location.range.end.col,
    )
}

fn location_priority_key(
    location: &LocationPayload,
    focus_uri: &str,
    focus_line: i64,
) -> (u8, u8, i64, String, i64, i64) {
    let is_same_uri = location.uri == focus_uri;
    let same_uri_rank = if is_same_uri { 0 } else { 1 };
    let local_before_cursor_rank = if is_same_uri && location.range.start.line <= focus_line {
        0
    } else {
        1
    };
    let local_line_distance = if is_same_uri {
        abs_i64_diff(location.range.start.line, focus_line)
    } else {
        i64::MAX
    };

    (
        same_uri_rank,
        local_before_cursor_rank,
        local_line_distance,
        location.uri.clone(),
        location.range.start.line,
        location.range.start.col,
    )
}

fn reference_priority_key(
    location: &LocationPayload,
    focus_uri: &str,
    focus_line: i64,
    declaration_locations: &HashSet<(String, i64, i64, i64, i64)>,
) -> (u8, u8, i64, String, i64, i64) {
    let is_declaration = declaration_locations.contains(&location_identity(location));
    let declaration_rank = if is_declaration { 0 } else { 1 };
    let same_uri_rank = if location.uri == focus_uri { 0 } else { 1 };
    let local_line_distance = if same_uri_rank == 0 {
        abs_i64_diff(location.range.start.line, focus_line)
    } else {
        i64::MAX
    };

    (
        declaration_rank,
        same_uri_rank,
        local_line_distance,
        location.uri.clone(),
        location.range.start.line,
        location.range.start.col,
    )
}

fn text_edit_identity(edit: &TextEditPayload) -> (String, i64, i64, i64, i64) {
    (
        edit.uri.clone(),
        edit.range.start.line,
        edit.range.start.col,
        edit.range.end.line,
        edit.range.end.col,
    )
}

fn text_edit_priority_key(
    edit: &TextEditPayload,
    focus_uri: &str,
    focus_line: i64,
) -> (u8, i64, String, i64, i64) {
    let same_uri_rank = if edit.uri == focus_uri { 0 } else { 1 };
    let local_line_distance = if same_uri_rank == 0 {
        abs_i64_diff(edit.range.start.line, focus_line)
    } else {
        i64::MAX
    };
    (
        same_uri_rank,
        local_line_distance,
        edit.uri.clone(),
        edit.range.start.line,
        edit.range.start.col,
    )
}

fn identifier_tokens_by_line(text: &str) -> Vec<Vec<(String, usize, usize)>> {
    let mut by_line = Vec::new();
    let mut comment_depth = 0usize;
    let mut in_string = false;

    for line in text.lines() {
        let chars = line.char_indices().collect::<Vec<_>>();
        let mut tokens = Vec::new();
        let mut index = 0usize;

        while index < chars.len() {
            let ch = chars[index].1;
            let next = chars.get(index + 1).map(|(_, value)| *value);

            if comment_depth > 0 {
                if ch == '(' && next == Some('*') {
                    comment_depth = comment_depth.saturating_add(1);
                    index = index.saturating_add(2);
                    continue;
                }
                if ch == '*' && next == Some(')') {
                    comment_depth = comment_depth.saturating_sub(1);
                    index = index.saturating_add(2);
                    continue;
                }
                index = index.saturating_add(1);
                continue;
            }

            if in_string {
                if ch == '\\' {
                    index = index.saturating_add(2);
                    continue;
                }
                if ch == '"' {
                    in_string = false;
                }
                index = index.saturating_add(1);
                continue;
            }

            if ch == '(' && next == Some('*') {
                comment_depth = 1;
                index = index.saturating_add(2);
                continue;
            }
            if ch == '"' {
                in_string = true;
                index = index.saturating_add(1);
                continue;
            }
            if !is_identifier_char(ch) {
                index = index.saturating_add(1);
                continue;
            }

            let start = index;
            while index + 1 < chars.len() && is_identifier_char(chars[index + 1].1) {
                index += 1;
            }
            let end = index;

            let start_byte = chars[start].0;
            let end_byte = if end + 1 < chars.len() {
                chars[end + 1].0
            } else {
                line.len()
            };

            tokens.push((line[start_byte..end_byte].to_string(), start + 1, end + 1));
            index = index.saturating_add(1);
        }

        by_line.push(tokens);
    }

    by_line
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
    fn theory_header_collects_multiline_imports() {
        let text = "theory Demo\nimports Main A B\nbegin\nend\n";
        let header = theory_header(text).expect("theory header should parse");
        assert_eq!(header.name, "Demo");
        assert_eq!(
            header.imports,
            vec!["A".to_string(), "B".to_string(), "Main".to_string()]
        );
    }

    #[test]
    fn related_document_uris_returns_connected_theories_only() {
        let docs = vec![
            (
                "file:///tmp/A.thy".to_string(),
                "theory A imports Main begin\nlemma a: True by simp\nend\n".to_string(),
            ),
            (
                "file:///tmp/B.thy".to_string(),
                "theory B imports A begin\nlemma b: a by simp\nend\n".to_string(),
            ),
            (
                "file:///tmp/C.thy".to_string(),
                "theory C imports Main begin\nlemma a: True by simp\nend\n".to_string(),
            ),
        ];

        let related = related_document_uris(&docs, "file:///tmp/B.thy")
            .expect("connected theory set should resolve");
        assert!(related.contains("file:///tmp/A.thy"));
        assert!(related.contains("file:///tmp/B.thy"));
        assert!(!related.contains("file:///tmp/C.thy"));
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
    fn declaration_symbols_extracts_isabelle_entities() {
        let text = "theory Demo imports Main begin\nlemma foo: True by simp\ndefinition bar where \"bar = True\"\nend\n";
        let symbols = declaration_symbols(text);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "foo");
        assert_eq!(symbols[1].name, "bar");
    }

    #[test]
    fn declaration_symbols_ignores_comments_and_strings() {
        let text = "theory Demo imports Main begin\n(* lemma hidden: True by simp *)\ntext \"lemma quoted: True\"\nlemma visible: True by simp\nend\n";
        let symbols = declaration_symbols(text);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "visible");
    }

    #[test]
    fn identifier_ranges_finds_all_occurrences() {
        let text = "lemma foo: True\nhave foo by simp\nshow foo by simp\n";
        let ranges = identifier_ranges(text, "foo");
        assert_eq!(ranges.len(), 3);
    }

    #[test]
    fn identifier_ranges_ignores_comments_and_strings() {
        let text = "lemma foo: True\n(* foo *)\ntext \"foo\"\nhave foo by simp\n";
        let ranges = identifier_ranges(text, "foo");
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn completion_items_from_text_filters_by_prefix() {
        let text = "lemma foo_bar: True by simp\ndefinition fooz where \"fooz = True\"\n";
        let items = completion_items_from_text(text, "foo");
        let labels = items.into_iter().map(|item| item.label).collect::<Vec<_>>();
        assert!(labels.contains(&"foo_bar".to_string()));
        assert!(labels.contains(&"fooz".to_string()));
    }

    #[tokio::test]
    async fn rename_edits_rewrite_all_identifier_occurrences() {
        let uri = "file:///tmp/Example.thy";
        let text = "lemma foo: True\nhave foo by simp\nshow foo by simp\n";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let edits = state
            .rename_edits(uri, Position { line: 1, col: 7 }, "bar".to_string())
            .await;
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|edit| edit.new_text == "bar"));
    }

    #[tokio::test]
    async fn code_actions_provides_quickfix_for_sorry() {
        let uri = "file:///tmp/Example.thy";
        let text = "theory Example imports Main begin\nlemma foo: True\n  sorry\nend\n";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let actions = state.code_actions(uri).await;
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, "quickfix");
        assert_eq!(actions[0].edits[0].new_text, "by simp");
    }

    #[tokio::test]
    async fn code_actions_ignores_sorry_in_comments_and_strings() {
        let uri = "file:///tmp/Example.thy";
        let text = "theory Example imports Main begin\n(* sorry *)\ntext \"sorry\"\nlemma foo: True by simp\nend\n";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let actions = state.code_actions(uri).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn semantic_tokens_marks_keywords_and_declarations() {
        let uri = "file:///tmp/Example.thy";
        let text = "theory Demo imports Main begin\nlemma foo: True by simp\nend\n";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let tokens = state.semantic_tokens(uri).await;
        assert!(
            tokens
                .iter()
                .any(|token| token.token_type == "keyword" && token.line == 1)
        );
        assert!(
            tokens
                .iter()
                .any(|token| token.token_type == "function" && token.line == 2)
        );
    }

    #[tokio::test]
    async fn workspace_symbols_collects_and_filters_across_documents() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma alpha: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports Main begin\ndefinition beta where \"beta = True\"\nend\n"
                    .to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let all = state.workspace_symbols("").await;
        assert!(all.iter().any(|symbol| symbol.name == "alpha"));
        assert!(all.iter().any(|symbol| symbol.name == "beta"));

        let filtered = state.workspace_symbols("alp").await;
        assert!(filtered.iter().all(|symbol| symbol.name == "alpha"));
    }

    #[tokio::test]
    async fn definition_locations_searches_across_cached_documents() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports A begin\nlemma uses_foo: foo by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let locations = state
            .definition_locations(uri_b, Position { line: 2, col: 17 })
            .await;
        assert!(!locations.is_empty());
        assert!(locations.iter().any(|location| location.uri == uri_a));
    }

    #[tokio::test]
    async fn definition_locations_prioritizes_local_declaration() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports A begin\nlemma foo: True by simp\nlemma use: foo by simp\nend\n"
                    .to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let locations = state
            .definition_locations(uri_b, Position { line: 3, col: 13 })
            .await;
        assert!(!locations.is_empty());
        assert_eq!(locations[0].uri, uri_b);
        assert_eq!(locations[0].range.start.line, 2);
    }

    #[tokio::test]
    async fn rename_edits_include_related_cached_documents() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports A begin\nlemma uses_foo: foo by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let edits = state
            .rename_edits(uri_b, Position { line: 2, col: 17 }, "bar".to_string())
            .await;
        assert!(edits.iter().any(|edit| edit.uri == uri_a));
        assert!(edits.iter().any(|edit| edit.uri == uri_b));
    }

    #[tokio::test]
    async fn rename_edits_prioritizes_local_edits() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports A begin\nlemma use_foo: foo by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let edits = state
            .rename_edits(uri_b, Position { line: 2, col: 16 }, "bar".to_string())
            .await;
        assert!(!edits.is_empty());
        assert_eq!(edits[0].uri, uri_b);
    }

    #[tokio::test]
    async fn reference_locations_excludes_unrelated_documents() {
        let uri_a = "file:///tmp/A.thy";
        let uri_b = "file:///tmp/B.thy";
        let uri_c = "file:///tmp/C.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri_a.to_string(),
            CachedDocument {
                text: "theory A imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_b.to_string(),
            CachedDocument {
                text: "theory B imports A begin\nlemma uses_foo: foo by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );
        state.latest_documents.insert(
            uri_c.to_string(),
            CachedDocument {
                text: "theory C imports Main begin\nlemma foo: True by simp\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let locations = state
            .reference_locations(uri_b, Position { line: 2, col: 17 })
            .await;
        assert!(locations.iter().any(|location| location.uri == uri_a));
        assert!(locations.iter().any(|location| location.uri == uri_b));
        assert!(!locations.iter().any(|location| location.uri == uri_c));
    }

    #[tokio::test]
    async fn reference_locations_prioritizes_declaration_before_usage() {
        let uri = "file:///tmp/Example.thy";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: "theory Example imports Main begin\nlemma foo: True by simp\nlemma use: foo by simp\nend\n"
                    .to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let locations = state
            .reference_locations(uri, Position { line: 3, col: 13 })
            .await;
        assert!(!locations.is_empty());
        assert_eq!(locations[0].uri, uri);
        assert_eq!(locations[0].range.start.line, 2);
    }

    #[tokio::test]
    async fn code_actions_add_append_end_quickfix_for_unfinished_theory() {
        let uri = "file:///tmp/Example.thy";
        let text = "theory Example imports Main begin\nlemma foo: True by simp\n";
        let mut state =
            RealAdapterState::new("isabelle".to_string(), "HOL".to_string(), Vec::new());
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: vec![Diagnostic {
                    uri: uri.to_string(),
                    range: Range {
                        start: Position { line: 2, col: 1 },
                        end: Position { line: 2, col: 2 },
                    },
                    severity: Severity::Error,
                    message: "but end-of-input was found".to_string(),
                }],
            },
        );

        let actions = state.code_actions(uri).await;
        assert!(
            actions
                .iter()
                .any(|action| action.title == "Append missing end")
        );
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
            CachedDocument {
                text: "theory Example imports Main begin\nend\n".to_string(),
                version: 1,
                diagnostics: Vec::new(),
            },
        );

        let dirs = state.session_dirs_for_uri("file:///tmp/work/Example.thy");
        assert_eq!(dirs, vec![PathBuf::from("/tmp/work")]);
    }

    #[tokio::test]
    async fn update_document_reuses_cached_diagnostics_when_text_is_unchanged() {
        let uri = "file:///tmp/Example.thy";
        let text = "theory Example imports Main begin\nlemma x: True by simp\nend\n";
        let cached = vec![Diagnostic {
            uri: uri.to_string(),
            range: Range {
                start: Position { line: 2, col: 1 },
                end: Position { line: 2, col: 2 },
            },
            severity: Severity::Info,
            message: "cached".to_string(),
        }];

        let mut state = RealAdapterState::new(
            "definitely-not-a-real-isabelle".to_string(),
            "HOL".to_string(),
            Vec::new(),
        );
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: text.to_string(),
                version: 1,
                diagnostics: cached.clone(),
            },
        );

        let diagnostics = state.update_document(uri, text.to_string(), 2).await;
        assert_eq!(diagnostics, cached);
        assert_eq!(
            state.latest_documents.get(uri).map(|doc| doc.version),
            Some(2)
        );
    }

    #[tokio::test]
    async fn check_document_reuses_cached_diagnostics_for_matching_version() {
        let uri = "file:///tmp/Example.thy";
        let cached = vec![Diagnostic {
            uri: uri.to_string(),
            range: Range {
                start: Position { line: 1, col: 1 },
                end: Position { line: 1, col: 2 },
            },
            severity: Severity::Warning,
            message: "cached check".to_string(),
        }];

        let mut state = RealAdapterState::new(
            "definitely-not-a-real-isabelle".to_string(),
            "HOL".to_string(),
            Vec::new(),
        );
        state.latest_documents.insert(
            uri.to_string(),
            CachedDocument {
                text: "theory Example imports Main begin\nend\n".to_string(),
                version: 7,
                diagnostics: cached.clone(),
            },
        );

        let diagnostics = state.check_document(uri, 7).await;
        assert_eq!(diagnostics, cached);
    }
}
