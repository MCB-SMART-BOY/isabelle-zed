# Isabelle-Zed

Isabelle theorem prover support for the Zed code editor.

## Architecture

```
┌─────────────┐     NDJSON      ┌──────────┐     NDJSON      ┌──────────────┐
│    Zed      │ ──────────────►│  Bridge  │ ──────────────►│ Scala Adapter│
│  Extension  │◄──────────────│  (Rust)  │◄────────────────│   (Scala)    │
└─────────────┘                └──────────┘                 └──────────────┘
```

## Components

- **[bridge/](bridge/README.md)** - Rust NDJSON bridge process
- **[scala-adapter/](scala-adapter/README.md)** - Scala PIDE adapter
- **[zed-extension/](zed-extension/README.md)** - Zed editor extension

## Quick Start

### Mock Mode (no Isabelle required)

```bash
# Terminal 1: Start bridge in mock mode
cd bridge
cargo run -- --mock --socket /tmp/isabelle.sock

# Terminal 2: Send test message
echo '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///test.thy","text":"theory Test begin end"}}' | nc -U /tmp/isabelle.sock
```

### With Real Isabelle

```bash
# Terminal 1: Start bridge
cd bridge
cargo run -- --socket /tmp/isabelle.sock

# Terminal 2: Start Scala adapter
cd scala-adapter
sbt "run"

# Terminal 3: Use in Zed
# Load zed-extension in Zed dev mode
```

## Development

### Build

```bash
# Bridge (Rust)
cd bridge && cargo build --release

# Scala adapter
cd scala-adapter && sbt compile

# Zed extension
cd zed-extension && cargo build --release
```

### Test

```bash
# Rust tests
cd bridge && cargo test

# Scala tests
cd scala-adapter && sbt test
```

## References

- [Isabelle/jEdit — a Prover IDE within the PIDE framework](https://arxiv.org/abs/1207.3441)
- [Isabelle System Manual](https://isabelle.in.tum.de/doc/system.pdf)
- [Zed Extensions](https://zed.dev/docs/extensions/developing-extensions)

## License

MIT
