#!/usr/bin/python3
"""Prepare only an own Fl worker in an offline private namespace."""
exec(open('/tmp/cm-i01-inventory.py').read().split("report={'utc'")[0])
import signal,struct,tempfile,time
source=Path('/home/somovas/.local/share/com.follow.clash/config.yaml');source_raw=source.read_bytes();saved=yaml.safe_load(source_raw)
node=next(n for n in saved['proxies'] if n.get('type')=='vless' and n.get('network','tcp')=='tcp' and n.get('tls') is True and isinstance(n.get('reality-opts'),dict) and n['reality-opts'].get('public-key'))
binary=Path('/usr/lib/flclash/FlClashCore')
r={'utc':utc(),'scope':'single own Fl worker config validation; private network, no public requests','status':'BLOCKED','binary_sha256':digest(binary.read_bytes()),'source_generation':{'kind':'FlClash saved file, first eligible VLESS TCP/REALITY node in file order; active selection unknown','sha256':digest(source_raw),'mtime_utc':fileinfo(source)['mtime_utc']},'node':{'type':'vless','network':'tcp','reality':True,'full_node_sha256':digest(json.dumps(node,sort_keys=True).encode())},'capabilities_dropped':'ALL','no_tun_device':True,'network_namespace':'private offline','public_requests':0,'steps':[]}
with tempfile.TemporaryDirectory(prefix='i01-fl-config-') as dirname:
 root=Path(dirname);home=root/'home';home.mkdir(mode=0o700)
 cfg={'mode':'rule','log-level':'silent','ipv6':False,'allow-lan':False,'bind-address':'127.0.0.1','mixed-port':48123,'port':0,'socks-port':0,'redir-port':0,'tproxy-port':0,'external-controller':'','external-controller-unix':'/mnt/home/api.sock','secret':'','listeners':[],'tunnels':[],'interface-name':'','routing-mark':0,'find-process-mode':'off','unified-delay':True,'tcp-concurrent':True,'keep-alive-interval':30,'geo-auto-update':False,'ntp':{'enable':False,'write-to-system':False},'tun':{'enable':False,'auto-route':False,'auto-redirect':False,'auto-detect-interface':False,'dns-hijack':[]},'dns':{'enable':True,'listen':'','ipv6':False,'enhanced-mode':'redir-host','respect-rules':False,'use-hosts':False,'use-system-hosts':False,'nameserver':['1.1.1.1'],'default-nameserver':['1.1.1.1'],'proxy-server-nameserver':['1.1.1.1']},'proxy-providers':{},'rule-providers':{},'proxies':[node],'proxy-groups':[{'name':'I01_FIXED','type':'select','proxies':[node['name']]}],'rules':['MATCH,I01_FIXED'],'profile':{'store-selected':False,'store-fake-ip':False}}
 raw=yaml.safe_dump(cfg,allow_unicode=True,sort_keys=False).encode();(home/'config.yaml').write_bytes(raw);(home/'config.yaml').chmod(0o600);r['shared_config_sha256']=digest(raw);r['original_full_node_preserved']=cfg['proxies'][0]==node
 listener=socket.socket(socket.AF_UNIX);listener.settimeout(4);listener.bind(str(root/'ipc.sock'));listener.listen(1)
 argv=['bwrap','--unshare-user','--unshare-net','--unshare-pid','--cap-drop','ALL','--ro-bind','/','/','--bind',str(root),'/mnt','--tmpfs','/tmp','--dev','/dev','--proc','/proc',str(binary),'/mnt/ipc.sock']
 worker=subprocess.Popen(argv,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True);conn=None
 try:
  conn,_=listener.accept();conn.settimeout(5)
  def exact(n):
   b=b''
   while len(b)<n:
    chunk=conn.recv(n-len(b))
    if not chunk:raise EOFError()
    b+=chunk
   return b
  seq=0
  def rpc(method,args=None):
   global seq
   seq+=1;ident='i01-'+str(seq);body=json.dumps({'id':ident,'method':method,'arguments':args}).encode();conn.sendall(struct.pack('<I',len(body))+body)
   deadline=time.monotonic()+5
   while time.monotonic()<deadline:
    size=struct.unpack('<I',exact(4))[0]
    if size>8*1024*1024:raise ValueError('frame too large')
    reply=json.loads(exact(size))
    if reply.get('id')!=ident:continue
    if reply.get('error') is not None:raise ValueError('private RPC error')
    return reply.get('result')
   raise TimeoutError()
  for method,args,expected in [('getIsInit',None,False),('initClash',{'home-dir':'/mnt/home','version':0},True),('validateConfig','/mnt/home/config.yaml',''),('setupConfig',{'selected-map':{'I01_FIXED':node['name']},'test-url':'https://www.gstatic.com/generate_204'},''),('startListener',None,True)]:
   value=rpc(method,args);ok=value==expected;r['steps'].append({'method':method,'expected_result_observed':ok})
   if not ok:raise ValueError('unexpected RPC result')
  pdata=rpc('getProxies');mapping=pdata.get('proxies',{}) if isinstance(pdata,dict) else {}
  r['active_proxies']={'test_node_present':node['name'] in mapping,'fixed_group_present':'I01_FIXED' in mapping,'fixed_now_matches':(mapping.get('I01_FIXED') or {}).get('now')==node['name']}
  rawconfig=rpc('getConfig','/mnt/home/config.yaml');r['raw_config_full_node_equal']=isinstance(rawconfig,dict) and rawconfig.get('proxies')==[node]
  version=api(str(home/'api.sock'),cfg);r['private_rest_api_summary']=version
  _,general=get_unix(str(home/'api.sock'),'/configs');r['effective_general_known_fields']={k:v for k in ['mixed-port','port','socks-port','redir-port','tproxy-port','allow-lan','bind-address','mode','ipv6','find-process-mode'] if isinstance(v:=(general or {}).get(k),(str,int,bool))}
  r['effective_tun_known_fields']={k:v for k in ['enable','auto-route','auto-redirect','auto-detect-interface'] if isinstance(v:=((general or {}).get('tun') or {}).get(k),bool)}
  r['source_file_unchanged']=source.read_bytes()==source_raw;r['own_config_unchanged']=(home/'config.yaml').read_bytes()==raw
  r['effective_general_matches_explicit_config']=all(r['effective_general_known_fields'].get(k)==cfg[k] for k in ['mixed-port','port','socks-port','redir-port','tproxy-port','allow-lan','bind-address','mode','ipv6','find-process-mode'])
  r['status']='PASS' if r['effective_general_matches_explicit_config'] and all(r['active_proxies'].values()) and r['raw_config_full_node_equal'] and r['own_config_unchanged'] else 'INCONCLUSIVE'
 except (OSError,EOFError,ValueError,TypeError,http.client.HTTPException) as e:r['error_class']=type(e).__name__
 finally:
  if conn:conn.close()
  listener.close()
  try:out,err=worker.communicate(timeout=3)
  except subprocess.TimeoutExpired:
   os.killpg(worker.pid,signal.SIGTERM)
   try:out,err=worker.communicate(timeout=2)
   except subprocess.TimeoutExpired:os.killpg(worker.pid,signal.SIGKILL);out,err=worker.communicate(timeout=2)
  r.update(worker_exit_code=worker.returncode,own_process_reaped=True,output_redacted=True,output_bytes=len(out)+len(err))
r['temporary_directory_removed']=not root.exists();r['finished_utc']=utc()
Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/c19-flclash-private-config-probe.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps(r,indent=2))
