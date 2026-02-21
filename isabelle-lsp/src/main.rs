use bridge::protocol::{
    Diagnostic as BridgeDiagnostic, DocumentCheckPayload, DocumentPushPayload, MarkupPayload,
    Message, MessageType, Position as BridgePosition, Severity, parse_message, to_ndjson,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
use tokio::time::Duration;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, ExecuteCommandOptions, Hover, HoverContents,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, MarkedString,
    MessageType as LspMessageType, Position, PublishDiagnosticsParams, Range, ServerCapabilities,
    ServerInfo, TextDocumentContentChangeEvent, TextDocumentItem, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{debug, error, info, warn};

const DEFAULT_BRIDGE_SOCKET: &str = "/tmp/isabelle.sock";
const DEFAULT_SESSION: &str = "s1";
const DEFAULT_BRIDGE_AUTOSTART_TIMEOUT_MS: u64 = 5_000;

const ENV_BRIDGE_AUTOSTART_CMD: &str = "ISABELLE_BRIDGE_AUTOSTART_CMD";
const ENV_BRIDGE_AUTOSTART_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS";

const COMMAND_START_SESSION: &str = "isabelle.start_session";
const COMMAND_STOP_SESSION: &str = "isabelle.stop_session";
const COMMAND_RUN_CHECK: &str = "isabelle.run_check";

#[derive(Debug, Error)]
enum BridgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bridge returned EOF")]
    Eof,
    #[error("invalid response from bridge: {0}")]
    InvalidResponse(String),
}

struct BridgeConnection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl BridgeConnection {
    async fn connect(path: &PathBuf) -> Result<Self, BridgeError> {
        let stream = UnixStream::connect(path).await?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        })
    }
}

#[derive(Clone)]
struct BridgeTransport {
    socket_path: PathBuf,
    session: String,
    next_id: Arc<AtomicU64>,
    connection: Arc<Mutex<Option<BridgeConnection>>>,
}

impl BridgeTransport {
    fn new(socket_path: PathBuf, session: String) -> Self {
        Self {
            socket_path,
            session,
            next_id: Arc::new(AtomicU64::new(1)),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    fn next_message_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("msg-{id:04}")
    }

    async fn request(
        &self,
        msg_type: MessageType,
        version: i64,
        payload: Value,
    ) -> Result<Message, BridgeError> {
        let request = Message {
            id: self.next_message_id(),
            msg_type,
            session: self.session.clone(),
            version,
            payload,
        };

        let line =
            to_ndjson(&request).map_err(|err| BridgeError::InvalidResponse(err.to_string()))?;

        // Retry once after reconnecting if the bridge closes the socket mid-request.
        for attempt in 0..2 {
            let mut guard = self.connection.lock().await;
            if guard.is_none() {
                *guard = Some(BridgeConnection::connect(&self.socket_path).await?);
            }

            let mut should_retry = false;
            if let Some(connection) = guard.as_mut() {
                if let Err(err) = connection.writer.write_all(line.as_bytes()).await {
                    *guard = None;
                    if attempt == 0 {
                        debug!("bridge write failed, reconnecting: {err}");
                        should_retry = true;
                    } else {
                        return Err(BridgeError::Io(err));
                    }
                } else if let Err(err) = connection.writer.flush().await {
                    *guard = None;
                    if attempt == 0 {
                        debug!("bridge flush failed, reconnecting: {err}");
                        should_retry = true;
                    } else {
                        return Err(BridgeError::Io(err));
                    }
                } else {
                    let mut response = String::new();
                    match connection.reader.read_line(&mut response).await {
                        Ok(0) => {
                            *guard = None;
                            if attempt == 0 {
                                debug!("bridge returned EOF, reconnecting");
                                should_retry = true;
                            } else {
                                return Err(BridgeError::Eof);
                            }
                        }
                        Ok(_) => {
                            let parsed = parse_message(response.trim_end())
                                .map_err(|err| BridgeError::InvalidResponse(err.to_string()))?;
                            return Ok(parsed);
                        }
                        Err(err) => {
                            *guard = None;
                            if attempt == 0 {
                                debug!("bridge read failed, reconnecting: {err}");
                                should_retry = true;
                            } else {
                                return Err(BridgeError::Io(err));
                            }
                        }
                    }
                }
            }

            drop(guard);
            if should_retry {
                continue;
            }
        }

        Err(BridgeError::Eof)
    }
}

#[derive(Clone)]
struct DocumentState {
    text: String,
    version: i64,
}

struct IsabelleLanguageServer {
    client: Client,
    bridge: BridgeTransport,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
}

impl IsabelleLanguageServer {
    fn new(client: Client, bridge_socket: PathBuf, session: String) -> Self {
        Self {
            client,
            bridge: BridgeTransport::new(bridge_socket, session),
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn upsert_document(&self, item: TextDocumentItem) {
        self.documents.write().await.insert(
            item.uri,
            DocumentState {
                text: item.text,
                version: i64::from(item.version),
            },
        );
    }

    async fn apply_change(
        &self,
        uri: &Url,
        version: i64,
        changes: Vec<TextDocumentContentChangeEvent>,
    ) -> Option<String> {
        let latest_text = changes.last().map(|change| change.text.clone())?;
        self.documents.write().await.insert(
            uri.clone(),
            DocumentState {
                text: latest_text.clone(),
                version,
            },
        );
        Some(latest_text)
    }

    async fn document_snapshot(&self, uri: &Url) -> Option<DocumentState> {
        self.documents.read().await.get(uri).cloned()
    }

    async fn push_document(&self, uri: &Url, version: i64, text: String) -> Result<(), String> {
        let payload = serde_json::to_value(DocumentPushPayload {
            uri: uri.to_string(),
            text,
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::DocumentPush, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        self.publish_diagnostics(uri.clone(), version, response)
            .await
    }

    async fn check_document(&self, uri: &Url, version: i64) -> Result<(), String> {
        let payload = serde_json::to_value(DocumentCheckPayload {
            uri: uri.to_string(),
            version,
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::DocumentCheck, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        self.publish_diagnostics(uri.clone(), version, response)
            .await
    }

    async fn publish_diagnostics(
        &self,
        uri: Url,
        version: i64,
        response: Message,
    ) -> Result<(), String> {
        if response.msg_type != MessageType::Diagnostics {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response
            .diagnostics_payload()
            .map_err(|err| err.to_string())?;
        let diagnostics = payload
            .iter()
            .map(bridge_diagnostic_to_lsp)
            .collect::<Vec<_>>();

        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: Some(i32::try_from(version).unwrap_or(i32::MAX)),
        };
        self.client
            .publish_diagnostics(params.uri, params.diagnostics, params.version)
            .await;
        Ok(())
    }

    async fn hover(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Option<Hover>, String> {
        let payload = serde_json::to_value(MarkupPayload {
            uri: uri.to_string(),
            offset: BridgePosition {
                line: i64::from(position.line),
                col: i64::from(position.character),
            },
            info: String::new(),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::Markup, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::Markup {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let markup_payload: MarkupPayload = response.payload_as().map_err(|err| err.to_string())?;

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(markup_payload.info)),
            range: None,
        }))
    }

    async fn run_check_command(&self, target_uri: Option<String>) -> Result<(), String> {
        let targets = if let Some(uri) = target_uri {
            let parsed = Url::parse(&uri).map_err(|err| err.to_string())?;
            if let Some(state) = self.document_snapshot(&parsed).await {
                vec![(parsed, state.version)]
            } else {
                vec![(parsed, 1)]
            }
        } else {
            self.documents
                .read()
                .await
                .iter()
                .map(|(uri, state)| (uri.clone(), state.version))
                .collect::<Vec<_>>()
        };

        for (uri, version) in targets {
            self.check_document(&uri, version).await?;
        }

        Ok(())
    }

    async fn clear_diagnostics(&self) {
        let uris = self
            .documents
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();

        for uri in uris {
            self.client.publish_diagnostics(uri, Vec::new(), None).await;
        }
    }

    async fn clear_diagnostics_for_uri(&self, uri: Url) {
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn remove_document(&self, uri: &Url) {
        self.documents.write().await.remove(uri);
    }

    async fn log_error(&self, message: String) {
        error!("{message}");
        self.client
            .log_message(LspMessageType::ERROR, message)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for IsabelleLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> JsonRpcResult<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "isabelle-zed-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        COMMAND_START_SESSION.to_string(),
                        COMMAND_STOP_SESSION.to_string(),
                        COMMAND_RUN_CHECK.to_string(),
                    ],
                    ..ExecuteCommandOptions::default()
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("isabelle-zed-lsp initialized");
    }

    async fn did_open(&self, params: tower_lsp::lsp_types::DidOpenTextDocumentParams) {
        let text_document = params.text_document;
        let uri = text_document.uri.clone();
        let version = i64::from(text_document.version);
        let text = text_document.text.clone();

        self.upsert_document(text_document).await;

        if let Err(err) = self.push_document(&uri, version, text).await {
            self.log_error(format!("failed to push document on open: {err}"))
                .await;
        }
    }

    async fn did_change(&self, params: tower_lsp::lsp_types::DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = i64::from(params.text_document.version);

        if let Some(text) = self
            .apply_change(&uri, version, params.content_changes)
            .await
            && let Err(err) = self.push_document(&uri, version, text).await
        {
            self.log_error(format!("failed to push document on change: {err}"))
                .await;
        }
    }

    async fn did_save(&self, params: tower_lsp::lsp_types::DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;

        let state = if let Some(text) = params.text {
            let version = self
                .document_snapshot(&uri)
                .await
                .map(|snapshot| snapshot.version)
                .unwrap_or(1);

            let new_state = DocumentState { text, version };
            self.documents
                .write()
                .await
                .insert(uri.clone(), new_state.clone());
            Some(new_state)
        } else {
            self.document_snapshot(&uri).await
        };

        if let Some(state) = state
            && let Err(err) = self.push_document(&uri, state.version, state.text).await
        {
            self.log_error(format!("failed to push document on save: {err}"))
                .await;
        }
    }

    async fn did_close(&self, params: tower_lsp::lsp_types::DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.remove_document(&uri).await;
        self.clear_diagnostics_for_uri(uri).await;
    }

    async fn hover(
        &self,
        params: tower_lsp::lsp_types::HoverParams,
    ) -> JsonRpcResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.hover(&uri, position, version).await {
            Ok(hover) => Ok(hover),
            Err(err) => {
                self.log_error(format!("failed to request hover: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn execute_command(
        &self,
        params: tower_lsp::lsp_types::ExecuteCommandParams,
    ) -> JsonRpcResult<Option<Value>> {
        let command = params.command.as_str();

        let result = match command {
            COMMAND_START_SESSION => {
                self.client
                    .log_message(LspMessageType::INFO, "Isabelle session started")
                    .await;
                Ok(())
            }
            COMMAND_STOP_SESSION => {
                self.clear_diagnostics().await;
                self.client
                    .log_message(LspMessageType::INFO, "Isabelle session stopped")
                    .await;
                Ok(())
            }
            COMMAND_RUN_CHECK => {
                self.run_check_command(command_target_uri(params.arguments.first()))
                    .await
            }
            _ => Err(format!("unknown command: {command}")),
        };

        if let Err(err) = result {
            self.log_error(format!("command failed ({command}): {err}"))
                .await;
        }

        Ok(None)
    }

    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }
}

fn bridge_diagnostic_to_lsp(diagnostic: &BridgeDiagnostic) -> Diagnostic {
    Diagnostic {
        range: bridge_range_to_lsp(&diagnostic.range),
        severity: Some(bridge_severity_to_lsp(diagnostic.severity)),
        message: diagnostic.message.clone(),
        source: Some("isabelle".to_string()),
        ..Diagnostic::default()
    }
}

fn bridge_range_to_lsp(range: &bridge::protocol::Range) -> Range {
    Range {
        start: Position {
            line: clamp_i64_to_u32(range.start.line),
            character: clamp_i64_to_u32(range.start.col),
        },
        end: Position {
            line: clamp_i64_to_u32(range.end.line),
            character: clamp_i64_to_u32(range.end.col),
        },
    }
}

fn bridge_severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

fn clamp_i64_to_u32(value: i64) -> u32 {
    if value <= 0 {
        return 0;
    }

    u32::try_from(value).unwrap_or(u32::MAX)
}

fn command_target_uri(argument: Option<&Value>) -> Option<String> {
    let value = argument?;
    match value {
        Value::String(uri) => Some(uri.clone()),
        Value::Object(object) => object
            .get("uri")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

async fn autostart_bridge_if_needed(socket_path: &Path) -> Option<Child> {
    if socket_path.exists() {
        return None;
    }

    let command = std::env::var(ENV_BRIDGE_AUTOSTART_CMD).ok()?;
    if command.trim().is_empty() {
        return None;
    }

    info!("autostarting bridge via command: {command}");
    let child = match Command::new("bash")
        .arg("-lc")
        .arg(command)
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

    let timeout_ms = std::env::var(ENV_BRIDGE_AUTOSTART_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_BRIDGE_AUTOSTART_TIMEOUT_MS);

    if !wait_for_socket(socket_path, Duration::from_millis(timeout_ms)).await {
        warn!(
            "bridge autostart command launched but socket {} was not ready within {}ms",
            socket_path.display(),
            timeout_ms
        );
    }

    Some(child)
}

async fn wait_for_socket(socket_path: &Path, timeout: Duration) -> bool {
    let start = tokio::time::Instant::now();
    while start.elapsed() <= timeout {
        if socket_path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let bridge_socket = std::env::var("ISABELLE_BRIDGE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BRIDGE_SOCKET));
    let session = std::env::var("ISABELLE_SESSION").unwrap_or_else(|_| DEFAULT_SESSION.to_string());
    let _bridge_child = autostart_bridge_if_needed(&bridge_socket).await;

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        IsabelleLanguageServer::new(client, bridge_socket.clone(), session.clone())
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge::protocol::{Position, Range, diagnostics_message_from_request};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::net::UnixListener;

    #[test]
    fn converts_bridge_diagnostic_to_lsp() {
        let diagnostic = BridgeDiagnostic {
            uri: "file:///tmp/example.thy".to_string(),
            range: Range {
                start: Position { line: 1, col: 2 },
                end: Position { line: 3, col: 4 },
            },
            severity: Severity::Warning,
            message: "warning message".to_string(),
        };

        let mapped = bridge_diagnostic_to_lsp(&diagnostic);
        assert_eq!(mapped.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(mapped.range.start.line, 1);
        assert_eq!(mapped.range.start.character, 2);
        assert_eq!(mapped.range.end.line, 3);
        assert_eq!(mapped.range.end.character, 4);
        assert_eq!(mapped.message, "warning message");
    }

    #[tokio::test]
    async fn bridge_transport_round_trip() {
        let temp = tempdir().expect("tempdir");
        let socket_path = temp.path().join("bridge.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind unix socket");

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");

            let request = parse_message(line.trim_end()).expect("parse request");
            let response = diagnostics_message_from_request(
                &request,
                "file:///home/user/example.thy",
                Severity::Error,
                "Parse error",
            )
            .expect("build diagnostics response");

            let ndjson = to_ndjson(&response).expect("serialize diagnostics response");
            write_half
                .write_all(ndjson.as_bytes())
                .await
                .expect("write diagnostics");
        });

        let transport = BridgeTransport::new(socket_path, "s1".to_string());
        let payload = serde_json::to_value(DocumentPushPayload {
            uri: "file:///home/user/example.thy".to_string(),
            text: "theory Example imports Main begin\nend\n".to_string(),
        })
        .expect("serialize payload");

        let response = transport
            .request(MessageType::DocumentPush, 1, payload)
            .await
            .expect("request must succeed");

        assert_eq!(response.msg_type, MessageType::Diagnostics);
        let diagnostics = response
            .diagnostics_payload()
            .expect("diagnostics payload should parse");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Parse error");

        server.await.expect("mock bridge server should finish");
    }

    #[test]
    fn extracts_command_target_uri_from_string_and_object() {
        assert_eq!(
            command_target_uri(Some(&json!("file:///tmp/test.thy"))),
            Some("file:///tmp/test.thy".to_string())
        );

        assert_eq!(
            command_target_uri(Some(&json!({ "uri": "file:///tmp/test2.thy" }))),
            Some("file:///tmp/test2.thy".to_string())
        );

        assert_eq!(command_target_uri(Some(&json!(42))), None);
        assert_eq!(command_target_uri(None), None);
    }
}
