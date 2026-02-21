#!/usr/bin/env bash
set -euo pipefail

resolve_default_keymap_path() {
  local os
  os="$(uname -s)"
  case "$os" in
    Linux)
      echo "$HOME/.config/zed/keymap.json"
      ;;
    Darwin)
      echo "$HOME/Library/Application Support/Zed/keymap.json"
      ;;
    *)
      echo ""
      ;;
  esac
}

keymap_path="${ISABELLE_ZED_KEYMAP_PATH:-$(resolve_default_keymap_path)}"
if [ -z "$keymap_path" ]; then
  echo "unsupported platform for automatic keymap uninstall: $(uname -s)" >&2
  echo "Set ISABELLE_ZED_KEYMAP_PATH manually and retry." >&2
  exit 1
fi

if [ ! -f "$keymap_path" ]; then
  echo "keymap file does not exist: $keymap_path"
  exit 0
fi

python3 - "$keymap_path" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
start_marker = "// >>> isabelle shortcuts >>>"
end_marker = "// <<< isabelle shortcuts <<<"

text = path.read_text(encoding="utf-8")
pattern = re.compile(
    rf"\n?\s*{re.escape(start_marker)}.*?{re.escape(end_marker)}\s*,?\n?",
    re.DOTALL,
)
new_text, count = pattern.subn("\n", text)

if count == 0:
    print(f"Isabelle shortcuts were not found in keymap: {path}")
    sys.exit(0)

if not new_text.endswith("\n"):
    new_text += "\n"

path.write_text(new_text, encoding="utf-8")
print(f"Removed Isabelle shortcuts from keymap: {path}")
PY

