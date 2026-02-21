use bridge::protocol::{parse_message, MessageType};
use bridge::process::ProcessManager;
use bridge::queue::DebounceQueue;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;
use tracing::{debug, error, info};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "bridge")]
#[command(about = "NDJSON bridge between editor and Isabelle Scala adapter")]
struct Args {
    #[arg(long, default_value = "/tmp/isabelle.sock")]
    socket: PathBuf,
    
    #[arg(long, default_value = "isabelle")]
    isabelle_path: String,
    
    #[arg(long, default_value = "300")]
    debounce_ms: u64,
    
    #[arg(long)]
    log_dir: Option<PathBuf>,
    
    #[arg(long)]
    debug: bool,
    
    #[arg(long)]
    mock: bool,
}

async fn handle_socket_connection(
    socket_path: &PathBuf,
    isabelle_path: String,
    mock: bool,
    debounce: Arc<DebounceQueue>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    
    let listener = UnixListener::bind(socket_path)?;
    info!("Listening on socket: {:?}", socket_path);
    
    while let Ok((stream, _)) = listener.accept().await {
        let isabelle_path = isabelle_path.clone();
        let debounce = debounce.clone();
        
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, isabelle_path, mock, debounce).await {
                error!("Connection error: {}", e);
            }
        });
    }
    
    Ok(())
}

async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    isabelle_path: String,
    mock: bool,
    debounce: Arc<DebounceQueue>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    
    
    let (reader, _writer) = stream.split();
    let mut lines = BufReader::new(reader).lines();
    let mut process = ProcessManager::new(isabelle_path, mock);
    
    process.start().await?;
    
    loop {
        tokio::select! {
            result = lines.next_line() => {
                match result {
                    Ok(Some(line)) => {
                        debug!("Received from socket: {}", line);
                        match parse_message(&line) {
                            Ok(msg) => {
                                if msg.msg_type == MessageType::DocumentPush {
                                    debounce.enqueue(msg);
                                } else if let Err(e) = process.send(&msg).await {
                                    error!("Send error: {}", e);
                                    let _ = process.restart().await;
                                }
                            }
                            Err(e) => {
                                error!("Parse error: {}", e);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }
        }
    }
    
    Ok(())
}

async fn run_stdin_mode(
    isabelle_path: String,
    mock: bool,
    debounce: Arc<DebounceQueue>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut process = ProcessManager::new(isabelle_path, mock);
    
    process.start().await?;
    
    while let Ok(Some(line)) = lines.next_line().await {
        debug!("Received from stdin: {}", line);
        match parse_message(&line) {
            Ok(msg) => {
                if msg.msg_type == MessageType::DocumentPush {
                    debounce.enqueue(msg);
                } else if let Err(e) = process.send(&msg).await {
                    error!("Send error: {}", e);
                    let _ = process.restart().await;
                }
            }
            Err(e) => {
                error!("Parse error: {}", e);
            }
        }
    }
    
    Ok(())
}

fn setup_logging(log_dir: Option<&PathBuf>, debug: bool) {
    let filter = if debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };
    
    let subscriber = tracing_subscriber::registry().with(filter);
    
    if let Some(dir) = log_dir {
        let file_appender = RollingFileAppender::new(
            Rotation::DAILY,
            dir,
            "isabelle-bridge.log",
        );
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        
        subscriber
            .with(fmt::layer().with_writer(non_blocking))
            .init();
    } else {
        subscriber
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    
    setup_logging(args.log_dir.as_ref(), args.debug);
    
    info!("Starting Isabelle bridge");
    info!("Mock mode: {}", args.mock);
    info!("Debounce: {}ms", args.debounce_ms);
    
    let debounce = Arc::new(DebounceQueue::new(args.debounce_ms));
    
    let debounce_runner = debounce.clone();
    tokio::spawn(async move {
        debounce_runner.run().await;
    });
    
    if args.socket.to_string_lossy() != "" && args.socket.to_string_lossy() != "/tmp/isabelle.sock" {
        handle_socket_connection(&args.socket, args.isabelle_path, args.mock, debounce).await?;
    } else {
        run_stdin_mode(args.isabelle_path, args.mock, debounce).await?;
    }
    
    Ok(())
}
