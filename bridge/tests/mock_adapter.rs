use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn start_mock_adapter() -> std::process::Child {
    let child = Command::new("cat")
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .expect("Failed to start mock adapter");
    child
}

#[test]
fn test_mock_adapter_roundtrip() {
    let mut child = start_mock_adapter();

    let stdin = child.stdin.as_mut().expect("Failed to get stdin");
    let stdout = child.stdout.as_mut().expect("Failed to get stdout");
    let mut reader = BufReader::new(stdout);

    let request = r#"{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}"#;

    stdin
        .write_all(format!("{}\n", request).as_bytes())
        .expect("Failed to write");
    stdin.flush().expect("Failed to flush");

    let mut response = String::new();
    reader
        .read_line(&mut response)
        .expect("Failed to read response");

    assert!(response.contains("document.push"));

    let _ = child.kill();
}

#[test]
fn test_mock_diagnostics_response() {
    let mut child = start_mock_adapter();

    let stdin = child.stdin.as_mut().expect("Failed to get stdin");
    let stdout = child.stdout.as_mut().expect("Failed to get stdout");
    let mut reader = BufReader::new(stdout);

    let request = r#"{"id":"msg-0002","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}"#;

    stdin
        .write_all(format!("{}\n", request).as_bytes())
        .expect("Failed to write");
    stdin.flush().expect("Failed to flush");

    std::thread::sleep(std::time::Duration::from_millis(100));

    let _ = child.kill();
}
