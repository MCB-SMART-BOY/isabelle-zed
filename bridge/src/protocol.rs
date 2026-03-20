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
pub struct DocumentUriPayload {
    pub uri: String,
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

    pub fn location_payload(&self) -> Result<Vec<LocationPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn completion_payload(&self) -> Result<Vec<CompletionItemPayload>, ProtocolError> {
        self.payload_as()
    }

    pub fn symbols_payload(&self) -> Result<Vec<SymbolPayload>, ProtocolError> {
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
