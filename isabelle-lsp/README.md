# Isabelle Zed LSP Proxy / Isabelle Zed LSP 代理

## 中文（简体）

`isabelle-zed-lsp` 是一个轻量 LSP 服务器，将标准 LSP 消息转换为 `bridge` 使用的 NDJSON 协议。
该代理通过 Unix Domain Socket 与 bridge 通信，目前仅支持 Unix（Linux/macOS）。

### 主要映射

- `textDocument/didOpen`、`didChange`、`didSave` -> `document.push`
- `workspace/executeCommand`（`isabelle.run_check`）-> `document.check`
- `textDocument/hover` -> `markup`
- `diagnostics` 响应 -> `textDocument/publishDiagnostics`

命令语义：

- `isabelle.start_session`：恢复会话并触发一次检查
- `isabelle.stop_session`：停止会话并清空诊断（后续 push/check/hover 会被忽略）

### 构建（在 `<repo-root>`）

```bash
cargo build -p isabelle-zed-lsp --release
```

### 运行（在 `<repo-root>`）

```bash
ISABELLE_BRIDGE_SOCKET=/tmp/isabelle.sock \
  cargo run -p isabelle-zed-lsp --release
```

可选环境变量：

- `ISABELLE_BRIDGE_SOCKET`（默认 `/tmp/isabelle.sock`）
- `ISABELLE_SESSION`（默认 `s1`）
- `ISABELLE_BRIDGE_AUTOSTART_CMD`（可选）：bridge socket 不存在时用于自动拉起 bridge 的命令行（按 argv 解析，不经 `bash -lc`）
- `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`（默认 `5000`）
- `ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS`（默认 `30000`）：单次 bridge 请求超时时间，超时会重连重试一次

### 诊断位置与跨文件诊断

- bridge/adapter 侧诊断位置按 1-based 传输，LSP 发布时会转换为 LSP 的 0-based 坐标。
- hover 请求位置会从 LSP 0-based 转换为 bridge 1-based。
- 发布诊断时会按诊断自身 `uri` 分组并分别发布，支持跨文件诊断语义。

## English

`isabelle-zed-lsp` is a lightweight LSP server that translates standard LSP messages into the bridge NDJSON protocol.
It communicates with bridge over Unix domain sockets and is currently Unix-only (Linux/macOS).

### Message mapping

- `textDocument/didOpen`, `didChange`, `didSave` -> `document.push`
- `workspace/executeCommand` (`isabelle.run_check`) -> `document.check`
- `textDocument/hover` -> `markup`
- `diagnostics` bridge responses -> `textDocument/publishDiagnostics`

Command semantics:

- `isabelle.start_session`: resume session and trigger one check
- `isabelle.stop_session`: stop session and clear diagnostics (subsequent push/check/hover are ignored)

### Build (from `<repo-root>`)

```bash
cargo build -p isabelle-zed-lsp --release
```

### Run (from `<repo-root>`)

```bash
ISABELLE_BRIDGE_SOCKET=/tmp/isabelle.sock \
  cargo run -p isabelle-zed-lsp --release
```

Optional environment variables:

- `ISABELLE_BRIDGE_SOCKET` (default: `/tmp/isabelle.sock`)
- `ISABELLE_SESSION` (default: `s1`)
- `ISABELLE_BRIDGE_AUTOSTART_CMD` (optional): command line used to auto-start bridge when socket is missing (parsed into argv, without `bash -lc`)
- `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS` (default: `5000`)
- `ISABELLE_BRIDGE_REQUEST_TIMEOUT_MS` (default: `30000`): per-request timeout for bridge calls, with one reconnect retry

### Diagnostic positions and cross-file diagnostics

- bridge/adapter diagnostic positions are transported as 1-based and converted to LSP 0-based positions before publish.
- hover request positions are converted from LSP 0-based to bridge 1-based.
- diagnostics are grouped and published by each diagnostic `uri`, preserving cross-file semantics.
