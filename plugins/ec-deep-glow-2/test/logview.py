import subprocess, json, numpy as np
from PIL import Image
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
OLD='/mnt/user-data/attachments/931ef3ac-edb6-5905-96bf-6188087d3f93/19d71ec2-deep_glow_open_2.wasm'
W=H=801
def run(wasm,over,tag):
    base=dict(threshold=0,softness=0,exposure=0,intensity=100,glow_only=True,tonemap=0,gamma=1,radius=120,quality=3)
    if 'open_4' in wasm: base['dither']=0
    base.update(over); out=f'{S}/lv_{tag}.f32'
    r=subprocess.run(['node','harness.mjs',wasm,'impulse',out,json.dumps(base),str(W),str(H),'1'],capture_output=True,text=True); assert r.returncode==0,r.stderr
    return np.fromfile(out,np.float32).reshape(H,W,4)[...,0]/1000.0
def logmap(g):
    v=(np.log10(np.maximum(g,1e-9))+8)/8      # 1e-8..1 -> 0..1
    return (np.clip(v,0,1)*255).astype(np.uint8)
tiles=[]
for name,w,o in [('old',OLD,{}),('new',"../ec_deep_glow_2.wasm",{}),('new_aspect2.5',"../ec_deep_glow_2.wasm",dict(aspect=2.5)),('new_exp',"../ec_deep_glow_2.wasm",dict(glow_mode=1))]:
    g=run(w,o,name); tiles.append(logmap(g)); print(name,'sum=%.4f'%g.sum(),'peak=%.3g'%g.max())
Image.fromarray(np.concatenate([np.concatenate(tiles[:2],1),np.concatenate(tiles[2:],1)],0)).save(f'{S}/logview.png')
# also radial/axis symmetry metric: ratio of row vs column profiles for new (isotropy) and 45deg
g=run("../ec_deep_glow_2.wasm",{},'iso'); c=H//2
print('isotropy: along-x vs along-y vs diag @ r=10,40,120:')
for r in (10,40,120): print(r, '%.4g %.4g %.4g'%(g[c,c+r],g[c+r,c],g[c+int(r/2**.5),c+int(r/2**.5)]))
