#!/usr/bin/python3
from pathlib import Path
from datetime import datetime,timezone
import hashlib,json,re,urllib.parse
root=Path('/home/somovas/Проекты/upd/docs/design/cm-network-manager')
r={'utc':datetime.now(timezone.utc).isoformat(),'scope':'DC01 read-only local markdown and machine graph consistency','checks':{},'failures':[]}
mds=list(root.glob('*.md'));files={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in mds}
checked=0
for p in mds:
 fence=None
 for number,line in enumerate(p.read_text().splitlines(),1):
  m=re.match(r'^\s{0,3}(`{3,}|~{3,})(.*)$',line)
  if m:
   marker=m[1]
   if fence is None:fence=(marker[0],len(marker),number)
   elif marker[0]==fence[0] and len(marker)>=fence[1] and not m[2].strip():fence=None
   continue
  if fence:continue
  targets=[x[1] for x in re.findall(r'(?<!!)\[([^\]]+)\]\(([^)]+)\)',line)]
  definition=re.match(r'^\s*\[[^\]]+\]:\s*(\S+)',line)
  if definition:targets.append(definition[1])
  for target in targets:
   target=target.strip().split(' "')[0].strip('<>');u=urllib.parse.urlsplit(target)
   if u.scheme or not u.path:continue
   checked+=1;dest=p.parent/urllib.parse.unquote(u.path)
   if not dest.exists():r['failures'].append({'file':p.name,'line':number,'kind':'broken_local_link','target':target})
 if fence:r['failures'].append({'file':p.name,'line':fence[2],'kind':'unclosed_fence'})
d=json.loads((root/'DAG-PLAN.json').read_text());w=json.loads((root/'MODEL-WORKFLOW.json').read_text());nodes=d['nodes'];assign=w['assignments'];tasks=[t for n in nodes for t in n['tasks']]
ids={n['id'] for n in nodes};tids={t['id'] for t in tasks};directions={n['id'] for n in nodes if re.fullmatch(r'I\d{2}',n['id'])}
edges={(dep,n['id']) for n in nodes for dep in n['depends_on']};overlay={(dep,n['milestone']) for n in assign for dep in n['depends_on']}
def acyclic(vertices,edge_set):
 indegree={v:0 for v in vertices};out={v:[] for v in vertices}
 for a,b in edge_set:
  if a not in vertices or b not in vertices:return False
  indegree[b]+=1;out[a].append(b)
 ready=[v for v,n in indegree.items() if not n];seen=0
 while ready:
  v=ready.pop();seen+=1
  for nxt in out[v]:
   indegree[nxt]-=1
   if not indegree[nxt]:ready.append(nxt)
 return seen==len(vertices)
taskedges={(dep,t['id']) for t in tasks for dep in t['depends_on']};allvertices=ids|tids
r['checks']={'markdown_files':len(mds),'local_links_checked':checked,'directions_18':len(directions)==18,'nodes_20':len(nodes)==20 and len(ids)==20,'tasks_100':len(tasks)==100 and len(tids)==100,'iteration_count_18':d['iteration_count']==18,'node_graph_acyclic':acyclic(ids,edges),'task_graph_acyclic':acyclic(allvertices,edges|taskedges),'role_overlay_same_node_ids':{a['milestone'] for a in assign}==ids,'role_overlay_same_edges':edges==overlay,'only_I01_authorized':d['authorized_directions']==['I01'] and w['authorized_directions']==['I01'],'coordinator_runs_tests_false':w['coordinator_runs_tests'] is False,'package_build_not_allowed':d['package_build_allowed'] is False and w['package_build_allowed'] is False,'dag_in_progress':d['status']=='IN_PROGRESS','statuses_valid':all(n['status'] in ['PLANNED','IN_PROGRESS','BLOCKED','DONE','COMPLETE'] for n in nodes+tasks)}
for k,v in r['checks'].items():
 if isinstance(v,bool) and not v:r['failures'].append({'kind':'machine_contract','check':k})
r['document_digests']=files;r['dag_sha256']=hashlib.sha256((root/'DAG-PLAN.json').read_bytes()).hexdigest();r['overlay_sha256']=hashlib.sha256((root/'MODEL-WORKFLOW.json').read_bytes()).hexdigest();r['status']='PASS' if not r['failures'] else 'FAIL'
(root/'i01-evidence/document-check.json').write_text(json.dumps(r,indent=2)+'\n');print({k:v for k,v in r.items() if k!='document_digests'})
