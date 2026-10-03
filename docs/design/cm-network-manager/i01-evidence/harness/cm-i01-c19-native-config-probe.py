#!/usr/bin/python3
"""Native core same-config validation and private startup, with no egress."""
exec(open('/tmp/cm-i01-inventory.py').read().split("report={'utc'")[0])
import signal,tempfile,time,shutil
# Reuse the exact config construction, not a separately edited configuration.
fltext=Path('/tmp/cm-i01-c19-fl-config-probe.py').read_text();construction=fltext.split(" cfg=",1)[1].split(" listener=",1)[0]
source=Path('/home/somovas/.local/share/com.follow.clash/config.yaml');source_raw=source.read_bytes();saved=yaml.safe_load(source_raw)
node=next(n for n in saved['proxies'] if n.get('type')=='vless' and n.get('network','tcp')=='tcp' and n.get('tls') is True and isinstance(n.get('reality-opts'),dict) and n['reality-opts'].get('public-key'))
binary=Path('/tmp/cm-i01-native-mihomo');r={'utc':utc(),'scope':'native same-config offline validate/start; private netns/cap-drop ALL/no TUN','binary_sha256':digest(binary.read_bytes()),'status':'BLOCKED','public_requests':0,'node':{'type':'vless','network':'tcp','reality':True,'full_node_sha256':digest(json.dumps(node,sort_keys=True).encode())}}
with tempfile.TemporaryDirectory(prefix='i01-native-config-') as dirname:
 root=Path(dirname);home=root/'home';home.mkdir(mode=0o700)
 # The extracted audited block only builds/writes our private config and summarizes hashes.
 exec('cfg='+construction.replace('\n ', '\n'))
 shutil.copyfile(binary,root/'mihomo');(root/'mihomo').chmod(0o700)
 base=['bwrap','--unshare-user','--unshare-net','--unshare-pid','--cap-drop','ALL','--ro-bind','/','/','--bind',str(root),'/mnt','--tmpfs','/tmp','--dev','/dev','--proc','/proc','/mnt/mihomo']
 validated=subprocess.run(base+['-t','-d','/mnt/home','-f','/mnt/home/config.yaml'],capture_output=True,timeout=10);r['native_validate_exit_code']=validated.returncode;r['validate_output_redacted']=True
 if validated.returncode==0:
  worker=subprocess.Popen(base+['-d','/mnt/home','-f','/mnt/home/config.yaml'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
  try:
   deadline=time.monotonic()+5
   while time.monotonic()<deadline and not (home/'api.sock').exists() and worker.poll() is None:time.sleep(.05)
   r['private_rest_api_summary']=api(str(home/'api.sock'),cfg)
   _,general=get_unix(str(home/'api.sock'),'/configs');r['effective_general_known_fields']={k:v for k in ['mixed-port','port','socks-port','redir-port','tproxy-port','allow-lan','bind-address','mode','ipv6','find-process-mode'] if isinstance(v:=(general or {}).get(k),(str,int,bool))}
   r['effective_tun_known_fields']={k:v for k in ['enable','auto-route','auto-redirect','auto-detect-interface'] if isinstance(v:=((general or {}).get('tun') or {}).get(k),bool)}
   r['effective_general_matches_explicit_config']=all(r['effective_general_known_fields'].get(k)==cfg[k] for k in ['mixed-port','port','socks-port','redir-port','tproxy-port','allow-lan','bind-address','mode','ipv6','find-process-mode'])
   status,pdata=get_unix(str(home/'api.sock'),'/proxies');proxies=(pdata or {}).get('proxies') or {};r['fixed_selection_matches']=(proxies.get('I01_FIXED') or {}).get('now')==node['name']
   r['source_file_unchanged']=source.read_bytes()==source_raw;r['own_config_unchanged']=(home/'config.yaml').read_bytes()==raw
   r['status']='PASS' if r['effective_general_matches_explicit_config'] and r['fixed_selection_matches'] and r['own_config_unchanged'] else 'INCONCLUSIVE'
  except (OSError,ValueError,http.client.HTTPException) as e:r['error_class']=type(e).__name__
  finally:
   if worker.poll() is None:os.killpg(worker.pid,signal.SIGTERM)
   try:out,err=worker.communicate(timeout=3)
   except subprocess.TimeoutExpired:os.killpg(worker.pid,signal.SIGKILL);out,err=worker.communicate(timeout=2)
   r.update(worker_exit_code=worker.returncode,own_process_reaped=True,output_redacted=True,output_bytes=len(out)+len(err))
r['temporary_directory_removed']=not root.exists();r['finished_utc']=utc()
Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/c19-native-private-config-probe.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r,indent=2))
