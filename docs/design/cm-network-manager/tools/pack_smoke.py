# Дымовой прогон контроллера (ревью пакета 2026-10-10). Запуск:
#   CM_BIN=... CM_TEST_MIHOMO=... unshare -U --map-root-user --map-auto -n -m \
#     sh -c "mount -t tmpfs tmpfs /run && ip link set lo up && python3 pack_smoke.py net"
# Сеть хоста не затрагивается: всё происходит в своём user+net+mount namespace.
import json, os, socket, subprocess, sys, time, shutil, tempfile
CM=os.environ['CM_BIN']  # собранный cm (musl, debug)
base=tempfile.mkdtemp(prefix='cmsmoke')
os.chmod(base,0o700)
sock=os.path.join(base,'c.sock')
env=dict(os.environ, CM_STATE_DIR=os.path.join(base,'state'), CM_HELPER_ALLOW='1', CM_CORE_BIN=os.environ['CM_TEST_MIHOMO'])
uid=os.getuid()
inst=os.path.join(base,f'u{uid}','instances','browser','config'); os.makedirs(inst)
for g in (1,2):
    p=os.path.join(inst,f'gen-{g}.json'); open(p,'w').write(json.dumps({"mode":"direct","ipv6":False,"find-process-mode":"off","log-level":"warning"})); os.chmod(p,0o600)
srv=subprocess.Popen([CM,'controller','serve','--socket',sock,'--base',base],env=env)
for _ in range(100):
    if os.path.exists(sock): break
    time.sleep(0.05)
n=[0]
def call(op):
    n[0]+=1
    s=socket.socket(socket.AF_UNIX); s.connect(sock)
    s.sendall((json.dumps({"v":1,"id":f"smoke-{n[0]:04d}","op":op})+"\n").encode())
    t=time.time(); buf=b''
    while not buf.endswith(b'\n'):
        d=s.recv(65536)
        if not d: break
        buf+=d
    r=json.loads(buf); print(op['type'], '->', r['code'], r.get('data'), f"{time.time()-t:.2f}s"); return r
try:
    mode=sys.argv[1] if len(sys.argv)>1 else 'plain'
    if mode=='net':
        call({"type":"net_apply","instance":"browser","generation":1})
        subprocess.run('ip -br a; ip rule; ip route show table 100; nft list table inet cm; ip netns',shell=True)
        print(open(os.path.join(base,'etc-netns','cm-0','resolv.conf')).read())
    call({"type":"worker_status","instance":"browser"})
    call({"type":"worker_start","instance":"browser","generation":1})
    call({"type":"worker_status","instance":"browser"})
    if os.environ.get('DBG'):
        d=f"{base}/u0/instances/browser"
        subprocess.run(f'cat /sys/class/net/cmtun0/flags; sed -i s/warning/debug/ {d}/config/config.json; ({env["CM_CORE_BIN"]} -d {d} -f {d}/config/config.json 2>&1 | tail -n +5 | head -20 &) ; sleep 2; ls -la {d}/run; cat /sys/class/net/cmtun0/flags; curl -q -s --noproxy "*" --unix-socket {d}/run/sock0.sock http://x/version; echo; pkill -f "mihomo -d {d}"; sleep 0.5',shell=True)
    if mode=='net':
        out=os.path.join(base,'app.out')
        os.makedirs(os.path.join(base,'netns'),exist_ok=True)
        if not os.path.exists(os.path.join(base,'netns','cm-0')): os.symlink('/run/netns/cm-0', os.path.join(base,'netns','cm-0'))
        r=call({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/sh","args":["-c",f"(id -u; ip -br a; cat /etc/resolv.conf; pwd; grep CapEff /proc/self/status) > {out} 2>&1"]})
        time.sleep(1)
        print(open(out).read() if os.path.exists(out) else 'NO APP OUTPUT')
        subprocess.run('ip netns add inet && ip link add wan0 type veth peer name wan1 && ip link set wan1 netns inet && ip addr add 198.51.100.1/24 dev wan0 && ip link set wan0 up && ip -n inet addr add 198.51.100.2/24 dev wan1 && ip -n inet link set wan1 up && ip -n inet link set lo up && ip -n inet route add default via 198.51.100.1',shell=True,check=True)
        web=subprocess.Popen(['ip','netns','exec','inet','python3','-m','http.server','8080','--bind','198.51.100.2'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,cwd='/usr/share/doc' if os.path.isdir('/usr/share/doc') else '/')
        time.sleep(1)
        def app(cmd, wait=7):
            if os.path.exists(out): os.remove(out)
            call({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/sh","args":["-c",f"({cmd}) > {out}.tmp 2>&1; mv {out}.tmp {out}"]})
            for _ in range(wait*10):
                if os.path.exists(out): break
                time.sleep(0.1)
            return open(out).read().strip() if os.path.exists(out) else 'NO OUTPUT'
        C="curl -q -s --noproxy '*' -m 5 -o /dev/null -w '%{http_code} rc' http://198.51.100.2:8080/; echo $?"
        print('A http:', app(C))
        print('DNS:', app("nslookup -timeout=3 -retry=0 example.test 2>&1 | head -4"))
        subprocess.run('nft list table inet cm | grep counter',shell=True)
        pid=open(f"{base}/u0/instances/browser/core.pid").read().split()[0]
        os.kill(int(pid),9); time.sleep(0.3)
        print('B after kill -9:', app(C))
        call({"type":"worker_status","instance":"browser"})
        web.terminate()
        raise SystemExit  # дальше ядро мертво: остаток сценария относится к прогону без e2e
        print('ctl fds', len(os.listdir(f'/proc/{srv.pid}/fd')))
        for _ in range(5): call({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/true","args":[]})
        time.sleep(0.5); print('ctl fds', len(os.listdir(f'/proc/{srv.pid}/fd')))
    call({"type":"worker_reload","instance":"browser","generation":1,"next_generation":2})
    call({"type":"worker_stop","instance":"browser","generation":2})
    call({"type":"worker_status","instance":"browser"})
    if mode=='net':
        call({"type":"net_revert","instance":"browser","generation":1})
        subprocess.run('ip -br a; ip rule; nft list table inet cm; ip netns',shell=True)
finally:
    srv.terminate(); print('srv exit', srv.wait(10))
    shutil.rmtree(base,ignore_errors=True)
