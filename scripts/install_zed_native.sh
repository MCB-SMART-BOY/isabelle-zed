#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

resolve_default_extensions_dir() {
  local os
  os="$(uname -s)"
  case "$os" in
    Linux)
      echo "$HOME/.local/share/zed/extensions/installed"
      ;;
    Darwin)
      echo "$HOME/Library/Application Support/Zed/extensions/installed"
      ;;
    *)
      echo ""
      ;;
  esac
}

extensions_dir="${ISABELLE_ZED_EXTENSIONS_DIR:-$(resolve_default_extensions_dir)}"
if [ -z "$extensions_dir" ]; then
  echo "unsupported platform for automatic Zed extension install: $(uname -s)" >&2
  echo "Set ISABELLE_ZED_EXTENSIONS_DIR manually and retry." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required" >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required" >&2
  exit 1
fi

if ! rustup target list --installed | grep -qx "wasm32-wasip2"; then
  echo "Installing Rust target wasm32-wasip2..."
  rustup target add wasm32-wasip2
fi

extension_id="$(awk -F'"' '/^id = / {print $2; exit}' "$repo_root/zed-extension/extension.toml")"
if [ -z "$extension_id" ]; then
  echo "failed to read extension id from zed-extension/extension.toml" >&2
  exit 1
fi

echo "Building extension wasm (release)..."
cargo build --manifest-path "$repo_root/zed-extension/Cargo.toml" --target wasm32-wasip2 --release

wasm_src="$repo_root/zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm"
if [ ! -f "$wasm_src" ]; then
  echo "extension wasm artifact not found: $wasm_src" >&2
  exit 1
fi

dest_dir="$extensions_dir/$extension_id"
mkdir -p "$extensions_dir"
rm -rf "$dest_dir"
mkdir -p "$dest_dir"

cp "$repo_root/zed-extension/extension.toml" "$dest_dir/"
cp "$wasm_src" "$dest_dir/extension.wasm"
cp -R "$repo_root/zed-extension/languages" "$dest_dir/languages"

if [ -d "$repo_root/zed-extension/grammars" ]; then
  cp -R "$repo_root/zed-extension/grammars" "$dest_dir/grammars"
fi

echo "Zed extension installed to: $dest_dir"
if command -v isabelle >/dev/null 2>&1; then
  echo "isabelle command detected: native mode is ready."
else
  echo "warning: 'isabelle' not found in PATH. native mode will not start until PATH is fixed." >&2
fi

echo "Restart Zed (or reload extensions) and open a .thy file."
