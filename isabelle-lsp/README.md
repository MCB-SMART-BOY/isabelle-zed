# Isabelle Zed LSP Proxy / Isabelle Zed LSP 代理

## 中文（简体）

`isabelle-zed-lsp` 是一个轻量 LSP 服务器，将标准 LSP 消息转换为 `bridge` 使用的 NDJSON 协议。

### 主要映射

- `textDocument/didOpen`、`didChange`、`didSave` -> `document.push`
- `workspace/executeCommand`（`isabelle.run_check`）-> `document.check`
- `textDocument/hover` -> `markup`
- `diagnostics` 响应 -> `textDocument/publishDiagnostics`

### 构建（在 `<repo-root>`）

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
```

### 运行（在 `<repo-root>`）

```bash
ISABELLE_BRIDGE_SOCKET=/tmp/isabelle.sock \
  cargo run --manifest-path isabelle-lsp/Cargo.toml --release
```

可选环境变量：

- `ISABELLE_BRIDGE_SOCKET`（默认 `/tmp/isabelle.sock`）
- `ISABELLE_SESSION`（默认 `s1`）
- `ISABELLE_BRIDGE_AUTOSTART_CMD`（可选）：bridge socket 不存在时用于自动拉起 bridge 的命令行（按 argv 解析）
- `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`（默认 `5000`）
- `ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS`（默认 `12000`）：单次 bridge 请求超时时间，超时会重连重试一次

### 诊断位置与跨文件诊断

- bridge/adapter 侧诊断位置按 1-based 传输，LSP 发布时会转换为 LSP 的 0-based 坐标。
- 发布诊断时会按诊断自身 `uri` 分组并分别发布，支持跨文件诊断语义。

## English

`isabelle-zed-lsp` is a lightweight LSP server that translates standard LSP messages into the bridge NDJSON protocol.

### Message mapping

- `textDocument/didOpen`, `didChange`, `didSave` -> `document.push`
- `workspace/executeCommand` (`isabelle.run_check`) -> `document.check`
- `textDocument/hover` -> `markup`
- `diagnostics` bridge responses -> `textDocument/publishDiagnostics`

### Build (from `<repo-root>`)

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
```

### Run (from `<repo-root>`)

```bash
ISABELLE_BRIDGE_SOCKET=/tmp/isabelle.sock \
  cargo run --manifest-path isabelle-lsp/Cargo.toml --release
```

Optional environment variables:

- `ISABELLE_BRIDGE_SOCKET` (default: `/tmp/isabelle.sock`)
- `ISABELLE_SESSION` (default: `s1`)
- `ISABELLE_BRIDGE_AUTOSTART_CMD` (optional): command line used to auto-start bridge when socket is missing (parsed into argv)
- `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS` (default: `5000`)
- `ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS` (default: `12000`): per-request timeout for bridge calls, with one reconnect retry

### Diagnostic positions and cross-file diagnostics

- bridge/adapter diagnostic positions are transported as 1-based and converted to LSP 0-based positions before publish.
- diagnostics are grouped and published by each diagnostic `uri`, preserving cross-file semantics.
