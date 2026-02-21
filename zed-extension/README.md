# Zed Isabelle Extension / Zed Isabelle 扩展

## 中文（简体）

### 立即安装并使用（无需改 settings）

在仓库根目录（`<repo-root>`）执行：

```bash
make install-zed-native
```

脚本会把扩展安装到 Zed 本地扩展目录，扩展默认走 `native` 模式，无需手动编辑 `settings.json`。
同时会自动安装 Isabelle 快捷键到 Zed `keymap.json`（带标记块，支持卸载）。

要求：`isabelle` 在 `PATH` 中可用。

卸载：

```bash
make uninstall-zed-native
```

### 快捷键与可视输出

默认快捷键：

- `Alt-Shift-I`：检查当前 Theory（`process_theories`）
- `Alt-Shift-B`：构建当前 worktree（`build -D`）
- `Alt-I`：重跑上一次 Isabelle 任务

单独安装/卸载快捷键：

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

可视输出：

- diagnostics 走 Zed 标准问题视图
- 任务结果显示在 Zed 终端/任务输出
- 目前无自定义 Proof 侧边面板（受限于当前公开 API）

### 手动构建

在仓库根目录执行：

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
cargo build --manifest-path zed-extension/Cargo.toml --target wasm32-wasip2 --release
```

或：

```bash
make release-build
```

### 可选设置示例

示例文件在仓库根目录：

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`
- `examples/zed-keymap-isabelle.json`

### 官方收录预检查

```bash
make zed-official-check
```

该命令会输出提交到 `zed-industries/extensions` 时可用的 `extensions.toml` 片段。

### 发布与打包

```bash
make doctor
make release-package
```

打包产物包含扩展清单与 wasm 文件，以及 `bridge` / `isabelle-zed-lsp` 二进制。

### LSP 代理命令

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

这些命令由 `isabelle-zed-lsp` 的 `workspace/executeCommand` 处理。

## English

### Install and use immediately (no settings edits)

From repository root (`<repo-root>`):

```bash
make install-zed-native
```

The script installs the extension into Zed's local extension directory.
Default mode is `native`, so no manual `settings.json` edits are required.
It also installs Isabelle keybindings into Zed `keymap.json` (with marker block for clean removal).

Requirement: `isabelle` must be on `PATH`.

Uninstall:

```bash
make uninstall-zed-native
```

### Shortcuts and visual output

Default shortcuts:

- `Alt-Shift-I`: check current theory (`process_theories`)
- `Alt-Shift-B`: build current worktree (`build -D`)
- `Alt-I`: rerun latest Isabelle task

Install/uninstall shortcuts only:

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

Visual output:

- diagnostics through standard Zed diagnostics UI
- task output in Zed terminal/task output
- no custom proof side panel with current public API

### Manual build

From repository root:

```bash
cargo build --manifest-path isabelle-lsp/Cargo.toml --release
cargo build --manifest-path zed-extension/Cargo.toml --target wasm32-wasip2 --release
```

Or:

```bash
make release-build
```

### Optional settings examples

Example files are at repository root:

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`
- `examples/zed-keymap-isabelle.json`

### Official registry pre-check

```bash
make zed-official-check
```

This prints the `extensions.toml` snippet for your PR to `zed-industries/extensions`.

### Release and packaging

```bash
make doctor
make release-package
```

The package includes extension manifest + wasm artifact and `bridge` / `isabelle-zed-lsp` binaries.

### LSP proxy commands

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

These are handled in `workspace/executeCommand` by `isabelle-zed-lsp`.
