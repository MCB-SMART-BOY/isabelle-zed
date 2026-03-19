# Project Structure / 项目结构说明

## 中文（简体）

本仓库采用单一 Cargo workspace 结构，根目录 `Cargo.toml` 统一管理成员、依赖版本和发布 profile。

### 目录分层

- `bridge/`
  Rust NDJSON bridge（编辑器与 adapter 后端之间）。
- `isabelle-lsp/`
  Rust LSP 代理（Zed LSP 协议侧）。
- `zed-extension/`
  Zed 扩展（WASM）。
- `xtask/`
  Rust 工具入口，替代历史 shell/python/powershell 脚本。
  内部按分层组织：
  - `src/main.rs` 仅负责参数解析和命令分发。
  - `src/common.rs` 放公共工具函数（路径、命令执行、manifest 读取等）。
  - `src/commands/*` 按命令域拆分实现。
- `docs/`
  文档（官方提交、结构说明等）。
- `examples/`
  用户可选配置示例。
- `dist/`
  打包产物输出目录（已在 `.gitignore`）。
- `target/`
  workspace 统一构建产物目录（已在 `.gitignore`）。

### 命令约定

- 业务 crate 构建测试优先用 `cargo -p <package>`：
  - `isabelle-bridge`
  - `isabelle-zed-lsp`
  - `isabelle-zed-extension`
  - `isabelle-zed-xtask`
- 项目级操作（安装、打包、e2e、doctor）统一走：
  - `cargo run -p isabelle-zed-xtask -- <command>`

### 结构治理原则

- 避免新增跨语言脚本工具链依赖，优先 Rust 实现。
- 避免成员级 `Cargo.lock`，统一由根 workspace 管理（根 `Cargo.lock` 作为单一锁文件）。
- 发布流程入口统一在 `xtask`，`Makefile` 仅做别名层。

## English

The repository uses a single Cargo workspace. The root `Cargo.toml` is the source of truth for members, shared dependency versions, and release profile configuration.

### Layout

- `bridge/`
  Rust NDJSON bridge between editor and adapter backend.
- `isabelle-lsp/`
  Rust LSP proxy.
- `zed-extension/`
  Zed extension (WASM).
- `xtask/`
  Rust task runner replacing legacy shell/python/powershell scripts.
  Internal layering:
  - `src/main.rs` only handles CLI parse + dispatch.
  - `src/common.rs` hosts shared helpers (paths, command exec, manifest readers).
  - `src/commands/*` contains domain-specific command implementations.
- `docs/`
  Documentation (submission guide, structure guide, etc.).
- `examples/`
  Optional user configuration examples.
- `dist/`
  Packaging output directory (gitignored).
- `target/`
  Workspace-level build artifacts (gitignored).

### Command conventions

- Build/test member crates with `cargo -p <package>`.
- Use a single project-operations entrypoint:
  - `cargo run -p isabelle-zed-xtask -- <command>`

### Structure governance

- Prefer Rust implementations over new cross-language script dependencies.
- Keep lockfile ownership at workspace root (single tracked root `Cargo.lock`, no member `Cargo.lock`).
- Keep release/install/e2e flows centralized in `xtask`; `Makefile` remains an alias layer.
