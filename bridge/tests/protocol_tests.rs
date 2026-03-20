use bridge::protocol::{
    DIAGNOSTICS_EXAMPLE, DOCUMENT_PUSH_EXAMPLE, Diagnostic, DocumentPushPayload, MessageType,
    Severity, parse_message, to_ndjson,
};

#[test]
fn document_push_parses_exact_example() {
    let message = parse_message(DOCUMENT_PUSH_EXAMPLE).expect("document.push example should parse");
    assert_eq!(message.id, "msg-0001");
    assert_eq!(message.msg_type, MessageType::DocumentPush);
    assert_eq!(message.session, "s1");
    assert_eq!(message.version, 1);

    let payload: DocumentPushPayload = message.push_payload().expect("payload should decode");
    assert_eq!(payload.uri, "file:///home/user/example.thy");
    assert!(payload.text.contains("theory Example imports Main begin"));
}

#[test]
fn diagnostics_parses_exact_example() {
    let message = parse_message(DIAGNOSTICS_EXAMPLE).expect("diagnostics example should parse");
    assert_eq!(message.msg_type, MessageType::Diagnostics);

    let payload: Vec<Diagnostic> = message
        .diagnostics_payload()
        .expect("diagnostics payload should decode");
    assert_eq!(payload.len(), 1);
    assert_eq!(payload[0].severity, Severity::Error);
    assert_eq!(payload[0].message, "Parse error");
    assert_eq!(payload[0].range.start.line, 1);
    assert_eq!(payload[0].range.end.col, 7);
}

#[test]
fn message_round_trip_ndjson() {
    let message = parse_message(DOCUMENT_PUSH_EXAMPLE).expect("message should parse");
    let ndjson = to_ndjson(&message).expect("message should serialize to ndjson");

    assert!(ndjson.ends_with('\n'));
    let reparsed = parse_message(ndjson.trim_end()).expect("ndjson should parse back");
    assert_eq!(reparsed, message);
}

#[test]
fn missing_fields_fail_with_clear_error() {
    let invalid = r#"{"id":"msg-0001","type":"document.push","payload":{"uri":"file:///x.thy"}}"#;
    let error = parse_message(invalid).expect_err("missing fields must fail");
    let text = error.to_string();
    assert!(text.contains("invalid message JSON"));
    assert!(text.contains("missing field"));
}

#[test]
fn invalid_type_fails() {
    let invalid =
        r#"{"id":"msg-0001","type":"document.unknown","session":"s1","version":1,"payload":{}}"#;
    let error = parse_message(invalid).expect_err("unknown message types must fail");
    assert!(error.to_string().contains("invalid message JSON"));
}

#[test]
fn rename_result_payload_decodes_warning() {
    let raw = r#"{"id":"msg-0002","type":"rename","session":"s1","version":1,"payload":{"edits":[],"warning":"rename aborted: ambiguous symbol"}}"#;
    let message = parse_message(raw).expect("rename payload should parse");
    assert_eq!(message.msg_type, MessageType::Rename);

    let payload = message
        .rename_result_payload()
        .expect("rename result payload should decode");
    assert!(payload.edits.is_empty());
    assert_eq!(
        payload.warning.as_deref(),
        Some("rename aborted: ambiguous symbol")
    );
}
