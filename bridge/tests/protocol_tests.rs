use bridge::protocol::*;

#[test]
fn test_document_push_parsing() {
    let json = r#"{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}"#;
    let msg = parse_message(json).unwrap();

    assert_eq!(msg.id, "msg-0001");
    assert_eq!(msg.msg_type, MessageType::DocumentPush);
    assert_eq!(msg.session, Some("s1".to_string()));
    assert_eq!(msg.version, Some(1));

    let payload: DocumentPushPayload = serde_json::from_value(msg.payload).unwrap();
    assert_eq!(payload.uri, "file:///home/user/example.thy");
    assert!(payload.text.contains("theory Example"));
}

#[test]
fn test_diagnostics_parsing() {
    let json = r#"{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":{"diagnostics":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}}"#;
    let msg = parse_message(json).unwrap();

    assert_eq!(msg.msg_type, MessageType::Diagnostics);

    let payload: DiagnosticsPayload = serde_json::from_value(msg.payload).unwrap();
    assert_eq!(payload.diagnostics.len(), 1);

    let diag = &payload.diagnostics[0];
    assert_eq!(diag.severity, DiagnosticSeverity::Error);
    assert_eq!(diag.message, "Parse error");
    assert_eq!(diag.range.start.line, 1);
}

#[test]
fn test_markup_parsing() {
    let json = r#"{"id":"msg-0002","type":"markup","session":"s1","version":1,"payload":{"uri":"file:///test.thy","offset":{"line":5,"col":10},"info":"theorem foo: ..."}}"#;
    let msg = parse_message(json).unwrap();

    assert_eq!(msg.msg_type, MessageType::Markup);

    let payload: MarkupPayload = serde_json::from_value(msg.payload).unwrap();
    assert_eq!(payload.offset.line, 5);
    assert_eq!(payload.info, "theorem foo: ...");
}

#[test]
fn test_invalid_json() {
    let json = "not valid json";
    let result = parse_message(json);
    assert!(result.is_err());
}

#[test]
fn test_missing_required_fields() {
    let json = r#"{"id":"msg-0001"}"#;
    let msg: Result<JsonMessage, _> = parse_message(json);
    assert!(msg.is_err());
}

#[test]
fn test_roundtrip_serialization() {
    let original = r#"{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///test.thy","text":"test"}}"#;
    let msg = parse_message(original).unwrap();
    let serialized = serialize_message(&msg).unwrap();
    let msg2 = parse_message(&serialized).unwrap();

    assert_eq!(msg.id, msg2.id);
    assert_eq!(msg.msg_type, msg2.msg_type);
}
