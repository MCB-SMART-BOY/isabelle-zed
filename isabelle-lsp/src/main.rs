mod autostart;
mod diagnostics;
mod push;
mod transport;

#[cfg(all(test, unix))]
use bridge::protocol::DocumentPushPayload;
use bridge::protocol::{
    CodeActionPayload as BridgeCodeActionPayload, CompletionItemPayload, DocumentCheckPayload,
    DocumentFormattingPayload as BridgeDocumentFormattingPayload,
    DocumentLinkPayload as BridgeDocumentLinkPayload, DocumentUriPayload,
    FormattingOptionsPayload as BridgeFormattingOptionsPayload,
    InlayHintPayload as BridgeInlayHintPayload, LocationPayload, MarkupPayload, Message,
    MessageType, OnTypeFormattingPayload as BridgeOnTypeFormattingPayload,
    Position as BridgePosition, QueryPayload,
    RangeFormattingPayload as BridgeRangeFormattingPayload, RenamePayload, RenameResultPayload,
    SemanticTokenPayload, SignatureHelpPayload as BridgeSignatureHelpPayload, SymbolPayload,
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
use tower_lsp::lsp_types::request::{
    GotoDeclarationParams, GotoDeclarationResponse, GotoImplementationParams,
    GotoImplementationResponse, GotoTypeDefinitionParams, GotoTypeDefinitionResponse,
};
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, CodeLens, CodeLensOptions, CodeLensParams,
    Command, CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
    CompletionResponse, DeclarationCapability, DocumentFormattingParams, DocumentHighlight,
    DocumentHighlightKind, DocumentHighlightParams, DocumentLink, DocumentLinkOptions,
    DocumentLinkParams, DocumentOnTypeFormattingOptions, DocumentOnTypeFormattingParams,
    DocumentRangeFormattingParams, DocumentSymbolParams, DocumentSymbolResponse,
    ExecuteCommandOptions, FoldingRange, FoldingRangeParams, FoldingRangeProviderCapability,
    FormattingOptions, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverProviderCapability, ImplementationProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, InlayHint, InlayHintKind, InlayHintLabel, InlayHintOptions, InlayHintParams,
    InlayHintServerCapabilities, Location, MarkedString, MessageType as LspMessageType, OneOf,
    ParameterInformation, ParameterLabel, Position, PrepareRenameResponse, Range as LspRange,
    ReferenceParams, RenameOptions, RenameParams, SelectionRange, SelectionRangeParams,
    SelectionRangeProviderCapability, SemanticToken, SemanticTokenModifier, SemanticTokenType,
    SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities,
    ServerCapabilities, ServerInfo, SignatureHelp, SignatureHelpOptions, SignatureHelpParams,
    SignatureInformation, SymbolInformation, SymbolKind, TextDocumentContentChangeEvent,
    TextDocumentItem, TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextEdit, TypeDefinitionProviderCapability, Url, WorkspaceEdit, WorkspaceSymbolParams,
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

    async fn document_highlights(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Vec<DocumentHighlight>, String> {
        let mut highlights = self
            .reference_locations(uri, position, version)
            .await?
            .into_iter()
            .filter(|location| location.uri == *uri)
            .map(|location| DocumentHighlight {
                range: location.range,
                kind: Some(DocumentHighlightKind::TEXT),
            })
            .collect::<Vec<_>>();

        highlights.sort_by(|a, b| {
            a.range
                .start
                .line
                .cmp(&b.range.start.line)
                .then_with(|| a.range.start.character.cmp(&b.range.start.character))
                .then_with(|| a.range.end.line.cmp(&b.range.end.line))
                .then_with(|| a.range.end.character.cmp(&b.range.end.character))
        });
        highlights.dedup_by(|a, b| a.range == b.range);
        Ok(highlights)
    }

    async fn rename_target_range(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Option<LspRange>, String> {
        let locations = self.reference_locations(uri, position, version).await?;
        Ok(locations
            .into_iter()
            .find(|location| {
                location.uri == *uri && lsp_position_in_range(position, location.range)
            })
            .map(|location| location.range))
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

        let RenameResultPayload { edits, warning } = response
            .rename_result_payload()
            .map_err(|err| err.to_string())?;
        if let Some(message) = warning {
            self.client
                .log_message(LspMessageType::WARNING, message)
                .await;
            return Ok(None);
        }
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

    async fn type_definition_locations(
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
            .request(MessageType::TypeDefinition, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::TypeDefinition {
            return self.definition_locations(uri, position, version).await;
        }

        let payload = response.location_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_location_to_lsp)
            .collect())
    }

    async fn implementation_locations(
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
            .request(MessageType::Implementation, version, payload)
            .await
            .map_err(|err| err.to_string())?;

        if response.msg_type != MessageType::Implementation {
            return self.definition_locations(uri, position, version).await;
        }

        let payload = response.location_payload().map_err(|err| err.to_string())?;
        Ok(payload
            .into_iter()
            .filter_map(bridge_location_to_lsp)
            .collect())
    }

    async fn selection_ranges(
        &self,
        uri: &Url,
        positions: Vec<Position>,
    ) -> Result<Vec<SelectionRange>, String> {
        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        Ok(positions
            .into_iter()
            .map(|position| selection_range_for_position(&text, position))
            .collect())
    }

    async fn folding_ranges(&self, uri: &Url) -> Result<Vec<FoldingRange>, String> {
        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        Ok(folding_ranges_from_text(&text))
    }

    async fn signature_help_for_position(
        &self,
        uri: &Url,
        position: Position,
        version: i64,
    ) -> Result<Option<SignatureHelp>, String> {
        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;
            let bridge_payload = serde_json::to_value(QueryPayload {
                uri: uri.to_string(),
                offset: lsp_position_to_bridge(position),
            })
            .map_err(|err| err.to_string())?;

            let response = self
                .bridge
                .request(MessageType::SignatureHelp, version, bridge_payload)
                .await
                .map_err(|err| err.to_string())?;

            if response.msg_type == MessageType::SignatureHelp {
                let payload = response
                    .signature_help_payload()
                    .map_err(|err| err.to_string())?;
                if let Some(payload) = payload {
                    return Ok(Some(bridge_signature_help_to_lsp(payload)));
                }
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        Ok(signature_help_from_text(&text, position))
    }

    async fn document_links_for_uri(
        &self,
        uri: &Url,
        version: i64,
    ) -> Result<Vec<DocumentLink>, String> {
        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;

            let payload = serde_json::to_value(DocumentUriPayload {
                uri: uri.to_string(),
            })
            .map_err(|err| err.to_string())?;

            let response = self
                .bridge
                .request(MessageType::DocumentLinks, version, payload)
                .await
                .map_err(|err| err.to_string())?;

            if response.msg_type == MessageType::DocumentLinks {
                let links = response
                    .document_links_payload()
                    .map_err(|err| err.to_string())?;
                return Ok(links
                    .into_iter()
                    .filter_map(bridge_document_link_to_lsp)
                    .collect());
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        Ok(document_links_from_text(uri, &text))
    }

    async fn code_lenses_for_uri(&self, uri: &Url) -> Vec<CodeLens> {
        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        let session_running = self.is_session_running().await;
        code_lenses_for_document(uri, &text, session_running)
    }

    async fn inlay_hints_for_uri_range(&self, uri: &Url, range: LspRange) -> Vec<InlayHint> {
        let version = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;

            let payload = serde_json::to_value(DocumentUriPayload {
                uri: uri.to_string(),
            });
            if let Ok(payload) = payload
                && let Ok(response) = self
                    .bridge
                    .request(MessageType::InlayHints, version, payload)
                    .await
                && response.msg_type == MessageType::InlayHints
                && let Ok(hints) = response.inlay_hints_payload()
            {
                let mut mapped = hints
                    .into_iter()
                    .filter_map(bridge_inlay_hint_to_lsp)
                    .filter(|hint| lsp_position_in_range(hint.position, range))
                    .collect::<Vec<_>>();
                mapped.sort_by(|a, b| {
                    a.position
                        .line
                        .cmp(&b.position.line)
                        .then_with(|| a.position.character.cmp(&b.position.character))
                        .then_with(|| {
                            inlay_hint_label_text(&a.label).cmp(&inlay_hint_label_text(&b.label))
                        })
                });
                mapped.dedup_by(|a, b| {
                    a.position == b.position
                        && inlay_hint_label_text(&a.label) == inlay_hint_label_text(&b.label)
                });
                return mapped;
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        inlay_hints_from_text(&text, range)
    }

    async fn document_formatting_edits_for_uri(
        &self,
        uri: &Url,
        options: FormattingOptions,
    ) -> Vec<TextEdit> {
        let version = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;
            let payload = serde_json::to_value(BridgeDocumentFormattingPayload {
                uri: uri.to_string(),
                options: bridge_formatting_options_from_lsp(&options),
            });
            if let Ok(payload) = payload
                && let Ok(response) = self
                    .bridge
                    .request(MessageType::DocumentFormatting, version, payload)
                    .await
                && response.msg_type == MessageType::DocumentFormatting
                && let Ok(edits) = response.text_edits_payload()
            {
                return bridge_text_edits_for_uri_to_lsp(uri, edits);
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        document_formatting_edits(&text, &options)
    }

    async fn range_formatting_edits_for_uri(
        &self,
        uri: &Url,
        range: LspRange,
        options: FormattingOptions,
    ) -> Vec<TextEdit> {
        let version = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;
            let payload = serde_json::to_value(BridgeRangeFormattingPayload {
                uri: uri.to_string(),
                range: bridge_range_from_lsp(range),
                options: bridge_formatting_options_from_lsp(&options),
            });
            if let Ok(payload) = payload
                && let Ok(response) = self
                    .bridge
                    .request(MessageType::RangeFormatting, version, payload)
                    .await
                && response.msg_type == MessageType::RangeFormatting
                && let Ok(edits) = response.text_edits_payload()
            {
                return bridge_text_edits_for_uri_to_lsp(uri, edits);
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        range_formatting_edits(&text, range, &options)
    }

    async fn on_type_formatting_edits_for_uri(
        &self,
        uri: &Url,
        position: Position,
        ch: String,
        options: FormattingOptions,
    ) -> Vec<TextEdit> {
        let version = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        if self.is_session_running().await {
            self.flush_pushes(Some(vec![uri.clone()])).await;
            let payload = serde_json::to_value(BridgeOnTypeFormattingPayload {
                uri: uri.to_string(),
                offset: lsp_position_to_bridge(position),
                ch,
                options: bridge_formatting_options_from_lsp(&options),
            });
            if let Ok(payload) = payload
                && let Ok(response) = self
                    .bridge
                    .request(MessageType::OnTypeFormatting, version, payload)
                    .await
                && response.msg_type == MessageType::OnTypeFormatting
                && let Ok(edits) = response.text_edits_payload()
            {
                return bridge_text_edits_for_uri_to_lsp(uri, edits);
            }
        }

        let text = self
            .document_snapshot(uri)
            .await
            .map(|snapshot| snapshot.text)
            .unwrap_or_default();
        on_type_formatting_edits(&text, position, &options)
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
                type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
                implementation_provider: Some(ImplementationProviderCapability::Simple(true)),
                declaration_provider: Some(DeclarationCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string(), "_".to_string()]),
                    ..CompletionOptions::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    work_done_progress_options: Default::default(),
                }),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        work_done_progress_options: Default::default(),
                        resolve_provider: Some(false),
                    },
                ))),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                    first_trigger_character: "\n".to_string(),
                    more_trigger_character: Some(vec![":".to_string(), ";".to_string()]),
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
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
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
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

    async fn goto_type_definition(
        &self,
        params: GotoTypeDefinitionParams,
    ) -> JsonRpcResult<Option<GotoTypeDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self
            .type_definition_locations(&uri, position, version)
            .await
        {
            Ok(locations) if locations.is_empty() => Ok(None),
            Ok(locations) => Ok(Some(GotoTypeDefinitionResponse::Array(locations))),
            Err(err) => {
                self.log_error(format!("failed to request type definition: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn goto_implementation(
        &self,
        params: GotoImplementationParams,
    ) -> JsonRpcResult<Option<GotoImplementationResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.implementation_locations(&uri, position, version).await {
            Ok(locations) if locations.is_empty() => Ok(None),
            Ok(locations) => Ok(Some(GotoImplementationResponse::Array(locations))),
            Err(err) => {
                self.log_error(format!("failed to request implementation: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn goto_declaration(
        &self,
        params: GotoDeclarationParams,
    ) -> JsonRpcResult<Option<GotoDeclarationResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.definition_locations(&uri, position, version).await {
            Ok(locations) if locations.is_empty() => Ok(None),
            Ok(locations) => Ok(Some(GotoDeclarationResponse::Array(locations))),
            Err(err) => {
                self.log_error(format!("failed to request declaration: {err}"))
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

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> JsonRpcResult<Option<Vec<DocumentHighlight>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.document_highlights(&uri, position, version).await {
            Ok(highlights) => Ok(Some(highlights)),
            Err(err) => {
                self.log_error(format!("failed to request document highlights: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> JsonRpcResult<Option<Vec<SelectionRange>>> {
        let uri = params.text_document.uri;
        match self.selection_ranges(&uri, params.positions).await {
            Ok(ranges) => Ok(Some(ranges)),
            Err(err) => {
                self.log_error(format!("failed to compute selection ranges: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn folding_range(
        &self,
        params: FoldingRangeParams,
    ) -> JsonRpcResult<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri;
        match self.folding_ranges(&uri).await {
            Ok(ranges) => Ok(Some(ranges)),
            Err(err) => {
                self.log_error(format!("failed to compute folding ranges: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> JsonRpcResult<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self
            .signature_help_for_position(&uri, position, version)
            .await
        {
            Ok(help) => Ok(help),
            Err(err) => {
                self.log_error(format!("failed to compute signature help: {err}"))
                    .await;
                Ok(None)
            }
        }
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> JsonRpcResult<Option<Vec<DocumentLink>>> {
        let uri = params.text_document.uri;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);
        match self.document_links_for_uri(&uri, version).await {
            Ok(links) => Ok(Some(links)),
            Err(err) => {
                self.log_error(format!("failed to compute document links: {err}"))
                    .await;
                Ok(Some(Vec::new()))
            }
        }
    }

    async fn code_lens(&self, params: CodeLensParams) -> JsonRpcResult<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri;
        Ok(Some(self.code_lenses_for_uri(&uri).await))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> JsonRpcResult<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        Ok(Some(
            self.inlay_hints_for_uri_range(&uri, params.range).await,
        ))
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        Ok(Some(
            self.document_formatting_edits_for_uri(&uri, params.options)
                .await,
        ))
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        Ok(Some(
            self.range_formatting_edits_for_uri(&uri, params.range, params.options)
                .await,
        ))
    }

    async fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> JsonRpcResult<Option<Vec<TextEdit>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let ch = params.ch;
        Ok(Some(
            self.on_type_formatting_edits_for_uri(&uri, position, ch, params.options)
                .await,
        ))
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

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonRpcResult<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;
        let version = self
            .document_snapshot(&uri)
            .await
            .map(|snapshot| snapshot.version)
            .unwrap_or(1);

        match self.rename_target_range(&uri, position, version).await {
            Ok(Some(range)) => Ok(Some(PrepareRenameResponse::Range(range))),
            Ok(None) => Ok(None),
            Err(err) => {
                self.log_error(format!("failed to prepare rename: {err}"))
                    .await;
                Ok(None)
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

fn lsp_position_in_range(position: Position, range: LspRange) -> bool {
    if position.line < range.start.line || position.line > range.end.line {
        return false;
    }
    if position.line == range.start.line && position.character < range.start.character {
        return false;
    }
    if position.line == range.end.line && position.character > range.end.character {
        return false;
    }
    true
}

fn selection_range_for_position(text: &str, position: Position) -> SelectionRange {
    let mut selection = SelectionRange {
        range: full_document_range(text),
        parent: None,
    };

    let lines = text.lines().collect::<Vec<_>>();
    let line_index = usize::try_from(position.line).unwrap_or(usize::MAX);
    if line_index >= lines.len() {
        return selection;
    }

    let line = lines[line_index];
    let line_len = u32::try_from(line.chars().count()).unwrap_or(u32::MAX);
    let line_range = LspRange {
        start: Position {
            line: position.line,
            character: 0,
        },
        end: Position {
            line: position.line,
            character: line_len,
        },
    };
    selection = SelectionRange {
        range: line_range,
        parent: Some(Box::new(selection)),
    };

    if let Some((start, end)) = identifier_bounds_in_line(line, position.character) {
        selection = SelectionRange {
            range: LspRange {
                start: Position {
                    line: position.line,
                    character: start,
                },
                end: Position {
                    line: position.line,
                    character: end,
                },
            },
            parent: Some(Box::new(selection)),
        };
    }

    selection
}

fn full_document_range(text: &str) -> LspRange {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return LspRange {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        };
    }

    let last_line = u32::try_from(lines.len().saturating_sub(1)).unwrap_or(u32::MAX);
    let last_len = u32::try_from(lines.last().map(|line| line.chars().count()).unwrap_or(0))
        .unwrap_or(u32::MAX);
    LspRange {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: last_line,
            character: last_len,
        },
    }
}

fn identifier_bounds_in_line(line: &str, character: u32) -> Option<(u32, u32)> {
    let chars = line.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }

    let mut index = usize::try_from(character).unwrap_or(usize::MAX);
    if index >= chars.len() {
        index = chars.len().saturating_sub(1);
    }
    if !is_isabelle_identifier_char(chars[index])
        && index > 0
        && is_isabelle_identifier_char(chars[index - 1])
    {
        index = index.saturating_sub(1);
    }
    if !is_isabelle_identifier_char(chars[index]) {
        return None;
    }

    let mut start = index;
    while start > 0 && is_isabelle_identifier_char(chars[start - 1]) {
        start = start.saturating_sub(1);
    }

    let mut end = index;
    while end + 1 < chars.len() && is_isabelle_identifier_char(chars[end + 1]) {
        end += 1;
    }

    let start_u32 = u32::try_from(start).unwrap_or(0);
    let end_exclusive = u32::try_from(end.saturating_add(1)).unwrap_or(u32::MAX);
    Some((start_u32, end_exclusive))
}

fn is_isabelle_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '\'' | '.' | '-')
}

fn line_identifier_tokens(line: &str) -> Vec<String> {
    line_identifier_spans(line)
        .into_iter()
        .map(|(token, _, _)| token)
        .collect()
}

fn first_non_empty_line_range(text: &str) -> LspRange {
    for (line_index, line_text) in text.lines().enumerate() {
        if line_text.trim().is_empty() {
            continue;
        }
        let line = u32::try_from(line_index).unwrap_or(u32::MAX);
        let end_character = u32::try_from(line_text.chars().count()).unwrap_or(u32::MAX);
        return LspRange {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: end_character,
            },
        };
    }

    LspRange {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    }
}

fn code_lenses_for_document(uri: &Url, text: &str, session_running: bool) -> Vec<CodeLens> {
    let range = first_non_empty_line_range(text);
    let session_command = if session_running {
        Command {
            title: "Stop Isabelle Session".to_string(),
            command: COMMAND_STOP_SESSION.to_string(),
            arguments: None,
        }
    } else {
        Command {
            title: "Start Isabelle Session".to_string(),
            command: COMMAND_START_SESSION.to_string(),
            arguments: None,
        }
    };

    vec![
        CodeLens {
            range,
            command: Some(Command {
                title: "Run Isabelle Check".to_string(),
                command: COMMAND_RUN_CHECK.to_string(),
                arguments: Some(vec![serde_json::json!({ "uri": uri.as_str() })]),
            }),
            data: None,
        },
        CodeLens {
            range,
            command: Some(session_command),
            data: None,
        },
    ]
}

fn inlay_hints_from_text(text: &str, range: LspRange) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    for (line_index, line_text) in text.lines().enumerate() {
        let line = u32::try_from(line_index).unwrap_or(u32::MAX);
        if line < range.start.line || line > range.end.line {
            continue;
        }

        let spans = line_identifier_spans(line_text);
        if let Some((keyword, _, end)) = spans.first() {
            match keyword.as_str() {
                "lemma" | "theorem" | "corollary" | "proposition" => push_inlay_hint(
                    &mut hints,
                    Position {
                        line,
                        character: *end,
                    },
                    " : proposition".to_string(),
                    Some(InlayHintKind::TYPE),
                    &range,
                ),
                "definition" | "abbreviation" | "fun" | "function" | "primrec" => push_inlay_hint(
                    &mut hints,
                    Position {
                        line,
                        character: *end,
                    },
                    " : definition".to_string(),
                    Some(InlayHintKind::TYPE),
                    &range,
                ),
                _ => {}
            }
        }

        for pair in spans.windows(2) {
            let (prefix, _, _) = &pair[0];
            let (_, start, _) = &pair[1];
            let label = match prefix.as_str() {
                "by" | "apply" => Some("method: "),
                "using" => Some("facts: "),
                "unfolding" => Some("defs: "),
                _ => None,
            };
            if let Some(label) = label {
                push_inlay_hint(
                    &mut hints,
                    Position {
                        line,
                        character: *start,
                    },
                    label.to_string(),
                    Some(InlayHintKind::PARAMETER),
                    &range,
                );
            }
        }
    }

    hints.sort_by(|a, b| {
        a.position
            .line
            .cmp(&b.position.line)
            .then_with(|| a.position.character.cmp(&b.position.character))
            .then_with(|| inlay_hint_label_text(&a.label).cmp(&inlay_hint_label_text(&b.label)))
    });
    hints.dedup_by(|a, b| {
        a.position == b.position
            && inlay_hint_label_text(&a.label) == inlay_hint_label_text(&b.label)
    });
    hints
}

fn push_inlay_hint(
    hints: &mut Vec<InlayHint>,
    position: Position,
    label: String,
    kind: Option<InlayHintKind>,
    visible_range: &LspRange,
) {
    if !lsp_position_in_range(position, *visible_range) {
        return;
    }

    hints.push(InlayHint {
        position,
        label: InlayHintLabel::String(label),
        kind,
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: Some(true),
        data: None,
    });
}

fn inlay_hint_label_text(label: &InlayHintLabel) -> String {
    match label {
        InlayHintLabel::String(value) => value.clone(),
        InlayHintLabel::LabelParts(parts) => parts.iter().map(|part| part.value.clone()).collect(),
    }
}

fn document_lines(text: &str) -> Vec<&str> {
    let mut lines = text.split('\n').collect::<Vec<_>>();
    if text.ends_with('\n') {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push("");
    }
    lines
}

fn format_isabelle_text(text: &str, options: &FormattingOptions) -> String {
    let indent_width = usize::try_from(options.tab_size.max(1)).unwrap_or(1);
    let indent_unit = if options.insert_spaces {
        " ".repeat(indent_width)
    } else {
        "\t".to_string()
    };
    let trim_trailing_whitespace = options.trim_trailing_whitespace.unwrap_or(false);

    let mut indent_level = 0usize;
    let mut formatted_lines = Vec::new();
    for line in document_lines(text) {
        let without_trailing = if trim_trailing_whitespace {
            line.trim_end()
        } else {
            line
        };
        let trimmed = without_trailing.trim_start();
        let tokens = line_identifier_tokens(trimmed);

        if tokens
            .first()
            .is_some_and(|token| matches!(token.as_str(), "end" | "qed" | "oops"))
        {
            indent_level = indent_level.saturating_sub(1);
        }

        if trimmed.is_empty() {
            formatted_lines.push(String::new());
        } else {
            let mut normalized = String::new();
            for _ in 0..indent_level {
                normalized.push_str(&indent_unit);
            }
            normalized.push_str(trimmed);
            formatted_lines.push(normalized);
        }

        if tokens
            .iter()
            .any(|token| matches!(token.as_str(), "begin" | "proof"))
        {
            indent_level = indent_level.saturating_add(1);
        }
    }

    let mut formatted = formatted_lines.join("\n");
    if options.trim_final_newlines.unwrap_or(false) {
        while formatted.ends_with('\n') {
            formatted.pop();
        }
    }

    let insert_final_newline = options.insert_final_newline.unwrap_or(text.ends_with('\n'));
    if insert_final_newline && !formatted.ends_with('\n') {
        formatted.push('\n');
    }

    formatted
}

fn document_formatting_edits(text: &str, options: &FormattingOptions) -> Vec<TextEdit> {
    let formatted = format_isabelle_text(text, options);
    if formatted == text {
        return Vec::new();
    }

    vec![TextEdit {
        range: full_document_range(text),
        new_text: formatted,
    }]
}

fn range_formatting_edits(
    text: &str,
    range: LspRange,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    let formatted = format_isabelle_text(text, options);
    if formatted == text {
        return Vec::new();
    }

    let original_lines = document_lines(text);
    let formatted_lines = document_lines(&formatted);
    let start = usize::try_from(range.start.line).unwrap_or(usize::MAX);
    if start >= original_lines.len() || start >= formatted_lines.len() {
        return Vec::new();
    }

    let max_end = original_lines
        .len()
        .min(formatted_lines.len())
        .saturating_sub(1);
    let mut end = usize::try_from(range.end.line).unwrap_or(max_end);
    if end > max_end {
        end = max_end;
    }
    if end < start {
        return Vec::new();
    }

    let original_chunk = original_lines[start..=end].join("\n");
    let formatted_chunk = formatted_lines[start..=end].join("\n");
    if original_chunk == formatted_chunk {
        return Vec::new();
    }

    let start_line = u32::try_from(start).unwrap_or(0);
    let end_line = u32::try_from(end).unwrap_or(u32::MAX);
    let end_character = u32::try_from(original_lines[end].chars().count()).unwrap_or(u32::MAX);
    vec![TextEdit {
        range: LspRange {
            start: Position {
                line: start_line,
                character: 0,
            },
            end: Position {
                line: end_line,
                character: end_character,
            },
        },
        new_text: formatted_chunk,
    }]
}

fn on_type_formatting_edits(
    text: &str,
    position: Position,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    let formatted = format_isabelle_text(text, options);
    if formatted == text {
        return Vec::new();
    }

    let original_lines = document_lines(text);
    let formatted_lines = document_lines(&formatted);
    let line_index = usize::try_from(position.line).unwrap_or(usize::MAX);
    if line_index >= original_lines.len() || line_index >= formatted_lines.len() {
        return Vec::new();
    }
    if original_lines[line_index] == formatted_lines[line_index] {
        return Vec::new();
    }

    let line = u32::try_from(line_index).unwrap_or(u32::MAX);
    let end_character =
        u32::try_from(original_lines[line_index].chars().count()).unwrap_or(u32::MAX);
    vec![TextEdit {
        range: LspRange {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: end_character,
            },
        },
        new_text: formatted_lines[line_index].to_string(),
    }]
}

fn folding_ranges_from_text(text: &str) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    let mut stack = Vec::<u32>::new();

    for (line_index, line_text) in text.lines().enumerate() {
        let line = u32::try_from(line_index).unwrap_or(u32::MAX);
        for token in line_identifier_tokens(line_text) {
            match token.as_str() {
                "begin" | "proof" => stack.push(line),
                "end" | "qed" | "oops" => {
                    if let Some(start) = stack.pop()
                        && line > start
                    {
                        ranges.push(FoldingRange {
                            start_line: start,
                            start_character: None,
                            end_line: line,
                            end_character: None,
                            kind: None,
                            collapsed_text: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    ranges.sort_by(|a, b| {
        a.start_line
            .cmp(&b.start_line)
            .then_with(|| a.end_line.cmp(&b.end_line))
    });
    ranges.dedup_by(|a, b| a.start_line == b.start_line && a.end_line == b.end_line);
    ranges
}

fn signature_help_from_text(text: &str, position: Position) -> Option<SignatureHelp> {
    let lines = text.lines().collect::<Vec<_>>();
    let line_index = usize::try_from(position.line).ok()?;
    let line = lines.get(line_index)?;

    signature_help_from_call(line, position.character)
        .or_else(|| signature_help_from_keyword(line, position.character))
}

fn signature_help_from_call(line: &str, cursor_char: u32) -> Option<SignatureHelp> {
    let chars = line.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return None;
    }

    let cursor = usize::try_from(cursor_char)
        .unwrap_or(usize::MAX)
        .min(chars.len());
    let mut open_stack = Vec::<usize>::new();
    for (index, ch) in chars.iter().take(cursor).enumerate() {
        match *ch {
            '(' => open_stack.push(index),
            ')' => {
                open_stack.pop();
            }
            _ => {}
        }
    }
    let open = *open_stack.last()?;

    let mut name_end = open;
    while name_end > 0 && chars[name_end - 1].is_whitespace() {
        name_end -= 1;
    }
    let mut name_start = name_end;
    while name_start > 0 && is_isabelle_identifier_char(chars[name_start - 1]) {
        name_start -= 1;
    }
    if name_start == name_end {
        return None;
    }
    let callee = chars[name_start..name_end].iter().collect::<String>();
    if callee.is_empty() {
        return None;
    }

    let mut nested_depth = 0_i32;
    let mut active_param = 0_usize;
    for ch in chars.iter().take(cursor).skip(open + 1) {
        match *ch {
            '(' => nested_depth = nested_depth.saturating_add(1),
            ')' => {
                if nested_depth > 0 {
                    nested_depth -= 1;
                }
            }
            ',' if nested_depth == 0 => active_param = active_param.saturating_add(1),
            _ => {}
        }
    }

    Some(signature_help_for_callee(&callee, active_param))
}

fn signature_help_from_keyword(line: &str, cursor_char: u32) -> Option<SignatureHelp> {
    let spans = line_identifier_spans(line);
    if spans.is_empty() {
        return None;
    }

    let cursor = cursor_char;
    let token = spans
        .iter()
        .find(|(_, start, end)| cursor >= *start && cursor <= *end)
        .map(|(token, _, _)| token.as_str())
        .or_else(|| {
            spans
                .iter()
                .rev()
                .find(|(_, _, end)| *end <= cursor)
                .map(|(token, _, _)| token.as_str())
        })?;

    let prefix = line
        .chars()
        .take(usize::try_from(cursor).unwrap_or(0))
        .collect::<String>();

    match token {
        "lemma" | "theorem" | "corollary" | "proposition" => {
            let active = if prefix.contains(':') { 1 } else { 0 };
            Some(signature_help_from_template(
                token,
                vec!["name".to_string(), "statement".to_string()],
                active,
                Some(format!("{token} <name>: <statement>")),
            ))
        }
        "definition" | "abbreviation" | "fun" | "function" | "primrec" => {
            let active = if prefix.contains("where") { 1 } else { 0 };
            Some(signature_help_from_template(
                token,
                vec!["name".to_string(), "equation".to_string()],
                active,
                Some(format!("{token} <name> where <equation>")),
            ))
        }
        "have" | "show" | "thus" | "hence" => Some(signature_help_from_template(
            token,
            vec!["statement".to_string()],
            0,
            Some(format!("{token} <statement>")),
        )),
        _ => None,
    }
}

fn signature_help_for_callee(callee: &str, active_param: usize) -> SignatureHelp {
    match callee {
        "lemma" | "theorem" | "corollary" | "proposition" => signature_help_from_template(
            callee,
            vec!["name".to_string(), "statement".to_string()],
            active_param,
            Some(format!("{callee} <name>: <statement>")),
        ),
        "definition" | "abbreviation" | "fun" | "function" | "primrec" => {
            signature_help_from_template(
                callee,
                vec!["name".to_string(), "equation".to_string()],
                active_param,
                Some(format!("{callee} <name> where <equation>")),
            )
        }
        _ => {
            let count = active_param.saturating_add(1).clamp(1, 6);
            let params = (1..=count)
                .map(|index| format!("arg{index}"))
                .collect::<Vec<_>>();
            signature_help_from_template(callee, params, active_param, None)
        }
    }
}

fn signature_help_from_template(
    callee: &str,
    params: Vec<String>,
    active_param: usize,
    documentation: Option<String>,
) -> SignatureHelp {
    let joined = params.join(", ");
    let label = if joined.is_empty() {
        callee.to_string()
    } else {
        format!("{callee}({joined})")
    };

    let bounded_active = if params.is_empty() {
        0
    } else {
        active_param.min(params.len().saturating_sub(1))
    };
    let parameters = params
        .into_iter()
        .map(|param| ParameterInformation {
            label: ParameterLabel::Simple(param),
            documentation: None,
        })
        .collect::<Vec<_>>();

    SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: documentation.map(tower_lsp::lsp_types::Documentation::String),
            parameters: Some(parameters),
            active_parameter: Some(u32::try_from(bounded_active).unwrap_or(0)),
        }],
        active_signature: Some(0),
        active_parameter: Some(u32::try_from(bounded_active).unwrap_or(0)),
    }
}

fn document_links_from_text(uri: &Url, text: &str) -> Vec<DocumentLink> {
    let base_dir = uri
        .to_file_path()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()));
    let mut links = Vec::new();

    for (line_index, line) in text.lines().enumerate() {
        let line_number = u32::try_from(line_index).unwrap_or(u32::MAX);

        for (start, end, target) in http_links_in_line(line) {
            let Ok(target_url) = Url::parse(&target) else {
                continue;
            };
            links.push(DocumentLink {
                range: LspRange {
                    start: Position {
                        line: line_number,
                        character: start,
                    },
                    end: Position {
                        line: line_number,
                        character: end,
                    },
                },
                target: Some(target_url),
                tooltip: Some("Open external link".to_string()),
                data: None,
            });
        }

        if let Some(base) = &base_dir {
            for (name, start, end) in import_tokens_in_line(line) {
                if let Some(target_url) = resolve_import_target(base, &name) {
                    links.push(DocumentLink {
                        range: LspRange {
                            start: Position {
                                line: line_number,
                                character: start,
                            },
                            end: Position {
                                line: line_number,
                                character: end,
                            },
                        },
                        target: Some(target_url),
                        tooltip: Some(format!("Open imported theory `{name}`")),
                        data: None,
                    });
                }
            }
        }
    }

    links.sort_by(|a, b| {
        let a_target = a
            .target
            .as_ref()
            .map(Url::as_str)
            .unwrap_or_default()
            .to_string();
        let b_target = b
            .target
            .as_ref()
            .map(Url::as_str)
            .unwrap_or_default()
            .to_string();
        a.range
            .start
            .line
            .cmp(&b.range.start.line)
            .then_with(|| a.range.start.character.cmp(&b.range.start.character))
            .then_with(|| a.range.end.line.cmp(&b.range.end.line))
            .then_with(|| a.range.end.character.cmp(&b.range.end.character))
            .then_with(|| a_target.cmp(&b_target))
    });
    links.dedup_by(|a, b| {
        a.range == b.range
            && a.target.as_ref().map(Url::as_str) == b.target.as_ref().map(Url::as_str)
    });
    links
}

fn http_links_in_line(line: &str) -> Vec<(u32, u32, String)> {
    let mut links = Vec::new();
    let bytes = line.as_bytes();
    let mut start_byte = 0usize;

    while start_byte < bytes.len() {
        let candidate = &line[start_byte..];
        let relative = if let Some(index) = candidate.find("https://") {
            Some(index)
        } else {
            candidate.find("http://")
        };
        let Some(relative_index) = relative else {
            break;
        };
        let absolute_start = start_byte.saturating_add(relative_index);
        let remainder = &line[absolute_start..];
        let end_offset = remainder
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ')' | ']' | '>'))
            .unwrap_or(remainder.len());
        let absolute_end = absolute_start.saturating_add(end_offset);

        let target = line[absolute_start..absolute_end].to_string();
        let start = u32::try_from(line[..absolute_start].chars().count()).unwrap_or(0);
        let end = u32::try_from(line[..absolute_end].chars().count()).unwrap_or(start);
        if end > start {
            links.push((start, end, target));
        }
        start_byte = absolute_end.saturating_add(1);
    }

    links
}

fn import_tokens_in_line(line: &str) -> Vec<(String, u32, u32)> {
    let spans = line_identifier_spans(line);
    let Some(imports_index) = spans.iter().position(|(token, _, _)| token == "imports") else {
        return Vec::new();
    };

    spans
        .into_iter()
        .skip(imports_index + 1)
        .take_while(|(token, _, _)| token != "begin")
        .collect()
}

fn resolve_import_target(base_dir: &std::path::Path, theory_name: &str) -> Option<Url> {
    let direct = base_dir.join(format!("{theory_name}.thy"));
    if direct.is_file() {
        return Url::from_file_path(direct).ok();
    }

    let nested = base_dir.join(format!("{}.thy", theory_name.replace('.', "/")));
    if nested.is_file() {
        return Url::from_file_path(nested).ok();
    }
    None
}

fn line_identifier_spans(line: &str) -> Vec<(String, u32, u32)> {
    let chars = line.chars().collect::<Vec<_>>();
    if chars.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if !is_isabelle_identifier_char(chars[index]) {
            index += 1;
            continue;
        }

        let start = index;
        while index + 1 < chars.len() && is_isabelle_identifier_char(chars[index + 1]) {
            index += 1;
        }
        let end = index.saturating_add(1);
        let token = chars[start..end].iter().collect::<String>();
        spans.push((
            token,
            u32::try_from(start).unwrap_or(0),
            u32::try_from(end).unwrap_or(u32::MAX),
        ));
        index = end;
    }

    spans
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

fn bridge_range_from_lsp(range: LspRange) -> bridge::protocol::Range {
    bridge::protocol::Range {
        start: lsp_position_to_bridge(range.start),
        end: lsp_position_to_bridge(range.end),
    }
}

fn bridge_formatting_options_from_lsp(
    options: &FormattingOptions,
) -> BridgeFormattingOptionsPayload {
    BridgeFormattingOptionsPayload {
        tab_size: options.tab_size,
        insert_spaces: options.insert_spaces,
        trim_trailing_whitespace: options.trim_trailing_whitespace,
        insert_final_newline: options.insert_final_newline,
        trim_final_newlines: options.trim_final_newlines,
    }
}

fn bridge_text_edits_for_uri_to_lsp(uri: &Url, edits: Vec<TextEditPayload>) -> Vec<TextEdit> {
    let uri_text = uri.as_str();
    edits
        .into_iter()
        .filter(|edit| edit.uri == uri_text)
        .map(|edit| TextEdit {
            range: bridge_range_to_lsp(edit.range),
            new_text: edit.new_text,
        })
        .collect()
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

fn bridge_document_link_to_lsp(payload: BridgeDocumentLinkPayload) -> Option<DocumentLink> {
    let target = payload.target.and_then(|value| Url::parse(&value).ok());
    target.as_ref()?;

    Some(DocumentLink {
        range: bridge_range_to_lsp(payload.range),
        target,
        tooltip: payload.tooltip,
        data: None,
    })
}

fn bridge_inlay_hint_to_lsp(payload: BridgeInlayHintPayload) -> Option<InlayHint> {
    if payload.label.is_empty() {
        return None;
    }

    let line = u32::try_from(payload.position.line.saturating_sub(1)).ok()?;
    let character = u32::try_from(payload.position.col.saturating_sub(1)).ok()?;
    let kind = match payload.kind.as_deref() {
        Some("type") => Some(InlayHintKind::TYPE),
        Some("parameter") => Some(InlayHintKind::PARAMETER),
        _ => None,
    };

    Some(InlayHint {
        position: Position { line, character },
        label: InlayHintLabel::String(payload.label),
        kind,
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: Some(true),
        data: None,
    })
}

fn bridge_signature_help_to_lsp(payload: BridgeSignatureHelpPayload) -> SignatureHelp {
    let parameter_count = payload.parameters.len();
    let active_parameter = usize::try_from(payload.active_parameter.max(0)).unwrap_or(0);
    let bounded_active = if parameter_count == 0 {
        0
    } else {
        active_parameter.min(parameter_count.saturating_sub(1))
    };

    let parameters = payload
        .parameters
        .into_iter()
        .map(|parameter| ParameterInformation {
            label: ParameterLabel::Simple(parameter),
            documentation: None,
        })
        .collect::<Vec<_>>();

    SignatureHelp {
        signatures: vec![SignatureInformation {
            label: payload.label,
            documentation: payload
                .documentation
                .map(tower_lsp::lsp_types::Documentation::String),
            parameters: Some(parameters),
            active_parameter: Some(u32::try_from(bounded_active).unwrap_or(0)),
        }],
        active_signature: Some(0),
        active_parameter: Some(u32::try_from(bounded_active).unwrap_or(0)),
    }
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
        Diagnostic as BridgeDiagnostic, Message, Position as BridgePosition, Range as BridgeRange,
        Severity, diagnostics_message_from_request, parse_message, to_ndjson,
    };
    use serde_json::json;
    use std::collections::HashMap;
    #[cfg(unix)]
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    #[cfg(unix)]
    use tokio::net::UnixListener;
    #[cfg(unix)]
    use tokio::time::sleep;
    use tower_lsp::lsp_types::DiagnosticSeverity;

    fn formatting_options() -> FormattingOptions {
        FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            properties: HashMap::new(),
            trim_trailing_whitespace: Some(true),
            insert_final_newline: Some(true),
            trim_final_newlines: Some(true),
        }
    }

    #[test]
    fn converts_bridge_diagnostic_to_lsp() {
        let diagnostic = BridgeDiagnostic {
            uri: "file:///tmp/example.thy".to_string(),
            range: BridgeRange {
                start: BridgePosition { line: 1, col: 2 },
                end: BridgePosition { line: 3, col: 4 },
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

    #[test]
    fn lsp_position_in_range_accepts_boundaries() {
        let range = LspRange {
            start: Position {
                line: 2,
                character: 4,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };
        assert!(super::lsp_position_in_range(
            Position {
                line: 2,
                character: 4,
            },
            range
        ));
        assert!(super::lsp_position_in_range(
            Position {
                line: 2,
                character: 10,
            },
            range
        ));
    }

    #[test]
    fn lsp_position_in_range_rejects_outside() {
        let range = LspRange {
            start: Position {
                line: 2,
                character: 4,
            },
            end: Position {
                line: 2,
                character: 10,
            },
        };
        assert!(!super::lsp_position_in_range(
            Position {
                line: 2,
                character: 3,
            },
            range
        ));
        assert!(!super::lsp_position_in_range(
            Position {
                line: 2,
                character: 11,
            },
            range
        ));
    }

    #[test]
    fn selection_range_for_position_builds_nested_ranges() {
        let text = "theory Demo imports Main begin\nlemma foo_bar: True by simp\nend\n";
        let selection = super::selection_range_for_position(
            text,
            Position {
                line: 1,
                character: 8,
            },
        );
        assert_eq!(selection.range.start.line, 1);
        assert_eq!(selection.range.start.character, 6);
        assert_eq!(selection.range.end.character, 13);
        assert!(selection.parent.is_some());
        assert!(
            selection
                .parent
                .as_ref()
                .and_then(|line| line.parent.as_ref())
                .is_some()
        );
    }

    #[test]
    fn folding_ranges_from_text_detects_theory_and_proof_blocks() {
        let text =
            "theory Demo imports Main begin\nlemma foo: True\nproof\nshow True by simp\nqed\nend\n";
        let ranges = super::folding_ranges_from_text(text);
        assert!(
            ranges
                .iter()
                .any(|range| range.start_line == 0 && range.end_line == 5)
        );
        assert!(
            ranges
                .iter()
                .any(|range| range.start_line == 2 && range.end_line == 4)
        );
    }

    #[test]
    fn signature_help_from_text_tracks_active_parameter() {
        let text = "theory Demo imports Main begin\nfoo(arg1, arg2)\nend\n";
        let help = super::signature_help_from_text(
            text,
            Position {
                line: 1,
                character: 10,
            },
        )
        .expect("signature help should be available");
        assert_eq!(help.active_parameter, Some(1));
        assert_eq!(help.signatures.len(), 1);
        assert!(help.signatures[0].label.starts_with("foo("));
    }

    #[test]
    fn bridge_signature_help_to_lsp_maps_payload() {
        let payload = BridgeSignatureHelpPayload {
            label: "lemma(name, statement)".to_string(),
            parameters: vec!["name".to_string(), "statement".to_string()],
            active_parameter: 1,
            documentation: Some("lemma <name>: <statement>".to_string()),
        };
        let help = super::bridge_signature_help_to_lsp(payload);
        assert_eq!(help.active_parameter, Some(1));
        assert_eq!(help.signatures.len(), 1);
        assert_eq!(help.signatures[0].label, "lemma(name, statement)");
    }

    #[test]
    fn bridge_document_link_to_lsp_maps_target_and_range() {
        let link = super::bridge_document_link_to_lsp(BridgeDocumentLinkPayload {
            range: bridge::protocol::Range {
                start: BridgePosition { line: 1, col: 6 },
                end: BridgePosition { line: 1, col: 18 },
            },
            target: Some("https://isabelle.in.tum.de".to_string()),
            tooltip: Some("Open external link".to_string()),
        })
        .expect("document link should map");

        assert_eq!(link.range.start.line, 0);
        assert_eq!(link.range.start.character, 5);
        assert_eq!(
            link.target.as_ref().map(Url::as_str),
            Some("https://isabelle.in.tum.de/")
        );
    }

    #[test]
    fn bridge_inlay_hint_to_lsp_maps_position_label_and_kind() {
        let hint = super::bridge_inlay_hint_to_lsp(BridgeInlayHintPayload {
            position: BridgePosition { line: 2, col: 6 },
            label: "method: ".to_string(),
            kind: Some("parameter".to_string()),
        })
        .expect("inlay hint should map");

        assert_eq!(hint.position.line, 1);
        assert_eq!(hint.position.character, 5);
        assert!(matches!(hint.kind, Some(InlayHintKind::PARAMETER)));
    }

    #[test]
    fn bridge_formatting_options_from_lsp_maps_core_fields() {
        let lsp = formatting_options();
        let bridge = super::bridge_formatting_options_from_lsp(&lsp);
        assert_eq!(bridge.tab_size, 2);
        assert!(bridge.insert_spaces);
        assert_eq!(bridge.trim_trailing_whitespace, Some(true));
        assert_eq!(bridge.insert_final_newline, Some(true));
        assert_eq!(bridge.trim_final_newlines, Some(true));
    }

    #[test]
    fn bridge_text_edits_for_uri_to_lsp_filters_other_uris() {
        let uri = Url::parse("file:///tmp/Example.thy").expect("file url");
        let edits = vec![
            TextEditPayload {
                uri: "file:///tmp/Example.thy".to_string(),
                range: bridge::protocol::Range {
                    start: BridgePosition { line: 1, col: 1 },
                    end: BridgePosition { line: 1, col: 2 },
                },
                new_text: "x".to_string(),
            },
            TextEditPayload {
                uri: "file:///tmp/Other.thy".to_string(),
                range: bridge::protocol::Range {
                    start: BridgePosition { line: 1, col: 1 },
                    end: BridgePosition { line: 1, col: 2 },
                },
                new_text: "y".to_string(),
            },
        ];
        let mapped = super::bridge_text_edits_for_uri_to_lsp(&uri, edits);
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].new_text, "x");
    }

    #[test]
    fn document_links_from_text_extracts_http_target() {
        let uri = Url::parse("file:///tmp/Example.thy").expect("file url");
        let links =
            super::document_links_from_text(&uri, "text \"https://isabelle.in.tum.de/index.html\"");
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].target.as_ref().map(Url::as_str),
            Some("https://isabelle.in.tum.de/index.html")
        );
    }

    #[test]
    fn code_lenses_include_run_check_and_session_stop_when_running() {
        let uri = Url::parse("file:///tmp/Example.thy").expect("file url");
        let text = "theory Example imports Main begin\nlemma demo: True by simp\nend\n";

        let lenses = super::code_lenses_for_document(&uri, text, true);
        assert_eq!(lenses.len(), 2);

        let run_check = lenses
            .iter()
            .filter_map(|lens| lens.command.as_ref())
            .find(|command| command.command == COMMAND_RUN_CHECK)
            .expect("run check command");
        assert_eq!(
            run_check.arguments.as_ref().and_then(|args| args.first()),
            Some(&json!({ "uri": uri.as_str() }))
        );

        let stop = lenses
            .iter()
            .filter_map(|lens| lens.command.as_ref())
            .find(|command| command.command == COMMAND_STOP_SESSION)
            .expect("stop session command");
        assert_eq!(stop.title, "Stop Isabelle Session");
    }

    #[test]
    fn code_lenses_include_start_when_session_not_running() {
        let uri = Url::parse("file:///tmp/Example.thy").expect("file url");
        let lenses =
            super::code_lenses_for_document(&uri, "theory Example imports Main begin\n", false);

        assert!(
            lenses
                .iter()
                .filter_map(|lens| lens.command.as_ref())
                .any(|command| command.command == COMMAND_START_SESSION)
        );
    }

    #[test]
    fn inlay_hints_emit_type_and_method_hints() {
        let text = "lemma plus_comm: \"a + b = b + a\"\n  by simp\n";
        let hints = super::inlay_hints_from_text(text, super::full_document_range(text));
        let labels = hints
            .iter()
            .map(|hint| super::inlay_hint_label_text(&hint.label))
            .collect::<Vec<_>>();

        assert!(labels.iter().any(|label| label == " : proposition"));
        assert!(labels.iter().any(|label| label == "method: "));
    }

    #[test]
    fn inlay_hints_respect_requested_range() {
        let text = "lemma plus_comm: \"a + b = b + a\"\n  by simp\n";
        let hints = super::inlay_hints_from_text(
            text,
            LspRange {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 20,
                },
            },
        );
        let labels = hints
            .iter()
            .map(|hint| super::inlay_hint_label_text(&hint.label))
            .collect::<Vec<_>>();

        assert!(labels.iter().any(|label| label == "method: "));
        assert!(!labels.iter().any(|label| label == " : proposition"));
    }

    #[test]
    fn format_isabelle_text_indents_theory_and_proof_blocks() {
        let text =
            "theory Demo imports Main begin\nlemma t: True\nproof\nshow True by simp\nqed\nend\n";
        let formatted = super::format_isabelle_text(text, &formatting_options());
        let expected = "theory Demo imports Main begin\n  lemma t: True\n  proof\n    show True by simp\n  qed\nend\n";
        assert_eq!(formatted, expected);
    }

    #[test]
    fn document_formatting_edits_rewrite_entire_document() {
        let text =
            "theory Demo imports Main begin\nlemma t: True\nproof\nshow True by simp\nqed\nend\n";
        let edits = super::document_formatting_edits(text, &formatting_options());
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range, super::full_document_range(text));
        assert!(edits[0].new_text.contains("\n  lemma t: True\n"));
    }

    #[test]
    fn range_formatting_edits_limit_to_requested_lines() {
        let text =
            "theory Demo imports Main begin\nlemma t: True\nproof\nshow True by simp\nqed\nend\n";
        let edits = super::range_formatting_edits(
            text,
            LspRange {
                start: Position {
                    line: 1,
                    character: 0,
                },
                end: Position {
                    line: 1,
                    character: 12,
                },
            },
            &formatting_options(),
        );
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.end.line, 1);
        assert_eq!(edits[0].new_text, "  lemma t: True");
    }

    #[test]
    fn on_type_formatting_edits_update_current_line_only() {
        let text = "theory Demo imports Main begin\nlemma t: True\nend\n";
        let edits = super::on_type_formatting_edits(
            text,
            Position {
                line: 1,
                character: 5,
            },
            &formatting_options(),
        );
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start.line, 1);
        assert_eq!(edits[0].range.end.line, 1);
        assert_eq!(edits[0].new_text, "  lemma t: True");
    }

    #[cfg(unix)]
    #[test]
    fn document_links_from_text_resolves_imported_theory_file() {
        let temp = tempdir().expect("tempdir");
        let imported = temp.path().join("Demo.thy");
        std::fs::write(&imported, "theory Demo imports Main begin\nend\n")
            .expect("write imported theory");

        let current = temp.path().join("Main.thy");
        let uri = Url::from_file_path(&current).expect("main uri");
        let expected = Url::from_file_path(&imported).expect("import uri");

        let links = super::document_links_from_text(&uri, "theory Main imports Demo begin\nend\n");
        assert!(
            links
                .iter()
                .any(|link| link.target.as_ref().map(Url::as_str) == Some(expected.as_str()))
        );
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
