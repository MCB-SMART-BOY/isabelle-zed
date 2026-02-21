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

## Quick start (usable in Zed now)

### 1. Build release artifacts

```bash
make release-build
```

### 2. Install as a dev extension in Zed

1. Open Zed command palette.
2. Run `zed: extensions`.
3. Click `Install Dev Extension`.
4. Select `.../isabelle-zed/zed-extension`.

### 3. Configure Zed settings

- Native mode example: `examples/zed-settings-native.json`
- Bridge mock example: `examples/zed-settings-bridge-mock.json`

Copy the JSON content into your Zed `settings.json`.

### 4. Open a `.thy` file

If native mode is active and `isabelle` is on `PATH`, diagnostics/hover are provided by Isabelle.

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
