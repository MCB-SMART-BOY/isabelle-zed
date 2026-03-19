use crate::common::{bridge_binary_path, command_exists, lsp_binary_path, run_command};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::FileTypeExt;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn run_dir() -> PathBuf {
    std::env::temp_dir().join("isabelle-zed")
}

fn bridge_release_binary(repo_root: &Path) -> PathBuf {
    bridge_binary_path(repo_root, "release")
}

fn process_is_running(pid: i32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

pub(crate) fn bridge_mock_up(repo_root: &Path, socket: &Path) -> Result<()> {
    let run_dir = run_dir();
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create run dir: {}", run_dir.display()))?;
    let pid_file = run_dir.join("bridge.pid");
    let log_file = run_dir.join("bridge.log");

    let bridge_bin = bridge_release_binary(repo_root);
    if !bridge_bin.is_file() {
        println!("bridge binary not found, building release...");
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("isabelle-bridge")
                .arg("--release"),
        )?;
    }

    if pid_file.is_file() {
        let pid_raw = fs::read_to_string(&pid_file)
            .with_context(|| format!("failed to read pid file: {}", pid_file.display()))?;
        if let Ok(pid) = pid_raw.trim().parse::<i32>()
            && process_is_running(pid)
        {
            bail!(
                "bridge already running (pid={pid}). stop first: cargo run -p isabelle-zed-xtask -- bridge-mock-down"
            );
        }
        let _ = fs::remove_file(&pid_file);
    }

    if socket.exists() {
        let _ = fs::remove_file(socket);
    }

    let log_stdout = fs::File::create(&log_file)
        .with_context(|| format!("failed to create log file: {}", log_file.display()))?;
    let log_stderr = log_stdout
        .try_clone()
        .with_context(|| format!("failed to clone log file handle: {}", log_file.display()))?;
    let child = Command::new(&bridge_bin)
        .arg("--mock")
        .arg("--socket")
        .arg(socket)
        .stdout(Stdio::from(log_stdout))
        .stderr(Stdio::from(log_stderr))
        .spawn()
        .with_context(|| format!("failed to start bridge binary: {}", bridge_bin.display()))?;
    let pid = child.id();
    fs::write(&pid_file, format!("{pid}\n"))
        .with_context(|| format!("failed to write pid file: {}", pid_file.display()))?;

    wait_for_socket_ready(socket, Duration::from_secs(12))
        .context("bridge started but socket was not ready in time")?;

    println!("bridge mock started");
    println!("  pid:    {pid}");
    println!("  socket: {}", socket.display());
    println!("  log:    {}", log_file.display());
    Ok(())
}

pub(crate) fn bridge_mock_down(socket: &Path) -> Result<()> {
    let run_dir = run_dir();
    let pid_file = run_dir.join("bridge.pid");

    if !pid_file.is_file() {
        println!("no bridge pid file found at {}", pid_file.display());
        let _ = fs::remove_file(socket);
        return Ok(());
    }

    let pid_raw = fs::read_to_string(&pid_file)
        .with_context(|| format!("failed to read pid file: {}", pid_file.display()))?;
    if let Ok(pid) = pid_raw.trim().parse::<i32>()
        && process_is_running(pid)
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
        for _ in 0..30 {
            if !process_is_running(pid) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    let _ = fs::remove_file(pid_file);
    let _ = fs::remove_file(socket);
    println!("bridge mock stopped");
    Ok(())
}

pub(crate) fn mock_send(socket: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut stream = UnixStream::connect(socket)
            .with_context(|| format!("failed to connect unix socket: {}", socket.display()))?;
        let request = json!({
            "id": "msg-0001",
            "type": "document.push",
            "session": "s1",
            "version": 1,
            "payload": {
                "uri": "file:///home/user/example.thy",
                "text": "theory Example imports Main begin\\nend\\n",
            }
        });
        let line = format!("{}\n", serde_json::to_string(&request)?);
        stream.write_all(line.as_bytes())?;
        stream.flush()?;
        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response)?;
        println!("{}", response.trim_end());
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
        bail!("mock-send is only available on unix platforms");
    }
}

fn send_lsp_message(stdin: &mut impl Write, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()?;
    Ok(())
}

fn read_lsp_message(stdout: &mut ChildStdout) -> Result<Option<Value>> {
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stdout.read(&mut byte) {
            Ok(0) => {
                if header.is_empty() {
                    return Ok(None);
                }
                bail!("LSP stream closed while reading header");
            }
            Ok(_) => {
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(err) => {
                bail!("failed to read LSP header: {err}");
            }
        }
    }

    let header_text = String::from_utf8_lossy(&header);
    let mut content_length: Option<usize> = None;
    for line in header_text.split("\r\n") {
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .context("invalid content length")?,
            );
            break;
        }
        if let Some(value) = line.strip_prefix("content-length:") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .context("invalid content length")?,
            );
            break;
        }
    }
    let content_length =
        content_length.ok_or_else(|| anyhow!("missing content-length header: {header_text}"))?;

    let mut body = vec![0_u8; content_length];
    stdout
        .read_exact(&mut body)
        .context("failed to read LSP message body")?;
    let message =
        serde_json::from_slice::<Value>(&body).context("failed to decode LSP JSON message")?;
    Ok(Some(message))
}

fn spawn_lsp_reader(stdout: ChildStdout) -> mpsc::Receiver<Result<Value>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut stdout = stdout;
        loop {
            match read_lsp_message(&mut stdout) {
                Ok(Some(message)) => {
                    if tx.send(Ok(message)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = tx.send(Err(err));
                    break;
                }
            }
        }
    });
    rx
}

fn recv_lsp_until<F>(
    rx: &mpsc::Receiver<Result<Value>>,
    timeout: Duration,
    predicate: F,
) -> Result<Value>
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| anyhow!("timed out waiting for LSP message"))?;
        let message = rx
            .recv_timeout(remaining)
            .context("timed out waiting for LSP message")??;
        if predicate(&message) {
            return Ok(message);
        }
    }
}

fn wait_for_socket_ready(socket: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        #[cfg(unix)]
        let ready = socket
            .symlink_metadata()
            .map(|meta| meta.file_type().is_socket())
            .unwrap_or(false);
        #[cfg(not(unix))]
        let ready = socket.exists();

        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("socket was not created in time: {}", socket.display());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn kill_child_quietly(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn mock_lsp_e2e(repo_root: &Path) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = repo_root;
        bail!("mock-lsp-e2e is only available on unix platforms");
    }
    #[cfg(unix)]
    {
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("isabelle-bridge"),
        )?;
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("isabelle-zed-lsp"),
        )?;

        let socket = PathBuf::from("/tmp/isabelle.sock");
        if socket.exists() {
            let _ = fs::remove_file(&socket);
        }

        let bridge_bin = bridge_binary_path(repo_root, "debug");
        let mut bridge = Command::new(&bridge_bin)
            .arg("--mock")
            .arg("--socket")
            .arg(&socket)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start bridge: {}", bridge_bin.display()))?;

        let result = (|| -> Result<()> {
            wait_for_socket_ready(&socket, Duration::from_secs(8))?;

            let lsp_bin = lsp_binary_path(repo_root, "debug");
            let mut lsp = Command::new(&lsp_bin)
                .env("ISABELLE_BRIDGE_SOCKET", &socket)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| format!("failed to start lsp: {}", lsp_bin.display()))?;

            let result = (|| -> Result<()> {
                let mut stdin = lsp
                    .stdin
                    .take()
                    .ok_or_else(|| anyhow!("failed to open lsp stdin"))?;
                let stdout = lsp
                    .stdout
                    .take()
                    .ok_or_else(|| anyhow!("failed to open lsp stdout"))?;
                let rx = spawn_lsp_reader(stdout);

                send_lsp_message(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "processId": Value::Null,
                            "rootUri": Value::Null,
                            "capabilities": {}
                        }
                    }),
                )?;
                let _initialize = recv_lsp_until(&rx, Duration::from_secs(8), |msg| {
                    msg.get("id") == Some(&json!(1))
                })?;

                send_lsp_message(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "initialized",
                        "params": {}
                    }),
                )?;

                send_lsp_message(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didOpen",
                        "params": {
                            "textDocument": {
                                "uri": "file:///home/user/example.thy",
                                "languageId": "isabelle",
                                "version": 1,
                                "text": "theory Example imports Main begin\\nend\\n"
                            }
                        }
                    }),
                )?;

                let diagnostics = recv_lsp_until(&rx, Duration::from_secs(8), |msg| {
                    msg.get("method") == Some(&json!("textDocument/publishDiagnostics"))
                })?;
                let params = diagnostics
                    .get("params")
                    .ok_or_else(|| anyhow!("missing diagnostics params"))?;
                if params.get("uri") != Some(&json!("file:///home/user/example.thy")) {
                    bail!("unexpected diagnostics uri: {params}");
                }
                let diagnostics_arr = params
                    .get("diagnostics")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("missing diagnostics array"))?;
                if diagnostics_arr.len() != 1 {
                    bail!("expected one diagnostic, got {}", diagnostics_arr.len());
                }
                let message = diagnostics_arr[0]
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if message != "Parse error" {
                    bail!("unexpected diagnostic message: {message}");
                }

                send_lsp_message(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "shutdown",
                        "params": Value::Null
                    }),
                )?;
                let _shutdown = recv_lsp_until(&rx, Duration::from_secs(8), |msg| {
                    msg.get("id") == Some(&json!(2))
                })?;
                send_lsp_message(
                    &mut stdin,
                    &json!({
                        "jsonrpc": "2.0",
                        "method": "exit",
                        "params": Value::Null
                    }),
                )?;

                Ok(())
            })();

            kill_child_quietly(&mut lsp);
            result
        })();

        kill_child_quietly(&mut bridge);
        let _ = fs::remove_file(socket);
        result
    }
}

pub(crate) fn native_lsp_smoke() -> Result<()> {
    let mut proc = Command::new("isabelle")
        .arg("vscode_server")
        .arg("-n")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start `isabelle vscode_server -n`")?;

    let result = (|| -> Result<()> {
        let mut stdin = proc
            .stdin
            .take()
            .ok_or_else(|| anyhow!("failed to open server stdin"))?;
        let stdout = proc
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to open server stdout"))?;
        let rx = spawn_lsp_reader(stdout);

        send_lsp_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "processId": Value::Null,
                    "rootUri": Value::Null,
                    "capabilities": {}
                }
            }),
        )?;
        let initialize = recv_lsp_until(&rx, Duration::from_secs(40), |msg| {
            msg.get("id") == Some(&json!(1))
        })?;
        if initialize
            .get("result")
            .and_then(|r| r.get("capabilities"))
            .is_none()
        {
            bail!("initialize response missing capabilities");
        }

        send_lsp_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {}
            }),
        )?;
        send_lsp_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "shutdown",
                "params": Value::Null
            }),
        )?;
        let _shutdown = recv_lsp_until(&rx, Duration::from_secs(40), |msg| {
            msg.get("id") == Some(&json!(2))
        })?;
        send_lsp_message(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": "exit",
                "params": Value::Null
            }),
        )?;
        Ok(())
    })();

    kill_child_quietly(&mut proc);
    if result.is_ok() {
        println!("isabelle vscode_server initialize/shutdown smoke test: OK");
    }
    result
}

pub(crate) fn bridge_real_smoke(repo_root: &Path) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = repo_root;
        bail!("bridge-real-smoke is only available on unix platforms");
    }
    #[cfg(unix)]
    {
        if !command_exists("isabelle") {
            bail!("bridge-real-smoke requires `isabelle` in PATH");
        }

        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("isabelle-bridge"),
        )?;

        let bridge_path = bridge_binary_path(repo_root, "debug");
        let socket = PathBuf::from("/tmp/isabelle-real-smoke.sock");
        if socket.exists() {
            let _ = fs::remove_file(&socket);
        }

        let mut bridge = Command::new(&bridge_path)
            .arg("--socket")
            .arg(&socket)
            .arg("--logic")
            .arg("HOL")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start bridge: {}", bridge_path.display()))?;

        let result = (|| -> Result<()> {
            wait_for_socket_ready(&socket, Duration::from_secs(10))?;

            let mut stream = UnixStream::connect(&socket).with_context(|| {
                format!("failed to connect bridge socket: {}", socket.display())
            })?;
            stream.set_read_timeout(Some(Duration::from_secs(20)))?;

            let req = json!({
                "id": "msg-0001",
                "type": "document.push",
                "session": "s1",
                "version": 1,
                "payload": {
                    "uri": "file:///tmp/BridgeRealSmoke.thy",
                    "text": "theory BridgeRealSmoke imports Main begin\nlemma broken\nend\n"
                }
            });
            let line = format!("{}\n", serde_json::to_string(&req)?);
            stream.write_all(line.as_bytes())?;

            let mut reader = BufReader::new(stream);
            let mut response = String::new();
            reader.read_line(&mut response)?;
            if response.trim().is_empty() {
                bail!("no response from bridge");
            }

            let message: Value = serde_json::from_str(response.trim())
                .context("failed to decode bridge response JSON")?;
            if message.get("type") != Some(&json!("diagnostics")) {
                bail!("unexpected response type: {message}");
            }
            let payload = message
                .get("payload")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing diagnostics payload array: {message}"))?;
            if payload.is_empty() {
                bail!("expected diagnostics from malformed theory, got empty payload");
            }
            println!("bridge real adapter smoke test: OK");
            Ok(())
        })();

        kill_child_quietly(&mut bridge);
        let _ = fs::remove_file(&socket);
        result
    }
}

#[cfg(unix)]
fn shell_escape(arg: &Path) -> String {
    let raw = arg.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

pub(crate) fn spawn_e2e_ndjson(repo_root: &Path) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = repo_root;
        bail!("spawn-e2e-ndjson is only available on unix platforms");
    }
    #[cfg(unix)]
    {
        run_command(
            Command::new("cargo")
                .arg("build")
                .arg("-p")
                .arg("isabelle-bridge"),
        )?;

        let bridge_path = bridge_binary_path(repo_root, "debug");
        let socket = PathBuf::from("/tmp/isabelle.sock");
        if socket.exists() {
            let _ = fs::remove_file(&socket);
        }

        let adapter_cmd = format!("{} --mock-adapter", shell_escape(&bridge_path));
        let mut bridge = Command::new(&bridge_path)
            .arg("--socket")
            .arg(&socket)
            .arg("--adapter-command")
            .arg(&adapter_cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start bridge: {}", bridge_path.display()))?;

        let result = (|| -> Result<()> {
            wait_for_socket_ready(&socket, Duration::from_secs(8))?;

            let mut stream = UnixStream::connect(&socket).with_context(|| {
                format!("failed to connect bridge socket: {}", socket.display())
            })?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let req = json!({
                "id": "msg-0001",
                "type": "document.push",
                "session": "s1",
                "version": 1,
                "payload": {
                    "uri": "file:///home/user/example.thy",
                    "text": "theory Example imports Main begin\nend\n"
                }
            });
            let line = format!("{}\n", serde_json::to_string(&req)?);
            stream.write_all(line.as_bytes())?;

            let mut reader = BufReader::new(stream);
            let mut response = String::new();
            reader.read_line(&mut response)?;
            if response.trim().is_empty() {
                bail!("no response from bridge");
            }

            let message: Value = serde_json::from_str(response.trim())
                .context("failed to decode bridge response JSON")?;
            if message.get("type") != Some(&json!("diagnostics")) {
                bail!("unexpected response type: {message}");
            }
            if message
                .get("payload")
                .and_then(Value::as_array)
                .map(|items| items.is_empty())
                .unwrap_or(true)
            {
                bail!("empty diagnostics payload: {message}");
            }
            println!("bridge --adapter-command NDJSON e2e: OK");
            Ok(())
        })();

        kill_child_quietly(&mut bridge);
        let _ = fs::remove_file(socket);
        result
    }
}
