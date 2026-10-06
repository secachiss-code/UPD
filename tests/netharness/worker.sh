#!/bin/sh
# H.11: client namespace → worker relay → remote echo, with an nft counter as capture.
# Rootless. Run as: unshare -rn sh tests/netharness/worker.sh
# Prints WORKER_OK. Host routes, interfaces and firewall are not touched.
set -eu
ip link set lo up
base="${TMPDIR:-/tmp}/cm-h11-$$"
mkdir -p "$base"
holder_r= holder_w= echo_pid= relay_pid=
cleanup() {
    kill "$echo_pid" "$relay_pid" "$holder_r" "$holder_w" 2>/dev/null || true
    rm -rf "$base"
}
trap cleanup EXIT

unshare -n sh -c 'echo $$ > "$0"; exec sleep 40' "$base/remote.pid" &
holder_r=$!
unshare -n sh -c 'echo $$ > "$0"; exec sleep 40' "$base/worker.pid" &
holder_w=$!
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    [ -s "$base/remote.pid" ] && [ -s "$base/worker.pid" ] && break
    sleep 0.1
done
remote=$(cat "$base/remote.pid")
worker=$(cat "$base/worker.pid")

ip link add c0 type veth peer name w0 netns "$worker"
ip addr add 10.98.1.1/24 dev c0
ip link set c0 up
nsenter -t "$worker" -n ip link add w1 type veth peer name r0 netns "$remote"
nsenter -t "$worker" -n sh -c 'ip link set lo up; ip addr add 10.98.1.2/24 dev w0; ip link set w0 up; ip addr add 10.98.2.1/24 dev w1; ip link set w1 up'
nsenter -t "$remote" -n sh -c 'ip link set lo up; ip addr add 10.98.2.2/24 dev r0; ip link set r0 up'

nsenter -t "$remote" -n python3 -c '
import socket
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("10.98.2.2", 7000)); s.listen(8)
while True:
    c, _ = s.accept(); c.sendall(c.recv(64)); c.close()
' &
echo_pid=$!

nsenter -t "$worker" -n python3 -c '
import socket
ls = socket.socket(); ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
ls.bind(("10.98.1.2", 7001)); ls.listen(8)
while True:
    c, _ = ls.accept()
    try:
        data = c.recv(64)
        u = socket.create_connection(("10.98.2.2", 7000), timeout=2)
        u.sendall(data)
        c.sendall(u.recv(64))
        u.close()
    except OSError:
        pass
    c.close()
' &
relay_pid=$!

probe() {
    python3 - <<'PY'
import socket, sys
try:
    c = socket.create_connection(("10.98.1.2", 7001), timeout=2)
    c.sendall(b"ping")
    sys.exit(0 if c.recv(4) == b"ping" else 1)
except OSError:
    sys.exit(1)
PY
}
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    probe && break
    sleep 0.2
done
probe || { echo "FAIL: client did not reach the echo through the worker"; exit 1; }

nft add table inet cm
nft add chain inet cm out '{ type filter hook output priority 0; policy accept; }'
nft add rule inet cm out ip daddr 10.98.1.2 tcp dport 7001 counter
probe || { echo "FAIL: capture rule blocked the worker path"; exit 1; }
nft list chain inet cm out | grep -q 'counter packets [1-9]' || {
    echo "FAIL: nft counter did not see client→worker packets"
    nft list chain inet cm out >&2
    exit 1
}

nsenter -t "$worker" -n ip link set w1 down
if probe; then echo "FAIL: link down on the worker egress still delivered"; exit 1; fi
echo WORKER_OK
