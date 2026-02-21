# Zed Isabelle Extension

This extension supports two runtime modes:

- `native` (default): launch `isabelle vscode_server` directly.
- `bridge`: launch `isabelle-zed-lsp` (Rust proxy), then forward to bridge/adapter.

Runtime chain (bridge mode):

```text
Zed -> isabelle-zed-lsp -> bridge -> scala-adapter (mock or real Isabelle)
```

## Build extension

```bash
cargo build --manifest-path zed-extension/Cargo.toml --target wasm32-wasip2 --release
```

## Build local language server proxy

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
```

## Zed dev setup

1. Open `zed: extensions`.
2. Click `Install Dev Extension`.
3. Select this folder: `.../isabelle-zed/zed-extension`.
4. Ensure `isabelle` is on `PATH` (native mode) or `isabelle-zed-lsp` is on `PATH` (bridge mode).

Example Zed settings (`settings.json`):

```json
{
  "lsp": {
    "isabelle-lsp": {
      "settings": {
        "mode": "native",
        "native_logic": "HOL",
        "native_no_build": false
      }
    }
  }
}
```

Bridge mode example:

```json
{
  "lsp": {
    "isabelle-lsp": {
      "binary": {
        "path": "/absolute/path/to/isabelle-zed-lsp"
      },
      "settings": {
        "mode": "bridge",
        "bridge_socket": "/tmp/isabelle.sock",
        "session": "s1",
        "bridge_autostart_command": "/absolute/path/to/bridge --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011",
        "bridge_autostart_timeout_ms": 10000
      }
    }
  }
}
```

## Commands handled by bridge-mode LSP

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

These are implemented in the LSP `workspace/executeCommand` handler.
