#!/usr/bin/env python3

import json
import os
import subprocess
import time


def send_message(stdin, message):
    body = json.dumps(message).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8")
    stdin.write(header)
    stdin.write(body)
    stdin.flush()


def read_message(stdout, timeout=5.0):
    deadline = time.time() + timeout
    header = b""
    while b"\r\n\r\n" not in header:
        if time.time() > deadline:
            raise TimeoutError("timed out waiting for LSP headers")
        chunk = stdout.read(1)
        if not chunk:
            raise RuntimeError("LSP server exited early")
        header += chunk

    headers = header.decode("utf-8").split("\r\n")
    content_length = None
    for line in headers:
        if line.lower().startswith("content-length:"):
            content_length = int(line.split(":", 1)[1].strip())
            break

    if content_length is None:
        raise RuntimeError(f"missing content-length header: {headers}")

    body = stdout.read(content_length)
    if len(body) != content_length:
        raise RuntimeError("incomplete LSP message body")

    return json.loads(body.decode("utf-8"))


def main():
    env = os.environ.copy()
    env["ISABELLE_BRIDGE_SOCKET"] = "/tmp/isabelle.sock"

    proc = subprocess.Popen(
        ["./isabelle-lsp/target/debug/isabelle-zed-lsp"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )

    try:
        send_message(
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

        init_response = read_message(proc.stdout)
        assert init_response.get("id") == 1, init_response

        send_message(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            },
        )

        send_message(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": "file:///home/user/example.thy",
                        "languageId": "isabelle",
                        "version": 1,
                        "text": "theory Example imports Main begin\\nend\\n",
                    }
                },
            },
        )

        diagnostics = None
        deadline = time.time() + 8.0
        while time.time() < deadline:
            message = read_message(proc.stdout, timeout=8.0)
            if message.get("method") == "textDocument/publishDiagnostics":
                diagnostics = message
                break

        if diagnostics is None:
            raise RuntimeError("did not receive publishDiagnostics")

        params = diagnostics["params"]
        assert params["uri"] == "file:///home/user/example.thy", diagnostics
        assert len(params["diagnostics"]) == 1, diagnostics
        assert params["diagnostics"][0]["message"] == "Parse error", diagnostics

        send_message(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "shutdown",
                "params": None,
            },
        )
        _ = read_message(proc.stdout)

        send_message(
            proc.stdin,
            {
                "jsonrpc": "2.0",
                "method": "exit",
                "params": None,
            },
        )
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=2)


if __name__ == "__main__":
    main()
