mod autostart;
mod diagnostics;
mod push;
mod transport;

#[cfg(all(test, unix))]
use bridge::protocol::DocumentPushPayload;
use bridge::protocol::{
    CodeActionPayload as BridgeCodeActionPayload, CompletionItemPayload, DocumentCheckPayload,
    DocumentUriPayload, LocationPayload, MarkupPayload, Message, MessageType,
    Position as BridgePosition, QueryPayload, RenamePayload, SemanticTokenPayload, SymbolPayload,
    TextEditPayload, WorkspaceSymbolQueryPayload,
};
use diagnostics::{PublishedDiagnosticTargets, publish_diagnostics_for};
use push::{PushEvent, spawn_push_worker};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::time::Duration;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, DocumentSymbolParams,
    DocumentSymbolResponse, ExecuteCommandOptions, GotoDefinitionParams, GotoDefinitionResponse,
    Hover, HoverContents, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, Location, MarkedString, MessageType as LspMessageType, OneOf, Position,
    Range as LspRange, ReferenceParams, RenameParams, SemanticToken, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, SymbolInformation,
    SymbolKind, TextDocumentContentChangeEvent, TextDocumentItem, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing::{error, info};
#[cfg(all(test, unix))]
use transport::BridgeError;
use transport::{BridgeEndpoint, BridgeTransport};

#[cfg(unix)]
const DEFAULT_BRIDGE_UNIX_SOCKET: &str = "/tmp/isabelle.sock";
#[cfg(not(unix))]
const DEFAULT_BRIDGE_TCP_ENDPOINT: &str = "127.0.0.1:39393";
const DEFAULT_SESSION: &str = "s1";
const DEFAULT_BRIDGE_AUTOSTART_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_BRIDGE_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_PUSH_DEBOUNCE_MS: u64 = 200;

const ENV_BRIDGE_ENDPOINT: &str = "ISABELLE_BRIDGE_ENDPOINT";
const ENV_BRIDGE_SOCKET: &str = "ISABELLE_BRIDGE_SOCKET";
const ENV_BRIDGE_AUTOSTART_CMD: &str = "ISABELLE_BRIDGE_AUTOSTART_CMD";
const ENV_BRIDGE_AUTOSTART_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS";
const ENV_BRIDGE_REQUEST_TIMEOUT_MS: &str = "ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS";

const COMMAND_START_SESSION: &str = "isabelle.start_session";
const COMMAND_STOP_SESSION: &str = "isabelle.stop_session";
const COMMAND_RUN_CHECK: &str = "isabelle.run_check";

#[derive(Clone)]
struct DocumentState {
    text: String,
    version: i64,
}

struct IsabelleLanguageServer {
    client: Client,
    bridge: BridgeTransport,
    documents: Arc<RwLock<HashMap<Url, DocumentState>>>,
    published_diagnostic_targets: PublishedDiagnosticTargets,
    session_running: Arc<RwLock<bool>>,
    push_tx: mpsc::UnboundedSender<PushEvent>,
}

impl IsabelleLanguageServer {
    fn new(
        client: Client,
        bridge_endpoint: BridgeEndpoint,
        session: String,
        request_timeout: Duration,
    ) -> Self {
        let bridge = BridgeTransport::new(bridge_endpoint, session, request_timeout);
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
            Duration::from_millis(DEFAULT_PUSH_DEBOUNCE_MS),
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

        self.publish_diagnostics(uri.clone(), version, response)
            .await
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
            offset: lsp_position_to_bridge(position),
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

    async fn definition_locations(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Vec<Location>, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(QueryPayload {
            uri: uri.to_string(),
            offset: lsp_position_to_bridge(position),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::Definition, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::Definition {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response.location_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_location_to_lsp)
            .collect())
    }

    async fn reference_locations(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Vec<Location>, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(QueryPayload {
            uri: uri.to_string(),
            offset: lsp_position_to_bridge(position),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::References, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::References {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response.location_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_location_to_lsp)
            .collect())
    }

    async fn completion_items(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Vec<CompletionItem>, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(QueryPayload {
            uri: uri.to_string(),
            offset: lsp_position_to_bridge(position),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::Completion, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::Completion {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response
            .completion_payload()
            .map_err(|err| err.to_string())?;
        Ok(payload.into_iter().map(bridge_completion_to_lsp).collect())
    }

    async fn document_symbols(
        &self,
        uri: &Url,
        version: i64,
    ) -> Result<Vec<SymbolInformation>, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(DocumentUriPayload {
            uri: uri.to_string(),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::DocumentSymbols, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::DocumentSymbols {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response.symbols_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_symbol_to_lsp)
            .collect())
    }

    async fn rename_workspace_edit(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
        new_name: String,
    ) -> Result<Option<WorkspaceEdit>, String> {
        if !self.is_session_running().await {
            return Ok(None);
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(RenamePayload {
            uri: uri.to_string(),
            offset: lsp_position_to_bridge(position),
            new_name,
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::Rename, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::Rename {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let edits = response
            .text_edits_payload()
            .map_err(|err| err.to_string())?;
        Ok(workspace_edit_from_payload(edits))
    }

    async fn code_actions(&self, uri: &Url, version: i64) -> Result<CodeActionResponse, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(DocumentUriPayload {
            uri: uri.to_string(),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::CodeAction, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::CodeAction {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let actions = response
            .code_actions_payload()
            .map_err(|err| err.to_string())?;
        Ok(actions
            .into_iter()
            .map(bridge_code_action_to_lsp)
            .map(CodeActionOrCommand::CodeAction)
            .collect())
    }

    async fn semantic_tokens(
        &self,
        uri: &Url,
        version: i64,
    ) -> Result<SemanticTokensResult, String> {
        if !self.is_session_running().await {
            return Ok(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: Vec::new(),
            }));
        }

        self.flush_pushes(Some(vec![uri.clone()])).await;

        let payload = serde_json::to_value(DocumentUriPayload {
            uri: uri.to_string(),
        })
        .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::SemanticTokens, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::SemanticTokens {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response
            .semantic_tokens_payload()
            .map_err(|err| err.to_string())?;
        Ok(bridge_semantic_tokens_to_lsp(payload))
    }

    async fn workspace_symbols(&self, query: String) -> Result<Vec<SymbolInformation>, String> {
        if !self.is_session_running().await {
            return Ok(Vec::new());
        }

        let payload = serde_json::to_value(WorkspaceSymbolQueryPayload { query })
            .map_err(|err| err.to_string())?;

        let response = self
            .bridge
            .request(MessageType::WorkspaceSymbols, 1, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::WorkspaceSymbols {
            return Err(format!(
                "unexpected response type from bridge: {:?}",
                response.msg_type
            ));
        }

        let payload = response.symbols_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_symbol_to_lsp)
            .collect())
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

pub(crate) async fn log_error_for(client: &Client, message: String) {
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
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string(), "_".to_string()]),
                    ..CompletionOptions::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: Default::default(),
                            legend: semantic_tokens_legend(),
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                workspace_symbol_provider: Some(OneOf::Left(true)),
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

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonRpcResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.definition_locations(&uri, position, version).await {
            Ok(locations) if locations.is_empty() => Ok(None),
            Ok(locations) => Ok(Some(GotoDefinitionResponse::Array(locations))),
            Err(err) => {
                self.log_error(format!("failed to request definition: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn references(&self, params: ReferenceParams) -> JsonRpcResult<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.reference_locations(&uri, position, version).await {
            Ok(locations) => Ok(Some(locations)),
            Err(err) => {
                self.log_error(format!("failed to request references: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> JsonRpcResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.completion_items(&uri, position, version).await {
            Ok(items) => Ok(Some(CompletionResponse::Array(items))),
            Err(err) => {
                self.log_error(format!("failed to request completion: {err}"))
                    .await;
                Ok(Some(CompletionResponse::Array(Vec::new())))
            }
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonRpcResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.document_symbols(&uri, version).await {
            Ok(symbols) => Ok(Some(DocumentSymbolResponse::Flat(symbols))),
            Err(err) => {
                self.log_error(format!("failed to request document symbols: {err}"))
                    .await;
                Ok(Some(DocumentSymbolResponse::Flat(Vec::new())))
            }
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonRpcResult<Option<Vec<SymbolInformation>>> {
        match self.workspace_symbols(params.query).await {
            Ok(symbols) => Ok(Some(symbols)),
            Err(err) => {
                self.log_error(format!("failed to request workspace symbols: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn rename(&self, params: RenameParams) -> JsonRpcResult<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self
            .rename_workspace_edit(&uri, position, version, params.new_name)
            .await
        {
            Ok(edit) => Ok(edit),
            Err(err) => {
                self.log_error(format!("failed to request rename: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.code_actions(&uri, version).await {
            Ok(actions) => Ok(Some(actions)),
            Err(err) => {
                self.log_error(format!("failed to request code actions: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonRpcResult<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.semantic_tokens(&uri, version).await {
            Ok(tokens) => Ok(Some(tokens)),
            Err(err) => {
                self.log_error(format!("failed to request semantic tokens: {err}"))
                    .await;
                Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                    result_id: None,
                    data: Vec::new(),
                })))
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
                if start_result.is_ok() {
                    self.client
                        .log_message(LspMessageType::INFO, "Isabelle session started")
                        .await;
                }
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

fn lsp_position_to_bridge(position: Position) -> BridgePosition {
    BridgePosition {
        line: i64::from(position.line.saturating_add(1)),
        col: i64::from(position.character.saturating_add(1)),
    }
}

fn bridge_range_to_lsp(range: bridge::protocol::Range) -> LspRange {
    LspRange {
        start: Position {
            line: u32::try_from(range.start.line.saturating_sub(1)).unwrap_or(0),
            character: u32::try_from(range.start.col.saturating_sub(1)).unwrap_or(0),
        },
        end: Position {
            line: u32::try_from(range.end.line.saturating_sub(1)).unwrap_or(0),
            character: u32::try_from(range.end.col.saturating_sub(1)).unwrap_or(0),
        },
    }
}

fn bridge_location_to_lsp(location: LocationPayload) -> Option<Location> {
    let uri = Url::parse(&location.uri).ok()?;
    Some(Location {
        uri,
        range: bridge_range_to_lsp(location.range),
    })
}

fn bridge_completion_to_lsp(item: CompletionItemPayload) -> CompletionItem {
    let kind = if item.detail.as_deref() == Some("keyword") {
        Some(CompletionItemKind::KEYWORD)
    } else {
        Some(CompletionItemKind::TEXT)
    };

    CompletionItem {
        label: item.label,
        kind,
        detail: item.detail,
        ..CompletionItem::default()
    }
}

#[allow(deprecated)]
fn bridge_symbol_to_lsp(symbol: SymbolPayload) -> Option<SymbolInformation> {
    let uri = Url::parse(&symbol.uri).ok()?;
    let kind = bridge_symbol_kind(&symbol.kind);
    Some(SymbolInformation {
        name: symbol.name,
        kind,
        location: Location {
            uri,
            range: bridge_range_to_lsp(symbol.range),
        },
        tags: None,
        deprecated: None,
        container_name: None,
    })
}

fn bridge_symbol_kind(kind: &str) -> SymbolKind {
    match kind {
        "type" => SymbolKind::STRUCT,
        "module" => SymbolKind::MODULE,
        "function" => SymbolKind::FUNCTION,
        "theorem" => SymbolKind::CONSTANT,
        _ => SymbolKind::VARIABLE,
    }
}

fn workspace_edit_from_payload(edits: Vec<TextEditPayload>) -> Option<WorkspaceEdit> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for edit in edits {
        let Ok(uri) = Url::parse(&edit.uri) else {
            continue;
        };
        changes.entry(uri).or_default().push(TextEdit {
            range: bridge_range_to_lsp(edit.range),
            new_text: edit.new_text,
        });
    }

    if changes.is_empty() {
        return None;
    }

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

fn bridge_code_action_to_lsp(action: BridgeCodeActionPayload) -> CodeAction {
    let kind = bridge_code_action_kind(&action.kind);
    let edit = workspace_edit_from_payload(action.edits);
    CodeAction {
        title: action.title,
        kind,
        diagnostics: None,
        edit,
        command: None,
        is_preferred: None,
        disabled: None,
        data: None,
    }
}

fn bridge_code_action_kind(kind: &str) -> Option<CodeActionKind> {
    match kind {
        "quickfix" => Some(CodeActionKind::QUICKFIX),
        "refactor" => Some(CodeActionKind::REFACTOR),
        _ => None,
    }
}

fn semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::KEYWORD,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::TYPE,
            SemanticTokenType::NAMESPACE,
            SemanticTokenType::VARIABLE,
        ],
        token_modifiers: vec![SemanticTokenModifier::DECLARATION],
    }
}

fn semantic_token_type_index(token_type: &str) -> u32 {
    match token_type {
        "keyword" => 0,
        "function" => 1,
        "type" => 2,
        "namespace" => 3,
        _ => 4,
    }
}

fn bridge_semantic_tokens_to_lsp(tokens: Vec<SemanticTokenPayload>) -> SemanticTokensResult {
    let mut tokens = tokens
        .into_iter()
        .filter_map(|token| {
            let line = u32::try_from(token.line.saturating_sub(1)).ok()?;
            let start = u32::try_from(token.col.saturating_sub(1)).ok()?;
            let length = u32::try_from(token.length.max(0)).ok()?;
            Some((line, start, length, token.token_type))
        })
        .collect::<Vec<_>>();
    tokens.sort_by_key(|(line, start, _, _)| (*line, *start));

    let mut encoded = Vec::with_capacity(tokens.len());
    let mut prev_line = 0_u32;
    let mut prev_start = 0_u32;
    let mut is_first = true;

    for (line, start, length, token_type) in tokens {
        let (delta_line, delta_start) = if is_first {
            is_first = false;
            (line, start)
        } else if line == prev_line {
            (0, start.saturating_sub(prev_start))
        } else {
            (line.saturating_sub(prev_line), start)
        };

        prev_line = line;
        prev_start = start;

        encoded.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: semantic_token_type_index(&token_type),
            token_modifiers_bitset: 0,
        });
    }

    SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: encoded,
    })
}

fn default_bridge_endpoint() -> BridgeEndpoint {
    #[cfg(unix)]
    {
        BridgeEndpoint::Unix(PathBuf::from(DEFAULT_BRIDGE_UNIX_SOCKET))
    }
    #[cfg(not(unix))]
    {
        BridgeEndpoint::Tcp(DEFAULT_BRIDGE_TCP_ENDPOINT.to_string())
    }
}

fn resolve_bridge_endpoint() -> BridgeEndpoint {
    if let Ok(raw) = std::env::var(ENV_BRIDGE_ENDPOINT) {
        match BridgeEndpoint::parse(&raw) {
            Ok(endpoint) => return endpoint,
            Err(err) => {
                error!(
                    "invalid {}='{}': {}; using default endpoint",
                    ENV_BRIDGE_ENDPOINT, raw, err
                );
            }
        }
    }

    if let Ok(raw_socket) = std::env::var(ENV_BRIDGE_SOCKET) {
        return BridgeEndpoint::Unix(PathBuf::from(raw_socket));
    }

    default_bridge_endpoint()
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

    let bridge_endpoint = resolve_bridge_endpoint();
    let session = std::env::var("ISABELLE_SESSION").unwrap_or_else(|_| DEFAULT_SESSION.to_string());
    let request_timeout = std::env::var(ENV_BRIDGE_REQUEST_TIMEOUT_MS)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_BRIDGE_REQUEST_TIMEOUT_MS));
    let mut bridge_child = autostart::autostart_bridge_if_needed(
        &bridge_endpoint,
        ENV_BRIDGE_AUTOSTART_CMD,
        ENV_BRIDGE_AUTOSTART_TIMEOUT_MS,
        DEFAULT_BRIDGE_AUTOSTART_TIMEOUT_MS,
    )
    .await;

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| {
        IsabelleLanguageServer::new(
            client,
            bridge_endpoint.clone(),
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
    use bridge::protocol::{
        Diagnostic as BridgeDiagnostic, Message, Position, Range, Severity,
        diagnostics_message_from_request, parse_message, to_ndjson,
    };
    use serde_json::json;
    #[cfg(unix)]
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    #[cfg(unix)]
    use tokio::net::UnixListener;
    #[cfg(unix)]
    use tokio::time::sleep;
    use tower_lsp::lsp_types::DiagnosticSeverity;

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

        let mapped = diagnostics::bridge_diagnostic_to_lsp(&diagnostic);
        assert_eq!(mapped.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(mapped.range.start.line, 0);
        assert_eq!(mapped.range.start.character, 1);
        assert_eq!(mapped.range.end.line, 2);
        assert_eq!(mapped.range.end.character, 3);
        assert_eq!(mapped.message, "warning message");
    }

    #[cfg(unix)]
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

        let transport = BridgeTransport::new(
            BridgeEndpoint::Unix(socket_path),
            "s1".to_string(),
            Duration::from_secs(2),
        );
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

    #[cfg(unix)]
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

        let transport = BridgeTransport::new(
            BridgeEndpoint::Unix(socket_path),
            "s1".to_string(),
            Duration::from_secs(2),
        );
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

    #[cfg(unix)]
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

        let transport = BridgeTransport::new(
            BridgeEndpoint::Unix(socket_path),
            "s1".to_string(),
            Duration::from_millis(100),
        );
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
        let (program, args) = autostart::parse_autostart_command(
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
        assert!(autostart::parse_autostart_command("   ").is_err());
    }

    #[test]
    fn converts_lsp_hover_position_to_bridge_one_based() {
        let converted = lsp_position_to_bridge(tower_lsp::lsp_types::Position::new(0, 0));
        assert_eq!(converted.line, 1);
        assert_eq!(converted.col, 1);

        let converted = lsp_position_to_bridge(tower_lsp::lsp_types::Position::new(4, 9));
        assert_eq!(converted.line, 5);
        assert_eq!(converted.col, 10);
    }
}
