# Zed Isabelle Extension

This extension can run in two modes:

- `native` (recommended): run `isabelle vscode_server` directly.
- `bridge`: run `isabelle-zed-lsp`, which forwards to `bridge`.

## Build

```bash
cargo build --manifest-path ../isabelle-lsp/Cargo.toml --release
cargo build --manifest-path Cargo.toml --target wasm32-wasip2 --release
```

Or from repo root:

```bash
make release-build
```

## Install as dev extension

1. Open Zed command palette.
2. Run `zed: extensions`.
3. Click `Install Dev Extension`.
4. Select this folder: `.../isabelle-zed/zed-extension`.

## Native mode settings (recommended)

Use `examples/zed-settings-native.json` from repo root.

Minimal inline example:

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

Requirements:

- `isabelle` must be available on `PATH`.
- `isabelle vscode_server` must work in terminal.

## Bridge mode settings

Use `examples/zed-settings-bridge-mock.json` from repo root.

Minimal inline example:

```json
{
  "lsp": {
    "isabelle-lsp": {
      "binary": {
        "path": "/absolute/path/to/isabelle-zed/isabelle-lsp/target/release/isabelle-zed-lsp"
      },
      "settings": {
        "mode": "bridge",
        "bridge_socket": "/tmp/isabelle.sock",
        "session": "s1",
        "bridge_autostart_command": "/absolute/path/to/isabelle-zed/bridge/target/release/bridge --mock --socket /tmp/isabelle.sock",
        "bridge_autostart_timeout_ms": 10000
      }
    }
  }
}
```

## Release and packaging notes

From repo root:

```bash
make doctor
make release-package
```

The package includes extension sources, prebuilt wasm artifact, and `bridge`/`isabelle-zed-lsp` binaries.

## Commands exposed by the LSP proxy

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

These are handled in `workspace/executeCommand` by `isabelle-zed-lsp`.
