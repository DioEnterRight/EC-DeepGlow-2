import subprocess, json, numpy as np
from PIL import Image
S='/tmp/claude-0/-home-claude/931ef3ac-edb6-5905-96bf-6188087d3f93/scratchpad'
W,H=960,540
def enc(a):
    a=np.clip(a,0,1); return np.where(a<=0.0031308,12.92*a,1.055*np.power(a,1/2.4)-0.055)
def run(name,over):
    out=f'{S}/v_{name}.f32'
    r=subprocess.run(['node','harness.mjs','../ec_deep_glow_2.wasm','neon',out,json.dumps(over),str(W),str(H),'1'],capture_output=True,text=True); assert r.returncode==0,r.stderr
    return np.fromfile(out,np.float32).reshape(H,W,4)
V=[('default',{}),('aspect3',dict(aspect=3,radius=200)),('chroma_radius',dict(chroma=60,exposure=2.5)),('chroma_both',dict(chroma=60,ca_mode=2,exposure=2.5)),
   ('exp',dict(glow_mode=1,radius=150)),('aces_tint',dict(tonemap=2,tint_amount=70,exposure=3))]
tiles=[enc(run(n,o)[...,:3]) for n,o in V]
rows=[np.concatenate(tiles[i:i+2],axis=1) for i in range(0,6,2)]
Image.fromarray((np.concatenate(rows,axis=0)*255+.5).astype(np.uint8)).resize((1440,1215),Image.LANCZOS).save(f'{S}/variants.png')
