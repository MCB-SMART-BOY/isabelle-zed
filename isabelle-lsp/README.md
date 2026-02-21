# Isabelle Zed LSP Proxy

`isabelle-zed-lsp` is a lightweight LSP server that translates standard LSP messages into the
bridge NDJSON protocol used by `bridge/`.

## What it does

- `textDocument/didOpen`, `didChange`, `didSave` -> `document.push`
- `workspace/executeCommand` (`isabelle.run_check`) -> `document.check`
- `textDocument/hover` -> `markup`
- `diagnostics` bridge responses -> `textDocument/publishDiagnostics`

## Build

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
```

## Run

```bash
ISABELLE_BRIDGE_SOCKET=/tmp/isabelle.sock \
  cargo run --manifest-path isabelle-lsp/Cargo.toml --release
```

Optional env vars:

- `ISABELLE_BRIDGE_SOCKET` (default: `/tmp/isabelle.sock`)
- `ISABELLE_SESSION` (default: `s1`)
- `ISABELLE_BRIDGE_AUTOSTART_CMD` (optional): shell command used to start bridge when socket is missing
- `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS` (default: `5000`)
