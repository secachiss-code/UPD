"""Opt-in real Wayland smoke; opens five windows, uses isolated data and no package managers."""
import json, os, pathlib, signal, subprocess, tempfile, time
root = pathlib.Path(__file__).resolve().parents[1]
base = pathlib.Path(tempfile.mkdtemp(prefix='upd-wayland-smoke-'))
base.mkdir(exist_ok=True)
results = []
for page in ['updates', 'mirrors', 'vpn', 'maintenance', 'settings']:
    work = base / page
    for part in ['config', 'cache', 'data', 'state', 'bin']:
        (work / part).mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, UPD_STATE_DIR=str(work/'state'), UPD_CONF=str(work/'upd.conf'), UPD_HELPER_SOCK=str(work/'absent.sock'), XDG_CONFIG_HOME=str(work/'config'), XDG_CACHE_HOME=str(work/'cache'), XDG_DATA_HOME=str(work/'data'), PATH=str(work/'bin'), WAYLAND_DEBUG='1', WINIT_UNIX_BACKEND='wayland')
    log = work / 'client.log'
    with log.open('wb') as output:
        proc = subprocess.Popen(['/usr/bin/dbus-run-session', '--dbus-daemon=/usr/bin/dbus-daemon', '--', str(root/'dist/upd-cosmic-linux-amd64'), '--page', page], env=env, stdout=output, stderr=output, start_new_session=True)
        try:
            time.sleep(5)
            alive = proc.poll() is None
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
    text = log.read_text(errors='replace')
    result = dict(page=page, alive_after_5s=alive, surface_commit='wl_surface' in text and '.commit(' in text, surface_attach='.attach(' in text, panic='panicked at' in text, log=str(log))
    results.append(result)
(base/'results.json').write_text(json.dumps(results, indent=2)+'\n')
print(json.dumps(results, indent=2))
assert all(r['alive_after_5s'] and r['surface_commit'] and r['surface_attach'] and not r['panic'] for r in results)
