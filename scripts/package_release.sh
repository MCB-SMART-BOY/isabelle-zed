#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist_dir="$repo_root/dist"

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

version="$(awk -F'"' '/^version = / {print $2; exit}' "$repo_root/zed-extension/extension.toml")"
if [ -z "$version" ]; then
  echo "failed to read version from zed-extension/extension.toml" >&2
  exit 1
fi

"$repo_root/scripts/build_release.sh"

package_root="isabelle-zed-v${version}-${platform}"
package_dir="$dist_dir/$package_root"
archive_path="$dist_dir/${package_root}.tar.gz"

rm -rf "$package_dir"
mkdir -p "$package_dir/bin" "$package_dir/zed-extension" "$package_dir/examples" "$package_dir/docs"

install -m 0755 "$repo_root/bridge/target/release/bridge" "$package_dir/bin/bridge"
install -m 0755 "$repo_root/isabelle-lsp/target/release/isabelle-zed-lsp" "$package_dir/bin/isabelle-zed-lsp"

cp "$repo_root/zed-extension/extension.toml" "$package_dir/zed-extension/"
cp "$repo_root/zed-extension/Cargo.toml" "$package_dir/zed-extension/"
cp "$repo_root/zed-extension/README.md" "$package_dir/zed-extension/"
cp -R "$repo_root/zed-extension/src" "$package_dir/zed-extension/src"
cp -R "$repo_root/zed-extension/languages" "$package_dir/zed-extension/languages"
cp "$repo_root/zed-extension/target/wasm32-wasip2/release/isabelle_zed_extension.wasm" \
  "$package_dir/zed-extension/isabelle_zed_extension.wasm"

cp "$repo_root/examples/zed-settings-native.json" "$package_dir/examples/"
cp "$repo_root/examples/zed-settings-bridge-mock.json" "$package_dir/examples/"
cp "$repo_root/README.md" "$package_dir/docs/README.md"

rm -f "$archive_path"
tar -C "$dist_dir" -czf "$archive_path" "$package_root"

(
  cd "$dist_dir"
  sha256sum "$(basename "$archive_path")" > "$(basename "$archive_path").sha256"
)

echo "Release package created:"
echo "  $archive_path"
echo "  $archive_path.sha256"
