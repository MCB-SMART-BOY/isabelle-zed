use bridge::protocol::{
    Diagnostic as BridgeDiagnostic, DocumentCheckPayload, DocumentPushPayload, MarkupPayload,
    Message, MessageType, Position as BridgePosition, Severity, parse_message, to_ndjson,
};
use serde_json::Value;
use shell_words::split as split_shell_words;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};
use tokio::time::{Duration, Instant, MissedTickBehavior};
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
const DEFAULT_BRIDGE_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_PUSH_DEBOUNCE_MS: u64 = 200;

const ENV_BRIDGE_AUTOSTART_CMD: &str = "ISABELLE_BRIDGE_AUTOSTART_CMD";
const ENV_BRIDGE_AUTOSTART_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS";
const ENV_BRIDGE_REQUEST_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS";

const COMMAND_START_SESSION: &str = "isabelle.start_session";
const COMMAND_STOP_SESSION: &str = "isabelle.stop_session";
const COMMAND_RUN_CHECK: &str = "isabelle.run_check";

#[derive(Debug, Error)]
enum BridgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bridge request timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
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
    request_timeout: Duration,
    next_id: Arc<AtomicU64>,
    connection: Arc<Mutex<Option<BridgeConnection>>>,
}

impl BridgeTransport {
    fn new(socket_path: PathBuf, session: String, request_timeout: Duration) -> Self {
        Self {
            socket_path,
            session,
            request_timeout,
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

        let request_id = request.id.clone();

        // Retry once after reconnecting if the bridge closes the socket mid-request.
        for attempt in 0..2 {
            let mut guard = self.connection.lock().await;
            if guard.is_none() {
                *guard = Some(BridgeConnection::connect(&self.socket_path).await?);
            }

            let mut should_retry = false;
            if let Some(connection) = guard.as_mut() {
                let io_result = tokio::time::timeout(self.request_timeout, async {
                    connection.writer.write_all(line.as_bytes()).await?;
                    connection.writer.flush().await?;
                    loop {
                        let mut response = String::new();
                        let bytes_read = connection.reader.read_line(&mut response).await?;
                        if bytes_read == 0 {
                            return Ok::<Option<Message>, std::io::Error>(None);
                        }

                        let parsed = parse_message(response.trim_end()).map_err(|err| {
                            std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
                        })?;

                        if parsed.id == request_id {
                            return Ok(Some(parsed));
                        }

                        warn!(
                            "ignoring bridge response with unexpected id: expected {}, got {}",
                            request_id, parsed.id
                        );
                    }
                })
                .await;

                match io_result {
                    Ok(Ok(None)) => {
                        *guard = None;
                        if attempt == 0 {
                            debug!("bridge returned EOF, reconnecting");
                            should_retry = true;
                        } else {
                            return Err(BridgeError::Eof);
                        }
                    }
                    Ok(Ok(Some(parsed))) => {
                        return Ok(parsed);
                    }
                    Ok(Err(err)) => {
                        if err.kind() == std::io::ErrorKind::InvalidData {
                            return Err(BridgeError::InvalidResponse(err.to_string()));
                        }

                        *guard = None;
                        if attempt == 0 {
                            debug!("bridge I/O failed, reconnecting: {err}");
                            should_retry = true;
                        } else {
                            return Err(BridgeError::Io(err));
                        }
                    }
                    Err(_) => {
                        *guard = None;
                        if attempt == 0 {
                            debug!(
                                "bridge request timed out after {}ms, reconnecting",
                                self.request_timeout.as_millis()
                            );
                            should_retry = true;
                        } else {
                            return Err(BridgeError::Timeout {
                                timeout_ms: u64::try_from(self.request_timeout.as_millis())
                                    .unwrap_or(u64::MAX),
                            });
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

#[derive(Clone)]
struct PendingPush {
    uri: Url,
    version: i64,
    text: String,
    queued_at: Instant,
}

enum PushEvent {
    Update { uri: Url, version: i64, text: String },
    Flush {
        uris: Option<Vec<Url>>,
        respond_to: oneshot::Sender<()>,
    },
}

struct IsabelleLanguageServer {
    client: Client,
    bridge: BridgeTransport,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    published_diagnostic_targets: Arc<RwLock<HashMap<Url, Vec<Url>>>>,
    session_running: Arc<RwLock<bool>>,
    push_tx: mpsc::UnboundedSender<PushEvent>,
}

impl IsabelleLanguageServer {
    fn new(
        client: Client,
        bridge_socket: PathBuf,
        session: String,
        request_timeout: Duration,
    ) -> Self {
        let bridge = BridgeTransport::new(bridge_socket, session, request_timeout);
        let documents = Arc::new(RwLock::new(HashMap::new()));
        let published_diagnostic_targets = Arc::new(RwLock::new(HashMap::new()));
        let session_running = Arc::new(RwLock::new(true));
        let (push_tx, push_rx) = mpsc::unbounded_channel();

        spawn_push_worker(
            push_rx,
            client.clone(),
            bridge.clone(),
            published_diagnostic_targets.clone(),
            session_running.clone(),
        );

        Self {
            client,
            bridge,
            documents,
            published_diagnostic_targets,
            session_running,
            push_tx,
        }
    }

    async fn is_session_running(&self) -> bool {
        *self.session_running.read().await
    }

    async fn start_session(&self) -> Result<(), String> {
        *self.session_running.write().await = true;
        self.run_check_command(None).await
    }

    async fn stop_session(&self) {
        *self.session_running.write().await = false;
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

    fn schedule_push(&self, uri: Url, version: i64, text: String) {
        if self
            .push_tx
            .send(PushEvent::Update { uri, version, text })
            .is_err()
        {
            error!("push worker channel closed; dropping document.push");
        }
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

    async fn flush_pushes(&self, uris: Option<Vec<Url>>) {
        let (respond_to, response) = oneshot::channel();
        if self
            .push_tx
            .send(PushEvent::Flush { uris, respond_to })
            .is_err()
        {
            return;
        }

        let _ = response.await;
    }

    async fn check_document(&self, uri: &Url, version: i64) -> Result<(), String> {
        if !self.is_session_running().await {
            return Err("isabelle session is stopped".to_string());
        }

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

        self.publish_diagnostics(uri.clone(), version, response).await
    }

    async fn publish_diagnostics(
        &self,
        uri: Url,
        version: i64,
        response: Message,
    ) -> Result<(), String> {
        publish_diagnostics_for(
            &self.client,
            &self.published_diagnostic_targets,
            uri,
            version,
            response,
        )
        .await
    }

    async fn hover(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Option<Hover>, String> {
        if !self.is_session_running().await {
            return Ok(None);
        }

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
        if !self.is_session_running().await {
            return Err("isabelle session is stopped".to_string());
        }

        let (targets, flush_uris) = if let Some(uri) = target_uri {
            let parsed = Url::parse(&uri).map_err(|err| err.to_string())?;
            let version = self
                .document_snapshot(&parsed)
                .await
                .map(|snapshot| snapshot.version)
                .unwrap_or(1);
            (vec![(parsed.clone(), version)], Some(vec![parsed]))
        } else {
            let targets = self
                .documents
                .read()
                .await
                .iter()
                .map(|(uri, state)| (uri.clone(), state.version))
                .collect::<Vec<_>>();
            (targets, None)
        };

        self.flush_pushes(flush_uris).await;

        for (uri, version) in targets {
            self.check_document(&uri, version).await?;
        }

        Ok(())
    }

    async fn clear_diagnostics(&self) {
        let stale_targets = {
            let mut state = self.published_diagnostic_targets.write().await;
            let all = state.values().flatten().cloned().collect::<Vec<_>>();
            state.clear();
            all
        };

        for uri in stale_targets {
            self.client.publish_diagnostics(uri, Vec::new(), None).await;
        }

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
        let related_uris = {
            let mut state = self.published_diagnostic_targets.write().await;
            state.remove(&uri).unwrap_or_default()
        };

        for related_uri in related_uris {
            self.client
                .publish_diagnostics(related_uri, Vec::new(), None)
                .await;
        }

        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn remove_document(&self, uri: &Url) {
        self.documents.write().await.remove(uri);
    }

    async fn log_error(&self, message: String) {
        log_error_for(&self.client, message).await;
    }
}

fn spawn_push_worker(
    mut rx: mpsc::UnboundedReceiver<PushEvent>,
    client: Client,
    bridge: BridgeTransport,
    published_diagnostic_targets: Arc<RwLock<HashMap<Url, Vec<Url>>>>,
    session_running: Arc<RwLock<bool>>,
) {
    tokio::spawn(async move {
        let mut pending_by_uri: HashMap<Url, PendingPush> = HashMap::new();
        let mut flush_tick = tokio::time::interval(Duration::from_millis(50));
        flush_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Some(PushEvent::Update { uri, version, text }) => {
                            let queued_at = Instant::now();
                            match pending_by_uri.get_mut(&uri) {
                                Some(existing) if existing.version > version => {}
                                Some(existing) => {
                                    existing.version = version;
                                    existing.text = text;
                                    existing.queued_at = queued_at;
                                }
                                None => {
                                    pending_by_uri.insert(uri.clone(), PendingPush { uri, version, text, queued_at });
                                }
                            }
                        }
                        Some(PushEvent::Flush { uris, respond_to }) => {
                            let targets = match uris {
                                Some(list) => list,
                                None => pending_by_uri.keys().cloned().collect(),
                            };
                            for uri in targets {
                                if let Some(pending) = pending_by_uri.remove(&uri) {
                                    if let Err(err) = send_document_push(
                                        &bridge,
                                        &client,
                                        &published_diagnostic_targets,
                                        &session_running,
                                        &pending.uri,
                                        pending.version,
                                        pending.text,
                                    )
                                    .await
                                    {
                                        log_error_for(&client, format!("failed to push document: {err}")).await;
                                    }
                                }
                            }
                            let _ = respond_to.send(());
                        }
                        None => break,
                    }
                }
                _ = flush_tick.tick() => {
                    let now = Instant::now();
                    let ready = pending_by_uri
                        .iter()
                        .filter(|(_, pending)| now.duration_since(pending.queued_at) >= Duration::from_millis(DEFAULT_PUSH_DEBOUNCE_MS))
                        .map(|(uri, _)| uri.clone())
                        .collect::<Vec<_>>();

                    for uri in ready {
                        if let Some(pending) = pending_by_uri.remove(&uri) {
                            if let Err(err) = send_document_push(
                                &bridge,
                                &client,
                                &published_diagnostic_targets,
                                &session_running,
                                &pending.uri,
                                pending.version,
                                pending.text,
                            )
                            .await
                            {
                                log_error_for(&client, format!("failed to push document: {err}")).await;
                            }
                        }
                    }
                }
            }
        }
    });
}

async fn send_document_push(
    bridge: &BridgeTransport,
    client: &Client,
    published_diagnostic_targets: &Arc<RwLock<HashMap<Url, Vec<Url>>>>,
    session_running: &Arc<RwLock<bool>>,
    uri: &Url,
    version: i64,
    text: String,
) -> Result<(), String> {
    if !*session_running.read().await {
        return Ok(());
    }

    let payload = serde_json::to_value(DocumentPushPayload {
        uri: uri.to_string(),
        text,
    })
    .map_err(|err| err.to_string())?;

    let response = bridge
        .request(MessageType::DocumentPush, version, payload)
        .await
        .map_err(|err| err.to_string())?;

    publish_diagnostics_for(client, published_diagnostic_targets, uri.clone(), version, response)
        .await
}

async fn publish_diagnostics_for(
    client: &Client,
    published_diagnostic_targets: &Arc<RwLock<HashMap<Url, Vec<Url>>>>,
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
    let mut grouped: HashMap<Url, Vec<Diagnostic>> = HashMap::new();

    for diagnostic in &payload {
        let target_uri = match Url::parse(&diagnostic.uri) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(
                    "bridge diagnostic had invalid uri '{}': {err}; publishing to request uri",
                    diagnostic.uri
                );
                uri.clone()
            }
        };

        grouped
            .entry(target_uri)
            .or_default()
            .push(bridge_diagnostic_to_lsp(diagnostic));
    }

    grouped.entry(uri.clone()).or_default();

    let current_targets = grouped.keys().cloned().collect::<Vec<_>>();
    let stale_targets = {
        let mut state = published_diagnostic_targets.write().await;
        let previous = state.get(&uri).cloned().unwrap_or_default();
        state.insert(uri.clone(), current_targets.clone());
        previous
    };

    let current_target_set = current_targets.into_iter().collect::<HashSet<_>>();
    let request_version = Some(i32::try_from(version).unwrap_or(i32::MAX));

    for stale_uri in stale_targets {
        if !current_target_set.contains(&stale_uri) {
            let publish_version = if stale_uri == uri {
                request_version
            } else {
                None
            };
            client
                .publish_diagnostics(stale_uri, Vec::new(), publish_version)
                .await;
        }
    }

    for (target_uri, diagnostics) in grouped {
        let publish_version = if target_uri == uri {
            request_version
        } else {
            None
        };
        let params = PublishDiagnosticsParams {
            uri: target_uri,
            diagnostics,
            version: publish_version,
        };
        client
            .publish_diagnostics(params.uri, params.diagnostics, params.version)
            .await;
    }

    Ok(())
}

async fn log_error_for(client: &Client, message: String) {
    error!("{message}");
    client.log_message(LspMessageType::ERROR, message).await;
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
        self.schedule_push(uri, version, text);
    }

    async fn did_change(&self, params: tower_lsp::lsp_types::DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = i64::from(params.text_document.version);

        if let Some(text) = self
            .apply_change(&uri, version, params.content_changes)
            .await
        {
            self.schedule_push(uri, version, text);
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

        if let Some(state) = state {
            self.schedule_push(uri, state.version, state.text);
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
                let start_result = self.start_session().await;
                self.client
                    .log_message(LspMessageType::INFO, "Isabelle session started")
                    .await;
                start_result
            }
            COMMAND_STOP_SESSION => {
                self.stop_session().await;
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
            line: bridge_index_to_lsp(range.start.line),
            character: bridge_index_to_lsp(range.start.col),
        },
        end: Position {
            line: bridge_index_to_lsp(range.end.line),
            character: bridge_index_to_lsp(range.end.col),
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

fn bridge_index_to_lsp(value: i64) -> u32 {
    if value <= 1 {
        return 0;
    }

    u32::try_from(value - 1).unwrap_or(u32::MAX)
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

fn parse_autostart_command(command: &str) -> Result<(String, Vec<String>), String> {
    let parts = split_shell_words(command).map_err(|err| err.to_string())?;
    if parts.is_empty() {
        return Err("autostart command is empty".to_string());
    }

    Ok((parts[0].clone(), parts[1..].to_vec()))
}

async fn autostart_bridge_if_needed(socket_path: &Path) -> Option<Child> {
    if socket_path.exists() {
        if socket_is_healthy(socket_path).await {
            return None;
        }

        if let Err(err) = remove_stale_socket(socket_path) {
            error!("failed to remove stale bridge socket {}: {err}", socket_path.display());
            return None;
        }
    }

    let command = std::env::var(ENV_BRIDGE_AUTOSTART_CMD).ok()?;
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

async fn socket_is_healthy(socket_path: &Path) -> bool {
    match tokio::time::timeout(Duration::from_millis(200), UnixStream::connect(socket_path)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            true
        }
        Ok(Err(err)) => {
            debug!("bridge socket {} is not connectable: {err}", socket_path.display());
            false
        }
        Err(_) => false,
    }
}

fn remove_stale_socket(socket_path: &Path) -> Result<(), std::io::Error> {
    let metadata = std::fs::symlink_metadata(socket_path)?;
    #[cfg(unix)]
    {
        if metadata.file_type().is_socket() {
            return std::fs::remove_file(socket_path);
        }
        return Err(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "refusing to remove non-socket path before autostart: {}",
                socket_path.display()
            ),
        ));
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
    let request_timeout = std::env::var(ENV_BRIDGE_REQUEST_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_BRIDGE_REQUEST_TIMEOUT_MS));
    let mut bridge_child = autostart_bridge_if_needed(&bridge_socket).await;

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        IsabelleLanguageServer::new(
            client,
            bridge_socket.clone(),
            session.clone(),
            request_timeout,
        )
    });

    Server::new(stdin, stdout, socket).serve(service).await;

    if let Some(mut child) = bridge_child.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge::protocol::{Position, Range, diagnostics_message_from_request};
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::net::UnixListener;
    use tokio::time::sleep;

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
        assert_eq!(mapped.range.start.line, 0);
        assert_eq!(mapped.range.start.character, 1);
        assert_eq!(mapped.range.end.line, 2);
        assert_eq!(mapped.range.end.character, 3);
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

        let transport = BridgeTransport::new(socket_path, "s1".to_string(), Duration::from_secs(2));
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

    #[tokio::test]
    async fn bridge_transport_ignores_unmatched_response_ids() {
        let temp = tempdir().expect("tempdir");
        let socket_path = temp.path().join("bridge-mismatch-id.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind unix socket");

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = stream.into_split();
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");

            let request = parse_message(line.trim_end()).expect("parse request");

            let wrong_response = Message {
                id: "msg-9999".to_string(),
                msg_type: MessageType::Diagnostics,
                session: request.session.clone(),
                version: request.version,
                payload: request.payload.clone(),
            };

            let ndjson = to_ndjson(&wrong_response).expect("serialize wrong response");
            write_half
                .write_all(ndjson.as_bytes())
                .await
                .expect("write wrong response");

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

        let transport = BridgeTransport::new(socket_path, "s1".to_string(), Duration::from_secs(2));
        let payload = serde_json::to_value(DocumentPushPayload {
            uri: "file:///home/user/example.thy".to_string(),
            text: "theory Example imports Main begin\nend\n".to_string(),
        })
        .expect("serialize payload");

        let response = transport
            .request(MessageType::DocumentPush, 1, payload)
            .await
            .expect("request must succeed");
        assert_eq!(response.id, "msg-0001");
        assert_eq!(response.msg_type, MessageType::Diagnostics);

        server.await.expect("mock bridge server should finish");
    }

    #[tokio::test]
    async fn bridge_transport_times_out_when_bridge_does_not_reply() {
        let temp = tempdir().expect("tempdir");
        let socket_path = temp.path().join("bridge-timeout.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind unix socket");

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.expect("accept connection");
                let (read_half, _) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                let _ = reader.read_line(&mut line).await;
                sleep(Duration::from_millis(300)).await;
            }
        });

        let transport =
            BridgeTransport::new(socket_path, "s1".to_string(), Duration::from_millis(100));
        let payload = serde_json::to_value(DocumentPushPayload {
            uri: "file:///home/user/example.thy".to_string(),
            text: "theory Example imports Main begin\nend\n".to_string(),
        })
        .expect("serialize payload");

        let result = transport
            .request(MessageType::DocumentPush, 1, payload)
            .await;
        assert!(matches!(result, Err(BridgeError::Timeout { .. })));

        server.await.expect("timeout server should finish");
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

    #[test]
    fn parses_autostart_command_with_quoted_arguments() {
        let (program, args) = parse_autostart_command(
            "bridge --socket /tmp/isabelle.sock --adapter-command \"bridge --mock-adapter\"",
        )
        .expect("command should parse");

        assert_eq!(program, "bridge");
        assert_eq!(
            args,
            vec![
                "--socket".to_string(),
                "/tmp/isabelle.sock".to_string(),
                "--adapter-command".to_string(),
                "bridge --mock-adapter".to_string()
            ]
        );
    }

    #[test]
    fn rejects_empty_autostart_command() {
        assert!(parse_autostart_command("   ").is_err());
    }
}
