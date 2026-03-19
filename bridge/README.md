# Isabelle Bridge (MVP) / Isabelle Bridge（MVP）

## 中文（简体）

`bridge` 是 Rust 编写的 NDJSON 路由器，用于在编辑器与 Isabelle adapter 后端之间转发消息。

### 路径约定

本页命令默认在 `bridge/` 目录执行。
如果你在仓库根目录（`<repo-root>`）执行，请使用 `-p isabelle-bridge`。

### 构建

```bash
cargo build --release
```

### 运行

Unix Socket 模式（推荐）：

```bash
./target/release/bridge --socket /tmp/isabelle.sock
```

Stdin/Stdout 模式：

```bash
cat request.ndjson | ./target/release/bridge
```

Mock 模式（CI / 本地可复现测试）：

```bash
./target/release/bridge --mock --socket /tmp/isabelle.sock
```

外部适配器 Socket 模式：

```bash
./target/release/bridge --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011
```

### CLI 参数

- `--socket <PATH>`：监听 Unix Socket；省略则走 stdin/stdout
- `--isabelle-path <PATH>`：Isabelle 可执行路径，默认 `isabelle`
- `--logic <NAME>`：`process_theories` 使用的 logic image，默认 `HOL`
- `--adapter-socket <HOST:PORT>`：连接外部已运行适配器（TCP）
- `--adapter-command <CMD>`：使用自定义命令启动适配器进程（经 `bash -lc` 执行）
- `--debounce-ms <N>`：`document.push` 防抖窗口，默认 `300`
- `--log-dir <PATH>`：调试日志目录
- `--mock`：使用内置 mock adapter
- `--debug`：启用 debug 日志并写入滚动日志文件

未设置 `--adapter-socket` 与 `--adapter-command` 时，bridge 会自动拉起内置 Rust real adapter。

### 协议示例（精确）

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":1},"end":{"line":1,"col":7}},"severity":"error","message":"Parse error"}]}
```

### 调试日志

```bash
./target/release/bridge --mock --debug --log-dir /tmp/isabelle-bridge-logs --socket /tmp/isabelle.sock
```

### CI 一键命令（mock）

```bash
cargo run -- --mock --socket /tmp/isabelle.sock
```

另一个终端发送请求：

```bash
printf '%s\n' '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}' | nc -U /tmp/isabelle.sock
```

### 测试

```bash
cargo test
```

仓库根目录也提供了真实启动链路的本地回归命令：

```bash
make spawn-e2e-ndjson
```

## English

`bridge` is a Rust NDJSON router between an editor client and an Isabelle adapter backend.

### Path convention

Commands in this document assume you are in `bridge/`.
If you run from repository root (`<repo-root>`), use `-p isabelle-bridge`.

### Build

```bash
cargo build --release
```

### Run

Unix socket mode (recommended):

```bash
./target/release/bridge --socket /tmp/isabelle.sock
```

Stdin/stdout mode:

```bash
cat request.ndjson | ./target/release/bridge
```

Mock mode (CI / deterministic local testing):

```bash
./target/release/bridge --mock --socket /tmp/isabelle.sock
```

External adapter socket mode:

```bash
./target/release/bridge --socket /tmp/isabelle.sock --adapter-socket 127.0.0.1:9011
```

### CLI flags

- `--socket <PATH>`: listen on a Unix socket; if omitted uses stdin/stdout
- `--isabelle-path <PATH>`: Isabelle executable path (default `isabelle`)
- `--logic <NAME>`: logic image passed to `process_theories` (default `HOL`)
- `--adapter-socket <HOST:PORT>`: connect to external running adapter over TCP
- `--adapter-command <CMD>`: start adapter process with a custom command (via `bash -lc`)
- `--debounce-ms <N>`: debounce window for `document.push` (default `300`)
- `--log-dir <PATH>`: directory for debug logs
- `--mock`: use built-in mock adapter
- `--debug`: enable debug logging and rotating debug file output

If `--adapter-socket` and `--adapter-command` are both omitted, bridge starts its built-in Rust real adapter.

### Protocol examples (exact)

```json
{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}
```

```json
{"id":"msg-0001","type":"diagnostics","session":"s1","version":1,"payload":[{"uri":"file:///home/user/example.thy","range":{"start":{"line":1,"col":1},"end":{"line":1,"col":7}},"severity":"error","message":"Parse error"}]}
```

### Debug logging

```bash
./target/release/bridge --mock --debug --log-dir /tmp/isabelle-bridge-logs --socket /tmp/isabelle.sock
```

### CI one-liner (mock)

```bash
cargo run -- --mock --socket /tmp/isabelle.sock
```

Then send a request from another shell:

```bash
printf '%s\n' '{"id":"msg-0001","type":"document.push","session":"s1","version":1,"payload":{"uri":"file:///home/user/example.thy","text":"theory Example imports Main begin\nend\n"}}' | nc -U /tmp/isabelle.sock
```

### Tests

```bash
cargo test
```

From repository root, a local real-startup regression is also available:

```bash
make spawn-e2e-ndjson
```
