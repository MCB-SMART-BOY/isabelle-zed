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
如果仓库里缺少 `zed-extension/grammars/isabelle.wasm`，先执行：

```bash
make build-isabelle-grammar
```

卸载：

```bash
make uninstall-zed-native
```

### 快捷键与可视输出

默认快捷键：

- `Alt-Shift-I`：检查当前 Theory（`process_theories`）
- `Alt-Shift-B`：构建当前 worktree（`build -D`）
- `Alt-I`：重跑上一次 Isabelle 任务
- `F8`：备用检查快捷键（避免系统占用 `Alt-Shift`）
- `F9`：备用构建快捷键（避免系统占用 `Alt-Shift`）
- `F7`：备用重跑快捷键

单独安装/卸载快捷键：

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

补充任务（从任务面板运行）：

- `isabelle: check theory (process_theories -D, prompt)`：输入 Theory 名称（不含 `.thy`）
- `isabelle: build session (build -D, prompt)`：输入 Session 名称

说明：Zed 任务变量目前无法自动获取当前 Theory 名称，因此默认快捷键仍以 worktree/session 上下文执行。

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

Bridge 模式默认 socket：

- `/tmp/isabelle-<worktree-id>.sock`（可通过 `bridge_socket` 覆盖）

Bridge 自动拉起配置说明：

- 出于安全原因，bridge 自动拉起参数不再从工作区 `settings.json` 注入到 LSP 进程。
- 如需自动拉起 bridge，请在启动 Zed 前设置：
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

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
同时包含根目录 `LICENSE`、`docs/CHANGELOG.md` 和 `zed-extension/grammars/isabelle.wasm`。

### LSP 代理命令

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

这些命令由 `isabelle-zed-lsp` 的 `workspace/executeCommand` 处理。
`stop_session` 会暂停后续 push/check/hover 请求；`start_session` 会恢复并触发一次检查。

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
If `zed-extension/grammars/isabelle.wasm` is missing, run:

```bash
make build-isabelle-grammar
```

Uninstall:

```bash
make uninstall-zed-native
```

### Shortcuts and visual output

Default shortcuts:

- `Alt-Shift-I`: check current theory (`process_theories`)
- `Alt-Shift-B`: build current worktree (`build -D`)
- `Alt-I`: rerun latest Isabelle task
- `F8`: fallback check shortcut (when `Alt-Shift` is intercepted by OS)
- `F9`: fallback build shortcut (when `Alt-Shift` is intercepted by OS)
- `F7`: fallback rerun shortcut

Install/uninstall shortcuts only:

```bash
make install-zed-shortcuts
make uninstall-zed-shortcuts
```

Extra tasks (run from the task palette):

- `isabelle: check theory (process_theories -D, prompt)`: enter the theory name (without `.thy`)
- `isabelle: build session (build -D, prompt)`: enter the session name

Note: Zed task variables do not expose the current theory name, so the default shortcut still runs in the worktree/session context.

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

Bridge mode default socket:

- `/tmp/isabelle-<worktree-id>.sock` (override via `bridge_socket`)

Bridge autostart configuration note:

- For security reasons, bridge autostart settings are no longer injected from workspace `settings.json`.
- To autostart bridge, set these environment variables before launching Zed:
  - `ISABELLE_BRIDGE_AUTOSTART_CMD`
  - `ISABELLE_BRIDGE_AUTOSTART_TIMEOUT_MS`

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
It also includes root `LICENSE`, `docs/CHANGELOG.md`, and `zed-extension/grammars/isabelle.wasm`.

### LSP proxy commands

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

These are handled in `workspace/executeCommand` by `isabelle-zed-lsp`.
`stop_session` pauses subsequent push/check/hover requests; `start_session` resumes and triggers one check.
