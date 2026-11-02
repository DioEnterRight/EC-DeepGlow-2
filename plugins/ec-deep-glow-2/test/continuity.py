import subprocess, json, numpy as np
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
W=H=513
def run(over):
    base=dict(threshold=0,softness=0,exposure=0,intensity=100,glow_only=True,dither=0,tonemap=0,gamma=1,quality=3)
    base.update(over); out=f'{S}/c.f32'
    r=subprocess.run(['node','harness.mjs','../ec_deep_glow_2.wasm','impulse',out,json.dumps(base),str(W),str(H),'1'],capture_output=True,text=True); assert r.returncode==0,r.stderr
    return np.fromfile(out,np.float32).reshape(H,W,4)
def sweep(name,key,vals,extra={}):
    P=[]
    for v in vals:
        o=dict(extra); o[key]=v; g=run(o)
        c=H//2; P.append([g[c,c+r,ch]/1000 for r in (2,8,30,90) for ch in (0,2)])
    P=np.log(np.array(P)+1e-30); d1=np.abs(np.diff(P,axis=0)); d2=np.abs(np.diff(P,2,axis=0))
    print(f'{name}: max step {d1.max()*100:.2f}%  max 2nd-diff {d2.max()*100:.3f}%   (values {vals[0]}..{vals[-1]}, {len(vals)} pts)')
sweep('radius 100->110 (+0.5)','radius',list(np.arange(100,110.01,0.5)),dict(radius=100))
sweep('radius 20->21','radius',list(np.arange(20,21.01,0.1)))
sweep('chroma 0->20','chroma',list(np.arange(0,20.1,1.0)),dict(radius=80))
sweep('falloff 1.9->2.1','falloff',list(np.arange(1.9,2.101,0.02)),dict(radius=80))
sweep('aspect 0.98->1.02','aspect',[0.98,0.99,0.995,0.998,1.0,1.002,1.004,1.01,1.02],dict(radius=80))
sweep('radius_r 0.9->1.1','radius_r',list(np.arange(0.9,1.101,0.02)),dict(radius=80))
