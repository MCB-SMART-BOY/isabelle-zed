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
    -> scala-adapter (mock or Isabelle-backed)
```

### 零配置安装（推荐）

在仓库根目录（`<repo-root>`）执行：

```bash
make install-zed-native
```

然后重启 Zed（或重载扩展）并打开 `.thy` 文件即可使用。

要求：`isabelle` 命令可在 `PATH` 中找到。
如果仓库里缺少 `zed-extension/grammars/isabelle.wasm`，先执行：

```bash
make build-isabelle-grammar
```

该安装命令会同时安装 Isabelle 专用快捷键（写入 `keymap.json`，带标记块）。

卸载：

```bash
make uninstall-zed-native
```

### 快捷键与可视化输出

默认会安装以下快捷键：

- `Alt-Shift-I`：检查当前 Theory（`process_theories`）
- `Alt-Shift-B`：构建当前 worktree（存在 `ROOT/ROOTS` 时 `isabelle build -D`，否则回退 `process_theories -D`）
- `Alt-I`：重跑上一次 Isabelle 任务
- `F8`：备用检查快捷键（避免系统占用 `Alt-Shift`）
- `F9`：备用构建快捷键（避免系统占用 `Alt-Shift`）
- `F7`：备用重跑快捷键

单独安装/卸载快捷键：

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

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

Bridge 模式的自动拉起说明：

- 出于安全考虑，`bridge_autostart_command` / `bridge_autostart_timeout_ms` 不再从工作区 `settings.json` 读取。
- 如需自动拉起 bridge，请在启动 Zed 前通过环境变量设置：
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

### 提交到官方 Zed 扩展仓库

先运行预检查：

```bash
make zed-official-check
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
make doctor
make install-local
make release-build
make release-package
```

说明：

- `make install-local` 默认安装二进制到 `~/.local/bin`。
- 可通过环境变量 `ISABELLE_ZED_BIN_DIR` 自定义安装路径。

### Bridge mock 集成测试

```bash
make bridge-mock-up
make mock-lsp-e2e
make bridge-mock-down
```

### Bridge 真实链路（adapter-command）

bridge 默认真实模式会优先尝试在仓库内定位 `scala-adapter` 并启动：

```bash
sbt -batch "run --isabelle-path=isabelle"
```

你也可以显式指定自定义启动命令：

```bash
bridge --socket /tmp/isabelle.sock --adapter-command "<your-adapter-cmd>"
```

如果你希望 `isabelle-zed-lsp` 自动拉起 bridge，请在启动 Zed 前设置（示例）：

```bash
export ISABELLE_BRIDGE_AUTOSTART_CMD='bridge --socket /tmp/isabelle.sock --mock'
export ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS=10000
```

### 常用命令

```bash
make bridge-test
make lsp-test
make zed-check
make build-isabelle-grammar
make native-lsp-smoke
make spawn-e2e-ndjson
```

### Release 说明

- `make release-package` 会把根目录 `LICENSE` 一并打入发布包。
- `make release-package` 会校验并打入 `zed-extension/grammars/isabelle.wasm`。
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
    -> scala-adapter (mock or Isabelle-backed)
```

### Zero-config install (recommended)

From repository root (`<repo-root>`):

```bash
make install-zed-native
```

Then restart Zed (or reload extensions) and open a `.thy` file.

Requirement: `isabelle` must be available on `PATH`.
If `zed-extension/grammars/isabelle.wasm` is missing, run:

```bash
make build-isabelle-grammar
```

This install command also installs Isabelle-specific shortcuts (inserted into `keymap.json` with marker comments).

Uninstall:

```bash
make uninstall-zed-native
```

### Shortcuts and visual output

Installed default shortcuts:

- `Alt-Shift-I`: check current theory (`process_theories`)
- `Alt-Shift-B`: build current worktree (`isabelle build -D` when `ROOT/ROOTS` exists, otherwise fallback to `process_theories -D`)
- `Alt-I`: rerun the latest Isabelle task
- `F8`: fallback check shortcut (when `Alt-Shift` is intercepted by OS)
- `F9`: fallback build shortcut (when `Alt-Shift` is intercepted by OS)
- `F7`: fallback rerun shortcut

Install/uninstall shortcuts only:

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

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

Bridge autostart configuration note:

- For security hardening, `bridge_autostart_command` / `bridge_autostart_timeout_ms` are no longer read from workspace `settings.json`.
- To enable bridge autostart, set environment variables before launching Zed:
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

### Submit to the official Zed registry

Run pre-check first:

```bash
make zed-official-check
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
make doctor
make install-local
make release-build
make release-package
```

Notes:

- `make install-local` installs binaries to `~/.local/bin` by default.
- Override with `ISABELLE_ZED_BIN_DIR` if needed.

### Bridge mock integration flow

```bash
make bridge-mock-up
make mock-lsp-e2e
make bridge-mock-down
```

### Bridge real-path startup (adapter-command)

In real mode, bridge now tries to locate local `scala-adapter` and starts:

```bash
sbt -batch "run --isabelle-path=isabelle"
```

You can also provide an explicit startup command:

```bash
bridge --socket /tmp/isabelle.sock --adapter-command "<your-adapter-cmd>"
```

If you want `isabelle-zed-lsp` to autostart bridge, set these before launching Zed:

```bash
export ISABELLE_BRIDGE_AUTOSTART_CMD='bridge --socket /tmp/isabelle.sock --mock'
export ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS=10000
```

### Common commands

```bash
make bridge-test
make lsp-test
make zed-check
make build-isabelle-grammar
make native-lsp-smoke
make spawn-e2e-ndjson
```

### Release notes

- `make release-package` now includes root `LICENSE` in the archive.
- `make release-package` validates and includes `zed-extension/grammars/isabelle.wasm`.
- Release version is read from `zed-extension/extension.toml` (`version`).
