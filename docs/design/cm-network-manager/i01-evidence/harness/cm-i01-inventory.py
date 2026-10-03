#!/usr/bin/python3
"""I01 read-only host inventory. Never print raw configs, API bodies or journals."""
from pathlib import Path
from datetime import datetime, timezone
import collections, hashlib, http.client, json, os, re, socket, stat, subprocess, sys
import yaml

def utc(): return datetime.now(timezone.utc).isoformat()
def run(argv, timeout=20):
    try:
        p=subprocess.run(argv,capture_output=True,timeout=timeout)
        return p.returncode,p.stdout
    except (OSError,subprocess.TimeoutExpired): return -1,b''
def digest(raw): return hashlib.sha256(raw).hexdigest()
def fileinfo(p):
    p=Path(p)
    try:
        s=p.stat(); raw=p.read_bytes()
        return {'exists':True,'sha256':digest(raw),'size':s.st_size,'mode':oct(stat.S_IMODE(s.st_mode)),'uid':s.st_uid,'gid':s.st_gid,'mtime_utc':datetime.fromtimestamp(s.st_mtime,timezone.utc).isoformat()}
    except OSError as e: return {'exists':p.exists(),'error_class':type(e).__name__}
def safe_enum(x,allowed): return x if x in allowed else 'other_or_missing'
TYPES=['vless','vmess','ss','ssr','trojan','hysteria','hysteria2','tuic','socks5','http','wireguard','direct','reject','Selector','URLTest','Fallback','LoadBalance','VLESS','VMess','Shadowsocks','Trojan','Direct','Reject']
NETS=['tcp','ws','grpc','h2','http','quic','kcp']
def load(p):
    try: return yaml.safe_load(Path(p).read_bytes()) or {}
    except (OSError,yaml.YAMLError): return {}
def overview(c):
    nodes=c.get('proxies') or []
    allowed=['rule','global','direct','mixed','system','gvisor','fake-ip','redir-host','off','always']
    result={'node_count':len(nodes),'types':dict(collections.Counter(safe_enum(n.get('type'),TYPES) for n in nodes)),'networks':dict(collections.Counter(safe_enum(n.get('network','tcp'),NETS) for n in nodes)),'global_knobs':{},'providers':len(c.get('proxy-providers') or {}),'rules':len(c.get('rules') or [])}
    for section,keys in [('', ['mode','ipv6','unified-delay','tcp-concurrent','find-process-mode','keep-alive-interval']),('tun',['enable','stack','mtu','auto-route','auto-redirect','auto-detect-interface','strict-route']),('dns',['enable','ipv6','enhanced-mode','respect-rules','use-hosts','use-system-hosts','prefer-h3'])]:
        obj=c if not section else c.get(section) or {}
        result['global_knobs'][section or 'general']={k:v for k in keys if (isinstance(v:=obj.get(k),(bool,int)) or (isinstance(v,str) and v in allowed))}
    return result
def compare(a,b):
    an={n.get('name'):n for n in a.get('proxies') or []};bn={n.get('name'):n for n in b.get('proxies') or []}; common=set(an)&set(bn)
    # Only schema names, never arbitrary YAML map keys (headers may contain credentials).
    known=['name','type','server','port','uuid','password','cipher','network','tls','servername','sni','client-fingerprint','udp','skip-cert-verify','reality-opts','ws-opts','grpc-opts','http-opts','flow']
    changes=collections.Counter()
    for n in common:
        for k in set(an[n])|set(bn[n]):
            if an[n].get(k)!=bn[n].get(k): changes[k if k in known else 'other_field']+=1
    return {'a_count':len(an),'b_count':len(bn),'common_named_nodes':len(common),'full_identical_nodes':sum(an[n]==bn[n] for n in common),'different_fields':dict(changes),'full_node_equality_includes_credentials':True,'rules_equal':a.get('rules')==b.get('rules'),'dns_full_equal':a.get('dns')==b.get('dns')}
class UnixHTTP(http.client.HTTPConnection):
    def __init__(self,p): super().__init__('localhost',timeout=3);self.p=p
    def connect(self):
        self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.settimeout(3);self.sock.connect(self.p)
def get_unix(p,path,secret=None):
    c=UnixHTTP(p)
    try:
        c.request('GET',path,headers={'Authorization':'Bearer '+secret} if secret else {})
        r=c.getresponse();raw=r.read(8*1024*1024+1)
        if r.status!=200 or len(raw)>8*1024*1024: return r.status,None
        return r.status,json.loads(raw)
    finally:c.close()
def api(p,c):
    result={'transport':'unix','methods':['GET /version','GET /proxies'],'reachable':False}
    try:
        status,ver=get_unix(p,'/version',c.get('secret'));result['http_status']=status
        if not isinstance(ver,dict):return result
        v=ver.get('version','');result['version']=v if isinstance(v,str) and re.fullmatch(r'[v0-9.a-zA-Z_+\-]{1,100}',v) else 'unavailable';result['reachable']=True
        status,pdata=get_unix(p,'/proxies',c.get('secret'));result['proxies_http_status']=status
        if not isinstance(pdata,dict):return result
        proxies=pdata.get('proxies') or {};roots=[(r.split(',')[-1]) for r in c.get('rules') or [] if isinstance(r,str) and r.startswith('MATCH,')]
        root=roots[-1] if roots else ('GLOBAL' if c.get('mode')=='global' else None)
        seen=set();chain=[];end='missing_root';name=root
        for _ in range(20):
            if name in seen:end='loop';break
            if name not in proxies:end='missing_entry';break
            seen.add(name);e=proxies[name];h=e.get('history') or []
            chain.append({'type':safe_enum(e.get('type'),TYPES),'alive':e.get('alive') if isinstance(e.get('alive'),bool) else None,'latest_delay_ms':h[-1].get('delay') if h and isinstance(h[-1].get('delay'),int) else None})
            if not e.get('now'):
                end='terminal';node=next((n for n in c.get('proxies') or [] if n.get('name')==name),None)
                result['terminal_config_match']=node is not None
                if node:result['terminal_node']={'type':safe_enum(node.get('type'),TYPES),'network':safe_enum(node.get('network','tcp'),NETS)}
                break
            name=e['now']
        else:end='depth_limit'
        result.update(chain=chain,termination=end,root_basis='MATCH rule from current runtime file; API resolves now values',loop_detected=end=='loop')
    except (OSError,ValueError,http.client.HTTPException) as e:result['error_class']=type(e).__name__
    return result
def snapshot():
    paths=['/etc/upd/vpn','/etc/cm/vpn','/var/lib/upd/vpn','/var/lib/cm/vpn','/home/somovas/.local/share/com.follow.clash']
    result={}
    for label,root in enumerate(paths):
        p=Path(root);records=[];errors=0
        if not p.exists():result[str(label)]={'exists':False};continue
        for f in sorted(p.rglob('*')):
            try:
                s=f.lstat()
                if stat.S_ISREG(s.st_mode):records.append((str(f.relative_to(p)),s.st_mode,s.st_uid,s.st_gid,digest(f.read_bytes())))
                elif stat.S_ISDIR(s.st_mode):records.append((str(f.relative_to(p)),s.st_mode,s.st_uid,s.st_gid,'directory'))
                # Sockets are runtime resources, do not open or hash them.
            except OSError:errors+=1
        result[str(label)]={'exists':True,'entries':len(records),'errors':errors,'content_permissions_digest':digest(json.dumps(records,sort_keys=True).encode())}
    code,routes=run(['ip','-j','route','show','table','all']);result['routes']={'exit_code':code,'sha256':digest(routes)}
    result['resolv_conf']=fileinfo('/etc/resolv.conf')
    for unit in ['upd-vpn.service','cm-vpn.service','upd-helper.service','cm-helper.service']:
        code,out=run(['systemctl','show',unit,'--property=LoadState,ActiveState,SubState,MainPID'])
        result[unit]={'exit_code':code,'properties':out.decode(errors='replace').strip().splitlines()}
    return result
report={'utc':utc(),'mode':sys.argv[1] if len(sys.argv)>1 else 'baseline','uid':os.geteuid()}
report['snapshot']=snapshot()
if report['mode']=='snapshot':print(json.dumps(report,indent=2));sys.exit()
report['binaries']={p:fileinfo(p) for p in ['/usr/local/bin/upd','/usr/local/bin/cm','/var/lib/upd/vpn/bin/mihomo','/var/lib/cm/vpn/bin/mihomo']}
configs={label:load(p) for label,p in [('UPD_runtime','/var/lib/upd/vpn/config.yaml'),('FlClash_saved','/home/somovas/.local/share/com.follow.clash/config.yaml'),('CM_runtime','/var/lib/cm/vpn/config.yaml')]}
report['configs']={label:{'file':fileinfo(p),'overview':overview(configs[label])} for label,p in [('UPD_runtime','/var/lib/upd/vpn/config.yaml'),('FlClash_saved','/home/somovas/.local/share/com.follow.clash/config.yaml'),('CM_runtime','/var/lib/cm/vpn/config.yaml')]}
try:
    meta=json.loads(Path('/etc/upd/vpn/subs.json').read_bytes());active=next((x for x in meta.get('list',[]) if x.get('id')==meta.get('active')),None)
    if active and re.fullmatch('[A-Za-z0-9_-]+',active.get('id','')) and active.get('kind')=='clash':
        p=Path('/var/lib/upd/vpn/profiles')/(active['id']+'.yaml');configs['UPD_source']=load(p);report['configs']['UPD_source']={'file':fileinfo(p),'overview':overview(configs['UPD_source'])}
except (OSError,ValueError):report['source_metadata_unavailable']=True
report['comparisons']={a+'_vs_'+b:compare(configs[a],configs[b]) for a,b in [('UPD_runtime','FlClash_saved'),('UPD_source','UPD_runtime'),('UPD_source','FlClash_saved')] if a in configs and b in configs}
report['processes']=[];sockets=set()
for p in Path('/proc').iterdir():
    if not p.name.isdigit():continue
    try:
        comm=(p/'comm').read_text().strip()
        if comm.lower() not in ['flclash','flclashcore','mihomo','upd','cm']:continue
        args=(p/'cmdline').read_bytes().split(b'\0');args=[x.decode(errors='replace') for x in args if x];exe=str((p/'exe').resolve());s=p.stat()
        report['processes'].append({'pid':int(p.name),'comm':comm,'uid':s.st_uid,'exe_basename':Path(exe).name,'binary':fileinfo(p/'exe'),'argv_shape':[('flag:'+x if re.fullmatch('-[a-zA-Z-]{1,30}',x) else 'value_redacted') for x in args]})
        for flag in ['-ext-ctl-unix','--ext-ctl-unix']:
            if flag in args and args.index(flag)+1<len(args):sockets.add(args[args.index(flag)+1])
    except OSError:continue
report['api']={'UPD':api('/var/lib/upd/vpn/mihomo.sock',configs['UPD_runtime'])}
flsock=configs['FlClash_saved'].get('external-controller-unix')
if isinstance(flsock,str):sockets.add(flsock)
report['flclash_unix_candidate_count']=len(sockets)
report['flclash_api']=[api(p,configs['FlClash_saved']) for p in sorted(sockets) if p!='/var/lib/upd/vpn/mihomo.sock']
report['journals']={}
for unit in ['upd-vpn.service','cm-vpn.service']:
    code,out=run(['journalctl','-u',unit,'--no-pager','-o','json'],timeout=30);counts=collections.Counter();first=None;last=None;events=0;errors=0;boots=set()
    for line in out.splitlines():
        try:
            e=json.loads(line);msg=e.get('MESSAGE','');ts=int(e.get('__REALTIME_TIMESTAMP',0));events+=1
            if isinstance(msg,list):msg=''
            first=ts if first is None else min(first,ts);last=ts if last is None else max(last,ts)
            if e.get('_BOOT_ID'):boots.add(e['_BOOT_ID'])
            low=msg.lower()
            if any(x in low for x in ['error','failed','failure','timeout','context deadline']):
                errors+=1
                for cat,words in {'DNS':['dns','resolve','lookup'],'dial':['dial','connect:','connection refused'],'TLS':['tls','handshake','certificate'],'WS':['websocket',' ws ','wsarecv'],'timeout':['timeout','deadline exceeded'],'TUN':['tun','route','device or resource busy']}.items():
                    if any(w in low for w in words):counts[cat]+=1
        except (ValueError,TypeError):continue
    report['journals'][unit]={'exit_code':code,'events':events,'error_events':errors,'nonexclusive_error_classes':dict(counts),'first_utc':datetime.fromtimestamp(first/1e6,timezone.utc).isoformat() if first else None,'last_utc':datetime.fromtimestamp(last/1e6,timezone.utc).isoformat() if last else None,'boot_generations':len(boots),'config_generation':'unknown','historical_causality':'INCONCLUSIVE'}
report['finished_utc']=utc()
print(json.dumps(report,indent=2,ensure_ascii=False))
