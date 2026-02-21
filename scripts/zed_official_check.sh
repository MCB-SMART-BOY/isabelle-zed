#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$repo_root/zed-extension/extension.toml"
root_license="$repo_root/LICENSE"
ext_license="$repo_root/zed-extension/LICENSE"

if [ ! -f "$manifest" ]; then
  echo "missing manifest: $manifest" >&2
  exit 1
fi

extension_id="$(awk -F'"' '/^id = / {print $2; exit}' "$manifest")"
version="$(awk -F'"' '/^version = / {print $2; exit}' "$manifest")"

if [ -z "$extension_id" ] || [ -z "$version" ]; then
  echo "failed to read id/version from $manifest" >&2
  exit 1
fi

if [[ ! "$extension_id" =~ ^[a-z0-9-]+$ ]]; then
  echo "invalid extension id '$extension_id': must match ^[a-z0-9-]+$" >&2
  exit 1
fi

if [[ "$extension_id" == zed-* || "$extension_id" == *-zed ]]; then
  echo "invalid extension id '$extension_id': should not start with 'zed-' or end with '-zed'" >&2
  exit 1
fi

for file in "$root_license" "$ext_license"; do
  if [ ! -f "$file" ]; then
    echo "missing license file: $file" >&2
    exit 1
  fi

  # Basic check aligned with accepted licenses in Zed registry validators.
  if ! grep -Eiq "(MIT License|Apache License|BSD|GNU GENERAL PUBLIC LICENSE|GNU LESSER GENERAL PUBLIC LICENSE|zlib)" "$file"; then
    echo "license file does not appear to match an accepted license: $file" >&2
    exit 1
  fi

done

remote_extensions_toml_url="https://raw.githubusercontent.com/zed-industries/extensions/main/extensions.toml"
tmpfile="$(mktemp)"
trap 'rm -f "$tmpfile"' EXIT

if curl -fsSL "$remote_extensions_toml_url" -o "$tmpfile"; then
  if grep -q "^\[$extension_id\]" "$tmpfile"; then
    echo "extension id '$extension_id' already exists in zed-industries/extensions" >&2
    exit 1
  fi
  echo "[ok] extension id '$extension_id' is not currently listed in official registry"
else
  echo "[warn] could not fetch official extensions.toml for duplicate ID check"
fi

echo "[ok] manifest id/version format passes"
echo "[ok] required license files present"

echo
echo "Suggested entry for zed-industries/extensions/extensions.toml:"
echo "[$extension_id]"
echo "submodule = \"extensions/$extension_id\""
echo "path = \"zed-extension\""
echo "version = \"$version\""
