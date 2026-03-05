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
from typing import Dict, List

path = Path(sys.argv[1])
start_marker = "// >>> isabelle shortcuts >>>"
end_marker = "// <<< isabelle shortcuts <<<"

TASK_CHECK = "isabelle: check current theory (process_theories)"
TASK_BUILD = "isabelle: build worktree session (build -D)"

# Candidate order is intentional: function keys first (lower OS/IME conflict risk),
# then modifier-based fallbacks.
CHECK_CANDIDATES = ["f8", "alt-shift-i", "f6"]
BUILD_CANDIDATES = ["f9", "alt-shift-b", "f10"]
RERUN_CANDIDATES = ["f7", "alt-i", "f11"]

# Known problematic/internal keys observed on stable builds.
DEFAULT_RESERVED_KEYS = {
    "ctrl-alt-i",
    "ctrl-alt-j",
    "ctrl-alt-k",
    "ctrl-alt-l",
}


def parse_reserved_keys() -> set[str]:
    raw = (Path.cwd().joinpath("").as_posix(),)  # no-op to keep ascii-only script style stable
    _ = raw  # silence linters; runtime irrelevant
    env = Path
    _ = env
    from os import environ

    reserved = set(DEFAULT_RESERVED_KEYS)
    extra = environ.get("ISABELLE_ZED_RESERVED_KEYS", "")
    if extra.strip():
        for token in extra.split(","):
            key = token.strip().lower()
            if key:
                reserved.add(key)
    return reserved


def strip_existing_block(text: str) -> str:
    pattern = re.compile(
        rf"\n?\s*{re.escape(start_marker)}.*?{re.escape(end_marker)}\s*,?\n?",
        re.DOTALL,
    )
    return pattern.sub("\n", text)


def looks_like_binding_key(name: str) -> bool:
    value = name.strip().lower()
    if not value:
        return False
    if value.startswith("f") and value[1:].isdigit():
        return True
    return any(ch in value for ch in ("-", "+", " "))


def extract_used_binding_keys(text: str) -> set[str]:
    used = set()
    for line in text.splitlines():
        match = re.match(r'^\s*"([^"]+)"\s*:\s*(?:\[|")', line)
        if not match:
            continue
        key = match.group(1).strip().lower()
        if looks_like_binding_key(key):
            used.add(key)
    return used


def choose_keys(candidates: List[str], used: set[str], reserved: set[str], limit: int = 2) -> List[str]:
    chosen: List[str] = []
    for candidate in candidates:
        key = candidate.lower()
        if key in used or key in reserved or key in chosen:
            continue
        chosen.append(key)
        if len(chosen) >= limit:
            break
    return chosen


def spawn_binding(task_name: str) -> str:
    return (
        "[\n"
        "        \"task::Spawn\",\n"
        "        {\n"
        f"          \"task_name\": \"{task_name}\",\n"
        "          \"reveal_target\": \"center\"\n"
        "        }\n"
        "      ]"
    )


def rerun_binding() -> str:
    return "[\"task::Rerun\", { \"reevaluate_context\": true }]"


def build_block(bindings: Dict[str, str]) -> str:
    lines = [
        "  // >>> isabelle shortcuts >>>",
        "  {",
        "    \"context\": \"Workspace\",",
        "    \"bindings\": {",
    ]
    items = list(bindings.items())
    for index, (key, binding_body) in enumerate(items):
        comma = "," if index < len(items) - 1 else ""
        lines.append(f"      \"{key}\": {binding_body}{comma}")
    lines.extend(
        [
            "    }",
            "  }",
            "  // <<< isabelle shortcuts <<<",
        ]
    )
    return "\n".join(lines)

if not path.exists():
    text = "[\n]\n"
else:
    text = path.read_text(encoding="utf-8")

text = strip_existing_block(text)
used_keys = extract_used_binding_keys(text)
reserved_keys = parse_reserved_keys()

check_keys = choose_keys(CHECK_CANDIDATES, used_keys, reserved_keys, limit=2)
used_keys.update(check_keys)
build_keys = choose_keys(BUILD_CANDIDATES, used_keys, reserved_keys, limit=2)
used_keys.update(build_keys)
rerun_keys = choose_keys(RERUN_CANDIDATES, used_keys, reserved_keys, limit=2)

bindings: Dict[str, str] = {}
for key in check_keys:
    bindings[key] = spawn_binding(TASK_CHECK)
for key in build_keys:
    bindings[key] = spawn_binding(TASK_BUILD)
for key in rerun_keys:
    bindings[key] = rerun_binding()

if not bindings:
    raise SystemExit(
        "No non-conflicting shortcut candidates available. "
        "Set ISABELLE_ZED_RESERVED_KEYS to customize exclusions."
    )

block = build_block(bindings)

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
print(f"Installed Isabelle shortcuts into keymap: {path}")
print("Selected Isabelle key bindings:")
for key in bindings:
    action = (
        TASK_CHECK
        if key in check_keys
        else TASK_BUILD
        if key in build_keys
        else "task::Rerun"
    )
    print(f"  {key} -> {action}")
PY
