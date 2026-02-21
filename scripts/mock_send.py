#!/usr/bin/env python3

import json
import socket

REQUEST = {
    "id": "msg-0001",
    "type": "document.push",
    "session": "s1",
    "version": 1,
    "payload": {
        "uri": "file:///home/user/example.thy",
        "text": "theory Example imports Main begin\\nend\\n",
    },
}

sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.connect("/tmp/isabelle.sock")
sock.sendall((json.dumps(REQUEST) + "\n").encode())

data = b""
while not data.endswith(b"\n"):
    chunk = sock.recv(4096)
    if not chunk:
        break
    data += chunk

sock.close()
print(data.decode().strip())
