# Дымовой прогон контроллера: контроллер — root пространства имён, клиент — другой uid.
# Запуск (сеть хоста не затрагивается: свой user+net+mount namespace):
#   CM_BIN=<cm> CM_TEST_MIHOMO=<mihomo> unshare -U --map-root-user --map-auto -n -m \
#     sh -c "mount -t tmpfs tmpfs /run && ip link set lo up && python3 pack_smoke.py"
# Код выхода 0 — все проверки сошлись.
import json, os, shutil, subprocess, sys, tempfile, time

UID = 1000
base = tempfile.mkdtemp(prefix="cmsmoke", dir="/tmp")
os.chmod(base, 0o711)
core = os.path.join(base, "mihomo")
shutil.copy(os.environ["CM_TEST_MIHOMO"], core)
os.chmod(core, 0o755)
sock = os.path.join(base, "c.sock")
inst = f"{base}/u{UID}/instances/browser"
env = dict(os.environ, CM_STATE_DIR=f"{base}/state", CM_HELPER_ALLOW="1", CM_CORE_BIN=core)
AS_USER = ["setpriv", f"--reuid={UID}", f"--regid={UID}", "--clear-groups"]
CLIENT = r"""
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall((sys.argv[2] + "\n").encode())
buf = b""
while not buf.endswith(b"\n"):
    d = s.recv(65536)
    if not d: break
    buf += d
print(buf.decode().strip())
"""
failures = []
n = [0]


def check(name, ok, got=""):
    print(("PASS " if ok else "FAIL ") + name + (f": {got}" if got and not ok else ""))
    if not ok:
        failures.append(name)


def call(op, as_root=False):
    n[0] += 1
    frame = json.dumps({"v": 1, "id": f"smoke-{n[0]:04d}", "op": op})
    cmd = ([] if as_root else AS_USER) + ["python3", "-c", CLIENT, sock, frame]
    t = time.time()
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    r = json.loads(out)
    print(f"  {op['type']} -> {r['code']} {r.get('data')} {time.time() - t:.2f}s")
    return r


def user(cmd):
    return subprocess.run(AS_USER + ["sh", "-c", cmd], capture_output=True, text=True)


def app(cmd, generation=1, wait=8, extra=None):
    out = f"{inst}/config/app.out"
    user(f"rm -f {out}")
    op = {"type": "app_launch", "instance": "browser", "generation": generation,
          "program": "/bin/sh", "args": ["-c", f"({cmd}) > {out}.tmp 2>&1; mv {out}.tmp {out}"]}
    op.update(extra or {})
    reply = call(op)
    if not reply["ok"]:
        return reply["code"]
    for _ in range(wait * 10):
        if os.path.exists(out):
            return open(out).read().strip()
        time.sleep(0.1)
    return "NO OUTPUT"


def mode(path):
    st = os.lstat(path)
    return f"{st.st_uid}:{oct(st.st_mode & 0o777)}"


srv = subprocess.Popen([os.environ["CM_BIN"], "controller", "serve", "--socket", sock, "--base", base], env=env)
try:
    for _ in range(100):
        if os.path.exists(sock):
            break
        time.sleep(0.05)

    print("== каталоги и владельцы")
    check("instance_prepare", call({"type": "instance_prepare", "instance": "browser"})["ok"])
    check("дерево принадлежит контроллеру, проходимо, не листается",
          [mode(p) for p in (f"{base}/u{UID}", f"{base}/u{UID}/instances", inst, f"{inst}/core")] == ["0:0o711"] * 4)
    check("config, cache, run принадлежат владельцу",
          [mode(f"{inst}/{d}") for d in ("config", "cache", "run")] == [f"{UID}:0o700"] * 3)
    check("владелец не может создать запись рядом с каталогами контроллера",
          user(f"touch {inst}/x; ln -s /etc {inst}/core/x").returncode != 0 and not os.path.lexists(f"{inst}/x"))
    conf = json.dumps({"mode": "direct", "ipv6": False, "find-process-mode": "off", "log-level": "warning"})
    for g in (1, 2):
        user(f"umask 077; printf '%s' '{conf}' > {inst}/config/gen-{g}.json")
    check("владелец положил конфиг во входящие", os.path.exists(f"{inst}/config/gen-1.json"))

    print("== сеть и ядро")
    check("net_apply", call({"type": "net_apply", "instance": "browser", "generation": 1})["data"] == {"type": "net", "index": 0, "netns": "cm-0"})
    os.makedirs(f"{base}/netns", exist_ok=True)
    os.symlink("/run/netns/cm-0", f"{base}/netns/cm-0")
    check("app_launch без ядра -> not_running", app("true") == "not_running")
    check("worker_start", call({"type": "worker_start", "instance": "browser", "generation": 1})["ok"])
    status = call({"type": "worker_status", "instance": "browser"})["data"]
    check("статус: running, api_ready, route_ready",
          (status["running"], status["api"], status["route"]) == (True, "api_ready", "route_ready"), status)
    pid = int(open(f"{inst}/core.pid").read().split()[0])
    uids = [line.split()[1] for line in open(f"/proc/{pid}/status") if line.startswith("Uid:")]
    caps = [line.split()[1] for line in open(f"/proc/{pid}/status") if line.startswith("CapEff:")]
    check("ядро работает под uid владельца без привилегий", uids == [str(UID)] and caps == ["0000000000000000"], (uids, caps))
    check("конфиг ядра: файл владельца в каталоге контроллера", mode(f"{inst}/core/config.json") == f"{UID}:0o600")
    check("pid-файл и журнал владельцу недоступны",
          mode(f"{inst}/core.pid") == "0:0o600" and mode(f"{base}/u{UID}/journal.jsonl") == "0:0o600")
    check("чужой uid (root пространства) не видит worker владельца",
          call({"type": "worker_status", "instance": "browser"}, as_root=True)["data"]["running"] is False)

    print("== приложение")
    seen = app("id -u; ip -br addr | awk '{print $1, $3}'; cat /etc/resolv.conf; grep CapEff /proc/self/status; echo W=$WAYLAND_DISPLAY L=$LD_PRELOAD",
               extra={"env": {"WAYLAND_DISPLAY": "wayland-7"}})
    lines = seen.splitlines()
    check("uid приложения — uid клиента", lines[:1] == [str(UID)], seen)
    check("интерфейсы: только lo и cmv0n", lines[1:3] == ["lo 127.0.0.1/8", "cmv0n@if3 10.213.0.2/30"] or lines[1:3] == ["lo 127.0.0.1/8", "cmv0n@if2 10.213.0.2/30"], lines[1:3])
    check("resolv.conf приложения — адрес туннеля", lines[3:5] == ["nameserver 198.18.0.2", "options edns0"], lines[3:5])
    check("CapEff приложения 0", "CapEff:\t0000000000000000" in seen)
    check("окружение сессии: разрешённая переменная передана", lines[-1:] == ["W=wayland-7 L="], lines[-1:])
    check("окружение сессии: запрещённая переменная -> bad_argument",
          app("true", extra={"env": {"LD_PRELOAD": "/x.so"}}) == "bad_argument")
    fds = len(os.listdir(f"/proc/{srv.pid}/fd"))
    for _ in range(5):
        call({"type": "app_launch", "instance": "browser", "generation": 1, "program": "/bin/true", "args": []})
    time.sleep(0.5)
    check("дескрипторы контроллера не растут", len(os.listdir(f"/proc/{srv.pid}/fd")) == fds)

    print("== путь данных")
    subprocess.run("ip netns add inet && ip link add wan0 type veth peer name wan1 && ip link set wan1 netns inet"
                   " && ip addr add 198.51.100.1/24 dev wan0 && ip link set wan0 up"
                   " && ip -n inet addr add 198.51.100.2/24 dev wan1 && ip -n inet link set wan1 up"
                   " && ip -n inet link set lo up && ip -n inet route add default via 198.51.100.1", shell=True, check=True)
    web = subprocess.Popen(["ip", "netns", "exec", "inet", "python3", "-m", "http.server", "8080", "--bind", "198.51.100.2"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd="/usr/share/licenses")
    time.sleep(1)
    CURL = "curl -q -s --noproxy '*' -m 5 -o /dev/null -w '%{http_code}' http://198.51.100.2:8080/; echo \" rc$?\""
    check("HTTP из приложения через TUN: 200", app(CURL) == "200 rc0")
    check("DNS из приложения отвечает ядро туннеля", "198.18.0.2" in app("nslookup -timeout=3 -retry=0 example.test 2>&1 | head -3"))
    check("worker_reload на поколение 2", call({"type": "worker_reload", "instance": "browser", "generation": 1, "next_generation": 2})["ok"])
    check("старое поколение -> generation_mismatch", app("true", generation=1) == "generation_mismatch")
    check("HTTP после перезагрузки: 200", app(CURL, generation=2) == "200 rc0")

    print("== гибель ядра")
    pid = int(open(f"{inst}/core.pid").read().split()[0])
    os.kill(pid, 9)
    time.sleep(0.5)
    status = call({"type": "worker_status", "instance": "browser"})["data"]
    check("статус после kill -9: running false", status["running"] is False, status)
    check("запуск приложения отклонён", app("true", generation=2) == "not_running")
    blocked = subprocess.run(["ip", "netns", "exec", "cm-0", "sh", "-c", CURL], capture_output=True, text=True).stdout.strip()
    check("трафик из сети приложения заблокирован (тайм-аут)", blocked == "000 rc28", blocked)
    check("worker_stop после гибели", call({"type": "worker_stop", "instance": "browser", "generation": 2})["ok"])
    check("повторный worker_start", call({"type": "worker_start", "instance": "browser", "generation": 2})["ok"])
    check("HTTP после повторного запуска: 200", app(CURL, generation=2) == "200 rc0")
    web.terminate()

    print("== остановка")
    check("worker_stop", call({"type": "worker_stop", "instance": "browser", "generation": 2})["ok"])
    check("net_revert", call({"type": "net_revert", "instance": "browser", "generation": 1})["ok"])
    left = subprocess.run("ip -br link; ip rule; ip netns", shell=True, capture_output=True, text=True).stdout
    check("сеть убрана", not any(word in left for word in ("cmv0h", "cmtun0", "cm-0", "lookup 100")), left)
finally:
    srv.terminate()
    srv.wait(10)
    shutil.rmtree(base, ignore_errors=True)
print("ИТОГ:", "все проверки сошлись" if not failures else f"не сошлось {len(failures)}: {failures}")
sys.exit(1 if failures else 0)
