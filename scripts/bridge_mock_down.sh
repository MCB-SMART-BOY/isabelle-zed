#!/usr/bin/env bash
set -euo pipefail

socket_path="${1:-/tmp/isabelle.sock}"
run_dir="/tmp/isabelle-zed"
pid_file="$run_dir/bridge.pid"

if [ ! -f "$pid_file" ]; then
  echo "no bridge pid file found at $pid_file"
  rm -f "$socket_path"
  exit 0
fi

pid="$(cat "$pid_file")"
if kill -0 "$pid" 2>/dev/null; then
  kill "$pid" 2>/dev/null || true
  for _ in $(seq 1 30); do
    if ! kill -0 "$pid" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
fi

rm -f "$pid_file"
rm -f "$socket_path"

echo "bridge mock stopped"
