#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist_dir="$repo_root/dist"

log() {
  echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*"
}

require_file() {
  local path="$1"
  if [ ! -f "$path" ]; then
    echo "missing required file: $path" >&2
    exit 1
  fi
}

platform=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --platform)
      platform="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [ -z "$platform" ]; then
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  platform="${os}-${arch}"
fi

log "Running release preflight checks..."
require_file "$repo_root/README.md"
require_file "$repo_root/CHANGELOG.md"
require_file "$repo_root/LICENSE"
require_file "$repo_root/zed-extension/extension.toml"
require_file "$repo_root/zed-extension/README.md"
require_file "$repo_root/zed-extension/Cargo.toml"
if [ ! -d "$repo_root/zed-extension/languages" ]; then
  echo "missing required directory: $repo_root/zed-extension/languages" >&2
  exit 1
fi
if [ ! -d "$repo_root/examples" ]; then
  echo "missing required directory: $repo_root/examples" >&2
  exit 1
fi

version="$(awk -F'"' '/^version = / {print $2; exit}' "$repo_root/zed-extension/extension.toml")"
if [ -z "$version" ]; then
  echo "failed to read version from zed-extension/extension.toml" >&2
  exit 1
fi

log "Building release artifacts..."
"$repo_root/scripts/build_release.sh"

package_root="isabelle-zed-v${version}-${platform}"
package_dir="$dist_dir/$package_root"
archive_path="$dist_dir/${package_root}.tar.gz"

log "Preparing package layout: $package_root"
rm -rf "$package_dir"
mkdir -p "$package_dir/bin" "$package_dir/zed-extension" "$package_dir/examples" "$package_dir/docs"

install -m 0755 "$repo_root/bridge/target/release/bridge" "$package_dir/bin/bridge"
install -m 0755 "$repo_root/isabelle-lsp/target/release/isabelle-zed-lsp" "$package_dir/bin/isabelle-zed-lsp"

cp "$repo_root/zed-extension/extension.toml" "$package_dir/zed-extension/"
cp "$repo_root/zed-extension/Cargo.toml" "$package_dir/zed-extension/"
cp "$repo_root/zed-extension/README.md" "$package_dir/zed-extension/"
cp -R "$repo_root/zed-extension/src" "$package_dir/zed-extension/src"
cp -R "$repo_root/zed-extension/languages" "$package_dir/zed-extension/languages"
if [ ! -f "$repo_root/zed-extension/grammars/isabelle.wasm" ]; then
  echo "missing grammar artifact: $repo_root/zed-extension/grammars/isabelle.wasm" >&2
  echo "build it first: $repo_root/scripts/build_isabelle_grammar.sh" >&2
  exit 1
fi
cp -R "$repo_root/zed-extension/grammars" "$package_dir/zed-extension/grammars"
cp "$repo_root/zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm" \
  "$package_dir/zed-extension/extension.wasm"

cp "$repo_root/examples/zed-settings-native.json" "$package_dir/examples/"
cp "$repo_root/examples/zed-settings-bridge-mock.json" "$package_dir/examples/"
cp "$repo_root/examples/zed-keymap-isabelle.json" "$package_dir/examples/"
cp "$repo_root/README.md" "$package_dir/docs/README.md"
cp "$repo_root/CHANGELOG.md" "$package_dir/docs/CHANGELOG.md"
cp "$repo_root/LICENSE" "$package_dir/LICENSE"

log "Creating tarball and checksum..."
rm -f "$archive_path"
tar -C "$dist_dir" -czf "$archive_path" "$package_root"

(
  cd "$dist_dir"
  sha256sum "$(basename "$archive_path")" > "$(basename "$archive_path").sha256"
)

log "Release package created:"
echo "  $archive_path"
echo "  $archive_path.sha256"
