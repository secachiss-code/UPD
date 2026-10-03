from pathlib import Path
from datetime import datetime,timezone
import json,hashlib,re,collections,statistics
root=Path('/home/somovas/Проекты/upd');e=root/'docs/design/cm-network-manager/i01-evidence'
def load(n):return json.loads((e/n).read_text())
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
r={'utc':datetime.now(timezone.utc).isoformat(),'scope':'read-only consolidation of actually recorded evidence; no re-execution','log_results':{},'first_failure_logs':{},'c14_pre_c19':{},'current_source_matches_final_bin_manifest':{},'c19':{}}
for p in sorted(e.glob('*.log')):
 t=p.read_text(errors='replace');r['log_results'][p.name]={'sha256':sha(p),'test_results':re.findall(r'test result: .*',t),'explicit_pass_lines':[l for l in t.splitlines() if l.startswith('PASS:')]}
 for k in ['cargo-lib.log','c18-bin-unit.log']:
  if p.name==k:
   r['first_failure_logs'][k]={'failed_tests':re.findall(r'^---- (.+) stdout ----$',t,re.M),'error_classes':{x:t.count(x) for x in ['PermissionDenied','Operation not permitted','Timeout','TimedOut']}}
a=load('baseline.json');b=load('final-host-baseline.json');r['c14_pre_c19']={'before_uid':a['uid'],'after_uid':b['uid'],'before_utc':a['utc'],'after_utc':b['utc'],'snapshot_equal_keys':{k:v==b['snapshot'].get(k) for k,v in a['snapshot'].items()},'journals_nonempty':(e/'final-journals.json').stat().st_size>0}
for name,h in load('final-bin-run.json')['source_after']['files'].items():r['current_source_matches_final_bin_manifest'][name]=(root/name).exists() and sha(root/name)==h
for name in ['c19-controls.json','c19-pilot.json']:
 if not (e/name).exists():continue
 d=load(name)
 if name=='c19-controls.json':r['c19']['controls']={'status':d['status'],'total':len(d['requests']),'successes':sum(x['success'] for x in d['requests']),'error_classes':dict(collections.Counter(x['error_class'] for x in d['requests'] if not x['success']))}
 else:
  out=[]
  for p in d['phases']:
   ms=[x for x in p.get('requests',[]) if not x['warmup']];ws=[x for x in p.get('requests',[]) if x['warmup']]
   out.append({'phase':p['phase'],'status':p['status'],'measured':len(ms),'successes':sum(x['success'] for x in ms),'warmup_successes':sum(x['success'] for x in ws),'error_classes':dict(collections.Counter(x['error_class'] for x in ms if not x['success'])),'targets':{t:{'total':sum(x['target']==t for x in ms),'successes':sum(x['target']==t and x['success'] for x in ms)} for t in ['gstatic','cloudflare_trace']},'all_attempt_latency_median_ms':statistics.median(x['latency_wall_seconds']*1000 for x in ms) if ms else None,'successful_latency_median_ms':statistics.median(x['latency_wall_seconds']*1000 for x in ms if x['success']) if any(x['success'] for x in ms) else None,'cleanup':{k:p.get(k) for k in ['own_core_reaped','temporary_directory_removed','own_config_unmodified']}})
  r['c19']['phases']=out;r['c19']['total_public_requests']=d['measured_requests']+d['warmup_requests']+r['c19'].get('controls',{}).get('total',0);r['c19']['performance_status']='INCONCLUSIVE'
  r['c19']['network_snapshot_unchanged']=d['network_snapshot_unchanged'];r['c19']['source_file_unchanged']=d['source_file_unchanged']
if (e/'c19-host-after.json').exists() and (e/'c19-host-after.json').stat().st_size:
 a=load('c19-host-before.json');b=load('c19-host-after.json');r['c19']['host_snapshot_equal_keys']={k:v==b['snapshot'].get(k) for k,v in a['snapshot'].items()};r['c19']['live_proxy_ownership_unchanged']=a['live_proxy_candidates']==b['live_proxy_candidates']
r['harness_sha256']={p.name:sha(p) for p in Path('/tmp').glob('cm-i01*.py')};(e/'execution-consolidation.json').write_text(json.dumps(r,indent=2)+'\n');print(json.dumps({k:v for k,v in r.items() if k not in ['log_results','current_source_matches_final_bin_manifest','harness_sha256']},indent=2));print('current source all equal',all(r['current_source_matches_final_bin_manifest'].values()))
