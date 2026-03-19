#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ext_manifest="$repo_root/zed-extension/extension.toml"
out_dir="$repo_root/zed-extension/grammars"
out_file="$out_dir/isabelle.wasm"

for cmd in git clang rustc; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "missing required command: $cmd" >&2
    exit 1
  fi
done

grammar_rev="$({
  awk -F'"' '
    /^\[grammars\.isabelle\]/ { in_section = 1; next }
    in_section && /^\[/ { in_section = 0 }
    in_section && /^repository = / { print $2 > "/tmp/isabelle_grammar_repo_url" }
    in_section && /^rev = / { print $2; exit }
  ' "$ext_manifest"
} || true)"

grammar_repo_url="$(cat /tmp/isabelle_grammar_repo_url 2>/dev/null || true)"
rm -f /tmp/isabelle_grammar_repo_url

if [ -z "$grammar_repo_url" ]; then
  echo "failed to read [grammars.isabelle].repository from $ext_manifest" >&2
  exit 1
fi

if [ -z "$grammar_rev" ]; then
  echo "failed to read [grammars.isabelle].rev from $ext_manifest" >&2
  exit 1
fi

sysroot="$(rustc --print sysroot)"
host="$(rustc -vV | awk '/host:/ {print $2}')"
rust_lld="$sysroot/lib/rustlib/$host/bin/rust-lld"

if [ ! -x "$rust_lld" ]; then
  echo "rust-lld not found at expected path: $rust_lld" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

grammar_repo="$tmp_dir/tree-sitter-isabelle"

echo "Cloning tree-sitter-isabelle ($grammar_rev)..."
git clone --depth 1 --branch "$grammar_rev" "$grammar_repo_url" "$grammar_repo" >/dev/null 2>&1 || {
  # Fall back to detached checkout when shallow branch clone by SHA is unsupported.
  git clone --depth 1 "$grammar_repo_url" "$grammar_repo" >/dev/null 2>&1
  git -C "$grammar_repo" fetch --depth 1 origin "$grammar_rev" >/dev/null 2>&1
  git -C "$grammar_repo" checkout "$grammar_rev" >/dev/null 2>&1
}

mkdir -p "$tmp_dir/include"
cat > "$tmp_dir/include/stdlib.h" <<'H'
#ifndef _STDLIB_H
#define _STDLIB_H
#define NULL ((void*)0)
#endif
H

cat > "$tmp_dir/include/wctype.h" <<'H'
#ifndef _WCTYPE_H
#define _WCTYPE_H
static inline int iswspace(int c) {
  return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == '\v';
}
#endif
H

echo "Compiling Isabelle grammar wasm..."
clang \
  --target=wasm32-unknown-unknown \
  -O2 \
  -fPIC \
  -I"$tmp_dir/include" \
  -I"$grammar_repo/src" \
  -c "$grammar_repo/src/parser.c" \
  -o "$tmp_dir/parser.o"

objects=("$tmp_dir/parser.o")
exports=("--export=tree_sitter_isabelle")

if [ -f "$grammar_repo/src/scanner.c" ]; then
  clang \
    --target=wasm32-unknown-unknown \
    -O2 \
    -fPIC \
    -I"$tmp_dir/include" \
    -I"$grammar_repo/src" \
    -c "$grammar_repo/src/scanner.c" \
    -o "$tmp_dir/scanner.o"
  objects+=("$tmp_dir/scanner.o")
  exports+=(
    "--export=tree_sitter_isabelle_external_scanner_create"
    "--export=tree_sitter_isabelle_external_scanner_destroy"
    "--export=tree_sitter_isabelle_external_scanner_scan"
    "--export=tree_sitter_isabelle_external_scanner_serialize"
    "--export=tree_sitter_isabelle_external_scanner_deserialize"
  )
fi

"$rust_lld" -flavor wasm \
  --shared \
  "${exports[@]}" \
  "${objects[@]}" \
  -o "$tmp_dir/isabelle.wasm"

mkdir -p "$out_dir"
cp "$tmp_dir/isabelle.wasm" "$out_file"

echo "Wrote grammar artifact: $out_file"
