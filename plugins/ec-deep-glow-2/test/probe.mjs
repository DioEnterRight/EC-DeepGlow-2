// node probe.mjs <wasm> — numeric probes of the v4 engine
import fs from 'fs';
const wasmPath = process.argv[2] || 'ec_deep_glow_2.wasm';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const ex = instance.exports;
const mem = () => new Uint8Array(ex.memory.buffer);
const man = JSON.parse(new TextDecoder().decode(mem().slice(ex.ec_manifest_ptr(), ex.ec_manifest_ptr()+ex.ec_manifest_len())));
const idx = {}; const base = [];
for (const p of man.params){ idx[p.id]=base.length; if(p.type==='color') base.push(...p.default); else base.push(p.type==='checkbox'?(p.default?1:0):p.default); }
export function run(over, W, H, make, scale=1){
  const slots = base.slice();
  for (const [k,v] of Object.entries(over)){ if(!(k in idx)) throw new Error('unknown param '+k);
    if(Array.isArray(v)) v.forEach((x,i)=>slots[idx[k]+i]=x); else slots[idx[k]]=v; }
  const bytes = W*H*16;
  const ptr = ex.ec_alloc(bytes + slots.length*8 + 64);
  const img = make(W,H);
  new Float32Array(ex.memory.buffer, ptr, W*H*4).set(img);
  const pp = (ptr + bytes + 15) & ~15;
  new Float64Array(ex.memory.buffer, pp, slots.length).set(slots);
  const rc = ex.ec_render(ptr, W, H, pp, slots.length, 0, scale);
  if (rc !== 0) throw new Error('rc '+rc);
  const out = new Float32Array(ex.memory.buffer, ptr, W*H*4).slice();
  return { out, W, H, at:(x,y)=>[0,1,2,3].map(c=>out[((y*W+x)*4)+c]),
           sum:(c)=>{let s=0; for(let i=0;i<W*H;i++) s+=out[i*4+c]; return s;} };
}
export const dot = (v=1)=> (W,H)=>{ const a=new Float32Array(W*H*4); const i=(((H>>1)*W)+(W>>1))*4; a[i]=a[i+1]=a[i+2]=v; a[i+3]=1; return a; };
export const empty = ()=> (W,H)=> new Float32Array(W*H*4);
