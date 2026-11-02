import subprocess, json, random, numpy as np, sys
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
random.seed(7)
def run(over,W,H,scale=1,scene='neon',wasm='../ec_deep_glow_2.wasm'):
    out=f'{S}/f.f32'
    r=subprocess.run(['node','harness.mjs',wasm,scene,out,json.dumps(over),str(W),str(H),str(scale)],capture_output=True,text=True)
    if r.returncode: return None,r.stderr.strip().splitlines()[-1][:200]
    a=np.fromfile(out,dtype=np.float32)
    return a,json.loads(r.stdout.strip().splitlines()[-1])
sizes=[(1,1),(2,3),(7,5),(31,17),(64,64),(129,33),(333,211),(640,360),(5,400),(400,5)]
fails=0; n=0
for (W,H) in sizes:
    for t in range(6):
        o=dict(threshold=random.choice([0,10,35,100,250,400]),softness=random.choice([0,50,100]),
               radius=random.choice([1,2.5,30,300,1000,5000]),exposure=random.choice([-8,0,1.5,6,12]),
               intensity=random.choice([0,100,1000]),glow_mode=random.choice([0,1]),falloff=random.choice([0.25,1,2,4]),
               gamma=random.choice([0.2,1,2.2,5]),radius_r=random.choice([0,1,8]),radius_g=random.choice([1,3]),radius_b=random.choice([0,1,0.1]),
               aspect=random.choice([0.1,0.5,1,1.001,2.5,10]),chroma=random.choice([0,30,100]),tint_amount=random.choice([0,100]),
               dirt_amount=random.choice([0,100]),dirt_seed=random.choice([0,1,9999]),src_opacity=random.choice([0,100]),
               blend=random.choice([0,1]),tonemap=random.choice([0,1,2,3,4]),quality=random.choice([0,1,2,3,4,5]),
               glow_only=random.choice([False,True]),thr_mode=random.choice([0,1]),ca_mode=random.choice([0,1,2]),tm_mix=random.choice([0,50,100]),dither=random.choice([0,100]))
        sc=random.choice([1,0.5,2,0.25])
        a,info=run(o,W,H,sc); n+=1
        if a is None: fails+=1; print('FAIL',W,H,o,info); continue
        if info['rc']!=0 or not np.isfinite(a).all(): fails+=1; print('BAD',W,H,info,'nonfinite' if not np.isfinite(a).all() else '',o)
        elif (a.reshape(-1,4)[:,:3]<-1e-4).any(): fails+=1; print('NEG',W,H,o)
print(f'{n} runs, {fails} failures')
# old 36-param preset on new plugin
a,info=run(dict(),640,360,1,'neon','../ec_deep_glow_2.wasm')
print('defaults ok',info)
