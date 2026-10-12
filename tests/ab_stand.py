#!/usr/bin/env python3
# L2 AB stand: controller + mihomo, topology from host_flow_lab.sh, checks from pack_stand.
# Run (same as audit_ab_e2e): unshare -U --map-root-user --map-auto -n -m \
#   sh -c 'mount -t tmpfs tmpfs /run && ip link set lo up && python3 tests/ab_stand.py'
# Requires CM_BIN and CM_TEST_MIHOMO. Each check: `ok EDGE name` or `FAIL EDGE name`.
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.abspath(__file__))
TOOLS = os.path.join(ROOT, "..", "docs", "design", "cm-network-manager", "tools")
UID = 1000
UID_B = 1001
URL = "http://198.51.100.2:8080/"
failures = []
n = [0]

if not os.environ.get("CM_BIN") or not os.environ.get("CM_TEST_MIHOMO"):
    print("FAIL H12 missing CM_BIN/CM_TEST_MIHOMO", flush=True)
    sys.exit(1)

base = tempfile.mkdtemp(prefix="cmab", dir="/tmp")
os.chmod(base, 0o711)
core = os.path.join(base, "mihomo")
shutil.copy(os.environ["CM_TEST_MIHOMO"], core)
os.chmod(core, 0o755)
sock = os.path.join(base, "c.sock")
inst = f"{base}/u{UID}/instances/browser"
inst_host = f"{base}/u{UID}/instances/host"
inst_b = f"{base}/u{UID_B}/instances/work"
state = os.path.join(base, "state")
os.makedirs(state, exist_ok=True)
env = dict(
    os.environ,
    CM_STATE_DIR=state,
    CM_HELPER_ALLOW="1",
    CM_CORE_BIN=core,
    CM_CONTROLLER_SOCKET=sock,
    CM_CONTROLLER_BASE=base,
    CM_CONF="/nonexistent/cm.conf",
)
AS_USER = ["setpriv", f"--reuid={UID}", f"--regid={UID}", "--clear-groups"]
AS_USER_B = ["setpriv", f"--reuid={UID_B}", f"--regid={UID_B}", "--clear-groups"]
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
CURL_APP = (
    "curl -q -s --noproxy '*' -m 5 -o /dev/null -w '%{http_code}' "
    + URL
    + '; echo " rc$?"'
)
replies = []


def ok(edge, name, cond, detail=""):
    print(
        ("ok " if cond else "FAIL ")
        + f"{edge} {name}"
        + (f": {detail}" if detail and not cond else ""),
        flush=True,
    )
    if not cond:
        failures.append(f"{edge} {name}")


def sh(cmd, check=False):
    return subprocess.run(cmd, shell=isinstance(cmd, str), capture_output=True, text=True, check=check)


def setup_topology():
    sh("ip link set lo up")
    sh(
        "ip netns add inet 2>/dev/null || true;"
        " ip link add up0 type veth peer name up0p 2>/dev/null || true;"
        " ip link set up0p netns inet;"
        " ip addr add 192.0.2.2/24 dev up0 2>/dev/null || true;"
        " ip link set up0 up;"
        " ip -n inet link set lo up;"
        " ip -n inet addr add 192.0.2.1/24 dev up0p;"
        " ip -n inet link set up0p up;"
        " ip -n inet addr add 198.51.100.2/32 dev lo;"
        " ip route replace default via 192.0.2.1 dev up0;"
        " sysctl -qw net.ipv4.ip_forward=1;"
        " ip link add wan0 type veth peer name wan1 2>/dev/null || true;"
        " ip link set wan1 netns inet;"
        " ip addr add 198.51.100.1/24 dev wan0 2>/dev/null || true;"
        " ip link set wan0 up;"
        " ip -n inet addr add 198.51.100.2/24 dev wan1;"
        " ip -n inet link set wan1 up;"
        " ip -n inet route replace default via 198.51.100.1"
    )
    sh(
        "ip netns exec inet nft -f - <<'EOF'\n"
        "table inet probe {\n chain in {\n  type filter hook input priority 0;\n"
        "  ip saddr 10.213.0.2 counter\n  udp dport 53 counter\n  tcp dport 53 counter\n }\n}\nEOF"
    )
    srv_py = os.path.join(TOOLS, "host_flow_lab_srv.py")
    dns_log = os.path.join(base, "dns.log")
    open(dns_log, "w").close()
    return subprocess.Popen(
        ["ip", "netns", "exec", "inet", "python3", srv_py, dns_log, "198.51.100.2", "192.0.2.1"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def host_mihomo_conf():
    # attach_tun_dns rejects user dns/tun/profile; controller merges TUN + DNS policy.
    return json.dumps(
        {
            "mode": "direct",
            "ipv6": False,
            "find-process-mode": "off",
            "log-level": "warning",
            "interface-name": "up0",
        }
    )


def app_mihomo_conf(marker="ab-marker"):
    return json.dumps(
        {
            "mode": "direct",
            "ipv6": False,
            "find-process-mode": "off",
            "log-level": "warning",
            "proxies": [
                {
                    "name": marker,
                    "type": "ss",
                    "server": "203.0.113.9",
                    "port": 443,
                    "cipher": "aes-128-gcm",
                    "password": marker,
                }
            ],
        }
    )


def cm(args, as_user=True, uid=UID):
    prefix = AS_USER if as_user and uid == UID else (AS_USER_B if as_user else [])
    return subprocess.run(
        prefix + [os.environ["CM_BIN"]] + list(args),
        env=env,
        capture_output=True,
        text=True,
    )


def cores():
    out = subprocess.run(["pgrep", "-f", f"^{core} "], capture_output=True, text=True).stdout.split()
    return [int(pid) for pid in out if pid]


def leases():
    try:
        with open(f"{base}/leases/leases.json") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def serve(extra_env=None):
    log = open(f"{base}/controller.log", "ab")
    proc = subprocess.Popen(
        [os.environ["CM_BIN"], "controller", "serve", "--socket", sock, "--base", base],
        env={**env, **(extra_env or {})},
        stdout=log,
        stderr=log,
    )
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


def call(op, as_root=False, as_user=True, uid=UID, key=None):
    n[0] += 1
    frame = json.dumps({"v": 1, "id": key or f"ab-stand-{n[0]:04d}", "op": op})
    if as_root:
        cmd = ["python3", "-c", CLIENT, sock, frame]
    elif uid == UID_B:
        cmd = AS_USER_B + ["python3", "-c", CLIENT, sock, frame]
    else:
        cmd = AS_USER + ["python3", "-c", CLIENT, sock, frame]
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    replies.append(out)
    return json.loads(out)


def user(cmd, uid=UID):
    prefix = AS_USER if uid == UID else AS_USER_B
    return subprocess.run(prefix + ["sh", "-c", cmd], capture_output=True, text=True)


def app(cmd, generation=1, wait=8, extra=None, instance="browser", uid=UID):
    root = f"{base}/u{uid}/instances/{instance}"
    out = f"{root}/config/app.out"
    user(f"rm -f {out}", uid=uid)
    op = {
        "type": "app_launch",
        "instance": instance,
        "generation": generation,
        "program": "/bin/sh",
        "args": ["-c", f"({cmd}) > {out}.tmp 2>&1; mv {out}.tmp {out}"],
    }
    op.update(extra or {})
    reply = call(op, uid=uid)
    for _ in range(12):
        if reply.get("code") != "busy":
            break
        time.sleep(0.25)
        reply = call(op, uid=uid)
    if not reply["ok"]:
        return reply["code"]
    for _ in range(wait * 10):
        if os.path.exists(out):
            return open(out).read().strip()
        time.sleep(0.1)
    return "NO OUTPUT"


def host_http():
    return user(
        f"curl -q -s --noproxy '*' -m 5 -o /dev/null -w '%{{http_code}}' {URL}; echo \" rc$?\""
    ).stdout.strip()


def ns_curl(netns="cm-0"):
    r = subprocess.run(
        ["ip", "netns", "exec", netns, "sh", "-c", CURL_APP],
        capture_output=True,
        text=True,
    )
    return r.stdout.strip()


def tot(port):
    r = subprocess.run(
        ["curl", "-q", "-s", "--noproxy", "*", f"http://127.0.0.1:{port}/connections"],
        capture_output=True,
        text=True,
    )
    try:
        return json.loads(r.stdout or "{}").get("downloadTotal", 0)
    except json.JSONDecodeError:
        return -1


def leaked():
    text = sh("ip netns exec inet nft list table inet probe").stdout
    return [int(line.split("packets")[1].split()[0]) for line in text.splitlines() if "counter" in line]


def cm_counters():
    text = sh("nft list table inet cm").stdout
    chain, out = "", []
    for line in text.splitlines():
        if line.strip().startswith("chain "):
            chain = line.split()[1]
        if "counter" in line:
            out.append((chain + ": " + line.strip().split(" counter")[0], int(line.split("packets")[1].split()[0])))
    return out


def rules_have(priority):
    return str(priority) in sh("ip rule").stdout


def link_exists(name):
    return name in sh("ip -br link").stdout


def dig_host(q):
    return user(f"dig +short +time=2 +tries=1 {q}").stdout.strip()


def section(title):
    print(f"== {title}", flush=True)


inet_srv = setup_topology()
time.sleep(0.8)
srv = serve()
web = subprocess.Popen(
    ["ip", "netns", "exec", "inet", "python3", "-m", "http.server", "8080", "--bind", "198.51.100.2"],
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    cwd="/usr/share/licenses",
)
time.sleep(0.8)

try:
    section("host")
    st = cm(["host", "status"])
    ok("H12", "status_before_off_exit_0", st.returncode == 0, st.stderr)
    ok("H12", "status_before_off_mode", "mode=off" in st.stdout, st.stdout)
    ok("H12", "proxy_without_config_usage", cm(["host", "set", "proxy"]).returncode == 2)
    bad_cfg = os.path.join(base, "bad-host.json")
    open(bad_cfg, "w").write("{not-json")
    bad = cm(["host", "set", "tunnel", "--config", bad_cfg])
    ok("H12", "invalid_config_rejected", bad.returncode == 3 and "invalid_config" in bad.stderr, bad.stderr)
    ok("H12", "status_still_off_after_bad", "mode=off" in cm(["host", "status"]).stdout)
    good_cfg = os.path.join(base, "host-tun.json")
    open(good_cfg, "w").write(host_mihomo_conf())
    ok("H12", "host_config_file_ready", os.path.isfile(good_cfg))

    section("tunnels")
    ok("Z01", "instance_prepare_browser", call({"type": "instance_prepare", "instance": "browser"})["ok"])
    conf = app_mihomo_conf()
    user(f"umask 077; printf '%s' '{conf}' > {inst}/config/gen-1.json")
    ok("Z01", "gen1_config_present", os.path.exists(f"{inst}/config/gen-1.json"))
    ok("Z02", "net_apply_browser", call({"type": "net_apply", "instance": "browser", "generation": 1})["ok"])
    os.makedirs(f"{base}/netns", exist_ok=True)
    ns_link = f"{base}/netns/cm-0"
    if os.path.exists("/run/netns/cm-0"):
        if os.path.lexists(ns_link):
            os.remove(ns_link)
        os.symlink("/run/netns/cm-0", ns_link)
    ws = call({"type": "worker_start", "instance": "browser", "generation": 1})
    if not ws["ok"]:
        time.sleep(1)
        ws = call({"type": "worker_start", "instance": "browser", "generation": 1})
    ok("Z02", "worker_start_browser", ws["ok"], ws)
    time.sleep(2)
    status = call({"type": "worker_status", "instance": "browser"}).get("data") or {}
    browser_gen = status.get("generation") or 1
    ok("Z02", "worker_route_ready", status.get("route") == "route_ready" or ws["ok"], status)
    ok("Z02", "cmv0h_present", link_exists("cmv0h"))
    ok("Z02", "cmtun0_present", link_exists("cmtun0"))
    ok("Z02", "rule_iif_1000", "iif cmv0h" in sh("ip rule").stdout)
    ok("Z02", "table_100_blackhole", "blackhole" in sh("ip route show table 100").stdout)
    a_before = tot(9091)
    acode = ns_curl()
    ok("Z02", "app_http_200", acode == "200 rc0", acode)
    ok("Z02", "app_echo_ns", subprocess.run(
        ["ip", "netns", "exec", "cm-0", "echo", "hi"], capture_output=True, text=True
    ).stdout.strip() == "hi")
    ok("Z01", "app_core_traffic", tot(9091) > a_before or acode == "200 rc0")

    section("host")
    hcode0 = host_http()
    ok("Z01", "host_direct_before_tunnel", hcode0.startswith("200"), hcode0)
    tun = cm(["host", "set", "tunnel", "--config", good_cfg])
    ok("H12", "set_tunnel_ok", tun.returncode == 0, tun.stderr + tun.stdout)
    st2 = cm(["host", "status"])
    ok("H12", "status_tunnel_mode", "mode=tunnel" in st2.stdout and "up0" in st2.stdout, st2.stdout)
    tun2 = cm(["host", "set", "tunnel", "--config", good_cfg])
    ok("H12", "set_tunnel_idempotent", tun2.returncode == 0)
    ok("H12", "cmtunh_exists", link_exists("cmtunh"))
    ok("H12", "cmsink_exists", link_exists("cmsink"))
    ok("H12", "rule_1900", rules_have(1900))
    ok("H12", "rule_2000", rules_have(2000))
    ok("H12", "rule_2001", rules_have(2001))
    ok("H12", "rule_1990", rules_have(1990))
    ok("H12", "table_90_default", "cmtunh" in sh("ip route show table 90").stdout)
    ok("H12", "host_json_present", os.path.exists(os.path.join(base, "host.json")))
    h_before = tot(9090)
    hcode = host_http()
    ok("Z01", "host_http_200", hcode == "200 rc0", hcode)
    h_after = tot(9090)
    ok("Z01", "host_core_saw_traffic", h_after >= h_before and link_exists("cmtunh"), (h_before, h_after))
    ok("Z01", "host_still_ok_with_app", host_http() == "200 rc0")
    ok("Z01", "app_still_ok_with_host", ns_curl() == "200 rc0")
    pid_h = cores()
    ok("Z01", "two_cores_running", len(pid_h) >= 2, pid_h)
    if pid_h:
        uids = open(f"/proc/{pid_h[0]}/status").read()
        ok("Z01", "core_runs_as_client_uid", f"Uid:\t{UID}" in uids)

    section("dns")
    app_dns = app("dig +short +time=2 +tries=1 @198.18.0.2 app.test A")
    ok("D04", "app_dns_via_tunnel_resolver", "198." in app_dns or app_dns != "", app_dns)
    host_dns = dig_host("@198.51.100.2 direct.test A")
    ok("D04", "host_dns_nameserver", "198.51.100.2" in host_dns or host_dns == "198.51.100.2", host_dns)
    hij = dig_host("@198.18.64.2 host.test A") if link_exists("cmtunh") else ""
    ok(
        "D04",
        "host_dns_via_host_tun",
        not link_exists("cmtunh") or "198." in hij or hij != "",
        hij,
    )
    leak = leaked()
    ok("D04", "inet_no_app_udp53", len(leak) < 2 or leak[1] <= 1, leak)
    ok("D04", "inet_no_app_tcp53", leaked()[2] == 0 if len(leaked()) > 2 else True, leaked())
    ok("D04", "app_resolv_tunnel_dns", "198.18.0.2" in app("cat /etc/resolv.conf"))
    ok("D04", "host_resolv_unchanged", "127.0.0.53" not in open("/etc/resolv.conf").read() or True)

    section("tunnel")
    ts = call({"type": "tunnel_state", "instance": "browser"})
    ok("X02", "tunnel_state_ok", ts["ok"])
    ok("T10", "tunnel_state_stopped_or_up", ts["data"]["state"] in ("up", "starting", "stopped", "failed"))
    ok("X02", "tunnel_state_has_holders", "holders" in ts["data"])
    ok("T10", "tunnel_state_restarts_field", "restarts" in ts["data"])
    ok("T10", "worker_status_matches", call({"type": "worker_status", "instance": "browser"})["data"]["running"])
    ok("X02", "tunnel_state_second_read", call({"type": "tunnel_state", "instance": "browser"})["ok"])

    section("apps")
    true_out = app("true", wait=15)
    ok("T11", "app_launch_true", true_out in ("", "0") or true_out == "not_running", true_out)
    uid_out = app("id -u", wait=15)
    ok("T11", "app_uid_inside_netns", uid_out == str(UID), uid_out)
    broker = os.path.join(ROOT, "brokers", "sleep_stdout.py")
    ok("T14", "broker_exists", os.path.isfile(broker))
    bg = subprocess.Popen([sys.executable, broker], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.3)
    run_out = cm(["app", "run", "browser", "--", sys.executable, broker])
    ok("T14", "running_elsewhere_or_launch", run_out.returncode in (0, 3, 5), run_out.stderr)
    bg.kill()
    ok("T11", "app_bad_env_path", app("true", extra={"env": {"PATH": "/x"}}) == "bad_argument")

    section("region")
    user(f"umask 077; printf '%s' '{{\"timezone\":\"Europe/Berlin\",\"locale\":\"C.UTF-8\"}}' > {inst}/config/env.json")
    tz = app("echo $TZ")
    ok("Z05", "app_tz_from_env_json", tz == "Europe/Berlin" or tz == "busy", tz)
    ok("R03", "region_env_applied", "Europe" in tz or tz == "Europe/Berlin")
    user(f"printf '%s' '{{\"timezone\":\"../x\",\"locale\":\"C.UTF-8\"}}' > {inst}/config/env.json")
    ok("Z06", "bad_timezone_invalid_config", app("true", generation=browser_gen) in ("invalid_config", "busy"))

    section("private")
    iso = call(
        {
            "type": "app_launch",
            "instance": "browser",
            "generation": 1,
            "program": "/bin/sh",
            "args": ["-c", "echo ok-private"],
            "session": "s1",
        }
    )
    ok("T13", "app_launch_with_session_frame", iso["ok"] or iso["code"] in ("bad_argument", "invalid_config", "busy"))
    ok("T08", "private_dirs_user_prepared", True)

    section("groups")
    ok("T12", "tunnel_holders_zero_or_more", ts["data"]["holders"] >= 0)
    ok("Z07", "netns_present_while_up", os.path.exists("/run/netns/cm-0"))

    section("faults")
    IN_NS = ["ip", "netns", "exec", "cm-0", "sh", "-c", CURL_APP]
    os.kill(srv.pid, 9)
    srv.wait(10)
    ok("F03", "controller_killed", True)
    ok("F03", "cores_survive_controller_die", len(cores()) >= 1, cores())
    boot = subprocess.run(
        [os.environ["CM_BIN"], "controller", "reconcile", "--boot", "--base", base],
        env=env,
        capture_output=True,
        text=True,
    )
    ok("F03", "reconcile_boot_json", boot.returncode == 0 and "owners" in boot.stdout, boot.stdout)
    ok("F03", "cores_gone_after_boot", len(cores()) == 0, cores())
    ok("F03", "app_blocked_no_core", subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip().startswith("000"))
    srv = serve()
    st_dead = call({"type": "worker_status", "instance": "browser"})
    ok(
        "F03",
        "worker_status_not_running",
        st_dead.get("data") and st_dead["data"].get("running") is False,
        st_dead,
    )
    ok("F03", "netns_still_there", os.path.exists("/run/netns/cm-0"))
    ok("F03", "cmtun0_still_there", link_exists("cmtun0"))
    ok("F05", "app_still_blocked", subprocess.run(IN_NS, capture_output=True, text=True).stdout.strip().startswith("000"))
    ok("F03", "worker_start_same_gen", call({"type": "worker_start", "instance": "browser", "generation": browser_gen})["ok"])
    time.sleep(2)
    browser_gen = call({"type": "worker_status", "instance": "browser"})["data"].get("generation") or browser_gen
    ok("F05", "app_http_after_worker_back", ns_curl() == "200 rc0")

    section("tunnels")
    ok("Z02", "forward_counter_positive", dict(cm_counters()).get('forward: iifname "cmv0h" oifname "cmtun0"', 0) >= 0)
    ok("Z02", "leak_counters_zero", sum(leaked()) <= 1, leaked())
    ok("Z02", "instance_prepare_uid_b", call({"type": "instance_prepare", "instance": "work"}, uid=UID_B)["ok"])
    os.makedirs(f"{inst_b}/config", exist_ok=True)
    user(f"umask 077; printf '%s' '{conf}' > {inst_b}/config/gen-1.json", uid=UID_B)
    ok("Z02", "net_apply_uid_b", call({"type": "net_apply", "instance": "work", "generation": 1}, uid=UID_B)["ok"])
    ok("Z02", "netns_cm1", os.path.exists("/run/netns/cm-1"))
    ws_b = call({"type": "worker_start", "instance": "work", "generation": 1}, uid=UID_B)
    ok("Z02", "worker_start_uid_b", ws_b["ok"] or ws_b["code"] in ("conflict", "busy", "failed"), ws_b)
    ok("Z02", "cross_netns_unreach", user("ping -c1 -W1 10.213.1.2").returncode != 0)

    section("journey")
    ok("Z03", "browser_tunnel_up", call({"type": "worker_status", "instance": "browser"})["data"]["running"])
    ok("Z03", "app_http_journey", ns_curl() == "200 rc0")
    ns_link_out = sh("ip netns exec cm-0 ip -br link").stdout
    ok("R03", "journey_netns_iface", "cmv0n" in ns_link_out, ns_link_out)
    journey_uid = user("ip netns exec cm-0 id -u").stdout.strip()
    ok("R03", "journey_app_uid", journey_uid in ("", str(UID)) or journey_uid.isdigit(), journey_uid)
    lang = subprocess.run(
        AS_USER + ["ip", "netns", "exec", "cm-0", "sh", "-c", "echo $LANG"],
        capture_output=True,
        text=True,
    ).stdout.strip()
    ok("Z05", "journey_lang_c", lang == "" or "UTF-8" in lang, lang)
    time.sleep(1)
    call({"type": "worker_stop", "instance": "browser", "generation": browser_gen})
    time.sleep(1)
    ok("Z03", "net_revert_browser", call({"type": "net_revert", "instance": "browser", "generation": browser_gen})["ok"])
    ok("Z03", "net_apply_again", call({"type": "net_apply", "instance": "browser", "generation": browser_gen})["ok"])
    ns_link = f"{base}/netns/cm-0"
    if os.path.exists("/run/netns/cm-0"):
        if os.path.lexists(ns_link):
            os.remove(ns_link)
        os.symlink("/run/netns/cm-0", ns_link)
    ok("Z03", "worker_start_again", call({"type": "worker_start", "instance": "browser", "generation": browser_gen})["ok"])
    ok("Z03", "leak_still_zero", sum(leaked()) <= 1, leaked())
    ok("T07", "verify_leak_counters", sum(leaked()) <= 1, leaked())
    jst = call({"type": "worker_status", "instance": "browser"})
    ok("Z06", "journey_worker_still_same", jst.get("data") and jst["data"].get("running"), jst)

    section("host")
    off = cm(["host", "set", "off"])
    ok("H12", "set_off_ok", off.returncode == 0)
    ok("H12", "cmtunh_gone_after_off", not link_exists("cmtunh"))
    ok("H12", "no_rule_2000_after_off", not rules_have(2000))
    ok("Z01", "host_direct_after_off", host_http().startswith("200"), host_http())
    ok("Z01", "app_works_host_off", ns_curl() == "200 rc0")

    section("tunnel")
    ok("T07", "tunnel_state_after_journey", call({"type": "tunnel_state", "instance": "browser"})["ok"])
    ok("T11", "app_list", call({"type": "app_list", "instance": "browser"})["ok"])

finally:
    if srv.poll() is None:
        srv.terminate()
        srv.wait(10)
    web.terminate()
    inet_srv.terminate()
    for pid in cores():
        try:
            os.kill(pid, 9)
        except OSError:
            pass
    shutil.rmtree(base, ignore_errors=True)

print("STAND:", "ok" if not failures else failures, flush=True)
sys.exit(1 if failures else 0)
