#!/usr/bin/python3
"""Runs inside own read-only user/pid/mount sandbox, sharing unchanged host net."""
exec(open('/mnt/inventory.py').read().split("report={'utc'")[0])
import signal,struct,time
cfg=load('/mnt/home/config.yaml');node=cfg['proxies'][0];phase=sys.argv[1]
r={'phase':phase,'started_utc':utc(),'scope':'host-conditioned proxy path; host FlClash TUN may carry outer traffic','kernel_host_uid':int(sys.argv[2]),'network_namespace_inode':os.stat('/proc/self/ns/net').st_ino,'requests':[],'status':'BLOCKED','public_endpoint':'https://www.gstatic.com/generate_204'}
core=worker=None;ipc=None;listener=None
def own_listener(pid,port):
    inodes=set()
    for fd in Path(f'/proc/{pid}/fd').iterdir():
        try:
            m=re.fullmatch(r'socket:\[([0-9]+)\]',os.readlink(fd))
            if m:inodes.add(m[1])
        except OSError:pass
    for line in Path('/proc/net/tcp').read_text().splitlines()[1:]:
        f=line.split()
        if len(f)>=10 and f[9] in inodes and f[3]=='0A' and f[1]==f'0100007F:{port:04X}':return True
    return False
try:
    reserve=socket.socket();reserve.bind(('127.0.0.1',cfg['mixed-port']));reserve.close()
    if phase=='B':
        listener=socket.socket(socket.AF_UNIX);listener.settimeout(5);listener.bind('/mnt/ipc.sock');listener.listen(1)
        worker=subprocess.Popen(['/usr/lib/flclash/FlClashCore','/mnt/ipc.sock'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
        ipc,_=listener.accept();ipc.settimeout(5);seq=0
        def exact(n):
            raw=b''
            while len(raw)<n:
                c=ipc.recv(n-len(raw))
                if not c:raise EOFError()
                raw+=c
            return raw
        def rpc(method,args=None):
            global seq
            seq+=1;ident='i01-'+str(seq);body=json.dumps({'id':ident,'method':method,'arguments':args}).encode();ipc.sendall(struct.pack('<I',len(body))+body)
            for _ in range(100):
                size=struct.unpack('<I',exact(4))[0]
                if size>8*1024*1024:raise ValueError('large frame')
                reply=json.loads(exact(size))
                if reply.get('id')!=ident:continue
                if reply.get('error') is not None:raise ValueError('rpc error')
                return reply.get('result')
            raise ValueError('missing reply')
        for method,args,expected in [('initClash',{'home-dir':'/mnt/home','version':0},True),('setupConfig',{'selected-map':{'I01_FIXED':node['name']},'test-url':r['public_endpoint']},''),('startListener',None,True)]:
            if rpc(method,args)!=expected:raise ValueError('unexpected RPC result')
    else:
        worker=subprocess.Popen(['/mnt/mihomo','-d','/mnt/home','-f','/mnt/home/config.yaml'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
    deadline=time.monotonic()+5
    while time.monotonic()<deadline and not Path('/mnt/home/api.sock').exists() and worker.poll() is None:time.sleep(.05)
    status,ver=get_unix('/mnt/home/api.sock','/version');r['core_api_version']=ver.get('version') if isinstance(ver,dict) and re.fullmatch(r'[v0-9.a-zA-Z_+\-]{1,100}',str(ver.get('version'))) else 'unavailable'
    _,general=get_unix('/mnt/home/api.sock','/configs');_,pdata=get_unix('/mnt/home/api.sock','/proxies')
    r['same_fixed_node_active']=(pdata.get('proxies',{}).get('I01_FIXED') or {}).get('now')==node['name']
    r['expected_loopback_proxy']=(general or {}).get('mixed-port')==cfg['mixed-port'] and (general or {}).get('bind-address')=='127.0.0.1' and (general or {}).get('allow-lan') is False
    r['tun_disabled']=(general.get('tun') or {}).get('enable') is False and (general.get('tun') or {}).get('auto-route') is False
    r['own_proxy_socket_confirmed']=own_listener(worker.pid,cfg['mixed-port'])
    cap=Path(f'/proc/{worker.pid}/status').read_text();r['core_capabilities_zero']=all(int(re.search(r'^'+k+r':\s*([0-9a-fA-F]+)',cap,re.M)[1],16)==0 for k in ['CapInh','CapPrm','CapEff','CapBnd','CapAmb'])
    if not all(r.get(k) for k in ['same_fixed_node_active','expected_loopback_proxy','tun_disabled','own_proxy_socket_confirmed','core_capabilities_zero']):raise ValueError('readiness/ownership failed')
    for index in range(11):
        t=utc();start=time.monotonic()
        argv=['/usr/bin/curl','--noproxy','','--proxy','http://127.0.0.1:'+str(cfg['mixed-port']),'--max-time','10','--connect-timeout','10','--silent','--show-error','--output','/dev/null','--write-out','%{http_code} %{time_total} %{time_starttransfer} %{size_download}',r['public_endpoint']]
        response=subprocess.run(argv,capture_output=True,timeout=12);elapsed=time.monotonic()-start
        fields=response.stdout.decode(errors='replace').strip().split();http=int(fields[0]) if len(fields)==4 and fields[0].isdigit() else None
        success=response.returncode==0 and http==204 and fields[3]=='0'
        err=response.stderr.decode(errors='replace').lower();error_class=None
        if not success:
            error_class={5:'proxy_dns',6:'endpoint_dns',7:'proxy_connect',28:'timeout',35:'tls',56:'receive_or_proxy_connect'}.get(response.returncode,'http_status_or_other')
            m=re.search(r'connect tunnel failed, response (\d{3})',err)
            if m:error_class='proxy_connect_response_'+m[1]
        r['requests'].append({'utc':t,'index':index,'warmup':index==0,'success':success,'curl_exit_code':response.returncode,'http_status':http,'latency_wall_seconds':elapsed,'curl_total_seconds':float(fields[1]) if len(fields)==4 else None,'ttfb_seconds':float(fields[2]) if len(fields)==4 else None,'error_class':error_class})
    r['status']='PASS' if all(x['success'] for x in r['requests'] if not x['warmup']) else 'FAIL'
except (OSError,EOFError,ValueError,TypeError,subprocess.TimeoutExpired,http.client.HTTPException) as e:r['error_class']=type(e).__name__
finally:
    if ipc:ipc.close()
    if listener:listener.close()
    if worker:
        if phase!='B' and worker.poll() is None:os.killpg(worker.pid,signal.SIGTERM)
        try:out,err=worker.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(worker.pid,signal.SIGTERM)
            try:out,err=worker.communicate(timeout=2)
            except subprocess.TimeoutExpired:os.killpg(worker.pid,signal.SIGKILL);out,err=worker.communicate(timeout=2)
        r.update(own_core_reaped=True,core_exit_code=worker.returncode,core_output_redacted=True)
r['finished_utc']=utc();print(json.dumps(r))
