#!/usr/bin/env python3

import json
import subprocess
import time


def send(stdin, message):
    body = json.dumps(message).encode("utf-8")
    stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8"))
    stdin.write(body)
    stdin.flush()


def recv(stdout, timeout=20.0):
    deadline = time.time() + timeout
    header = b""
    while b"\r\n\r\n" not in header:
        if time.time() > deadline:
            raise TimeoutError("timed out waiting for LSP headers")
        chunk = stdout.read(1)
        if not chunk:
            raise RuntimeError("LSP server exited unexpectedly")
        header += chunk

    headers = header.decode("utf-8").split("\r\n")
    content_length = None
    for line in headers:
        if line.lower().startswith("content-length:"):
            content_length = int(line.split(":", 1)[1].strip())
            break

    if content_length is None:
        raise RuntimeError(f"missing content-length header in {headers}")

    body = stdout.read(content_length)
    if len(body) != content_length:
        raise RuntimeError("incomplete LSP message body")

    return json.loads(body.decode("utf-8"))


def wait_for_response(stdout, request_id, timeout=40.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        message = recv(stdout, timeout=timeout)
        if message.get("id") == request_id:
            return message
    raise TimeoutError(f"did not receive response for id={request_id}")


def main():
    proc = subprocess.Popen(
        ["isabelle", "vscode_server", "-n"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    try:
        send(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "processId": None,
                    "rootUri": None,
                    "capabilities": {},
                },
            },
        )

        initialize = wait_for_response(proc.stdout, 1)
        if "result" not in initialize or "capabilities" not in initialize["result"]:
            raise RuntimeError("initialize response missing capabilities")

        send(proc.stdin, {"jsonrpc": "2.0", "method": "initialized", "params": {}})
        send(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "shutdown",
                "params": None,
            },
        )

        _ = wait_for_response(proc.stdout, 2)
        send(proc.stdin, {"jsonrpc": "2.0", "method": "exit", "params": None})

        print("isabelle vscode_server initialize/shutdown smoke test: OK")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)


if __name__ == "__main__":
    main()
