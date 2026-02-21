# Isabelle Bridge

NDJSON bridge between editor (Zed) and Isabelle Scala adapter.

## Build

```bash
cargo build --release
```

## Run

### With Mock Adapter (CI/Testing)

```bash
./target/release/bridge --mock --socket /tmp/isabelle.sock
```

### With Real Isabelle

```bash
./target/release/bridge --isabelle-path /path/to/isabelle --socket /tmp/isabelle.sock
```

### Stdin Mode

```bash
echo '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///test.thy","text":"theory Test begin end"}}' | ./target/release/bridge --mock
```

## CLI Flags

- `--socket`: Unix socket path (default: `/tmp/isabelle.sock`)
- `--isabelle-path`: Path to Isabelle binary (default: `isabelle`)
- `--debounce-ms`: Debounce delay in milliseconds (default: 300)
- `--log-dir`: Directory for rotating log files
- `--debug`: Enable debug logging
- `--mock`: Use mock mode (echo stdin to stdout, for testing)

## Protocol

### Message Types

1. **document.push**: Push document content
   ```json
   {"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///test.thy","text":"theory Test begin end"}}
   ```

2. **document.check**: Request document check
   ```json
   {"id":"msg-0002","type":"document.check","session":"s1","version":1,"payload":{"uri":"file:///test.thy","version":1}}
   ```

3. **diagnostics**: Return diagnostics
   ```json
   {"id":"msg-0003","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///test.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}
   ```

4. **markup**: Request markup/hover info
   ```json
   {"id":"msg-0004","type":"markup","session":"s1","version":1,"payload":{"uri":"file:///test.thy","offset":{"line":1,"col":5},"info":"theorem foo"}}
   ```

## Debug

Enable debug logging:

```bash
./target/release/bridge --debug --log-dir /tmp/bridge-logs
```

## Test

```bash
cargo test
```

Run with mock adapter:

```bash
./target/release/bridge --mock --socket /tmp/isabelle.sock &
echo '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///test.thy","text":"theory Test begin end"}}' | nc -U /tmp/isabelle.sock
```
