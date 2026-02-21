# Isabelle Scala Adapter (MVP)

`scala-adapter` is a long-running NDJSON process that translates editor requests into diagnostics/markup responses. It supports:

- `stdin/stdout` transport (default)
- `--socket=<host>:<port>` transport
- deterministic `--mock` mode for CI

## Build

```bash
sbt compile
```

## Run

### Mock mode (CI / local deterministic)

```bash
sbt "run --mock"
```

### Real mode (Isabelle-backed via process_theories)

```bash
sbt "run --isabelle-path=isabelle"
```

Optional logic image:

```bash
sbt "run --isabelle-path=isabelle --logic=HOL"
```

### Socket mode

```bash
sbt "run --mock --socket=127.0.0.1:9011"
```

## Protocol examples (exact)

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}
```

## Bridge wiring

Use stdio transport between bridge and adapter:

1. Start adapter (mock):

```bash
cd scala-adapter
sbt "run --mock"
```

2. Start bridge in non-mock mode (so it launches `isabelle scala` command path you provide) or use bridge mock when testing bridge only.

## Testing

```bash
sbt test
```

The test suite runs `AdapterMain` in `--mock` mode with piped streams and verifies a full `document.push -> diagnostics` roundtrip.

## Backend notes

- `--mock`: deterministic CI mode (`Parse error` diagnostics).
- real mode: checks pushed theory text via `isabelle process_theories -D <tmp> -O <Theory>`.
- hover in real mode currently returns a placeholder info string.
