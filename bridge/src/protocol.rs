use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DOCUMENT_PUSH_EXAMPLE: &str = r#"{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}"#;
pub const DIAGNOSTICS_EXAMPLE: &str = r#"{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":1},"end":{"line":1,"col":7}},"severity":"error","message":"Parse error"}]}"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
    #[serde(rename = "document.push")]
    DocumentPush,
    #[serde(rename = "document.check")]
    DocumentCheck,
    #[serde(rename = "definition")]
    Definition,
    #[serde(rename = "references")]
    References,
    #[serde(rename = "completion")]
    Completion,
    #[serde(rename = "document.symbols")]
    DocumentSymbols,
    #[serde(rename = "rename")]
    Rename,
    #[serde(rename = "code_action")]
    CodeAction,
    #[serde(rename = "semantic_tokens")]
    SemanticTokens,
    #[serde(rename = "workspace.symbols")]
    WorkspaceSymbols,
    #[serde(rename = "signature_help")]
    SignatureHelp,
    #[serde(rename = "document_links")]
    DocumentLinks,
    #[serde(rename = "inlay_hints")]
    InlayHints,
    #[serde(rename = "document_formatting")]
    DocumentFormatting,
    #[serde(rename = "range_formatting")]
    RangeFormatting,
    #[serde(rename = "on_type_formatting")]
    OnTypeFormatting,
    #[serde(rename = "diagnostics")]
    Diagnostics,
    #[serde(rename = "markup")]
    Markup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub session: String,
    pub version: i64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentPushPayload {
    pub uri: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentCheckPayload {
    pub uri: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MarkupPayload {
    pub uri: String,
    pub offset: Position,
    pub info: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueryPayload {
    pub uri: String,
    pub offset: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenamePayload {
    pub uri: String,
    pub offset: Position,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentUriPayload {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct FormattingOptionsPayload {
    pub tab_size: u32,
    pub insert_spaces: bool,
    #[serde(default)]
    pub trim_trailing_whitespace: Option<bool>,
    #[serde(default)]
    pub insert_final_newline: Option<bool>,
    #[serde(default)]
    pub trim_final_newlines: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentFormattingPayload {
    pub uri: String,
    pub options: FormattingOptionsPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RangeFormattingPayload {
    pub uri: String,
    pub range: Range,
    pub options: FormattingOptionsPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OnTypeFormattingPayload {
    pub uri: String,
    pub offset: Position,
    pub ch: String,
    pub options: FormattingOptionsPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSymbolQueryPayload {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LocationPayload {
    pub uri: String,
    pub range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompletionItemPayload {
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SymbolPayload {
    pub uri: String,
    pub name: String,
    pub kind: String,
    pub range: Range,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextEditPayload {
    pub uri: String,
    pub range: Range,
    pub new_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RenameResultPayload {
    pub edits: Vec<TextEditPayload>,
    #[serde(default)]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CodeActionPayload {
    pub title: String,
    pub kind: String,
    pub edits: Vec<TextEditPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SemanticTokenPayload {
    pub uri: String,
    pub line: i64,
    pub col: i64,
    pub length: i64,
    pub token_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignatureHelpPayload {
    pub label: String,
    pub parameters: Vec<String>,
    pub active_parameter: i64,
    #[serde(default)]
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentLinkPayload {
    pub range: Range,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InlayHintPayload {
    pub position: Position,
    pub label: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub uri: String,
    pub range: Range,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: i64,
    pub col: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid message JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("payload decode error for {msg_type:?}: {source}")]
    InvalidPayload {
        msg_type: MessageType,
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_message(line: &str) -> Result<Message, ProtocolError> {
    serde_json::from_str(line).map_err(ProtocolError::from)
}

pub fn to_ndjson(message: &Message) -> Result<String, ProtocolError> {
    let mut serialized = serde_json::to_string(message)?;
    serialized.push('\n');
    Ok(serialized)
}

impl Message {
    pub fn payload_as<T>(&self) -> Result<T, ProtocolError>
    where
        T: for<'de> Deserialize<'de>,
    {
        serde_json::from_value(self.payload.clone()).map_err(|source| {
            ProtocolError::InvalidPayload {
                msg_type: self.msg_type,
                source,
            }
        })
    }

    pub fn diagnostics_payload(&self) -> Result<Vec<Diagnostic>, ProtocolError> {
        self.payload_as()
    }

    pub fn push_payload(&self) -> Result<DocumentPushPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn check_payload(&self) -> Result<DocumentCheckPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn query_payload(&self) -> Result<QueryPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn document_uri_payload(&self) -> Result<DocumentUriPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn document_formatting_payload(&self) -> Result<DocumentFormattingPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn range_formatting_payload(&self) -> Result<RangeFormattingPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn on_type_formatting_payload(&self) -> Result<OnTypeFormattingPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn rename_payload(&self) -> Result<RenamePayload, ProtocolError> {
        self.payload_as()
    }

    pub fn location_payload(&self) -> Result<Vec<LocationPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn completion_payload(&self) -> Result<Vec<CompletionItemPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn symbols_payload(&self) -> Result<Vec<SymbolPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn text_edits_payload(&self) -> Result<Vec<TextEditPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn rename_result_payload(&self) -> Result<RenameResultPayload, ProtocolError> {
        self.payload_as()
    }

    pub fn code_actions_payload(&self) -> Result<Vec<CodeActionPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn semantic_tokens_payload(&self) -> Result<Vec<SemanticTokenPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn signature_help_payload(&self) -> Result<Option<SignatureHelpPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn document_links_payload(&self) -> Result<Vec<DocumentLinkPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn inlay_hints_payload(&self) -> Result<Vec<InlayHintPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn workspace_symbol_query_payload(
        &self,
    ) -> Result<WorkspaceSymbolQueryPayload, ProtocolError> {
        self.payload_as()
    }
}

pub fn diagnostics_message_from_request(
    request: &Message,
    uri: &str,
    severity: Severity,
    message: &str,
) -> Result<Message, ProtocolError> {
    let diagnostics = vec![Diagnostic {
        uri: uri.to_string(),
        range: Range {
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 7 },
        },
        severity,
        message: message.to_string(),
    }];

    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::Diagnostics,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(diagnostics)?,
    })
}

pub fn markup_message_from_request(
    request: &Message,
    uri: &str,
    offset: Position,
    info: &str,
) -> Result<Message, ProtocolError> {
    let payload = MarkupPayload {
        uri: uri.to_string(),
        offset,
        info: info.to_string(),
    };

    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::Markup,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(payload)?,
    })
}

pub fn location_message_from_request(
    request: &Message,
    msg_type: MessageType,
    locations: Vec<LocationPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(locations)?,
    })
}

pub fn completion_message_from_request(
    request: &Message,
    items: Vec<CompletionItemPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::Completion,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(items)?,
    })
}

pub fn symbols_message_from_request(
    request: &Message,
    symbols: Vec<SymbolPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::DocumentSymbols,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(symbols)?,
    })
}

pub fn text_edits_message_from_request(
    request: &Message,
    msg_type: MessageType,
    edits: Vec<TextEditPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(edits)?,
    })
}

pub fn rename_message_from_request(
    request: &Message,
    edits: Vec<TextEditPayload>,
    warning: Option<String>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::Rename,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(RenameResultPayload { edits, warning })?,
    })
}

pub fn code_actions_message_from_request(
    request: &Message,
    actions: Vec<CodeActionPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::CodeAction,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(actions)?,
    })
}

pub fn semantic_tokens_message_from_request(
    request: &Message,
    tokens: Vec<SemanticTokenPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::SemanticTokens,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(tokens)?,
    })
}

pub fn workspace_symbols_message_from_request(
    request: &Message,
    symbols: Vec<SymbolPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::WorkspaceSymbols,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(symbols)?,
    })
}

pub fn signature_help_message_from_request(
    request: &Message,
    signature_help: Option<SignatureHelpPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::SignatureHelp,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(signature_help)?,
    })
}

pub fn document_links_message_from_request(
    request: &Message,
    links: Vec<DocumentLinkPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::DocumentLinks,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(links)?,
    })
}

pub fn inlay_hints_message_from_request(
    request: &Message,
    hints: Vec<InlayHintPayload>,
) -> Result<Message, ProtocolError> {
    Ok(Message {
        id: request.id.clone(),
        msg_type: MessageType::InlayHints,
        session: request.session.clone(),
        version: request.version,
        payload: serde_json::to_value(hints)?,
    })
}
