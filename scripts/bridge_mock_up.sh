#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
socket_path="${1:-/tmp/isabelle.sock}"
run_dir="/tmp/isabelle-zed"
pid_file="$run_dir/bridge.pid"
log_file="$run_dir/bridge.log"
bridge_bin="$repo_root/bridge/target/release/bridge"

mkdir -p "$run_dir"

if [ ! -x "$bridge_bin" ]; then
  echo "bridge binary not found, building release..."
  cargo build --manifest-path "$repo_root/bridge/Cargo.toml" --release
fi

if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file")"
  if kill -0 "$old_pid" 2>/dev/null; then
    echo "bridge already running (pid=$old_pid). stop first: make bridge-mock-down"
    exit 1
  fi
  rm -f "$pid_file"
fi

rm -f "$socket_path"

"$bridge_bin" --mock --socket "$socket_path" >"$log_file" 2>&1 &
bridge_pid=$!
echo "$bridge_pid" >"$pid_file"

for i in $(seq 1 120); do
  if [ -S "$socket_path" ]; then
    echo "bridge mock started"
    echo "  pid:    $bridge_pid"
    echo "  socket: $socket_path"
    echo "  log:    $log_file"
    exit 0
  fi
  sleep 0.1
done

echo "bridge started but socket was not ready in time" >&2
exit 1
