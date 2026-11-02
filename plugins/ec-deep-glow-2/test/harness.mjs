// node harness.mjs <wasm> <scene> <out.f32> [overrides-json] [W H] [scale]
import fs from 'fs';
const [,, wasmPath, scene, outPath, ovr='{}', Ws='960', Hs='540', scl='1'] = process.argv;
const W=+Ws, H=+Hs, over=JSON.parse(ovr);
const wasm = fs.readFileSync(wasmPath);
const { instance } = await WebAssembly.instantiate(wasm, {});
const ex = instance.exports;
if (ex.ec_api_version()!==1) throw new Error('api');
const mem = () => new Uint8Array(ex.memory.buffer);
const man = JSON.parse(new TextDecoder().decode(mem().slice(ex.ec_manifest_ptr(), ex.ec_manifest_ptr()+ex.ec_manifest_len())));
// params in manifest order, colours take 4 slots
const slots=[], idx={};
for (const p of man.params){ idx[p.id]=slots.length; if(p.type==='color') slots.push(...p.default); else slots.push(p.type==='checkbox'?(p.default?1:0):p.default); }
const oldCount = 36;
for (const [k,v] of Object.entries(over)){ if(!(k in idx)) { if(wasmPath.includes('open_2')||wasmPath.includes('19d71')) continue; throw new Error('unknown '+k);} if(Array.isArray(v)) v.forEach((x,i)=>slots[idx[k]+i]=x); else slots[idx[k]]=v; }
const np = process.env.NP ? +process.env.NP : (wasmPath.match(/open_4/) ? slots.length : oldCount);

function scene_img(name){
  const a=new Float32Array(W*H*4);
  const put=(x,y,r,g,b,al=1)=>{ if(x<0||y<0||x>=W||y>=H) return; const i=(y*W+x)*4; a[i]=r*al; a[i+1]=g*al; a[i+2]=b*al; a[i+3]=al; };
  if(name==='impulse'){ put(W>>1,H>>1,1000,1000,1000); return a; }
  if(name==='hdrcolor'){ for(let i=0;i<W*H;i++){a[i*4]=8;a[i*4+1]=2;a[i*4+2]=0.5;a[i*4+3]=1;} return a; }
  if(name==='gray'){ for(let i=0;i<W*H;i++){a[i*4]=a[i*4+1]=a[i*4+2]=0.5;a[i*4+3]=1;} return a; }
  if(name==='uniform'){ for(let i=0;i<W*H;i++){a[i*4]=a[i*4+1]=a[i*4+2]=1;a[i*4+3]=1;} return a; }
  // "neon" scene : dark bg, HDR point lights, neon bars, saturated blobs, rect at frame edge, soft gradient
  for(let y=0;y<H;y++)for(let x=0;x<W;x++){ const v=0.02+0.03*y/H; put(x,y,v*0.6,v*0.8,v,1); }
  const disc=(cx,cy,rad,r,g,b)=>{ for(let y=-rad-1;y<=rad+1;y++)for(let x=-rad-1;x<=rad+1;x++){ const d=Math.hypot(x,y); const c=Math.min(1,Math.max(0,rad+0.5-d)); if(c>0) put(cx+x,cy+y,r,g,b,1) /*aa below*/; } };
  const aadisc=(cx,cy,rad,r,g,b)=>{ for(let y=-rad-2;y<=rad+2;y++)for(let x=-rad-2;x<=rad+2;x++){ const d=Math.hypot(x,y); const c=Math.min(1,Math.max(0,rad+0.5-d)); if(c>0&&cx+x>=0&&cy+y>=0&&cx+x<W&&cy+y<H){ const i=((cy+y)*W+cx+x)*4; a[i]=a[i]*(1-c)+r*c; a[i+1]=a[i+1]*(1-c)+g*c; a[i+2]=a[i+2]*(1-c)+b*c; } } };
  aadisc(W*0.22|0,H*0.35|0,3,30,30,30);              // tiny HDR white light
  aadisc(W*0.50|0,H*0.50|0,10,12,12,12);             // bigger HDR white
  aadisc(W*0.78|0,H*0.30|0,22,0.1,0.9,1.0);          // saturated cyan (lum<1)
  aadisc(W*0.70|0,H*0.72|0,18,1.0,0.05,0.6);         // magenta
  aadisc(W*0.30|0,H*0.75|0,14,1.0,0.5,0.05);         // orange
  for(let x=W*0.08|0;x<W*0.45;x++) for(let t=0;t<3;t++) { const y=(H*0.58|0)+t; const i=(y*W+x)*4; a[i]=0.2; a[i+1]=1.6; a[i+2]=0.3; }          // neon green bar
  for(let y=H*0.1|0;y<H*0.55;y++) for(let t=0;t<2;t++){ const x=(W*0.90|0)+t; const i=(y*W+x)*4; a[i]=8; a[i+1]=0.4; a[i+2]=0.4; }              // HDR red bar
  for(let y=H-12;y<H;y++) for(let x=W*0.55|0;x<W*0.7;x++){ const i=(y*W+x)*4; a[i]=4; a[i+1]=4; a[i+2]=3.5; }                                      // bright rect touching bottom edge
  return a;
}
const img = scene_img(scene);
const bytes = W*H*16;
ex.ec_alloc(bytes);                      // host contract: ec_alloc(src bytes) -> offset
const ptr = 65536;
const pptr = (()=>{ // params live in low memory below manifest? use end of image region + slack (host decides) : place at 1<<20-aligned after image
  return ptr + bytes + 4096; })();
// ensure memory for image + params ; render grows more itself. params must not be clobbered by scratch: scratch starts after buf, so put params BEFORE buffer? place in first page region 60000..65000
const pOff = 40000;                      // inside page 0, away from data/stack
mem(); new Float32Array(ex.memory.buffer, ptr, W*H*4).set(img);
new Float64Array(ex.memory.buffer, pOff, slots.length).set(slots);
const t0=performance.now();
const rc = ex.ec_render(ptr, W, H, pOff, np, 0.0, +scl);
const dt=performance.now()-t0;
const out = new Float32Array(ex.memory.buffer, ptr, W*H*4);
fs.writeFileSync(outPath, Buffer.from(out.buffer, out.byteOffset, out.byteLength));
console.log(JSON.stringify({wasm:wasmPath.split('/').pop(), scene, rc, ms:Math.round(dt), mem_MB:+(ex.memory.buffer.byteLength/1048576).toFixed(1), np}));
