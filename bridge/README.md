# Isabelle Bridge (MVP)

`bridge` is a Rust NDJSON router between an editor client and an Isabelle Scala adapter process.

## Build

```bash
cargo build --release
```

## Run

### Unix socket mode (recommended)

```bash
./target/release/bridge --socket /tmp/isabelle.sock
```

### Stdin/stdout mode

```bash
cat request.ndjson | ./target/release/bridge
```

### Mock mode (CI / local deterministic testing)

```bash
./target/release/bridge --mock --socket /tmp/isabelle.sock
```

### External Scala adapter socket mode

```bash
./target/release/bridge --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011
```

## CLI flags

- `--socket <PATH>`: listen on a Unix socket (if omitted, bridge uses stdin/stdout)
- `--isabelle-path <PATH>`: Isabelle executable path (default: `isabelle`)
- `--adapter-socket <HOST:PORT>`: connect to an already-running adapter over TCP instead of spawning `isabelle scala`
- `--debounce-ms <N>`: debounce window for `document.push` (default: `300`)
- `--log-dir <PATH>`: directory for rotating debug logs
- `--mock`: spawn a deterministic mock adapter subprocess instead of `isabelle scala`
- `--debug`: enable debug-level logging and rotating log file output

## Protocol examples (exact)

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}
```

## Debug logging

With `--debug`, all incoming/outgoing NDJSON lines are logged and written to a rotating file:

```bash
./target/release/bridge --mock --debug --log-dir /tmp/isabelle-bridge-logs --socket /tmp/isabelle.sock
```

## CI one-liner (mock)

```bash
cargo run --mock --socket /tmp/isabelle.sock
```

Then from another shell:

```bash
printf '%s\n' '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}' | nc -U /tmp/isabelle.sock
```

## Tests

```bash
cargo test
```
