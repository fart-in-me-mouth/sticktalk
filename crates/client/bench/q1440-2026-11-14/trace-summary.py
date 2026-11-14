"""Sampled startup queue maxima and all resets, attributed by authenticated peer."""
import pathlib,re,json
ROOT=pathlib.Path(__file__).resolve().parent
out={}
for run in sorted(ROOT.iterdir()):
    if not (run/'run.json').exists(): continue
    log=ROOT/(run.name+'.trace.log')
    if not log.exists(): continue
    peers={}
    resets=[]
    for line in log.read_text(errors='replace').splitlines():
        if 'original_queue_reset' in line: resets.append(line)
        if 'startup_queue' not in line: continue
        peer=re.search(r'peer=(VoiceSession \{.*?\})',line)[1]
        data={k:float(v) for k,v in re.findall(r'(\w+)=([0-9.]+)',line)}
        p=peers.setdefault(peer,{'samples':0,'max_bytes':0,'max_age_ms':0,'max_resets':0,'max_keyframe_bytes':0,'min_bwe':1e99,'max_bwe':0})
        p['samples']+=1
        for dest,src in [('max_bytes','bytes'),('max_age_ms','age_ms'),('max_resets','resets'),('max_keyframe_bytes','keyframe_bytes'),('max_bwe','bwe')]: p[dest]=max(p[dest],data[src])
        p['min_bwe']=min(p['min_bwe'],data['bwe'])
    out[run.name]={'sampled_first_15s':peers,'all_guard_resets':resets}
(ROOT/'startup-trace-summary.json').write_text(json.dumps(out,indent=2))
print(json.dumps(out,indent=2))
