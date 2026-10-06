#!/bin/sh
# Rootless L2 harness smoke (D6): two network namespaces inside one user namespace.
#   client ns  <-- veth 10.98.0.1/24 <-> 10.98.0.2/24 -->  remote ns (TCP echo)
# Checks: reachability across veth; nftables `policy drop` on client output blocks it
# (fail-closed); an explicit accept for the remote re-opens it. No host network is touched.
# Usage: unshare -rn sh tests/netharness/smoke.sh   (prints SMOKE_OK on success)
set -eu
ip link set lo up
# Remote namespace: a child process holding its own netns.
unshare -n sh -c 'echo $$ > "$0"; exec sleep 30' "${TMPDIR:-/tmp}/cm-netharness-$$.pid" &
holder=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -s "${TMPDIR:-/tmp}/cm-netharness-$$.pid" ] && break; sleep 0.1; done
remote=$(cat "${TMPDIR:-/tmp}/cm-netharness-$$.pid"); rm -f "${TMPDIR:-/tmp}/cm-netharness-$$.pid"
trap 'kill $holder $echo 2>/dev/null || true' EXIT
ip link add cmc0 type veth peer name cmr0 netns "$remote"
ip addr add 10.98.0.1/24 dev cmc0 && ip link set cmc0 up
nsenter -t "$remote" -n sh -c 'ip link set lo up; ip addr add 10.98.0.2/24 dev cmr0; ip link set cmr0 up'
nsenter -t "$remote" -n python3 -c '
import socket
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("10.98.0.2", 7000)); s.listen(8)
while True:
    c, _ = s.accept(); c.sendall(c.recv(64)); c.close()
' &
echo=$!
probe() {
  python3 - <<'PY'
import socket, sys
try:
    c = socket.create_connection(("10.98.0.2", 7000), timeout=1); c.sendall(b"ping")
    sys.exit(0 if c.recv(4) == b"ping" else 1)
except OSError:
    sys.exit(1)
PY
}
for _ in 1 2 3 4 5 6 7 8 9 10; do probe && break; sleep 0.2; done
probe || { echo "FAIL: no reachability across veth"; exit 1; }
nft add table inet cm
nft add chain inet cm out '{ type filter hook output priority 0; policy drop; }'
if probe; then echo "FAIL: policy drop did not block"; exit 1; fi
nft add rule inet cm out ip daddr 10.98.0.2 tcp dport 7000 accept
probe || { echo "FAIL: explicit accept did not re-open"; exit 1; }
echo SMOKE_OK
