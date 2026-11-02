import json,random,subprocess,sys,os
random.seed(7)
man=json.load(open('../manifest.json'))
ids=[p['id'] for p in man['params']]
fails=0
for run in range(70):
    o={}
    for p in man['params']:
        if random.random()<0.45: continue
        t=p['type']
        if t=='slider': o[p['id']]=round(random.uniform(p['min'],p['max']),3)
        elif t=='popup': o[p['id']]=random.randrange(len(p['options']))
        elif t=='checkbox': o[p['id']]=random.choice([0,1])
        else: o[p['id']]=[round(random.uniform(0,2),3) for _ in range(3)]+[1.0]
    W=random.choice([1,2,7,33,96,160,320]); H=random.choice([1,3,17,64,120,240])
    sc=random.choice([0.25,0.5,1,2])
    r=subprocess.run(['node','test/harness.mjs','ec_deep_glow_2.wasm','neon','f.f32',json.dumps(o),str(W),str(H),str(sc)],
                     capture_output=True,text=True,cwd='..')
    if r.returncode!=0:
        fails+=1; print('CRASH',W,H,sc,json.dumps(o)[:300]); print(r.stderr[-600:]); continue
    import struct
    d=open('../f.f32','rb').read()
    vals=struct.unpack('<%df'%(len(d)//4),d)
    bad=[v for v in vals if v!=v or abs(v)==float('inf')]
    neg=[v for v in vals if v<-1e-4]
    if bad or neg:
        fails+=1; print('BAD',W,H,sc,'nan/inf',len(bad),'neg',len(neg),json.dumps(o)[:300])
print(f'{70-fails}/70 ok')
