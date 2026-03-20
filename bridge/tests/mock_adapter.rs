#![cfg(unix)]

use bridge::protocol::{
    DIAGNOSTICS_EXAMPLE, DOCUMENT_PUSH_EXAMPLE, MessageType, Severity,
    diagnostics_message_from_request, parse_message, to_ndjson,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixStream};
use tokio::process::Command;
use tokio::task::JoinHandle;
use tokio::time::timeout;

fn bridge_binary_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_bridge") {
        return PathBuf::from(path);
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("bridge")
}

async fn wait_for_socket(path: &Path) {
    for _ in 0..80 {
        if path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("socket was not created in time: {}", path.display());
}

async fn send_document_push_and_read_diagnostics(socket_path: &Path) -> bridge::protocol::Message {
    let stream = UnixStream::connect(socket_path)
        .await
        .expect("client should connect to bridge socket");
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    writer
        .write_all(DOCUMENT_PUSH_EXAMPLE.as_bytes())
        .await
        .expect("request should be written");
    writer
        .write_all(b"\n")
        .await
        .expect("newline should be written");
    writer.flush().await.expect("request should flush");

    let mut line = String::new();
    timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .expect("timed out waiting for diagnostics")
        .expect("should read a diagnostics line");

    parse_message(line.trim_end()).expect("response should parse")
}

async fn start_tcp_mock_adapter() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock adapter tcp bind must succeed");
    let address = listener
        .local_addr()
        .expect("mock adapter address must be available")
        .to_string();

    let task = tokio::spawn(async move {
        let (stream, _) = listener
            .accept()
            .await
            .expect("bridge should connect to mock adapter");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            let request = parse_message(&line).expect("adapter should receive valid ndjson");
            if request.msg_type == MessageType::Diagnostics {
                continue;
            }

            let uri = match request.msg_type {
                MessageType::DocumentPush => {
                    request
                        .push_payload()
                        .expect("document.push payload must parse")
                        .uri
                }
                MessageType::DocumentCheck => {
                    request
                        .check_payload()
                        .expect("document.check payload must parse")
                        .uri
                }
                MessageType::Markup
                | MessageType::Definition
                | MessageType::References
                | MessageType::Completion
                | MessageType::DocumentSymbols => continue,
                MessageType::Diagnostics => continue,
            };

            let response =
                diagnostics_message_from_request(&request, &uri, Severity::Error, "Parse error")
                    .expect("mock diagnostics response should build");
            let ndjson = to_ndjson(&response).expect("mock diagnostics response should serialize");
            writer
                .write_all(ndjson.as_bytes())
                .await
                .expect("mock diagnostics should write");
            writer.flush().await.expect("mock diagnostics should flush");
        }
    });

    (address, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_mock_mode_roundtrip_document_push_to_diagnostics() {
    let temp = TempDir::new().expect("temp dir should be created");
    let socket_path = temp.path().join("isabelle.sock");

    let mut bridge = Command::new(bridge_binary_path())
        .arg("--mock")
        .arg("--socket")
        .arg(&socket_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("bridge should start");

    wait_for_socket(&socket_path).await;

    let response = send_document_push_and_read_diagnostics(&socket_path).await;
    assert_eq!(response.msg_type, MessageType::Diagnostics);

    let expected = parse_message(DIAGNOSTICS_EXAMPLE).expect("diagnostics example should parse");
    assert_eq!(response.id, expected.id);
    assert_eq!(response.session, expected.session);
    assert_eq!(response.version, expected.version);

    let actual_payload = response
        .diagnostics_payload()
        .expect("response diagnostics payload should parse");
    let expected_payload = expected
        .diagnostics_payload()
        .expect("expected diagnostics payload should parse");
    assert_eq!(actual_payload, expected_payload);

    bridge.kill().await.expect("bridge should be terminated");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_routes_to_external_adapter_socket() {
    let temp = TempDir::new().expect("temp dir should be created");
    let socket_path = temp.path().join("isabelle.sock");

    let (adapter_address, adapter_task) = start_tcp_mock_adapter().await;

    let mut bridge = Command::new(bridge_binary_path())
        .arg("--socket")
        .arg(&socket_path)
        .arg("--adapter-socket")
        .arg(&adapter_address)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("bridge should start");

    wait_for_socket(&socket_path).await;

    let response = send_document_push_and_read_diagnostics(&socket_path).await;
    assert_eq!(response.msg_type, MessageType::Diagnostics);

    let expected = parse_message(DIAGNOSTICS_EXAMPLE).expect("diagnostics example should parse");
    let actual_payload = response
        .diagnostics_payload()
        .expect("response diagnostics payload should parse");
    let expected_payload = expected
        .diagnostics_payload()
        .expect("expected diagnostics payload should parse");
    assert_eq!(actual_payload, expected_payload);

    bridge.kill().await.expect("bridge should be terminated");
    timeout(Duration::from_secs(1), adapter_task)
        .await
        .expect("mock adapter task should finish")
        .expect("mock adapter task should succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_routes_to_adapter_command_process() {
    let temp = TempDir::new().expect("temp dir should be created");
    let socket_path = temp.path().join("isabelle.sock");
    let bridge_path = bridge_binary_path();
    let adapter_command = format!("\"{}\" --mock-adapter", bridge_path.display());
    let mut bridge = Command::new(bridge_binary_path())
        .arg("--socket")
        .arg(&socket_path)
        .arg("--adapter-command")
        .arg(&adapter_command)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("bridge should start");

    wait_for_socket(&socket_path).await;

    let response = send_document_push_and_read_diagnostics(&socket_path).await;
    assert_eq!(response.msg_type, MessageType::Diagnostics);
    let payload = response
        .diagnostics_payload()
        .expect("diagnostics payload should parse");
    assert_eq!(payload.len(), 1);
    assert_eq!(payload[0].message, "Parse error");

    bridge.kill().await.expect("bridge should be terminated");
}
