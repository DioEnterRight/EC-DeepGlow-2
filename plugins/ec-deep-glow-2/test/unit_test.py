import subprocess, json, numpy as np, math
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
def run(scene,over,W=400,H=300,wasm='../ec_deep_glow_2.wasm'):
    base=dict(threshold=0,softness=0,exposure=0,intensity=100,glow_only=True,dither=0,tonemap=0,gamma=1,radius=8,quality=3)
    base.update(over); out=f'{S}/u.f32'
    r=subprocess.run(['node','harness.mjs',wasm,scene,out,json.dumps(base),str(W),str(H),'1'],capture_output=True,text=True); assert r.returncode==0,r.stderr
    return np.fromfile(out,np.float32).reshape(H,W,4)
c=np.array([8,2,0.5])
def tm(x,mode):
    if mode==0: return x
    if mode==1: return x/(1+x)
    if mode==2: return min(1,max(0,(x*(2.51*x+.03))/(x*(2.43*x+.59)+.14)))
    if mode==3:
        h=lambda x:(x*(.15*x+.05)+.004)/(x*(.15*x+.5)+.06)-.0666667
        return h(2*x)/h(11.2)
    if mode==4: return min(1,x*(1+x/16)/(1+x))
print('== tonemap (hue-preserving, on glow) : uniform HDR colour (8,2,0.5), interior pixel')
for mode,name in enumerate(['none','reinhard','aces','filmic','reinhard2']):
    for mix in (100,50):
        px=run('hdrcolor',dict(tonemap=mode,tm_mix=mix))[150,200,:3]
        s=tm(c.max(),mode)/c.max(); s=1+(s-1)*mix/100; exp=c*s
        print(f'{name:10s} mix={mix:3d} got {np.round(px,4)} exp {np.round(exp,4)}  hue(g/r,b/r) got {px[1]/px[0]:.4f},{px[2]/px[0]:.4f} want {c[1]/c[0]:.4f},{c[2]/c[0]:.4f}  maxerr {np.abs(px-exp).max():.1e}')
print('== threshold HDR: thr=100% on (8,2,.5) -> mask=(lum-1)/lum scales all channels equally')
lum=.2126*8+.7152*2+.0722*.5; px=run('hdrcolor',dict(threshold=100))[150,200,:3]; print(' got',np.round(px,4),'exp',np.round(c*(lum-1)/lum,4))
print('== gamma 2.2 on mid-grey field: must round-trip to 0.5 (linearise -> blur -> encode)')
print(' got',run('gray',dict(gamma=2.2))[150,200,:3],'(gain 1 energy-conserving interior)')
print('== exposure/intensity: +1 stop doubles, intensity 50% halves:', run('gray',dict(exposure=1))[150,200,0], run('gray',dict(intensity=50))[150,200,0])
print('== HDR-safe screen: src=0 -> glow added; src>1 untouched by screen, add mode adds')
for blend in (0,1):
    o=run('hdrcolor',dict(glow_only=False,threshold=0,blend=blend,radius=8))[150,200,:3]; print(' blend',blend,np.round(o,3))
print('== src opacity 0 + glow only off -> glow only RGB'); print(run('gray',dict(glow_only=False,src_opacity=0))[150,200])
