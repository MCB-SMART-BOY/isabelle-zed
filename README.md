# Isabelle-Zed

Isabelle support for Zed with two runtime modes:

```text
native mode (recommended):
  Zed Extension (WASM) -> isabelle vscode_server

bridge mode (integration/testing):
  Zed Extension (WASM)
    -> isabelle-zed-lsp (Rust LSP proxy)
    -> bridge (Rust NDJSON bridge)
    -> scala-adapter (mock or Isabelle-backed)
```

## Current status

- Zed extension registers `.thy` and launches a language server.
- Native mode is real Isabelle-backed through `isabelle vscode_server`.
- Bridge mode supports diagnostics + hover through the NDJSON protocol.
- Mock end-to-end checks exist for both NDJSON and LSP paths.

## Zero-config install (recommended)

Install extension into Zed and use native mode directly (no `settings.json` edits):

```bash
make install-zed-native
```

Then restart Zed (or reload extensions) and open a `.thy` file.

Requirement: `isabelle` must be available on your shell `PATH`.

Uninstall the extension:

```bash
make uninstall-zed-native
```

## Optional configuration examples

- Native custom settings: `examples/zed-settings-native.json`
- Bridge mock settings: `examples/zed-settings-bridge-mock.json`

Use these only if you need custom behavior.

## Submit to official Zed extensions

Run the submission pre-check:

```bash
make zed-official-check
```

Then open a PR to `zed-industries/extensions` with:

1. A new git submodule pointing to this repository at `extensions/isabelle`.
2. A new entry in their `extensions.toml`:

```toml
[isabelle]
submodule = "extensions/isabelle"
path = "zed-extension"
version = "<version-from-zed-extension/extension.toml>"
```

Detailed command-by-command guide:

- `docs/official-submission.md`

## Local install helpers

### Doctor check

```bash
make doctor
```

### Install local binaries (`bridge`, `isabelle-zed-lsp`)

```bash
make install-local
```

By default this installs into `~/.local/bin`.
Set `ISABELLE_ZED_BIN_DIR` to install elsewhere.

## Release packaging

Create a distributable tarball in `dist/`:

```bash
make release-package
```

This generates:

- `dist/isabelle-zed-v<version>-<platform>.tar.gz`
- `dist/isabelle-zed-v<version>-<platform>.tar.gz.sha256`

GitHub tag pushes like `v0.1.0` trigger `.github/workflows/release.yml` to build and attach the package to a release.

## Bridge mock workflow (for integration testing)

Start bridge mock server:

```bash
make bridge-mock-up
```

Run LSP end-to-end assertion:

```bash
make mock-lsp-e2e
```

Stop bridge mock server:

```bash
make bridge-mock-down
```

## Other useful commands

```bash
make bridge-test
make lsp-test
make zed-check
make native-lsp-smoke
```

## Real Isabelle-backed notes

- Native mode is the default path for daily use in Zed.
- Bridge mode remains available for custom protocol experiments and CI.
