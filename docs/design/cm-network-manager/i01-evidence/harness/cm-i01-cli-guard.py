#!/usr/bin/python3
from pathlib import Path
from datetime import datetime,timezone
import hashlib,json,os,socket,stat,subprocess,tempfile

binary=Path('/home/somovas/Проекты/upd/target/x86_64-unknown-linux-musl/debug/cm')
def inventory(root):
    records=[]
    for p in [root,*sorted(root.rglob('*'))]:
        s=p.lstat()
        val=os.readlink(p).encode() if p.is_symlink() else p.read_bytes() if stat.S_ISREG(s.st_mode) else b''
        records.append((str(p.relative_to(root)),s.st_mode,s.st_uid,s.st_gid,hashlib.sha256(val).hexdigest()))
    return records
cases=[('old-config','etc/upd.conf'),('old-new-config','etc/upd.conf'),('old-data','varlib/upd/vpn/config.yaml'),('dangling-config','etc/upd.conf'),('foreign-unit','etc/systemd/system/upd-vpn.service'),('enabled-link','etc/systemd/system/custom.target.wants/upd-net.timer'),('helper-socket','run/upd/helper.sock'),('cron','etc/cron.d/upd'),('desktop','usrlocal/share/applications/io.github.upd.desktop')]
report={'utc':datetime.now(timezone.utc).isoformat(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'scope':'actual CLI inside disposable user/net/pid/mount namespace; no host install','cases':[]}
for label,relative in cases:
  with tempfile.TemporaryDirectory(prefix='i01-cli-guard-') as dirname:
    root=Path(dirname)
    for d in ['etc','varlib','run','usrlocal','bin','state']: (root/d).mkdir()
    (root/'etc/os-release').write_bytes(Path('/etc/os-release').read_bytes())
    p=root/relative;p.parent.mkdir(parents=True,exist_ok=True)
    listener=None
    if label in ['dangling-config','enabled-link']:p.symlink_to('/nonexistent-i01-legacy')
    elif label=='helper-socket':listener=socket.socket(socket.AF_UNIX);listener.bind(str(p))
    else:p.write_bytes(b'foreign private fixture\0\xff\n');p.chmod(0o640)
    if label=='old-new-config': (root/'etc/cm.conf').write_bytes(b'invalid CM configuration must never be parsed\n')
    for name in ['systemctl','pacman','gsettings','nmcli']:
        q=root/'bin'/name;q.write_text('#!/bin/sh\necho forbidden-mutation >> /mnt/commands.log\nexit 91\n');q.chmod(0o700)
    env={k:v for k,v in os.environ.items() if not k.startswith(('CM_','UPD_'))}
    # Both families deliberately attempt to divert Config; actual root guard must win.
    env.update(CM_STATE_DIR='/mnt/state',UPD_STATE_DIR='/mnt/state',CM_CONF='/mnt/not-created.conf',UPD_CONF='/mnt/not-created.conf',PATH='/mnt/bin:/usr/bin:/bin')
    argv=['bwrap','--unshare-all','--uid','0','--gid','0','--ro-bind','/','/','--bind',str(root),'/mnt','--bind',str(root/'etc'),'/etc','--bind',str(root/'varlib'),'/var/lib','--bind',str(root/'run'),'/run','--bind',str(root/'usrlocal'),'/usr/local','--tmpfs','/tmp','--dev','/dev','--proc','/proc',str(binary),'install','--package']
    before=inventory(root)
    try:
        out=subprocess.run(argv,env=env,capture_output=True,timeout=10)
        same=before==inventory(root); text=(out.stdout+out.stderr).decode(errors='replace')
        if text.startswith('bwrap:'): print(text.strip())
        # Only bwrap setup failures and synthetic fixture metadata are exported.
        path='/var/lib/upd' if label=='old-data' else '/' + relative.replace('varlib/','var/lib/').replace('usrlocal/','usr/local/')
        passed=out.returncode!=0 and 'CM install blocked:' in text and path in text and same and not (root/'commands.log').exists() and not (root/'not-created.conf').exists()
        report['cases'].append({'case':label,'exit_code':out.returncode,'expected_legacy_path_reported':path in text,'blocked_before_config_or_commands':'CM install blocked:' in text,'full_content_permissions_unchanged':same,'status':'PASS' if passed else 'FAIL','output_redacted':True,'error_markers':{x:x in text for x in ['bwrap:', 'No such file', 'Operation not permitted', 'Permission denied', 'panic', 'package', 'unsupported']}})
    except subprocess.TimeoutExpired:report['cases'].append({'case':label,'status':'FAIL','reason':'timeout'})
    finally:
        if listener:listener.close()
report['status']='PASS' if all(x['status']=='PASS' for x in report['cases']) else 'FAIL'
report['finished_utc']=datetime.now(timezone.utc).isoformat()
Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/cli-install-guard.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
raise SystemExit(0 if report['status']=='PASS' else 1)
