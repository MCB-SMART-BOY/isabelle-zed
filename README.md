# Isabelle for Zed / Zed 的 Isabelle 扩展

## 中文（简体）

### 项目概览

本项目为 Zed 提供 Isabelle 支持，包含两种运行模式：

```text
native 模式（推荐）：
  Zed Extension (WASM) -> isabelle vscode_server

bridge 模式（集成测试/实验）：
  Zed Extension (WASM)
    -> isabelle-zed-lsp (Rust LSP proxy)
    -> bridge (Rust NDJSON bridge)
    -> 内置 Rust adapter（mock 或 Isabelle-backed）
```

项目结构与分层约定见：

- `docs/project-structure.md`

### 零配置安装（推荐）

在仓库根目录（`<repo-root>`）执行：

```bash
cargo run -p isabelle-zed-xtask -- install-zed-native
```

然后重启 Zed（或重载扩展）并打开 `.thy` 文件即可使用。

要求：`isabelle` 命令可在 `PATH` 中找到。
如果仓库里缺少 `zed-extension/grammars/isabelle.wasm`，先执行：

```bash
cargo run -p isabelle-zed-xtask -- build-isabelle-grammar
```

该安装命令会同时安装 Isabelle 专用快捷键（写入 `keymap.json`，带标记块）。

卸载：

```bash
cargo run -p isabelle-zed-xtask -- uninstall-zed-native
```

Windows 原生安装/卸载与 Linux/macOS 使用同一套 Rust 命令：

```bash
cargo run -p isabelle-zed-xtask -- install-zed-native
cargo run -p isabelle-zed-xtask -- uninstall-zed-native
```

说明：

- 上述命令是 native 模式（直接调用 `isabelle vscode_server`），支持 Windows/Linux/macOS。
- bridge 模式支持跨平台 endpoint：Unix 默认 `unix:/tmp/isabelle-<worktree-id>.sock`，Windows 默认 `tcp:127.0.0.1:<port>`。
- Windows 默认会安装快捷键；如需跳过，设置 `ISABELLE_ZED_SKIP_SHORTCUTS=1`。
- 单独安装/卸载快捷键：

```bash
cargo run -p isabelle-zed-xtask -- install-zed-shortcuts
cargo run -p isabelle-zed-xtask -- uninstall-zed-shortcuts
```

- 若 keymap 不在默认位置，设置 `ISABELLE_ZED_KEYMAP_PATH` 指向实际文件。
- 若 `isabelle` 不在 `PATH`，请在 Zed 设置中指定可执行文件路径（可指向 `isabelle` 或自建 wrapper）。

### 快捷键与可视化输出

默认会安装以下快捷键：

- `Alt-Shift-I`：检查当前 Theory（`process_theories`）
- `Alt-Shift-B`：构建当前 worktree（`isabelle build -D`）
- `Alt-I`：重跑上一次 Isabelle 任务
- `F8`：备用检查快捷键（避免系统占用 `Alt-Shift`）
- `F9`：备用构建快捷键（避免系统占用 `Alt-Shift`）
- `F7`：备用重跑快捷键

单独安装/卸载快捷键：

```bash
cargo run -p isabelle-zed-xtask -- install-zed-shortcuts
cargo run -p isabelle-zed-xtask -- uninstall-zed-shortcuts
```

补充任务（从任务面板运行）：

- `isabelle: check theory (process_theories -D, prompt)`：输入 Theory 名称（不含 `.thy`）
- `isabelle: build session (build -D, prompt)`：输入 Session 名称

说明：Zed 任务变量目前无法自动获取当前 Theory 名称，因此默认快捷键仍以 worktree/session 上下文执行。

可视化方式说明：

- 诊断信息：使用 Zed 的标准 Diagnostics（红线、Problems）
- 任务输出：显示在 Zed 终端/任务输出视图
- 当前 Zed 扩展 API 无法提供自定义侧边 Proof 面板（后续 API 支持后可补）

### 可选配置示例

以下文件位于仓库根目录：

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`
- `examples/zed-keymap-isabelle.json`

仅在你需要覆盖默认行为时使用。

Native 可选设置：

- `native_logic`：等同于 `-l <logic>`
- `native_no_build`：等同于 `-n`
- `native_session_dirs`：追加多个 `-d <dir>`
- `native_extra_args`：直接追加到 `vscode_server` 的参数列表（字符串数组）

Native 模式行为提示：

- 如果工作区根目录存在 `ROOT` 或 `ROOTS`，会自动附加 `-d <worktree-root>` 以便加载 session。

自动选择 `-l <logic>` 的优先级（从上到下匹配）：
1. 若工作区根目录 `ROOT` 里仅定义了一个 session，则使用该 session。
2. 否则，若 `ROOT/ROOTS` 中仅有一个 session 明确继承 `HOL`（例如 `session X = HOL`），则使用该 session。
3. 否则默认 `HOL`。

Bridge 模式的自动拉起说明：

- Bridge 模式默认 endpoint：Unix 为 `unix:/tmp/isabelle-<worktree-id>.sock`，Windows 为 `tcp:127.0.0.1:<port>`（可通过 `bridge_endpoint` 覆盖，`bridge_socket` 仍兼容旧配置）。
- 出于安全考虑，`bridge_autostart_command` / `bridge_autostart_timeout_ms` 不再从工作区 `settings.json` 读取。
- 如需自动拉起 bridge，请在启动 Zed 前通过环境变量设置：
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

### 提交到官方 Zed 扩展仓库

先运行预检查：

```bash
cargo run -p isabelle-zed-xtask -- zed-official-check
```

然后按文档提交到 `zed-industries/extensions`：

- `docs/official-submission.md`

推荐条目示例（提交到官方仓库的 `extensions.toml`）：

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "<version-from-zed-extension/extension.toml>"
```

### 本地辅助命令

```bash
cargo run -p isabelle-zed-xtask -- doctor
cargo run -p isabelle-zed-xtask -- install-local
cargo run -p isabelle-zed-xtask -- release-build
cargo run -p isabelle-zed-xtask -- release-package
```

说明：

- `cargo run -p isabelle-zed-xtask -- install-local` 默认安装二进制到 `~/.local/bin`。
- 可通过环境变量 `ISABELLE_ZED_BIN_DIR` 自定义安装路径。

### Bridge mock 集成测试

```bash
cargo run -p isabelle-zed-xtask -- bridge-mock-up
cargo run -p isabelle-zed-xtask -- mock-lsp-e2e
cargo run -p isabelle-zed-xtask -- bridge-mock-down
```

### Bridge 真实链路（adapter-command）

bridge 默认真实模式会自动拉起内置 Rust adapter（无需 Scala / sbt）。
bridge / bridge-mode LSP 链路支持两类监听端点：
- `--socket <path>`（Unix）
- `--tcp <host:port>`（跨平台）

你也可以显式指定自定义启动命令：

```bash
bridge --socket /tmp/isabelle.sock --adapter-command "<your-adapter-cmd>"
bridge --tcp 127.0.0.1:39393 --adapter-command "<your-adapter-cmd>"
```

`--adapter-command` 会按 argv 解析后直接执行（不经 `bash -lc`）。
bridge real adapter 支持 `--session-dir <path>`（可重复）补充 `process_theories -d` 搜索路径；
并会自动把目标 `.thy` 所在目录加入 session 搜索路径。

如果你希望 `isabelle-zed-lsp` 自动拉起 bridge，请在启动 Zed 前设置（示例）：

```bash
export ISABELLE_BRIDGE_AUTOSTART_CMD='bridge --socket /tmp/isabelle.sock --mock'
export ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS=10000
```

### 常用命令

```bash
cargo test -p isabelle-bridge
cargo test -p isabelle-zed-lsp
cargo check -p isabelle-zed-extension
cargo run -p isabelle-zed-xtask -- build-isabelle-grammar
cargo run -p isabelle-zed-xtask -- bridge-real-smoke
cargo run -p isabelle-zed-xtask -- native-lsp-smoke
cargo run -p isabelle-zed-xtask -- spawn-e2e-ndjson
```

### Release 说明

- `cargo run -p isabelle-zed-xtask -- release-package` 会把根目录 `LICENSE` 一并打入发布包。
- `cargo run -p isabelle-zed-xtask -- release-package` 会校验并打入 `zed-extension/grammars/isabelle.wasm`。
- 发布版本号来自 `zed-extension/extension.toml` 的 `version` 字段。

## English

### Overview

This project provides Isabelle support for Zed with two runtime modes:

```text
native mode (recommended):
  Zed Extension (WASM) -> isabelle vscode_server

bridge mode (integration/testing):
  Zed Extension (WASM)
    -> isabelle-zed-lsp (Rust LSP proxy)
    -> bridge (Rust NDJSON bridge)
    -> built-in Rust adapter (mock or Isabelle-backed)
```

Project layout and structure conventions:

- `docs/project-structure.md`

### Zero-config install (recommended)

From repository root (`<repo-root>`):

```bash
cargo run -p isabelle-zed-xtask -- install-zed-native
```

Then restart Zed (or reload extensions) and open a `.thy` file.

Requirement: `isabelle` must be available on `PATH`.
If `zed-extension/grammars/isabelle.wasm` is missing, run:

```bash
cargo run -p isabelle-zed-xtask -- build-isabelle-grammar
```

This install command also installs Isabelle-specific shortcuts (inserted into `keymap.json` with marker comments).

Uninstall:

```bash
cargo run -p isabelle-zed-xtask -- uninstall-zed-native
```

Windows uses the same Rust command entrypoints as Linux/macOS:

```bash
cargo run -p isabelle-zed-xtask -- install-zed-native
cargo run -p isabelle-zed-xtask -- uninstall-zed-native
```

Notes:

- The commands above target native mode (`isabelle vscode_server`) and work on Windows/Linux/macOS.
- Bridge mode now supports cross-platform endpoints: Unix defaults to `unix:/tmp/isabelle-<worktree-id>.sock`, Windows defaults to `tcp:127.0.0.1:<port>`.
- Shortcuts are installed by default; to skip, set `ISABELLE_ZED_SKIP_SHORTCUTS=1`.
- Install/uninstall shortcuts only:

```bash
cargo run -p isabelle-zed-xtask -- install-zed-shortcuts
cargo run -p isabelle-zed-xtask -- uninstall-zed-shortcuts
```

- If your keymap lives elsewhere, set `ISABELLE_ZED_KEYMAP_PATH` to the actual file.
- If `isabelle` is not on `PATH`, set the executable path in Zed settings (point to your `isabelle` or a wrapper).

### Shortcuts and visual output

Installed default shortcuts:

- `Alt-Shift-I`: check current theory (`process_theories`)
- `Alt-Shift-B`: build current worktree (`isabelle build -D`)
- `Alt-I`: rerun the latest Isabelle task
- `F8`: fallback check shortcut (when `Alt-Shift` is intercepted by OS)
- `F9`: fallback build shortcut (when `Alt-Shift` is intercepted by OS)
- `F7`: fallback rerun shortcut

Install/uninstall shortcuts only:

```bash
cargo run -p isabelle-zed-xtask -- install-zed-shortcuts
cargo run -p isabelle-zed-xtask -- uninstall-zed-shortcuts
```

Extra tasks (run from the task palette):

- `isabelle: check theory (process_theories -D, prompt)`: enter the theory name (without `.thy`)
- `isabelle: build session (build -D, prompt)`: enter the session name

Note: Zed task variables do not expose the current theory name, so the default shortcut still runs in the worktree/session context.

Visual output model:

- diagnostics: standard Zed diagnostics UI (squiggles/problems)
- task output: Zed terminal/task output view
- custom proof side panel is not currently possible with the public Zed extension API

### Optional settings examples

These files are at repository root:

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`
- `examples/zed-keymap-isabelle.json`

Use them only when you need custom overrides.

Optional native settings:

- `native_logic`: maps to `-l <logic>`
- `native_no_build`: maps to `-n`
- `native_session_dirs`: appends multiple `-d <dir>`
- `native_extra_args`: extra argv passed to `vscode_server` (string array)

Native mode behavior note:

- If `ROOT` or `ROOTS` exists at worktree root, the extension auto-adds `-d <worktree-root>` for session discovery.

Auto-selected `-l <logic>` priority (first match wins):
1. If worktree root `ROOT` defines exactly one session, use it.
2. Otherwise, if exactly one session across `ROOT/ROOTS` explicitly inherits `HOL` (e.g. `session X = HOL`), use it.
3. Otherwise default to `HOL`.

Bridge autostart configuration note:

- Bridge mode default endpoint: Unix `unix:/tmp/isabelle-<worktree-id>.sock`, Windows `tcp:127.0.0.1:<port>` (override via `bridge_endpoint`; legacy `bridge_socket` remains supported).
- For security hardening, `bridge_autostart_command` / `bridge_autostart_timeout_ms` are no longer read from workspace `settings.json`.
- To enable bridge autostart, set environment variables before launching Zed:
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

### Submit to the official Zed registry

Run pre-check first:

```bash
cargo run -p isabelle-zed-xtask -- zed-official-check
```

Then follow:

- `docs/official-submission.md`

Suggested entry for `extensions.toml` in `zed-industries/extensions`:

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "<version-from-zed-extension/extension.toml>"
```

### Local helper commands

```bash
cargo run -p isabelle-zed-xtask -- doctor
cargo run -p isabelle-zed-xtask -- install-local
cargo run -p isabelle-zed-xtask -- release-build
cargo run -p isabelle-zed-xtask -- release-package
```

Notes:

- `cargo run -p isabelle-zed-xtask -- install-local` installs binaries to `~/.local/bin` by default.
- Override with `ISABELLE_ZED_BIN_DIR` if needed.

### Bridge mock integration flow

```bash
cargo run -p isabelle-zed-xtask -- bridge-mock-up
cargo run -p isabelle-zed-xtask -- mock-lsp-e2e
cargo run -p isabelle-zed-xtask -- bridge-mock-down
```

### Bridge real-path startup (adapter-command)

In real mode, bridge starts its built-in Rust adapter by default (no Scala / sbt dependency).
The bridge / bridge-mode LSP path supports two listen endpoint styles:
- `--socket <path>` (Unix)
- `--tcp <host:port>` (cross-platform)

You can also provide an explicit startup command:

```bash
bridge --socket /tmp/isabelle.sock --adapter-command "<your-adapter-cmd>"
bridge --tcp 127.0.0.1:39393 --adapter-command "<your-adapter-cmd>"
```

`--adapter-command` is parsed into argv and executed directly (without `bash -lc`).
The bridge real adapter supports repeatable `--session-dir <path>` to extend `process_theories -d` lookup paths,
and also auto-adds the target `.thy` parent directory as a session search path.

If you want `isabelle-zed-lsp` to autostart bridge, set these before launching Zed:

```bash
export ISABELLE_BRIDGE_AUTOSTART_CMD='bridge --socket /tmp/isabelle.sock --mock'
export ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS=10000
```

### Common commands

```bash
cargo test -p isabelle-bridge
cargo test -p isabelle-zed-lsp
cargo check -p isabelle-zed-extension
cargo run -p isabelle-zed-xtask -- build-isabelle-grammar
cargo run -p isabelle-zed-xtask -- bridge-real-smoke
cargo run -p isabelle-zed-xtask -- native-lsp-smoke
cargo run -p isabelle-zed-xtask -- spawn-e2e-ndjson
```

### Release notes

- `cargo run -p isabelle-zed-xtask -- release-package` now includes root `LICENSE` in the archive.
- `cargo run -p isabelle-zed-xtask -- release-package` validates and includes `zed-extension/grammars/isabelle.wasm`.
- Release version is read from `zed-extension/extension.toml` (`version`).
