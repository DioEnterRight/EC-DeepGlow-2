// node plugin_render.mjs <wasm> <in.f32rgba> <W> <H> <out.f32rgba> '<overrides-json>'
// Runs the open wasm plugin on a raw float32 RGBA frame. Used by analyze.py for the self-test and comparator.
import fs from 'fs';
const [,, wasmPath, inPath, Ws, Hs, outPath, ovr = '{}'] = process.argv;
const W = +Ws, H = +Hs, over = JSON.parse(ovr);
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const ex = instance.exports;
const man = JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer).slice(ex.ec_manifest_ptr(), ex.ec_manifest_ptr() + ex.ec_manifest_len())));
const slots = [], idx = {};
for (const p of man.params) { idx[p.id] = slots.length; if (p.type === 'color') slots.push(...p.default); else slots.push(p.type === 'checkbox' ? (p.default ? 1 : 0) : p.default); }
for (const [k, v] of Object.entries(over)) {
  if (!(k in idx)) throw new Error('unknown param ' + k);
  if (Array.isArray(v)) v.forEach((x, i) => slots[idx[k] + i] = x); else slots[idx[k]] = (v === true ? 1 : v === false ? 0 : v);
}
const raw = fs.readFileSync(inPath);
const img = new Float32Array(raw.buffer, raw.byteOffset, W * H * 4);
const bytes = W * H * 16;
const ptr = ex.ec_alloc(bytes + slots.length * 8 + 64);
new Float32Array(ex.memory.buffer, ptr, W * H * 4).set(img);
const pp = (ptr + bytes + 15) & ~15;
new Float64Array(ex.memory.buffer, pp, slots.length).set(slots);
const rc = ex.ec_render(ptr, W, H, pp, slots.length, 0, 1);
if (rc !== 0) throw new Error('ec_render rc ' + rc);
const out = new Float32Array(ex.memory.buffer, ptr, W * H * 4);
fs.writeFileSync(outPath, Buffer.from(out.buffer, out.byteOffset, out.byteLength));
