use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    #[serde(rename = "document.push")]
    DocumentPush,
    #[serde(rename = "document.check")]
    DocumentCheck,
    #[serde(rename = "diagnostics")]
    Diagnostics,
    #[serde(rename = "markup")]
    Markup,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub session: Option<String>,
    pub version: Option<i64>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPushPayload {
    pub uri: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCheckPayload {
    pub uri: String,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsPayload {
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub uri: String,
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkupPayload {
    pub uri: String,
    pub offset: Position,
    pub info: String,
}

pub fn parse_message(line: &str) -> Result<JsonMessage, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn serialize_message(msg: &JsonMessage) -> Result<String, serde_json::Error> {
    serde_json::to_string(msg).map(|s| s + "\n")
}

pub fn create_document_push(uri: &str, text: &str, session: &str, version: i64) -> JsonMessage {
    JsonMessage {
        id: format!("msg-{:04}", rand_id()),
        msg_type: MessageType::DocumentPush,
        session: Some(session.to_string()),
        version: Some(version),
        payload: serde_json::to_value(DocumentPushPayload {
            uri: uri.to_string(),
            text: text.to_string(),
        })
        .unwrap(),
    }
}

pub fn create_diagnostic(
    uri: &str,
    line: u32,
    col: u32,
    severity: DiagnosticSeverity,
    message: &str,
) -> JsonMessage {
    JsonMessage {
        id: format!("msg-{:04}", rand_id()),
        msg_type: MessageType::Diagnostics,
        session: Some("s1".to_string()),
        version: Some(1),
        payload: serde_json::to_value(DiagnosticsPayload {
            diagnostics: vec![Diagnostic {
                uri: uri.to_string(),
                range: Range {
                    start: Position { line, col },
                    end: Position { line, col: col + 5 },
                },
                severity,
                message: message.to_string(),
            }],
        })
        .unwrap(),
    }
}

fn rand_id() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32
        % 10000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_push_roundtrip() {
        let json = r#"{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}"#;
        let msg = parse_message(json).unwrap();
        assert_eq!(msg.msg_type, MessageType::DocumentPush);
        assert_eq!(msg.session, Some("s1".to_string()));

        let serialized = serialize_message(&msg).unwrap();
        let msg2 = parse_message(&serialized).unwrap();
        assert_eq!(msg.id, msg2.id);
    }

    #[test]
    fn test_diagnostics_roundtrip() {
        let json = r#"{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":{"diagnostics":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}}"#;
        let msg = parse_message(json).unwrap();
        assert_eq!(msg.msg_type, MessageType::Diagnostics);

        let serialized = serialize_message(&msg).unwrap();
        assert!(serialized.contains("diagnostics"));
    }

    #[test]
    fn test_markup_parsing() {
        let json = r#"{"id":"msg-0002","type":"markup","session":"s1","version":1,"payload":{"uri":"file:///test.thy","offset":{"line":5,"col":10},"info":"theorem foo: ..."}}"#;
        let msg = parse_message(json).unwrap();

        assert_eq!(msg.msg_type, MessageType::Markup);
    }

    #[test]
    fn test_invalid_json() {
        let json = "not valid json";
        let result = parse_message(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_type() {
        let json =
            r#"{"id":"msg-0001","type":"unknown.type","session":"s1","version":1,"payload":{}}"#;
        let msg = parse_message(json).unwrap();
        assert_eq!(msg.msg_type, MessageType::Unknown);
    }

    #[test]
    fn test_create_document_push() {
        let msg = create_document_push("file:///test.thy", "theory Test begin end", "session1", 5);
        assert_eq!(msg.msg_type, MessageType::DocumentPush);
        assert_eq!(msg.session, Some("session1".to_string()));
        assert_eq!(msg.version, Some(5));
    }
}
