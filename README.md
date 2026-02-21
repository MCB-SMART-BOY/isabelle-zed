# Isabelle-Zed

Isabelle support for Zed with two runtime options:

```text
native mode (default):
  Zed Extension (WASM) -> isabelle vscode_server

bridge mode:
  Zed Extension (WASM)
    -> isabelle-zed-lsp (Rust LSP proxy)
    -> bridge (Rust NDJSON process/socket bridge)
    -> scala-adapter (Scala mock or Isabelle-backed adapter)
```

## What currently works

- `.thy` language registration in Zed extension manifest.
- Native mode via official `isabelle vscode_server` (real PIDE-backed LSP).
- Bridge-mode diagnostics pipeline (`didOpen`/`didChange`/`didSave` -> `document.push` -> diagnostics).
- Bridge-mode hover pipeline (`textDocument/hover` -> `markup`).
- Bridge mock mode and external adapter socket mode.
- Mock end-to-end CI checks (NDJSON and LSP path).

## Build

```bash
make bridge-build
make lsp-build
make zed-build
```

## Native mode smoke test

```bash
make native-lsp-smoke
```

## Mock demo (bridge-mode LSP end-to-end)

This verifies the same path Zed uses (LSP -> bridge):

```bash
make mock-lsp-e2e
```

## Mock demo (bridge + scala-adapter socket)

### Terminal 1

```bash
make mock-adapter
```

### Terminal 2

```bash
make mock-bridge-adapter
```

### Terminal 3

```bash
make mock-send
```

Expected NDJSON response:

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":0},"end":{"line":1,"col":6}},"severity":"error","message":"Parse error"}]}
```

## Use in Zed (dev extension)

1. Build the local LSP binary:

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
```

2. Build extension wasm:

```bash
cargo build --manifest-path zed-extension/Cargo.toml --target wasm32-wasip2 --release
```

3. In Zed, open `zed: extensions` -> `Install Dev Extension` -> select `zed-extension/`.
4. Native mode (default) uses `isabelle vscode_server`; make sure `isabelle` is on `PATH`.
5. For bridge mode, set `lsp.isabelle-lsp.settings.mode = "bridge"` and configure `binary.path` to `isabelle-zed-lsp` if needed.

## Real Isabelle-backed mode

- Native mode is already real Isabelle-backed through `isabelle vscode_server`.
- Bridge mode keeps the custom protocol path for experimentation and integration testing.
