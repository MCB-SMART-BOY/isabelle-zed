#!/usr/bin/env bash
set -euo pipefail

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

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
extensions_dir="${ISABELLE_ZED_EXTENSIONS_DIR:-$(resolve_default_extensions_dir)}"

if [ -z "$extensions_dir" ]; then
  echo "unsupported platform for automatic Zed extension uninstall: $(uname -s)" >&2
  echo "Set ISABELLE_ZED_EXTENSIONS_DIR manually and retry." >&2
  exit 1
fi

extension_id="$(awk -F'"' '/^id = / {print $2; exit}' "$repo_root/zed-extension/extension.toml")"
if [ -z "$extension_id" ]; then
  echo "failed to read extension id from zed-extension/extension.toml" >&2
  exit 1
fi

extension_dir="$extensions_dir/$extension_id"

if [ ! -d "$extension_dir" ]; then
  echo "extension is not installed at: $extension_dir"
  exit 0
fi

rm -rf "$extension_dir"
echo "Removed Zed extension: $extension_dir"
echo "Restart Zed (or reload extensions) to apply changes."
