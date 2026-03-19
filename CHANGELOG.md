# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Changed

- bridge default real mode now starts a built-in Rust real-adapter subprocess (`--real-adapter`) instead of depending on Scala/sbt fallback discovery.
- build/release/install/check automation is now unified under Rust `xtask` commands and wired into GitHub workflows/documentation.
- repository Cargo setup is now a single root workspace (`bridge` / `isabelle-lsp` / `zed-extension` / `xtask`) with shared dependency version management.
- workspace command style is now standardized on package-based Cargo entrypoints (`cargo -p ...` / `cargo run -p isabelle-zed-xtask -- ...`) across CI and docs.
- added `docs/project-structure.md` to document repository layering and governance conventions.
- xtask implementation is now split into layered modules (`cli` / `common` / `commands/*`) instead of a single monolithic `main.rs`.
- fixed `xtask doctor` artifact checks to use workspace-level `target/...` paths consistently.
- `isabelle-lsp` is now split into `main` + `transport` + `diagnostics` + `autostart` modules to reduce single-file coupling.
- `isabelle-lsp` push/debounce worker logic is now split into a dedicated `push` module.
- `zed-extension` now isolates ROOT/ROOTS session parsing and auto-logic selection in `session_logic.rs`.
- bridge `--adapter-command` now parses argv and executes directly, removing the `bash -lc` execution path.
- CI/release workflows now enforce workspace-wide quality gates (`fmt`, `clippy -D warnings`, `test`, wasm target check) before E2E/package steps.
- bridge mode transport is now endpoint-based (`unix:/path` or `tcp:host:port`) across bridge/lsp/extension wiring.
- bridge integration test path for `--adapter-command` no longer depends on `python3`.
- `isabelle.start_session` now logs “started” only after a successful start/check flow.
- documentation and runtime hints now use direct `cargo`/`xtask` commands (no `make` indirection).
- bridge real-adapter `markup` now returns syntax-level hover context (identifier/range/line text) instead of a placeholder message.
- `isabelle-zed-lsp` now converts hover positions from LSP 0-based to bridge 1-based before forwarding.
- bridge real-adapter now supports repeatable `--session-dir` (mapped to `process_theories -d`) and auto-adds the checked file's parent directory to session lookup paths.
- bridge real-adapter now caches diagnostics by in-memory document content/version, avoiding redundant `process_theories` runs for unchanged text.
- added `cargo run -p isabelle-zed-xtask -- bridge-real-smoke` for local real-adapter smoke validation against malformed theory input.
- bridge now supports `--tcp <host:port>` listener mode in addition to Unix `--socket`.
- added cross-platform `cargo run -p isabelle-zed-xtask -- mock-lsp-e2e-tcp` and wired it into CI (Linux + Windows) to validate endpoint transport path.

### Removed

- removed `scala-adapter/` (Scala codebase) from the runtime/tooling path.
- removed non-Rust helper scripts under `scripts/` (shell/python/powershell), replaced by Rust tooling commands.
- removed root `Makefile` alias layer.
- removed redundant root placeholder crate (`src/lib.rs`) and member lockfile duplication (`zed-extension/Cargo.lock`).

## [0.2.3] - 2026-03-05

### Added

- conflict-aware shortcut installer now auto-selects non-conflicting key candidates and supports `ISABELLE_ZED_RESERVED_KEYS`.

### Fixed

- hardened bridge autostart execution by parsing `ISABELLE_BRIDGE_AUTOSTART_CMD` into argv and spawning directly (removed `bash -lc` execution path).
- ignored bridge autostart environment overrides from extension settings, so workspace LSP settings cannot inject autostart commands.
- `isabelle-zed-lsp` bridge transport now validates response `id` and ignores out-of-order/unmatched responses.
- normalized bridge/adapter diagnostic examples and mock payloads to 1-based positions (`line`/`col`), consistent with documented protocol semantics.
- build task `isabelle: build worktree session (build -D)` now runs `isabelle build -D` directly (shell-agnostic across fish/bash/zsh).
- bridge now flushes pending debounced `document.push` messages before session shutdown on input EOF.
- bridge socket startup now refuses to delete pre-existing non-socket paths.
- root-level `cargo test` now works by adding a placeholder crate target (`src/lib.rs`).
- `isabelle.start_session` / `isabelle.stop_session` now control effective request flow (stop pauses push/check/hover; start resumes and triggers one check).
- check/build task invocations now prefer reusing an existing terminal session instead of opening a new terminal each run.
- release packaging now performs preflight file checks before heavy builds and emits timestamped stage logs for clearer CI progress.

## [0.2.2] - 2026-02-25

### Added

- local regression command `make spawn-e2e-ndjson` to validate bridge `--adapter-command` startup path end-to-end.
- grammar build command `make build-isabelle-grammar` to generate `zed-extension/grammars/isabelle.wasm`.

### Fixed

- native install/package scripts now fail fast when `zed-extension/grammars/isabelle.wasm` is missing (no more silent broken installs).
- release/install artifacts now include Isabelle grammar wasm, preventing `failed to load language Isabelle` at runtime.
- Scala adapter `Await.ready` call now uses the correct `atMost` parameter so `make scala-test` compiles and runs cleanly.

## [0.2.1] - 2026-02-25

### Fixed

- bridge real mode startup now launches the Scala adapter program path correctly (via local `scala-adapter` fallback or `--adapter-command`), instead of only `isabelle scala`.
- LSP bridge request path now has timeout + reconnect retry to prevent indefinite hangs.
- diagnostic range conversion now maps bridge/adapter 1-based positions to LSP 0-based positions.
- bridge now consumes child `stderr` to avoid potential pipe backpressure deadlocks.
- debounce queue no longer resets timer for stale `document.push` versions.
- diagnostics are published by each diagnostic `uri` (cross-file diagnostics preserved).

### Changed

- Scala adapter request handling now enforces bounded in-flight concurrency and safer shutdown waiting behavior.
- CI now includes an E2E job for bridge `--adapter-command` startup path.
- release package now includes root `LICENSE`.

## [0.2.0] - 2026-02-22

- Initial public package and CI baseline.
