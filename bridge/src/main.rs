use bridge::process::{ProcessError, ProcessManager, run_mock_adapter, run_real_adapter};
use bridge::protocol::{MarkupPayload, Message, MessageType, parse_message};
use bridge::queue::{DebounceQueue, QueueError};
use clap::Parser;
#[cfg(unix)]
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::time::Instant;
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::time::{Duration, MissedTickBehavior};
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Parser, Debug, Clone)]
#[command(name = "bridge")]
#[command(about = "NDJSON bridge between an editor client and Isabelle adapter backend")]
struct Args {
    #[arg(long)]
    socket: Option<PathBuf>,

    #[arg(long)]
    tcp: Option<String>,

    #[arg(long, default_value = "isabelle")]
    isabelle_path: String,

    #[arg(long, default_value = "HOL")]
    logic: String,

    #[arg(long = "session-dir")]
    session_dirs: Vec<PathBuf>,

    #[arg(long)]
    adapter_socket: Option<String>,

    #[arg(long)]
    adapter_command: Option<String>,

    #[arg(long, default_value_t = 300)]
    debounce_ms: u64,

    #[arg(long)]
    log_dir: Option<PathBuf>,

    #[arg(long)]
    debug: bool,

    #[arg(long)]
    mock: bool,

    #[arg(long, hide = true)]
    mock_adapter: bool,

    #[arg(long, hide = true)]
    real_adapter: bool,
}

#[derive(Debug, Clone)]
struct SessionConfig {
    isabelle_path: String,
    logic: String,
    session_dirs: Vec<PathBuf>,
    adapter_socket: Option<String>,
    adapter_command: Option<String>,
    debounce_ms: u64,
    mock: bool,
}

#[derive(Debug, Error)]
enum BridgeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("process error: {0}")]
    Process(#[from] ProcessError),
    #[error("queue error: {0}")]
    Queue(#[from] QueueError),
    #[error("configuration error: {0}")]
    Config(String),
}

#[tokio::main]
async fn main() -> Result<(), BridgeError> {
    let args = Args::parse();

    if args.mock_adapter {
        return run_mock_adapter().await.map_err(BridgeError::from);
    }
    if args.real_adapter {
        return run_real_adapter(args.isabelle_path, args.logic, args.session_dirs)
            .await
            .map_err(BridgeError::from);
    }

    let _log_guard = setup_logging(args.debug, args.log_dir.as_ref())?;
    let session = SessionConfig {
        isabelle_path: args.isabelle_path,
        logic: args.logic,
        session_dirs: args.session_dirs,
        adapter_socket: args.adapter_socket,
        adapter_command: args.adapter_command,
        debounce_ms: args.debounce_ms,
        mock: args.mock,
    };

    if args.socket.is_some() && args.tcp.is_some() {
        return Err(BridgeError::Config(
            "cannot use --socket and --tcp together".to_string(),
        ));
    }

    if let Some(socket_path) = args.socket {
        run_socket_server(&socket_path, session).await
    } else if let Some(tcp_address) = args.tcp {
        run_tcp_server(&tcp_address, session).await
    } else {
        run_stdio(session).await
    }
}

fn setup_logging(
    debug_enabled: bool,
    log_dir: Option<&PathBuf>,
) -> Result<Option<WorkerGuard>, BridgeError> {
    let filter = if debug_enabled {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    if debug_enabled {
        let dir = log_dir.cloned().unwrap_or_else(|| PathBuf::from("logs"));
        std::fs::create_dir_all(&dir)?;

        let file_appender = tracing_appender::rolling::daily(dir, "bridge.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
            .init();

        return Ok(Some(guard));
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    Ok(None)
}

#[cfg(unix)]
async fn run_socket_server(
    socket_path: &PathBuf,
    session: SessionConfig,
) -> Result<(), BridgeError> {
    if socket_path.exists() {
        let metadata = std::fs::symlink_metadata(socket_path)?;
        if metadata.file_type().is_socket() {
            std::fs::remove_file(socket_path)?;
        } else {
            return Err(BridgeError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!(
                    "refusing to remove non-socket path before bind: {}",
                    socket_path.display()
                ),
            )));
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    info!("bridge listening on unix socket {}", socket_path.display());

    loop {
        let (stream, _) = listener.accept().await?;
        let session = session.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_socket_client(stream, session).await {
                error!("socket client session failed: {err}");
            }
        });
    }
}

#[cfg(not(unix))]
async fn run_socket_server(
    socket_path: &PathBuf,
    session: SessionConfig,
) -> Result<(), BridgeError> {
    let _ = socket_path;
    let _ = session;
    Err(BridgeError::Config(
        "--socket is only supported on unix platforms; use --tcp <host:port> instead".to_string(),
    ))
}

async fn run_tcp_server(tcp_address: &str, session: SessionConfig) -> Result<(), BridgeError> {
    let listener = TcpListener::bind(tcp_address).await?;
    info!("bridge listening on tcp {}", tcp_address);

    loop {
        let (stream, _) = listener.accept().await?;
        let session = session.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_tcp_client(stream, session).await {
                error!("tcp client session failed: {err}");
            }
        });
    }
}

#[cfg(unix)]
async fn handle_socket_client(
    stream: UnixStream,
    session: SessionConfig,
) -> Result<(), BridgeError> {
    let (reader, writer) = stream.into_split();
    run_session(BufReader::new(reader), writer, session).await
}

async fn handle_tcp_client(stream: TcpStream, session: SessionConfig) -> Result<(), BridgeError> {
    let (reader, writer) = stream.into_split();
    run_session(BufReader::new(reader), writer, session).await
}

async fn run_stdio(session: SessionConfig) -> Result<(), BridgeError> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    run_session(BufReader::new(stdin), stdout, session).await
}

async fn run_session<R, W>(
    reader: R,
    mut writer: W,
    session: SessionConfig,
) -> Result<(), BridgeError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut process = ProcessManager::new(
        session.isabelle_path,
        session.logic,
        session.session_dirs,
        session.mock,
        session.adapter_socket,
        session.adapter_command,
    );
    process.start().await?;
    let mut adapter_output = process.take_output_receiver()?;

    let mut input_lines = reader.lines();
    let mut debounce = DebounceQueue::new(session.debounce_ms);

    let mut flush_tick = tokio::time::interval(Duration::from_millis(25));
    flush_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_line = input_lines.next_line() => {
                match maybe_line {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }

                        debug!("editor -> bridge: {line}");
                        match parse_message(&line) {
                            Ok(message) => {
                                if message.msg_type == MessageType::DocumentPush {
                                    debounce.enqueue(message)?;
                                } else {
                                    if let Some(uri) = pending_uri_for_message(&message)
                                        && let Some(pending) = debounce.drain_for_uri(&uri)
                                    {
                                        process.send(&pending).await?;
                                    }
                                    process.send(&message).await?;
                                }
                            }
                            Err(err) => {
                                warn!("dropping invalid editor message: {err}");
                            }
                        }
                    }
                    Ok(None) => {
                        for message in debounce.drain_all() {
                            process.send(&message).await?;
                        }
                        break;
                    }
                    Err(err) => {
                        return Err(BridgeError::Io(err));
                    }
                }
            }
            _ = flush_tick.tick() => {
                for message in debounce.drain_ready(Instant::now()) {
                    process.send(&message).await?;
                }
            }
            maybe_adapter_line = adapter_output.recv() => {
                if let Some(adapter_line) = maybe_adapter_line {
                    debug!("adapter -> bridge: {adapter_line}");
                    writer.write_all(adapter_line.as_bytes()).await?;
                    writer.write_all(b"\n").await?;
                    writer.flush().await?;
                } else {
                    warn!("adapter output channel closed");
                    break;
                }
            }
        }
    }

    process.stop().await?;
    Ok(())
}

fn pending_uri_for_message(message: &Message) -> Option<String> {
    match message.msg_type {
        MessageType::DocumentCheck => message.check_payload().ok().map(|payload| payload.uri),
        MessageType::Markup => message
            .payload_as::<MarkupPayload>()
            .ok()
            .map(|payload| payload.uri),
        MessageType::Definition
        | MessageType::References
        | MessageType::Completion
        | MessageType::SignatureHelp => message.query_payload().ok().map(|payload| payload.uri),
        MessageType::Rename => message.rename_payload().ok().map(|payload| payload.uri),
        MessageType::DocumentSymbols => message
            .document_uri_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::CodeAction => message
            .document_uri_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::SemanticTokens => message
            .document_uri_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::DocumentLinks => message
            .document_uri_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::InlayHints => message
            .document_uri_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::DocumentFormatting => message
            .document_formatting_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::RangeFormatting => message
            .range_formatting_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::OnTypeFormatting => message
            .on_type_formatting_payload()
            .ok()
            .map(|payload| payload.uri),
        MessageType::WorkspaceSymbols => None,
        MessageType::DocumentPush | MessageType::Diagnostics => None,
    }
}
