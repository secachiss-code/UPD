exec(open('/tmp/cm-i01-inventory.py').read().split("report={'utc'")[0])
import time
root=Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence');b=json.loads((root/'c19-host-before.json').read_text())
r={'started_utc':utc(),'status':'BLOCKED','scope':'live FlClash owned loopback proxy path, live selected node unknown; no IPC mutations','requests':[],'budget':{'controls_max':4,'concurrency':1,'timeout_seconds':10},'source_ownership_evidence':'c19-host-before.json'}
candidates=b['live_proxy_candidates']
if len(candidates)==1:
 c=candidates[0];r['owned_port']=c['port'];r['core_pid']=c['pid'];r['socket_inode']=c['socket_inode']
 for index in range(4):
  rows=[x.split() for x in Path('/proc/net/tcp').read_text().splitlines()[1:]]
  if not any(f[9]==c['socket_inode'] and f[3]=='0A' and f[1]==f'0100007F:{c["port"]:04X}' for f in rows):r['error_class']='owned_listener_changed';break
  target='gstatic' if index%2==0 else 'cloudflare_trace';endpoint='https://www.gstatic.com/generate_204' if target=='gstatic' else 'https://www.cloudflare.com/cdn-cgi/trace';expected=204 if target=='gstatic' else 200
  argv=['/usr/bin/curl','--noproxy','','--proxy','http://127.0.0.1:'+str(c['port']),'--max-time','10','--connect-timeout','10','--silent','--show-error','--output','/dev/null','--write-out','%{http_code} %{time_total} %{time_starttransfer} %{size_download}',endpoint]
  start=time.monotonic();p=subprocess.run(argv,capture_output=True,timeout=12);fields=p.stdout.decode(errors='replace').split();http=int(fields[0]) if len(fields)==4 and fields[0].isdigit() else None;ok=p.returncode==0 and http==expected and (expected!=204 or fields[3]=='0')
  r['requests'].append({'utc':utc(),'index':index,'target':target,'expected_http':expected,'http_status':http,'success':ok,'curl_exit_code':p.returncode,'latency_wall_seconds':time.monotonic()-start,'curl_total_seconds':float(fields[1]) if len(fields)==4 else None,'ttfb_seconds':float(fields[2]) if len(fields)==4 else None,'error_class':None if ok else {5:'proxy_dns',6:'endpoint_dns',7:'proxy_connect',28:'timeout',35:'tls',56:'receive_or_proxy_connect'}.get(p.returncode,'http_status_or_other')})
 if len(r['requests'])==4:r['status']='PASS' if all(x['success'] for x in r['requests']) else 'FAIL'
else:r['error_class']='no_unique_proven_live_loopback_proxy'
r['finished_utc']=utc();r['body_handling']='curl /dev/null; raw response bodies and errors not exported';(root/'c19-controls.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r,indent=2))
