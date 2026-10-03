#!/usr/bin/python3
"""Record dirty source and compiled test digests before real local execution."""
from pathlib import Path
from datetime import datetime,timezone
import hashlib,json,os,re,subprocess,sys
root=Path('/home/somovas/Проекты/upd');evidence=root/'docs/design/cm-network-manager/i01-evidence'
def utc():return datetime.now(timezone.utc).isoformat()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def source():
 ps=sorted(set(list(root.glob('src/**/*.rs'))+list(root.glob('cosmic/src/**/*.rs'))+list(root.glob('tests/*.rs'))+[root/'Cargo.toml',root/'Cargo.lock',root/'cosmic/Cargo.toml',root/'cosmic/Cargo.lock']))
 es={str(p.relative_to(root)):sha(p) for p in ps}
 return {'files':es,'sha256':hashlib.sha256(json.dumps(es,sort_keys=True).encode()).hexdigest()}
which=sys.argv[1];cwd=root if which in ['root','guard','bin','i18n'] else root/'cosmic';target='x86_64-unknown-linux-musl' if which in ['root','guard','bin','i18n'] else 'x86_64-unknown-linux-gnu'
argv=['cargo','test','--locked','--offline','--target',target]
if which=='bin':argv+=['--bin','cm']
if which=='i18n':argv+=['--lib','i18n::']
if which=='guard':argv+=['--test','audit_i01_install_guard']
if which=='root':argv+=['--lib','--bin','cm','--test','audit_contracts','--test','audit_i01_install_guard']
r={'scope':'local unit/fixture; no real VPN/migration','started_utc':utc(),'uid':os.geteuid(),'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'target':target,'compile_argv':argv+['--no-run'],'argv':argv+['--','--test-threads=1'],'source_before':source()}
env={k:v for k,v in os.environ.items() if not k.startswith(('CM_','UPD_'))}
with (evidence/f'final-{which}-compile.log').open('wb') as log:compiled=subprocess.run(r['compile_argv'],cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT)
r['compile_exit_code']=compiled.returncode;r['compiled_utc']=utc()
bindir=cwd/'target'/target/'debug'
r['compiled_binaries']={}
for p in [bindir/'cm',bindir/'cm-cosmic',*list((bindir/'deps').glob('*'))]:
 if p.is_file() and os.access(p,os.X_OK) and re.fullmatch(r'(cm|cm_cosmic|audit_contracts|audit_i01_install_guard)(-[a-f0-9]+)?|cm-cosmic',p.name):r['compiled_binaries'][str(p.relative_to(root))]=sha(p)
r['execution_started_utc']=utc()
if compiled.returncode==0:
 with (evidence/f'final-{which}-tests.log').open('wb') as log:p=subprocess.run(r['argv'],cwd=cwd,env=env,stdout=log,stderr=subprocess.STDOUT)
 r['exit_code']=p.returncode
else:r['exit_code']=None
r['finished_utc']=utc();r['source_after']=source();r['source_unchanged']=r['source_before']==r['source_after']
evidence.joinpath(f'final-{which}-run.json').write_text(json.dumps(r,indent=2)+'\n')
print({k:v for k,v in r.items() if k not in ['source_before','source_after','compiled_binaries']})
raise SystemExit(r['exit_code'] if r['exit_code'] is not None else 1)
