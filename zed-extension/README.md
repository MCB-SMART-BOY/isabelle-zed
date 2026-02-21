# Zed Isabelle Extension

This extension can run in two modes:

- `native` (recommended): run `isabelle vscode_server` directly.
- `bridge`: run `isabelle-zed-lsp`, which forwards to `bridge`.

## Install for immediate use (no settings edits)

From repository root:

```bash
make install-zed-native
```

This installs the extension directly into Zed's local `installed` extensions directory.
Default runtime mode is `native`, so no `settings.json` changes are required.

Requirement: `isabelle` must be on `PATH`.

Uninstall:

```bash
make uninstall-zed-native
```

## Build manually

```bash
cargo build --manifest-path ../isabelle-lsp/Cargo.toml --release
cargo build --manifest-path Cargo.toml --target wasm32-wasip2 --release
```

Or from repo root:

```bash
make release-build
```

## Optional settings examples

Use these only if you need custom overrides:

- `examples/zed-settings-native.json`
- `examples/zed-settings-bridge-mock.json`

## Official registry submission

From repo root, run:

```bash
make zed-official-check
```

The script prints the `extensions.toml` snippet to use in your PR to
`zed-industries/extensions`.

## Release and packaging notes

From repo root:

```bash
make doctor
make release-package
```

The package includes extension manifest + wasm artifact and `bridge`/`isabelle-zed-lsp` binaries.

## Commands exposed by the LSP proxy

- `isabelle.start_session`
- `isabelle.stop_session`
- `isabelle.run_check`

These are handled in `workspace/executeCommand` by `isabelle-zed-lsp`.
