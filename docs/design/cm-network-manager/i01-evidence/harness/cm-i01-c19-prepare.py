from pathlib import Path
old="n.get('type')=='vless' and n.get('network','tcp')=='ws'"
new="n.get('type')=='vless' and n.get('network','tcp')=='tcp' and n.get('tls') is True and isinstance(n.get('reality-opts'),dict) and n['reality-opts'].get('public-key')"
for name in ['fl-config-probe','native-config-probe','pilot','pilot-phase']:
 s=Path('/tmp/cm-i01-'+name+'.py').read_text().replace(old,new).replace('cm-i01-fl-config-probe.py','cm-i01-c19-fl-config-probe.py').replace('cm-i01-pilot-phase.py','cm-i01-c19-pilot-phase.py')
 s=s.replace('first VLESS WS node','first eligible VLESS TCP/REALITY node').replace("'network':'ws'","'network':'tcp','reality':True")
 s=s.replace('i01-evidence/flclash-private-config-probe.json','i01-evidence/c19-flclash-private-config-probe.json').replace('i01-evidence/native-private-config-probe.json','i01-evidence/c19-native-private-config-probe.json').replace('i01-evidence/pilot.json','i01-evidence/c19-pilot.json')
 if name=='pilot-phase':
  s=s.replace("for index in range(11):", "for index in range(11):\n        endpoint='https://www.gstatic.com/generate_204' if index==0 or index%2==1 else 'https://www.cloudflare.com/cdn-cgi/trace'\n        expected_http=204 if 'gstatic' in endpoint else 200")
  s=s.replace("r['public_endpoint']]","endpoint]").replace("http==204 and fields[3]=='0'","http==expected_http and (expected_http!=204 or fields[3]=='0')")
  s=s.replace("{'utc':t,'index':index", "{'utc':t,'target':'gstatic' if expected_http==204 else 'cloudflare_trace','expected_http':expected_http,'index':index")
  s=s.replace("r['finished_utc']=utc();print", "r['body_handling']='curl output /dev/null; raw bodies never exported';r['finished_utc']=utc();print")
 if name=='pilot':
  s=s.replace("begin=time.monotonic()", "assert all(json.loads(Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager/i01-evidence/'+f).read_text())['status']=='PASS' for f in ['c19-native-private-config-probe.json','c19-flclash-private-config-probe.json'])\nbegin=time.monotonic()")
  s=s.replace("'performance_status':'INCONCLUSIVE'", "'performance_status':'INCONCLUSIVE','contract':'C19','node_selection':'first eligible complete VLESS TCP REALITY in Fl saved file order; live selection unknown','targets':['https://www.gstatic.com/generate_204','https://www.cloudflare.com/cdn-cgi/trace']")
 Path('/tmp/cm-i01-c19-'+name+'.py').write_text(s)
