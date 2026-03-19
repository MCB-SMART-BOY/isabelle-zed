use bridge::protocol::{Message, MessageType, parse_message, to_ndjson};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::time::Duration;
use tracing::{debug, warn};

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
pub(crate) struct BridgeTransport {
    socket_path: PathBuf,
    session: String,
    request_timeout: Duration,
    next_id: Arc<AtomicU64>,
    connection: Arc<Mutex<Option<BridgeConnection>>>,
}

impl BridgeTransport {
    pub(crate) fn new(socket_path: PathBuf, session: String, request_timeout: Duration) -> Self {
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
