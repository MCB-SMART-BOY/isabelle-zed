# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Fixed

- hardened bridge autostart execution by parsing `ISABELLE_BRIDGE_AUTOSTART_CMD` into argv and spawning directly (removed `bash -lc` execution path).
- ignored bridge autostart environment overrides from extension settings, so workspace LSP settings cannot inject autostart commands.
- `isabelle-zed-lsp` bridge transport now validates response `id` and ignores out-of-order/unmatched responses.
- normalized bridge/adapter diagnostic examples and mock payloads to 1-based positions (`line`/`col`), consistent with documented protocol semantics.
- build task `isabelle: build worktree session (build -D)` now executes `isabelle build -D` when `ROOT/ROOTS` exists, with fallback to `process_theories -D`.
- bridge now flushes pending debounced `document.push` messages before session shutdown on input EOF.
- bridge socket startup now refuses to delete pre-existing non-socket paths.
- root-level `cargo test` now works by adding a placeholder crate target (`src/lib.rs`).

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
