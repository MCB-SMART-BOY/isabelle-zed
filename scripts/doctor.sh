#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ok() {
  echo "[ok] $1"
}

warn() {
  echo "[warn] $1"
}

fail() {
  echo "[fail] $1" >&2
  exit 1
}

check_cmd() {
  local cmd="$1"
  local required="$2"
  if command -v "$cmd" >/dev/null 2>&1; then
    ok "command '$cmd' is available"
  elif [ "$required" = "required" ]; then
    fail "required command '$cmd' is missing"
  else
    warn "optional command '$cmd' is missing"
  fi
}

echo "Running Isabelle-Zed doctor"

check_cmd cargo required
check_cmd rustup required
check_cmd python3 required
check_cmd isabelle optional
check_cmd sbt optional

if rustup target list --installed | grep -qx "wasm32-wasip2"; then
  ok "Rust target wasm32-wasip2 is installed"
else
  warn "Rust target wasm32-wasip2 is not installed (run: rustup target add wasm32-wasip2)"
fi

if [ -x "$repo_root/bridge/target/release/bridge" ]; then
  ok "bridge release binary is present"
else
  warn "bridge release binary not found (run: make release-build)"
fi

if [ -x "$repo_root/isabelle-lsp/target/release/isabelle-zed-lsp" ]; then
  ok "isabelle-zed-lsp release binary is present"
else
  warn "isabelle-zed-lsp release binary not found (run: make release-build)"
fi

if [ -f "$repo_root/zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm" ]; then
  ok "extension wasm artifact is present"
else
  warn "extension wasm artifact not found (run: make release-build)"
fi

if command -v isabelle >/dev/null 2>&1; then
  if isabelle version >/dev/null 2>&1; then
    ok "isabelle command runs successfully"
  else
    warn "isabelle command exists but 'isabelle version' failed"
  fi
fi

echo "Doctor check complete"
