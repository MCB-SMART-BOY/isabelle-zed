#!/usr/bin/env python3

import json
import os
import socket
import subprocess
import time


SOCKET_PATH = "/tmp/isabelle.sock"


def wait_for_socket(path: str, timeout: float = 8.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if os.path.exists(path):
            return
        time.sleep(0.1)
    raise TimeoutError(f"bridge socket was not created in time: {path}")


def main() -> None:
    bridge_path = "./bridge/target/debug/bridge"
    adapter_cmd = f"{bridge_path} --mock-adapter"

    if os.path.exists(SOCKET_PATH):
        os.remove(SOCKET_PATH)

    proc = subprocess.Popen(
        [bridge_path, "--socket", SOCKET_PATH, "--adapter-command", adapter_cmd],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    try:
        wait_for_socket(SOCKET_PATH)

        req = {
            "id": "msg-0001",
            "type": "document.push",
            "session": "s1",
            "version": 1,
            "payload": {
                "uri": "file:///home/user/example.thy",
                "text": "theory Example imports Main begin\nend\n",
            },
        }

        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        sock.settimeout(5.0)
        sock.connect(SOCKET_PATH)
        sock.sendall((json.dumps(req) + "\n").encode("utf-8"))

        data = b""
        while not data.endswith(b"\n"):
            chunk = sock.recv(4096)
            if not chunk:
                break
            data += chunk
        sock.close()

        if not data:
            raise RuntimeError("no response from bridge")

        msg = json.loads(data.decode("utf-8").strip())
        if msg.get("type") != "diagnostics":
            raise RuntimeError(f"unexpected response type: {msg}")
        if not isinstance(msg.get("payload"), list) or not msg["payload"]:
            raise RuntimeError(f"empty diagnostics payload: {msg}")

        print("bridge --adapter-command NDJSON e2e: OK")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=2)
        if os.path.exists(SOCKET_PATH):
            os.remove(SOCKET_PATH)


if __name__ == "__main__":
    main()
