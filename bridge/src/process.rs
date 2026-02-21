use crate::protocol::JsonMessage;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("Failed to spawn process: {0}")]
    SpawnError(String),
    #[error("Process I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Process exited: {0}")]
    ProcessExited(String),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("Max retries exceeded")]
    MaxRetriesExceeded,
}

pub struct ProcessManager {
    isabelle_path: String,
    mock_mode: bool,
    max_retries: u32,
    current_retry: u32,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    running: Arc<Mutex<bool>>,
}

impl ProcessManager {
    pub fn new(isabelle_path: String, mock_mode: bool) -> Self {
        Self {
            isabelle_path,
            mock_mode,
            max_retries: 3,
            current_retry: 0,
            child: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn start(&mut self) -> Result<(), ProcessError> {
        let mut cmd = if self.mock_mode {
            let mut c = Command::new("cat");
            c.stdout(Stdio::piped()).stdin(Stdio::piped());
            c
        } else {
            let mut c = Command::new(&self.isabelle_path);
            c.arg("scala").stdout(Stdio::piped()).stdin(Stdio::piped());
            c
        };

        info!("Starting Isabelle process: {:?}", cmd);
        
        let mut child = cmd.spawn().map_err(|e| ProcessError::SpawnError(e.to_string()))?;
        
        let stdin = child.stdin.take().ok_or_else(|| ProcessError::SpawnError("Failed to take stdin".to_string()))?;
        let stdout = child.stdout.take().ok_or_else(|| ProcessError::SpawnError("Failed to take stdout".to_string()))?;

        *self.child.lock().await = Some(child);
        *self.stdin.lock().await = Some(stdin);
        *self.running.lock().await = true;
        self.current_retry = 0;

        info!("Isabelle process started successfully");
        
        let stdout_reader = BufReader::new(stdout);
        let (tx, _rx) = mpsc::channel::<String>(100);
        
        tokio::spawn(async move {
            let mut lines = stdout_reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!("Received: {}", line);
                let _ = tx.send(line).await;
            }
        });

        Ok(())
    }

    pub async fn send(&mut self, msg: &JsonMessage) -> Result<(), ProcessError> {
        let mut stdin_guard = self.stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            let line = crate::protocol::serialize_message(msg)
                .map_err(|e| ProcessError::ProtocolError(e.to_string()))?;
            debug!("Sending: {}", line.trim());
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
            Ok(())
        } else {
            Err(ProcessError::ProcessExited("Process not running".to_string()))
        }
    }

    pub async fn restart(&mut self) -> Result<(), ProcessError> {
        self.stop().await;
        
        if self.current_retry >= self.max_retries {
            error!("Max retries ({}) exceeded", self.max_retries);
            return Err(ProcessError::MaxRetriesExceeded);
        }

        let backoff = Duration::from_millis(500 * 2u64.pow(self.current_retry));
        warn!("Restarting process after {}ms (retry {}/{})", 
              backoff.as_millis(), self.current_retry + 1, self.max_retries);
        
        tokio::time::sleep(backoff).await;
        self.current_retry += 1;
        
        self.start().await
    }

    pub async fn stop(&mut self) {
        *self.running.lock().await = false;
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill().await;
        }
        *self.stdin.lock().await = None;
        info!("Process stopped");
    }

    pub fn is_running(&self) -> bool {
        false
    }
}

pub struct ProcessHandle {
    pub stdin: mpsc::Sender<String>,
    pub stdout: mpsc::Receiver<String>,
}

pub async fn spawn_mock_adapter() -> Result<ProcessHandle, ProcessError> {
    let mut cmd = Command::new("cat");
    cmd.stdout(Stdio::piped()).stdin(Stdio::piped());
    
    let mut child = cmd.spawn().map_err(|e| ProcessError::SpawnError(e.to_string()))?;
    
    let stdin = child.stdin.take().ok_or_else(|| ProcessError::SpawnError("No stdin".to_string()))?;
    let stdout = child.stdout.take().ok_or_else(|| ProcessError::SpawnError("No stdout".to_string()))?;
    
    let (tx, rx) = mpsc::channel::<String>(100);
    let (stx, mut srx) = mpsc::channel::<String>(100);
    
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = tx_clone.send(line).await;
        }
    });
    
    tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(line) = srx.recv().await {
            let _ = stdin.write_all(format!("{}\n", line).as_bytes()).await;
            let _ = stdin.flush().await;
        }
    });
    
    Ok(ProcessHandle {
        stdin: stx,
        stdout: rx,
    })
}
