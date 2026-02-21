use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub uri: String,
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPushPayload {
    pub uri: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkupPayload {
    pub uri: String,
    pub offset: Position,
    #[serde(default)]
    pub info: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub version: Option<i64>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl JsonMessage {
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    pub fn serialize(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self).map(|s| s + "\n")
    }

    pub fn create_document_push(uri: &str, text: &str, session: &str, version: i64) -> Self {
        Self {
            id: format!("msg-{}", rand_id()),
            msg_type: "document.push".to_string(),
            session: Some(session.to_string()),
            version: Some(version),
            payload: serde_json::to_value(DocumentPushPayload {
                uri: uri.to_string(),
                text: text.to_string(),
            })
            .unwrap(),
        }
    }

    pub fn create_markup(uri: &str, line: u32, col: u32, session: &str, version: i64) -> Self {
        Self {
            id: format!("msg-{}", rand_id()),
            msg_type: "markup".to_string(),
            session: Some(session.to_string()),
            version: Some(version),
            payload: serde_json::to_value(MarkupPayload {
                uri: uri.to_string(),
                offset: Position { line, col },
                info: String::new(),
            })
            .unwrap(),
        }
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

pub struct IsabelleBridge {
    socket_path: String,
    session: String,
    version: i64,
    stream: Option<UnixStream>,
}

impl IsabelleBridge {
    pub fn new(socket_path: &str) -> Self {
        Self {
            socket_path: socket_path.to_string(),
            session: "s1".to_string(),
            version: 1,
            stream: None,
        }
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Connecting to socket: {}", self.socket_path);
        let stream = UnixStream::connect(&self.socket_path).await?;
        self.stream = Some(stream);
        Ok(())
    }

    pub async fn push_document(&mut self, uri: &str, text: &str) -> Result<Vec<Diagnostic>, Box<dyn std::error::Error + Send + Sync>> {
        if self.stream.is_none() {
            self.connect().await?;
        }

        let msg = JsonMessage::create_document_push(uri, text, &self.session, self.version);
        self.version += 1;

        if let Some(ref mut stream) = self.stream {
            stream.write_all(msg.serialize()?.as_bytes()).await?;
            stream.flush().await?;

            let mut reader = BufReader::new(stream).lines();
            if let Ok(Some(line)) = reader.next_line().await {
                debug!("Received: {}", line);
                let response = JsonMessage::parse(&line)?;
                return Ok(Self::parse_diagnostics(&response));
            }
        }

        Ok(vec![])
    }

    pub async fn request_markup(&mut self, uri: &str, line: u32, col: u32) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if self.stream.is_none() {
            self.connect().await?;
        }

        let msg = JsonMessage::create_markup(uri, line, col, &self.session, self.version);

        if let Some(ref mut stream) = self.stream {
            stream.write_all(msg.serialize()?.as_bytes()).await?;
            stream.flush().await?;

            let mut reader = BufReader::new(stream).lines();
            if let Ok(Some(line)) = reader.next_line().await {
                debug!("Received markup: {}", line);
                let response = JsonMessage::parse(&line)?;
                return Ok(Self::parse_markup_info(&response));
            }
        }

        Ok(String::new())
    }

    fn parse_diagnostics(msg: &JsonMessage) -> Vec<Diagnostic> {
        if let Ok(payload) = serde_json::from_value::<serde_json::Value>(msg.payload.clone()) {
            if let Ok(diagnostics) = serde_json::from_value::<Vec<Diagnostic>>(payload.get("diagnostics").cloned().unwrap_or(serde_json::Value::Array(vec![]))) {
                return diagnostics;
            }
        }
        vec![]
    }

    fn parse_markup_info(msg: &JsonMessage) -> String {
        if let Ok(payload) = serde_json::from_value::<serde_json::Value>(msg.payload.clone()) {
            if let Some(info) = payload.get("info").and_then(|v| v.as_str()) {
                return info.to_string();
            }
        }
        String::new()
    }

    pub fn disconnect(&mut self) {
        self.stream = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_push_creation() {
        let msg = JsonMessage::create_document_push("file:///test.thy", "theory Test", "s1", 1);
        assert_eq!(msg.msg_type, "document.push");
        assert_eq!(msg.session, Some("s1".to_string()));
    }

    #[test]
    fn test_markup_creation() {
        let msg = JsonMessage::create_markup("file:///test.thy", 5, 10, "s1", 1);
        assert_eq!(msg.msg_type, "markup");
    }

    #[test]
    fn test_parse_diagnostics() {
        let json = r#"{"id":"msg-1","type":"diagnostics","session":"s1","version":1,"payload":{"diagnostics":[{"uri":"file:///test.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}}"#;
        let msg = JsonMessage::parse(json).unwrap();
        let diags = IsabelleBridge::parse_diagnostics(&msg);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "Parse error");
    }
}
