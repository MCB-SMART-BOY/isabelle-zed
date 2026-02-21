#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
install_bin_dir="${ISABELLE_ZED_BIN_DIR:-$HOME/.local/bin}"

mkdir -p "$install_bin_dir"

"$repo_root/scripts/build_release.sh"

install -m 0755 "$repo_root/bridge/target/release/bridge" "$install_bin_dir/bridge"
install -m 0755 "$repo_root/isabelle-lsp/target/release/isabelle-zed-lsp" "$install_bin_dir/isabelle-zed-lsp"

echo "Installed binaries to: $install_bin_dir"
echo "  - bridge"
echo "  - isabelle-zed-lsp"

echo
if [[ ":${PATH}:" != *":$install_bin_dir:"* ]]; then
  echo "Add this directory to PATH if needed:"
  echo "  export PATH=\"$install_bin_dir:\$PATH\""
fi

echo
echo "Next step: install dev extension in Zed from $repo_root/zed-extension"
