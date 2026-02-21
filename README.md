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

卸载：

```bash
make uninstall-zed-native
```

### 可选配置示例

以下文件位于仓库根目录：

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`

仅在你需要覆盖默认行为时使用。

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

### 常用命令

```bash
make bridge-test
make lsp-test
make zed-check
make native-lsp-smoke
```

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

Uninstall:

```bash
make uninstall-zed-native
```

### Optional settings examples

These files are at repository root:

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`

Use them only when you need custom overrides.

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

### Common commands

```bash
make bridge-test
make lsp-test
make zed-check
make native-lsp-smoke
```
