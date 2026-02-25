# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Added

- local regression command `make spawn-e2e-ndjson` to validate bridge `--adapter-command` startup path end-to-end.

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
