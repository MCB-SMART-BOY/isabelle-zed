# Isabelle Scala Adapter (MVP) / Isabelle Scala 适配器（MVP）

## 中文（简体）

`scala-adapter` 是一个长期运行的 NDJSON 进程，将编辑器请求转换为 diagnostics/markup 响应。

支持：

- `stdin/stdout`（默认）
- `--socket=<host>:<port>`
- `--mock`（CI 可复现）
- 连接级并发上限与收尾保护（避免 `inflight` 无界增长）

### 路径约定

以下命令默认在 `<repo-root>/scala-adapter` 执行。

### 构建

```bash
sbt compile
```

### 运行

Mock 模式（CI / 本地可复现）：

```bash
sbt "run --mock"
```

真实模式（Isabelle `process_theories` 后端）：

```bash
sbt "run --isabelle-path=isabelle"
```

指定 logic image：

```bash
sbt "run --isabelle-path=isabelle --logic=HOL"
```

Socket 模式：

```bash
sbt "run --mock --socket=127.0.0.1:9011"
```

### 协议示例（精确）

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":1},"end":{"line":1,"col":7}},"severity":"error","message":"Parse error"}]}
```

### 与 bridge 联调（示例）

终端 1（启动 adapter mock）：

```bash
cd <repo-root>/scala-adapter
sbt "run --mock --socket=127.0.0.1:9011"
```

终端 2（从仓库根目录启动 bridge）：

```bash
cargo run --manifest-path <repo-root>/bridge/Cargo.toml -- --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011
```

### 测试

```bash
sbt test
```

测试会在 `--mock` 模式下验证 `document.push -> diagnostics` 的完整往返。

### 后端说明

- `--mock`：固定返回 `Parse error`，用于 CI
- 真实模式：通过 `isabelle process_theories -D <tmp> -O <Theory>` 检查 theory
- 真实模式 hover 当前返回占位信息
- 诊断位置信息按 1-based 输出（由 LSP 侧转换到 0-based）

## English

`scala-adapter` is a long-running NDJSON process that translates editor requests into diagnostics/markup responses.

Supported transports/modes:

- `stdin/stdout` (default)
- `--socket=<host>:<port>`
- deterministic `--mock` mode for CI
- connection-level concurrency cap and shutdown safeguards (to avoid unbounded `inflight` growth)

### Path convention

Commands below assume you are in `<repo-root>/scala-adapter`.

### Build

```bash
sbt compile
```

### Run

Mock mode (CI / deterministic local):

```bash
sbt "run --mock"
```

Real mode (Isabelle-backed via `process_theories`):

```bash
sbt "run --isabelle-path=isabelle"
```

Optional logic image:

```bash
sbt "run --isabelle-path=isabelle --logic=HOL"
```

Socket mode:

```bash
sbt "run --mock --socket=127.0.0.1:9011"
```

### Protocol examples (exact)

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":1},"end":{"line":1,"col":7}},"severity":"error","message":"Parse error"}]}
```

### Bridge wiring example

Terminal 1 (start adapter mock):

```bash
cd <repo-root>/scala-adapter
sbt "run --mock --socket=127.0.0.1:9011"
```

Terminal 2 (start bridge from repo root):

```bash
cargo run --manifest-path <repo-root>/bridge/Cargo.toml -- --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011
```

### Testing

```bash
sbt test
```

Tests run `AdapterMain` in `--mock` mode and validate a full `document.push -> diagnostics` roundtrip.

### Backend notes

- `--mock`: deterministic CI mode (`Parse error` diagnostics)
- real mode: checks pushed theory text via `isabelle process_theories -D <tmp> -O <Theory>`
- hover in real mode currently returns placeholder text
- diagnostic positions are emitted as 1-based (converted to LSP 0-based in `isabelle-zed-lsp`)
