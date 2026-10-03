exec(open('/tmp/cm-i01-inventory.py').read().split("report={'utc'")[0])
r={'utc':utc(),'uid':os.geteuid(),'scope':'read-only host snapshot and live FlClashCore socket ownership','snapshot':snapshot(),'live_proxy_candidates':[]}
c=load('/home/somovas/.local/share/com.follow.clash/config.yaml')
ports={c.get(k):k for k in ['mixed-port','port'] if isinstance(c.get(k),int) and c[k]>0}
for p in Path('/proc').iterdir():
 if not p.name.isdigit():continue
 try:
  if (p/'comm').read_text().strip()!='FlClashCore' or (p/'exe').resolve()!=Path('/usr/lib/flclash/FlClashCore'):continue
  inodes=set()
  for fd in (p/'fd').iterdir():
   try:
    m=re.fullmatch(r'socket:\[([0-9]+)\]',os.readlink(fd))
    if m:inodes.add(m[1])
   except OSError:pass
  for line in Path('/proc/net/tcp').read_text().splitlines()[1:]:
   f=line.split();addr,port=f[1].split(':');port=int(port,16)
   if f[9] in inodes and f[3]=='0A' and addr=='0100007F' and port in ports:
    r['live_proxy_candidates'].append({'pid':int(p.name),'uid':p.stat().st_uid,'port':port,'config_role':ports[port],'socket_inode':f[9],'bind':'127.0.0.1','exe_sha256':digest((p/'exe').read_bytes()),'ownership':'exact executable + process fd inode + LISTEN socket table + saved proxy port','netns_inode':(p/'ns/net').stat().st_ino})
 except OSError:continue
r['finished_utc']=utc();print(json.dumps(r,indent=2))
