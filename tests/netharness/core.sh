#!/bin/sh
# H.11 core: worker mihomo (generated config) -> remote mihomo inbound -> local echo.
# Rootless. Run as: unshare -rn sh tests/netharness/core.sh
# Prints CORE_OK. Host routes, interfaces and firewall are not touched.
# nft runs only inside the worker network namespace.
set -eu

if [ -z "${CM_TEST_MIHOMO:-}" ] || [ ! -x "${CM_TEST_MIHOMO}" ]; then
    echo "H.11 SKIPPED"
    exit 0
fi
if [ -z "${CM_TEST_WORKER_CONFIG:-}" ] || [ ! -f "${CM_TEST_WORKER_CONFIG}" ]; then
    echo "FAIL: worker config missing"
    exit 1
fi

ip link set lo up
base="${TMPDIR:-/tmp}/cm-h11-core-$$"
mkdir -p "$base/remote" "$base/worker"
holder_r= holder_w= echo_pid= remote_pid= worker_pid=
cleanup() {
    kill "$echo_pid" "$remote_pid" "$worker_pid" "$holder_r" "$holder_w" 2>/dev/null || true
    rm -rf "$base"
}
trap cleanup EXIT

unshare -n sh -c 'echo $$ > "$0"; exec sleep 80' "$base/remote.pid" &
holder_r=$!
unshare -n sh -c 'echo $$ > "$0"; exec sleep 80' "$base/worker.pid" &
holder_w=$!
i=0
while [ "$i" -lt 100 ]; do
    if [ -s "$base/remote.pid" ] && [ -s "$base/worker.pid" ]; then
        break
    fi
    i=$((i + 1))
    sleep 0.1
done
remote=$(cat "$base/remote.pid")
worker=$(cat "$base/worker.pid")

nsenter -t "$worker" -n ip link add w1 type veth peer name r0 netns "$remote"
nsenter -t "$worker" -n sh -c 'ip link set lo up; ip addr add 10.98.2.1/24 dev w1; ip link set w1 up'
nsenter -t "$remote" -n sh -c 'ip link set lo up; ip addr add 10.98.2.2/24 dev r0; ip link set r0 up'

cat > "$base/remote.json" <<'EOF'
{"mixed-port":17890,"allow-lan":true,"bind-address":"10.98.2.2","mode":"direct","log-level":"warning","find-process-mode":"off","ipv6":false}
EOF

nsenter -t "$remote" -n python3 -c '
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"pong"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, fmt, *args):
        return
ThreadingHTTPServer(("10.98.2.2", 7007), Handler).serve_forever()
' >"$base/echo.log" 2>&1 &
echo_pid=$!

nsenter -t "$remote" -n "$CM_TEST_MIHOMO" -d "$base/remote" -f "$base/remote.json" >"$base/remote-mihomo.log" 2>&1 &
remote_pid=$!
nsenter -t "$worker" -n "$CM_TEST_MIHOMO" -d "$base/worker" -f "$CM_TEST_WORKER_CONFIG" >"$base/worker-mihomo.log" 2>&1 &
worker_pid=$!

nsenter -t "$worker" -n nft add table inet cm
nsenter -t "$worker" -n nft add chain inet cm out '{ type filter hook output priority 0; policy accept; }'
nsenter -t "$worker" -n nft add rule inet cm out ip daddr 10.98.2.2 tcp dport 17890 counter

probe() {
    nsenter -t "$worker" -n python3 - <<'PY'
import urllib.request
proxy = urllib.request.ProxyHandler({"http": "http://127.0.0.1:18080"})
opener = urllib.request.build_opener(proxy)
try:
    data = opener.open("http://10.98.2.2:7007/", timeout=2).read()
except Exception:
    raise SystemExit(1)
raise SystemExit(0 if data == b"pong" else 1)
PY
}

i=0
ok=0
while [ "$i" -lt 100 ]; do
    if probe; then
        ok=1
        break
    fi
    i=$((i + 1))
    sleep 0.25
done
if [ "$ok" != 1 ]; then
    echo "FAIL: client did not receive pong through the worker"
    exit 1
fi
nsenter -t "$worker" -n nft list chain inet cm out | grep -q 'counter packets [1-9]' || {
    echo "FAIL: nft counter did not see worker to remote mixed-port packets"
    exit 1
}
echo CORE_OK
