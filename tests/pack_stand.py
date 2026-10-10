# Стенд L2 пакета: настоящий `cm controller serve` и настоящий mihomo. Контроллер — root
# пространства имён, клиент — другой uid. Рёбра: C07, C09, C16, K14, M03, X03, X04, Z02.
# Запускает `tests/audit_pack_e2e.rs`; вручную (сеть хоста не затрагивается):
#   CM_BIN=<cm> CM_TEST_MIHOMO=<mihomo> unshare -U --map-root-user --map-auto -n -m \
#     sh -c "mount -t tmpfs tmpfs /run && ip link set lo up && python3 tests/pack_stand.py"
# Каждая проверка печатает `PASS <ребро> <название>` или `FAIL ...`. Код выхода 0 — все сошлись.
import json, os, shutil, signal, socket, subprocess, sys, tempfile, time

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
    print(("PASS " if ok else "FAIL ") + name + (f": {got}" if got != "" and not ok else ""), flush=True)
    if not ok:
        failures.append(name)


def cores():
    out = subprocess.run(["pgrep", "-f", f"^{core} "], capture_output=True, text=True).stdout.split()
    return [int(pid) for pid in out]


def leases():
    try:
        return json.load(open(f"{base}/leases/leases.json"))
    except (OSError, ValueError):
        return {}


def serve(extra_env=None):
    log = open(f"{base}/controller.log", "ab")
    proc = subprocess.Popen([os.environ["CM_BIN"], "controller", "serve", "--socket", sock, "--base", base],
                            env=dict(env, **(extra_env or {})), stdout=log, stderr=log)
    for _ in range(100):
        if os.path.exists(sock):
            try:
                probe = socket.socket(socket.AF_UNIX)
                probe.connect(sock)
                probe.close()
                break
            except OSError:
                pass
        time.sleep(0.05)
    return proc


replies = []


def call(op, as_root=False, key=None):
    n[0] += 1
    frame = json.dumps({"v": 1, "id": key or f"smoke-{n[0]:04d}", "op": op})
    cmd = ([] if as_root else AS_USER) + ["python3", "-c", CLIENT, sock, frame]
    t = time.time()
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    replies.append(out)
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


srv = serve()
try:
    print("== C07: каталоги и владельцы")
    check("C07 instance_prepare", call({"type": "instance_prepare", "instance": "browser"})["ok"])
    check("C07 дерево принадлежит контроллеру, проходимо, не листается",
          [mode(p) for p in (f"{base}/u{UID}", f"{base}/u{UID}/instances", inst, f"{inst}/core")] == ["0:0o711"] * 4)
    check("C07 config, cache, run принадлежат владельцу",
          [mode(f"{inst}/{d}") for d in ("config", "cache", "run")] == [f"{UID}:0o700"] * 3)
    check("C07 владелец не может создать запись рядом с каталогами контроллера",
          user(f"touch {inst}/x; ln -s /etc {inst}/core/x").returncode != 0 and not os.path.lexists(f"{inst}/x"))
    conf = json.dumps({"mode": "direct", "ipv6": False, "find-process-mode": "off", "log-level": "warning",
                       "proxies": [{"name": "secret-marker-i06", "type": "ss", "server": "203.0.113.9", "port": 443,
                                    "cipher": "aes-128-gcm", "password": "secret-marker-i06"}]})
    for g in (1, 2):
        user(f"umask 077; printf '%s' '{conf}' > {inst}/config/gen-{g}.json")
    check("C07 владелец положил конфиг во входящие", os.path.exists(f"{inst}/config/gen-1.json"))

    print("== C16: сервис без сети")
    START = {"type": "worker_start", "instance": "browser", "generation": 1}
    STATUS = {"type": "worker_status", "instance": "browser"}
    first = call(START, key="same-key-0001")
    check("C16 worker_start без сети приложения", first["data"] == {"type": "started", "generation": 1}, first)
    check("C16 статус: running, api_ready", (lambda d: (d["running"], d["api"]))(call(STATUS)["data"]) == (True, "api_ready"))
    check("C16 тот же ключ: тот же ответ, второго ядра нет",
          call(START, key="same-key-0001")["data"] == first["data"] and len(cores()) == 1, cores())
    check("C16 другой ключ при работающем ядре -> conflict", call(START)["code"] == "conflict")
    check("C16 worker_reload на поколение 2",
          call({"type": "worker_reload", "instance": "browser", "generation": 1, "next_generation": 2})["ok"])
    check("C16 устаревший worker_stop -> generation_mismatch, ядро работает",
          call({"type": "worker_stop", "instance": "browser", "generation": 1})["code"] == "generation_mismatch"
          and call(STATUS)["data"]["running"] is True)
    check("C16 чужой uid не останавливает worker: not_running, ядро работает",
          call({"type": "worker_stop", "instance": "browser", "generation": 2}, as_root=True)["code"] == "not_running"
          and len(cores()) == 1 and not os.path.exists(f"{base}/u0/instances/browser/config"))
    for bad in ("x;rm -rf", "$(id)", ".."):
        check(f"C16 instance {bad!r} -> bad_instance",
              call({"type": "worker_start", "instance": bad, "generation": 1})["code"] == "bad_instance")
    check("C16 вне base нет новых файлов экземпляров", sorted(os.listdir(f"{base}/u{UID}/instances")) == ["browser"])
    big = socket.socket(socket.AF_UNIX)
    big.connect(sock)
    big.settimeout(10)
    try:
        big.sendall(b"x" * (1 << 20))
    except OSError:
        pass
    answer = b""
    try:
        while True:
            chunk = big.recv(4096)
            if not chunk:
                break
            answer += chunk
    except OSError:
        pass
    big.close()
    check("C16 кадр 1 МиБ без перевода строки -> too_large и закрытие", b'"too_large"' in answer, answer[:120])
    check("C16 сервис принимает новые соединения", call(STATUS)["ok"])
    held = []
    for _ in range(32):
        conn = socket.socket(socket.AF_UNIX)
        conn.connect(sock)
        held.append(conn)
    time.sleep(0.5)
    extra = socket.socket(socket.AF_UNIX)
    extra.connect(sock)
    extra.settimeout(5)
    try:
        extra.sendall((json.dumps({"v": 1, "id": "conn-0033", "op": STATUS}) + "\n").encode())
        closed = extra.recv(4096) == b""
    except OSError:
        closed = True
    held[0].sendall((json.dumps({"v": 1, "id": "conn-0001", "op": STATUS}) + "\n").encode())
    held[0].settimeout(5)
    check("C16 33-е соединение закрыто сразу, первые работают", closed and b'"ok":true' in held[0].recv(65536))
    for conn in held + [extra]:
        conn.close()
    time.sleep(0.3)
    check("C16 worker_stop: нет ядер и аренд",
          call({"type": "worker_stop", "instance": "browser", "generation": 2})["ok"] and cores() == []
          and not leases().get("leases"), leases())
    denied = subprocess.run(AS_USER + [os.environ["CM_BIN"], "controller", "serve", "--socket", f"{base}/x.sock"],
                            env={k: v for k, v in os.environ.items() if k not in ("CM_STATE_DIR", "UPD_STATE_DIR")},
                            capture_output=True)
    check("C16 без тестового режима и не от root -> код 4", denied.returncode == 4, denied.returncode)

    print("== C16: обрыв контроллера посреди запуска")
    srv.terminate()
    srv.wait(10)
    check("C16 SIGTERM: код 0, сокет удалён", srv.returncode == 0 and not os.path.exists(sock), srv.returncode)
    srv = serve({"CM_CONTROLLER_CRASH_BEFORE": "record_generation"})
    crashed = subprocess.run(AS_USER + ["python3", "-c", CLIENT, sock,
                                        json.dumps({"v": 1, "id": "crash-key-0001", "op": START})],
                             capture_output=True, text=True)
    srv.wait(10)
    check("C16 контроллер оборвался на шаге record_generation", srv.returncode not in (0, None) and crashed.stdout.strip() == "",
          (srv.returncode, crashed.stdout))
    orphan = cores()
    check("C16 после обрыва осталось ядро без записи поколения",
          len(orphan) == 1 and "browser" not in open(f"{base}/u{UID}/generations.json").read()
          if os.path.exists(f"{base}/u{UID}/generations.json") else len(orphan) == 1, orphan)
    if os.path.exists(sock):
        os.remove(sock)
    srv = serve()
    reconciled = call({"type": "reconcile"})
    time.sleep(0.3)
    check("C16 reconcile: compensated 1, ядер нет, аренд нет",
          reconciled["data"] == {"type": "reconciled", "compensated": 1} and cores() == [] and not leases().get("leases"),
          (reconciled, cores(), leases()))
    check("C16 повторный reconcile ничего не делает", call({"type": "reconcile"})["data"]["compensated"] == 0)

    print("== K14, M03: сеть и ядро")
    check("K14 net_apply", call({"type": "net_apply", "instance": "browser", "generation": 1})["data"] == {"type": "net", "index": 0, "netns": "cm-0"})
    os.makedirs(f"{base}/netns", exist_ok=True)
    os.symlink("/run/netns/cm-0", f"{base}/netns/cm-0")
    check("M03 app_launch без ядра -> not_running", app("true") == "not_running")
    check("X04 чужой uid: app_launch в сеть владельца -> not_running",
          call({"type": "app_launch", "instance": "browser", "generation": 1, "program": "/bin/true", "args": []},
               as_root=True)["code"] == "not_running")
    check("M03 worker_start с TUN", call(START)["ok"])
    status = call(STATUS)["data"]
    check("M03 статус: running, api_ready, route_ready",
          (status["running"], status["api"], status["route"]) == (True, "api_ready", "route_ready"), status)
    pid = int(open(f"{inst}/core.pid").read().split()[0])
    uids = [line.split()[1] for line in open(f"/proc/{pid}/status") if line.startswith("Uid:")]
    caps = [line.split()[1] for line in open(f"/proc/{pid}/status") if line.startswith("CapEff:")]
    check("C09 ядро работает под uid владельца без привилегий", uids == [str(UID)] and caps == ["0000000000000000"], (uids, caps))
    check("C07 конфиг ядра: файл владельца в каталоге контроллера", mode(f"{inst}/core/config.json") == f"{UID}:0o600")
    check("C07 pid-файл и журнал владельцу недоступны",
          mode(f"{inst}/core.pid") == "0:0o600" and mode(f"{base}/u{UID}/journal.jsonl") == "0:0o600")
    check("C16 чужой uid не видит worker владельца", call(STATUS, as_root=True)["data"]["running"] is False)
    check("X04 чужой uid при работающем ядре владельца -> not_running",
          call({"type": "app_launch", "instance": "browser", "generation": 1, "program": "/bin/true", "args": []},
               as_root=True)["code"] == "not_running")

    print("== M03: приложение")
    host_resolv = open("/etc/resolv.conf").read()
    seen = app("id -u; ip -br addr | awk '{print $1, $3}'; cat /etc/resolv.conf; grep CapEff /proc/self/status; env | sort | tr '\\n' ' '",
               extra={"env": {"WAYLAND_DISPLAY": "wayland-7"}})
    lines = seen.splitlines()
    check("M03 uid приложения — uid клиента", lines[:1] == [str(UID)], seen)
    check("M03 интерфейсы: только lo и cmv0n",
          [line.split()[0].split("@")[0] + " " + line.split()[-1] for line in lines[1:3]] == ["lo 127.0.0.1/8", "cmv0n 10.213.0.2/30"], lines[1:3])
    check("M03 resolv.conf приложения — адрес туннеля", lines[3:5] == ["nameserver 198.18.0.2", "options edns0"], lines[3:5])
    check("M03 resolv.conf вне приложения не изменился", open("/etc/resolv.conf").read() == host_resolv)
    check("M03 CapEff приложения 0", "CapEff:\t0000000000000000" in seen)
    session_like = app("grep -E 'NoNewPrivs|CapBnd|CapPrm|CapAmb' /proc/self/status | tr '\t\n' '  '")
    check("M03 права приложения как у обычной сессии: без привилегий, setuid-помощники доступны",
          "NoNewPrivs: 0" in session_like and "CapPrm: 0000000000000000" in session_like
          and "CapAmb: 0000000000000000" in session_like and "CapBnd: 0000000000000000" not in session_like, session_like)
    names = sorted(item.split("=")[0] for item in lines[-1].split() if "=" in item)
    check("M03 окружение: только заданное контроллером и переменная сессии",
          [name for name in names if name not in ("PWD", "SHLVL", "_", "OLDPWD")] == ["HOME", "PATH", "WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]
          and "CM_STATE_DIR" not in seen and "CM_HELPER_ALLOW" not in seen, names)
    for bad_env in ({"LD_PRELOAD": "/x.so"}, {"PATH": "/x"}):
        check(f"M03 запрещённая переменная {list(bad_env)[0]} -> bad_argument", app("true", extra={"env": bad_env}) == "bad_argument")
    user(f"umask 077; printf '%s' '{{\"timezone\":\"Europe/Berlin\",\"locale\":\"de_DE.UTF-8\"}}' > {inst}/config/env.json")
    check("M03 TZ и LANG из config/env.json", app("echo $TZ $LANG") == "Europe/Berlin de_DE.UTF-8")
    user(f"umask 077; printf '%s' '{{\"timezone\":\"../x\",\"locale\":\"de_DE.UTF-8\"}}' > {inst}/config/env.json")
    check("M03 timezone ../x -> invalid_config", app("true") == "invalid_config")
    user(f"chmod 666 {inst}/config/env.json")
    check("M03 env.json с записью для чужих -> invalid_config", app("true") == "invalid_config")
    user(f"rm {inst}/config/env.json; ln -s /dev/zero {inst}/config/env.json")
    check("M03 env.json — симлинк -> invalid_config", app("true") == "invalid_config")
    user(f"rm {inst}/config/env.json; umask 077; head -c 5000 /dev/zero | tr '\\0' ' ' > {inst}/config/env.json")
    check("M03 env.json длиннее 4096 байт -> invalid_config", app("true") == "invalid_config")
    user(f"rm {inst}/config/env.json")
    fds = len(os.listdir(f"/proc/{srv.pid}/fd"))
    for _ in range(5):
        call({"type": "app_launch", "instance": "browser", "generation": 1, "program": "/bin/true", "args": []})
    time.sleep(0.7)
    check("M03 дескрипторы контроллера не растут", len(os.listdir(f"/proc/{srv.pid}/fd")) == fds)
    zombies = subprocess.run(["ps", "-o", "stat=", "--ppid", str(srv.pid)], capture_output=True, text=True).stdout
    check("M03 у контроллера нет зомби-потомков", "Z" not in zombies, zombies)

    print("== Z02: путь данных")
    subprocess.run("ip netns add inet && ip link add wan0 type veth peer name wan1 && ip link set wan1 netns inet"
                   " && ip addr add 198.51.100.1/24 dev wan0 && ip link set wan0 up"
                   " && ip -n inet addr add 198.51.100.2/24 dev wan1 && ip -n inet link set wan1 up"
                   " && ip -n inet link set lo up && ip -n inet route add default via 198.51.100.1"
                   " && ip netns exec inet nft -f - <<'EOF'\n"
                   "table inet probe {\n chain in {\n  type filter hook input priority 0;\n  ip saddr 10.213.0.2 counter\n"
                   "  udp dport 53 counter\n  tcp dport 53 counter\n }\n}\nEOF", shell=True, check=True)

    def leaked():
        text = subprocess.run("ip netns exec inet nft list table inet probe", shell=True, capture_output=True, text=True).stdout
        return [int(line.split("packets")[1].split()[0]) for line in text.splitlines() if "counter" in line]

    def cm_counters():
        text = subprocess.run("nft list table inet cm", shell=True, capture_output=True, text=True).stdout
        chain, out = "", []
        for line in text.splitlines():
            if line.strip().startswith("chain "):
                chain = line.split()[1]
            if "counter" in line:
                out.append((chain + ": " + line.strip().split(" counter")[0], int(line.split("packets")[1].split()[0])))
        return out

    web = subprocess.Popen(["ip", "netns", "exec", "inet", "python3", "-m", "http.server", "8080", "--bind", "198.51.100.2"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd="/usr/share/licenses")
    time.sleep(1)
    CURL = "curl -q -s --noproxy '*' -m 5 -o /dev/null -w '%{http_code}' http://198.51.100.2:8080/; echo \" rc$?\""
    check("Z02 HTTP из приложения через TUN: 200", app(CURL) == "200 rc0")
    counters = dict(cm_counters())
    check("Z02 счётчик veth->TUN > 0, оба запрета cmv* == 0",
          counters.get('forward: iifname "cmv0h" oifname "cmtun0"', 0) > 0
          and counters.get('forward: iifname "cmv*"') == 0 and counters.get('forward: oifname "cmv*"') == 0, counters)
    listener = subprocess.Popen(["python3", "-m", "http.server", "8081", "--bind", "0.0.0.0"],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd="/usr/share/licenses")
    time.sleep(0.7)
    HOST = "curl -q -s --noproxy '*' -m 3 -o /dev/null -w '%{http_code}' http://HOSTADDR:8081/; echo \" rc$?\""
    reach = subprocess.run(["sh", "-c", HOST.replace("HOSTADDR", "10.213.0.1")], capture_output=True, text=True).stdout.strip()
    check("Q09 контроль: служба хоста слушает и с хоста доступна", reach == "200 rc0", reach)
    for address in ("10.213.0.1", "198.51.100.1"):
        got = app(HOST.replace("HOSTADDR", address))
        check(f"Q09 приложение не достаёт службу хоста по адресу {address}", got.startswith("000 rc"), got)
    check("Q09 счётчик запрета input вырос", dict(cm_counters()).get('input: iifname "cmv*"', 0) > 0, cm_counters())
    listener.terminate()
    api = subprocess.run(["curl", "-q", "-s", "--noproxy", "*", "--unix-socket", f"{inst}/run/sock0.sock", "http://x/connections"],
                         capture_output=True, text=True).stdout
    check("Z02 ядро туннеля видело трафик (downloadTotal > 0)", json.loads(api or "{}").get("downloadTotal", 0) > 0, api[:120])
    check("Z02 DNS из приложения отвечает ядро туннеля",
          "198.18.0.2" in app("nslookup -timeout=3 -retry=0 example.test 2>&1 | head -3"))
    check("Z02 наружу нет пакетов с адреса приложения и нет DNS", leaked() == [0, 0, 0], leaked())
    check("M03 worker_reload на поколение 2",
          call({"type": "worker_reload", "instance": "browser", "generation": 1, "next_generation": 2})["ok"])
    check("M03 старое поколение -> generation_mismatch", app("true", generation=1) == "generation_mismatch")
    check("Z02 HTTP после перезагрузки ядра: 200", app(CURL, generation=2) == "200 rc0")

    print("== Z02: гибель ядра")
    IN_NS = ["ip", "netns", "exec", "cm-0", "sh", "-c", CURL]
    os.kill(int(open(f"{inst}/core.pid").read().split()[0]), 9)
    time.sleep(0.5)
    status = call(STATUS)["data"]
    check("C07 статус после kill -9: running false, поколение прежнее, оси down",
          (status["running"], status["generation"], status["api"], status["route"]) == (False, 2, "down", "down"), status)
    check("M03 запуск приложения при мёртвом ядре отклонён", app("true", generation=2) == "not_running")
    blocked = subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip()
    check("Z02 после гибели ядра: тайм-аут, кода HTTP нет", blocked == "000 rc28", blocked)
    subprocess.run("ip link del cmtun0", shell=True)
    blocked = subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip()
    table = subprocess.run("ip route show table 100", shell=True, capture_output=True, text=True).stdout.strip()
    check("Z02 после удаления TUN: запрос не проходит, в таблице остался blackhole",
          blocked.startswith("000") and table == "blackhole default metric 200", (blocked, table))
    check("Z02 наружу по-прежнему ничего не ушло", leaked() == [0, 0, 0], leaked())
    check("C07 worker_stop после гибели", call({"type": "worker_stop", "instance": "browser", "generation": 2})["ok"])
    check("K14 net_revert", call({"type": "net_revert", "instance": "browser", "generation": 1})["ok"])
    left = subprocess.run("ip -br link; ip rule; ip netns; ip route show table 100 2>/dev/null", shell=True, capture_output=True, text=True).stdout
    check("K14 сеть убрана", not any(word in left for word in ("cmv0h", "cmtun0", "cm-0", "lookup 100", "blackhole")), left)
    check("K14 в таблице cm остались только запреты",
          [name for name, _ in cm_counters()] == ['forward: iifname "cmv*"', 'forward: oifname "cmv*"', 'input: iifname "cmv*"'], cm_counters())
    os.remove(f"{base}/netns/cm-0")

    print("== Z02: сеть заново, контроль без правил CM")
    check("K14 net_apply повторно получает номер 0",
          call({"type": "net_apply", "instance": "browser", "generation": 2})["data"]["index"] == 0)
    os.symlink("/run/netns/cm-0", f"{base}/netns/cm-0")
    check("C07 повторный worker_start", call({"type": "worker_start", "instance": "browser", "generation": 2})["ok"])
    check("Z02 HTTP после повторного запуска: 200", app(CURL, generation=2) == "200 rc0")
    srv.terminate()
    srv.wait(10)
    check("C16 SIGTERM при работающем ядре: код 0, ядер нет, сокет удалён",
          srv.returncode == 0 and cores() == [] and not os.path.exists(sock), (srv.returncode, cores()))
    blocked = subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip()
    check("Z02 контроллер остановлен: сеть приложения заблокирована", blocked == "000 rc28", blocked)
    subprocess.run("ip rule del iif cmv0h lookup 100 priority 1000; nft delete table inet cm;"
                   " nft -f - <<'EOF'\ntable ip labnat {\n chain post {\n  type nat hook postrouting priority 100;\n  oifname \"wan0\" masquerade\n }\n}\nEOF",
                   shell=True, check=True)
    direct = subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip()
    check("Z02 контроль: без правила iif и таблицы cm прямой путь есть (200)", direct == "200 rc0", direct)
    web.terminate()

    print("== утечки в ответах и журнале сервиса")
    log = open(f"{base}/controller.log", "rb").read().decode(errors="replace")
    journal = open(f"{base}/u{UID}/journal.jsonl").read()
    everything = "".join(replies) + log + journal
    check("C16 в ответах, выводе сервиса и журнале нет содержимого конфига", "secret-marker-i06" not in everything)
    check("C15 в ответах нет путей и имён файлов",
          not any(word in "".join(replies) for word in (base, "gen-", "/proc", "config.json")))
finally:
    if srv.poll() is None:
        srv.terminate()
        srv.wait(10)
    for pid in cores():
        os.kill(pid, 9)
    shutil.rmtree(base, ignore_errors=True)
print("ИТОГ:", "все проверки сошлись" if not failures else f"не сошлось {len(failures)}: {failures}")
sys.exit(1 if failures else 0)
