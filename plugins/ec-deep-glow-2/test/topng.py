import sys, numpy as np
from PIL import Image
def load(p,W,H): return np.fromfile(p,dtype=np.float32).reshape(H,W,4)
def enc(a):
    a=np.clip(a,0,1); return np.where(a<=0.0031308,12.92*a,1.055*np.power(a,1/2.4)-0.055)
def png(a,out):
    rgb=enc(a[...,:3]); Image.fromarray((rgb*255+0.5).astype(np.uint8)).save(out)
if __name__=='__main__':
    W,H=int(sys.argv[1]),int(sys.argv[2]); outp=sys.argv[3]; files=sys.argv[4:]
    ims=[enc(load(f,W,H)[...,:3]) for f in files]
    Image.fromarray((np.concatenate(ims,axis=0)*255+0.5).astype(np.uint8)).save(outp)
