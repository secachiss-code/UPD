#!/usr/bin/python3
"""Bounded sequential A1/B/A2 pilot. No live service or routing mutations."""
exec(open('/tmp/cm-i01-inventory.py').read().split("report={'utc'")[0])
import shutil,tempfile,time
source=Path('/home/somovas/.local/share/com.follow.clash/config.yaml');source_raw=source.read_bytes();saved=yaml.safe_load(source_raw)
node=next(n for n in saved['proxies'] if n.get('type')=='vless' and n.get('network','tcp')=='tcp' and n.get('tls') is True and isinstance(n.get('reality-opts'),dict) and n['reality-opts'].get('public-key'))
if any(node.get(k) for k in ['dialer-proxy','interface-name','routing-mark']):raise ValueError('unsafe node resource override')
construction=Path('/tmp/cm-i01-c19-fl-config-probe.py').read_text().split(' cfg=',1)[1].split(' listener=',1)[0]
def network_snapshot():
    out={}
    for key,argv in [('routes',['ip','-j','route','show','table','all']),('rules',['ip','-j','rule','show'])]:
        code,raw=run(argv);out[key]={'exit_code':code,'sha256':digest(raw)}
    out['namespace_inode']=os.stat('/proc/self/ns/net').st_ino;out['FlClashCore_live']=any((p/'comm').read_text().strip()=='FlClashCore' for p in Path('/proc').iterdir() if p.name.isdigit() and (p/'comm').exists())
    return out
r={'started_utc':utc(),'scope':'host-conditioned proxy pilot, same shared host network namespace; live FlClash TUN may carry outer traffic; no physical independence proof','source_file_sha256':digest(source_raw),'full_node_sha256':digest(json.dumps(node,sort_keys=True).encode()),'shared_config_sha256':None,'native_binary_sha256':digest(Path('/tmp/cm-i01-native-mihomo').read_bytes()),'fl_binary_sha256':digest(Path('/usr/lib/flclash/FlClashCore').read_bytes()),'before':network_snapshot(),'phases':[],'request_budget':{'measured_max':30,'warmup_max':3,'concurrency':1,'timeout_seconds':10,'deadline_seconds':900},'performance_status':'INCONCLUSIVE','contract':'C19','node_selection':'first eligible complete VLESS TCP REALITY in Fl saved file order; live selection unknown','targets':['https://www.gstatic.com/generate_204','https://www.cloudflare.com/cdn-cgi/trace']}
assert all(json.loads(Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/'+f).read_text())['status']=='PASS' for f in ['c19-native-private-config-probe.json','c19-flclash-private-config-probe.json'])
control_started=datetime.fromisoformat(json.loads(Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/c19-controls.json').read_text())['started_utc'])
assert (datetime.now(timezone.utc)-control_started).total_seconds()<300
begin=time.monotonic()-(datetime.now(timezone.utc)-control_started).total_seconds()
for phase in ['A1','B','A2']:
    if time.monotonic()-begin>900:raise TimeoutError('pilot deadline')
    with tempfile.TemporaryDirectory(prefix='i01-pilot-'+phase+'-') as dirname:
        root=Path(dirname);home=root/'home';home.mkdir(mode=0o700)
        # Reuse audited byte-identical config construction; no config goes to evidence.
        config_summary={};oldr=r;r=config_summary;exec('cfg='+construction.replace('\n ','\n'));r=oldr
        if r['shared_config_sha256'] is None:r['shared_config_sha256']=config_summary['shared_config_sha256']
        if r['shared_config_sha256']!=config_summary['shared_config_sha256']:raise ValueError('config parity lost')
        shutil.copyfile('/tmp/cm-i01-native-mihomo',root/'mihomo');(root/'mihomo').chmod(0o700)
        shutil.copyfile('/tmp/cm-i01-inventory.py',root/'inventory.py');shutil.copyfile('/tmp/cm-i01-c19-pilot-phase.py',root/'phase.py')
        argv=['bwrap','--unshare-user','--unshare-pid','--cap-drop','ALL','--ro-bind','/','/','--bind',str(root),'/mnt','--tmpfs','/tmp','--dev','/dev','--proc','/proc','/usr/bin/python3','/mnt/phase.py',phase,str(os.getuid())]
        env={k:v for k,v in os.environ.items() if not k.startswith(('CM_','UPD_')) and not k.lower().endswith('_proxy')}
        p=subprocess.run(argv,env=env,capture_output=True,timeout=min(160,900-(time.monotonic()-begin)))
        if p.returncode!=0:result={'phase':phase,'status':'BLOCKED','worker_namespace_exit_code':p.returncode,'output_redacted':True}
        else:result=json.loads(p.stdout)
        result['lifecycle']='fresh core and private home, profile selection/cache persistence disabled';result['same_config_sha256']=config_summary['shared_config_sha256'];r['phases'].append(result)
        result['own_config_unmodified']=(home/'config.yaml').read_bytes()==raw
    result['temporary_directory_removed']=not root.exists()
    if result['status']=='BLOCKED':break
r['after']=network_snapshot();r['network_snapshot_unchanged']=r['before']==r['after'];r['source_file_unchanged']=source.read_bytes()==source_raw
r['measured_requests']=sum(not x['warmup'] for p in r['phases'] for x in p.get('requests',[]));r['warmup_requests']=sum(x['warmup'] for p in r['phases'] for x in p.get('requests',[]))
r['connectivity_status']='PASS' if len(r['phases'])==3 and all(p['status']=='PASS' for p in r['phases']) else 'BLOCKED' if any(p['status']=='BLOCKED' for p in r['phases']) else 'FAIL'
r['finished_utc']=utc()
Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/c19-pilot.json').write_text(json.dumps(r,indent=2)+'\n');print({k:v for k,v in r.items() if k!='phases'})
