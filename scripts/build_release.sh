#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required" >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to install/check wasm32-wasip2 target" >&2
  exit 1
fi

if ! rustup target list --installed | grep -qx "wasm32-wasip2"; then
  echo "Installing Rust target wasm32-wasip2..."
  rustup target add wasm32-wasip2
fi

echo "Building bridge (release)..."
cargo build --manifest-path "$repo_root/bridge/Cargo.toml" --release

echo "Building isabelle-zed-lsp (release)..."
cargo build --manifest-path "$repo_root/isabelle-lsp/Cargo.toml" --release

echo "Building Zed extension wasm (release)..."
cargo build --manifest-path "$repo_root/zed-extension/Cargo.toml" --target wasm32-wasip2 --release

echo
echo "Build complete:"
echo "  bridge:            $repo_root/bridge/target/release/bridge"
echo "  isabelle-zed-lsp:  $repo_root/isabelle-lsp/target/release/isabelle-zed-lsp"
echo "  extension wasm:    $repo_root/zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm"
