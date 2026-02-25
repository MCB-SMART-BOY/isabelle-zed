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
  echo "unsupported platform for automatic keymap install: $(uname -s)" >&2
  echo "Set ISABELLE_ZED_KEYMAP_PATH manually and retry." >&2
  exit 1
fi

mkdir -p "$(dirname "$keymap_path")"

python3 - "$keymap_path" <<'PY'
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
start_marker = "// >>> isabelle shortcuts >>>"
end_marker = "// <<< isabelle shortcuts <<<"

block = """  // >>> isabelle shortcuts >>>
  {
    \"context\": \"Workspace\",
    \"bindings\": {
      \"alt-shift-i\": [
        \"task::Spawn\",
        {
          \"task_name\": \"isabelle: check current theory (process_theories)\",
          \"reveal_target\": \"center\"
        }
      ],
      \"f8\": [
        \"task::Spawn\",
        {
          \"task_name\": \"isabelle: check current theory (process_theories)\",
          \"reveal_target\": \"center\"
        }
      ],
      \"alt-shift-b\": [
        \"task::Spawn\",
        {
          \"task_name\": \"isabelle: build worktree session (build -D)\",
          \"reveal_target\": \"center\"
        }
      ],
      \"f9\": [
        \"task::Spawn\",
        {
          \"task_name\": \"isabelle: build worktree session (build -D)\",
          \"reveal_target\": \"center\"
        }
      ],
      \"alt-i\": [\"task::Rerun\", { \"reevaluate_context\": true }],
      \"f7\": [\"task::Rerun\", { \"reevaluate_context\": true }]
    }
  }
  // <<< isabelle shortcuts <<<"""

if not path.exists():
    path.write_text("[\n" + block + "\n]\n", encoding="utf-8")
    print(f"Installed Isabelle shortcuts into new keymap: {path}")
    sys.exit(0)

text = path.read_text(encoding="utf-8")
pattern = re.compile(
    rf"\n?\s*{re.escape(start_marker)}.*?{re.escape(end_marker)}\s*,?\n?",
    re.DOTALL,
)
text = pattern.sub("\n", text)

closing_index = text.rfind("]")
if closing_index == -1:
    raise SystemExit(f"Keymap file is not an array (missing closing ']'): {path}")

before = text[:closing_index].rstrip()
after = text[closing_index:]

if before.endswith("["):
    new_text = before + "\n" + block + "\n" + after.lstrip()
else:
    if not before.endswith(","):
        before += ","
    new_text = before + "\n" + block + "\n" + after.lstrip()

if not new_text.endswith("\n"):
    new_text += "\n"

path.write_text(new_text, encoding="utf-8")
print(f"Installed Isabelle shortcuts into existing keymap: {path}")
PY
