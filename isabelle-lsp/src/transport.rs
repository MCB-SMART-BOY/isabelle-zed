use bridge::protocol::{Message, MessageType, parse_message, to_ndjson};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tracing::{debug, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BridgeEndpoint {
    Unix(PathBuf),
    Tcp(String),
}

impl BridgeEndpoint {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        let value = raw.trim();
        if value.is_empty() {
            return Err("bridge endpoint is empty".to_string());
        }

        if let Some(path) = value.strip_prefix("unix:") {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                return Err("unix endpoint path is empty".to_string());
            }
            return Ok(Self::Unix(PathBuf::from(trimmed)));
        }

        if let Some(address) = value.strip_prefix("tcp:") {
            let trimmed = address.trim();
            validate_tcp_endpoint(trimmed)?;
            return Ok(Self::Tcp(trimmed.to_string()));
        }

        if value.starts_with('/') {
            return Ok(Self::Unix(PathBuf::from(value)));
        }

        if looks_like_host_port(value) {
            validate_tcp_endpoint(value)?;
            return Ok(Self::Tcp(value.to_string()));
        }

        Err(format!(
            "invalid bridge endpoint '{value}' (expected unix:/path or tcp:host:port)"
        ))
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Unix(path) => format!("unix:{}", path.display()),
            Self::Tcp(address) => format!("tcp:{address}"),
        }
    }
}

fn looks_like_host_port(value: &str) -> bool {
    let Some((host, port)) = value.rsplit_once(':') else {
        return false;
    };
    !host.trim().is_empty() && port.trim().parse::<u16>().is_ok()
}

fn validate_tcp_endpoint(value: &str) -> Result<(), String> {
    if looks_like_host_port(value) {
        Ok(())
    } else {
        Err(format!(
            "invalid tcp endpoint '{value}' (expected host:port)"
        ))
    }
}

#[derive(Debug, Error)]
pub(crate) enum BridgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bridge request timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("bridge returned EOF")]
    Eof,
    #[error("invalid response from bridge: {0}")]
    InvalidResponse(String),
}

struct BridgeConnection {
    reader: BufReader<Box<dyn AsyncRead + Unpin + Send>>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
}

impl BridgeConnection {
    async fn connect(endpoint: &BridgeEndpoint) -> Result<Self, BridgeError> {
        match endpoint {
            BridgeEndpoint::Unix(path) => {
                #[cfg(unix)]
                {
                    let stream = UnixStream::connect(path).await?;
                    let (read_half, write_half) = stream.into_split();
                    Ok(Self {
                        reader: BufReader::new(Box::new(read_half)),
                        writer: Box::new(write_half),
                    })
                }
                #[cfg(not(unix))]
                {
                    Err(BridgeError::Io(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        format!(
                            "unix socket {} is unsupported on this platform",
                            path.display()
                        ),
                    )))
                }
            }
            BridgeEndpoint::Tcp(address) => {
                let stream = TcpStream::connect(address).await?;
                let (read_half, write_half) = stream.into_split();
                Ok(Self {
                    reader: BufReader::new(Box::new(read_half)),
                    writer: Box::new(write_half),
                })
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct BridgeTransport {
    endpoint: BridgeEndpoint,
    session: String,
    request_timeout: Duration,
    next_id: Arc<AtomicU64>,
    connection: Arc<Mutex<Option<BridgeConnection>>>,
}

impl BridgeTransport {
    pub(crate) fn new(
        endpoint: BridgeEndpoint,
        session: String,
        request_timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
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

    pub(crate) async fn request(
        &self,
        msg_type: MessageType,
        version: i64,
        payload: serde_json::Value,
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

        // Retry once after reconnecting if the bridge closes the connection mid-request.
        for attempt in 0..2 {
            let mut guard = self.connection.lock().await;
            if guard.is_none() {
                *guard = Some(BridgeConnection::connect(&self.endpoint).await?);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unix_endpoint_from_prefix() {
        let endpoint = BridgeEndpoint::parse("unix:/tmp/isabelle.sock").expect("valid endpoint");
        assert_eq!(
            endpoint,
            BridgeEndpoint::Unix(PathBuf::from("/tmp/isabelle.sock"))
        );
    }

    #[test]
    fn parses_tcp_endpoint_from_prefix() {
        let endpoint = BridgeEndpoint::parse("tcp:127.0.0.1:39393").expect("valid endpoint");
        assert_eq!(endpoint, BridgeEndpoint::Tcp("127.0.0.1:39393".to_string()));
    }

    #[test]
    fn rejects_invalid_endpoint() {
        assert!(BridgeEndpoint::parse("invalid-endpoint").is_err());
    }
}
