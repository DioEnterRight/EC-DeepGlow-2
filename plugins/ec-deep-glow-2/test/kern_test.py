import subprocess, json, sys, numpy as np
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
def run(over, W=1025, H=1025, scene='impulse', wasm='../ec_deep_glow_2.wasm'):
    base=dict(threshold=0,softness=0,exposure=0,intensity=100,glow_only=True,dither=0,tonemap=0,gamma=1)
    base.update(over)
    out=f'{S}/k.f32'
    r=subprocess.run(['node','harness.mjs',wasm,scene,out,json.dumps(base),str(W),str(H),'1'],capture_output=True,text=True)
    if r.returncode: print(r.stderr); sys.exit(1)
    return np.fromfile(out,dtype=np.float32).reshape(H,W,4), json.loads(r.stdout.strip().splitlines()[-1])
def target(mode,R,p,a):
    rr=np.exp(np.linspace(np.log(a*1e-3),np.log(14*R),20000))
    def K(r):
        if mode==0: return (1+(r/a)**2)**(-p/2)*np.exp(-(r/R)**2)
        return np.exp(-3*r/R)
    integ=np.trapezoid(2*np.pi*rr*K(rr),rr)+np.pi*rr[0]**2
    return (lambda r: K(r)/integ)
def profile(img,W,H,ch=0):
    cy,cx=H//2,W//2; y,x=np.mgrid[0:H,0:W]; r=np.hypot(x-cx,y-cy)
    g=img[...,ch]/1000.0
    return r,g
def report(name,over,R,mode,p):
    img,info=run(over)
    r,g=profile(img,1025,1025)
    a=max(1.0,1.5,0.004*R)
    T=target(mode,R,p,a)
    # radial bins
    edges=np.unique(np.round(np.geomspace(1,480,25)).astype(int))
    errs=[]
    for lo,hi in zip(edges[:-1],edges[1:]):
        m=(r>=lo)&(r<hi)
        if m.sum()==0: continue
        rm=r[m].mean(); meas=g[m].mean(); tgt=T(rm)
        errs.append((rm,meas,tgt,meas/tgt-1))
    print(f'--- {name}: R={R} p={p} mode={mode} ms={info["ms"]}')
    bad=[(round(e[0],1),f'{e[3]*100:+.1f}%') for e in errs]
    print(' relerr by r:',bad)
    mid=[abs(e[3]) for e in errs if e[2]>1e-3*T(1)]  # exclude vanishing tail
    print(' max |rel err| (r where K>1e-3 of K(1px): ~3 decades): %.1f%%   median %.1f%%'%(100*max(mid),100*np.median(mid)))
    print(' energy sum: %.4f (frame clips tails)'%(g.sum()))
if __name__=='__main__':
    for q in [1,2,3,4]:
        report(f'invsq q={q}',dict(radius=100,falloff=2,glow_mode=0,quality=q),100,0,2)
    report('invsq p=1',dict(radius=100,falloff=1,glow_mode=0,quality=3),100,0,1)
    report('invsq p=3',dict(radius=100,falloff=3,glow_mode=0,quality=3),100,0,3)
    report('exp',dict(radius=100,glow_mode=1,quality=3),100,1,2)
