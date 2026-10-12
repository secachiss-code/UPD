#!/usr/bin/env python3
"""T14 single-instance: first holds unix socket, second forwards argv and exits 0."""
import os
import socket
import sys

sock_path = os.environ.get("CM_BROKER_SOCK", "/tmp/cm-broker-single.sock")
if len(sys.argv) > 1 and sys.argv[1] == "--client":
    s = socket.socket(socket.AF_UNIX)
    s.connect(sock_path)
    s.sendall((" ".join(sys.argv[2:]) + "\n").encode())
    sys.exit(0)
if os.path.exists(sock_path):
    sys.exit(0)
s = socket.socket(socket.AF_UNIX)
s.bind(sock_path)
s.listen(1)
while True:
    conn, _ = s.accept()
    conn.recv(4096)
    conn.close()
