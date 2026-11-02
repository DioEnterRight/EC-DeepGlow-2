import { run, dot } from './probe.mjs';
let fail = 0, n = 0;
const ok = (c, msg) => { n++; if (!c) { fail++; console.log('FAIL ' + msg); } };
const near = (a,b,t,msg)=>ok(Math.abs(a-b)<=t, `${msg} (${a} vs ${b})`);
const G = {quality:3, threshold:0, softness:0, gamma:1, dither:0, blend:1, exposure:0, intensity:100, src_opacity:0, unmult:1};

// 1. energy: an impulse keeps its light (Add, source removed)
for (const R of [8, 30, 90]) {
  const r = run({...G, radius:R}, 192, 192, dot(1));
  near(r.sum(1), 1.0, 0.06, `energy R=${R}`);
}
// 2. falls off monotonically and is round
{
  const r = run({...G, radius:40}, 201, 201, dot(1));
  const c = 100, p = d => r.at(c+d, c)[1];
  ok(p(4) > p(12) && p(12) > p(30) && p(30) > 0, 'radial falloff is monotone');
  for (const d of [5, 15, 35]) near(r.at(c+d,c)[1], r.at(c,c+d)[1], 0.08*r.at(c+d,c)[1]+1e-9, `round at ${d}`);
  near(r.at(c-9,c)[1], r.at(c+9,c)[1], 0.02*r.at(c+9,c)[1], 'symmetric');
}
// 3. aspect: wider along x, narrower along y, energy kept, profile still decays
{
  const iso = run({...G, radius:30}, 241, 241, dot(1));
  const wide = run({...G, radius:30, aspect:4}, 241, 241, dot(1));
  near(wide.sum(1), 1.0, 0.07, 'aspect energy');
  const c = 120;
  ok(wide.at(c+20,c)[1] > iso.at(c+20,c)[1], 'aspect 4 is wider along x');
  ok(wide.at(c,c+20)[1] < iso.at(c,c+20)[1], 'aspect 4 is narrower along y');
  const p = d => wide.at(c+d, c)[1];
  ok(p(6) > p(18) && p(18) > p(40) && p(40) > p(70), `aspect profile decays: ${[6,18,40,70].map(p).join(' ')}`);
}
// 4. angle: the long axis follows the angle
{
  const c = 120;
  for (const ang of [0, 30, 45, 60, 90]) {
    const r = run({...G, radius:30, aspect:4, angle_enable:1, angle:ang}, 241, 241, dot(1));
    const rad = ang * Math.PI/180;
    const along = d => r.at(Math.round(c + d*Math.cos(rad)), Math.round(c + d*Math.sin(rad)))[1];
    const across = d => r.at(Math.round(c - d*Math.sin(rad)), Math.round(c + d*Math.cos(rad)))[1];
    ok(along(14) > 0 && along(14) > along(34) && along(34) > 0, `angle ${ang}: decays along the axis`);
    ok(across(14) < 0.35*along(14), `angle ${ang}: narrow across (${across(14)} vs ${along(14)})`);
    near(r.sum(1), 1.0, 0.04, `angle ${ang} energy`);
  }
}
// 5. threshold / saturation bias
{
  const grey = (W,H)=>{const a=new Float32Array(W*H*4); const i=(((H>>1)*W)+(W>>1))*4; a[i]=a[i+1]=a[i+2]=1; a[i+3]=1; return a;};
  const sat  = (W,H)=>{const a=new Float32Array(W*H*4); const i=(((H>>1)*W)+(W>>1))*4; a[i]=1; a[i+3]=1; return a;};
  const g0 = run({...G, radius:20, sat_bias:0}, 96, 96, grey).sum(1);
  const g1 = run({...G, radius:20, sat_bias:100}, 96, 96, grey).sum(1);
  const s1 = run({...G, radius:20, sat_bias:100}, 96, 96, sat).sum(0);
  ok(g1 < 0.05*g0, `saturation bias kills grey light (${g1} of ${g0})`);
  ok(s1 > 0.9, `saturation bias keeps saturated light (${s1})`);
  const t = run({...G, radius:20, threshold:200}, 96, 96, dot(1)).sum(1);
  ok(t < 1e-6, `threshold 200 % cuts a luma-1 dot (${t})`);
}
// 6. views and unmult
{
  const v1 = run({...G, radius:20, view:1, src_opacity:100}, 96, 96, dot(1));
  ok(v1.at(48,30)[3] > 0 && v1.at(48,30)[3] <= 1, 'glow only has glow alpha');
  const v2 = run({...G, radius:20, view:2}, 96, 96, dot(1));
  near(v2.at(48,48)[1], 1.0, 1e-4, 'threshold matte shows the source');
  near(v2.at(48,40)[1], 0.0, 1e-6, 'threshold matte is not blurred');
  const un = run({...G, radius:20, view:1, unmult:0}, 96, 96, dot(1));
  near(un.at(48,30)[3], 1.0, 1e-6, 'unmult off is opaque');
}
// 7. exposure is linear, source opacity passes through
{
  const a = run({...G, radius:20, exposure:0}, 96, 96, dot(1)).sum(1);
  const b = run({...G, radius:20, exposure:1}, 96, 96, dot(1)).sum(1);
  near(b/a, 2.0, 0.01, 'exposure +1 stop doubles');
  const s = run({...G, radius:20, src_opacity:100, intensity:0}, 96, 96, dot(1));
  near(s.at(48,48)[1], 1.0, 1e-4, 'source passes through at intensity 0');
}
// 8. glow aberration splits the channel radii
{
  const r = run({...G, radius:30, radius_r:2, radius_b:0.5}, 161, 161, dot(1));
  const c = 80;
  ok(r.at(c+34,c)[0] > r.at(c+34,c)[2], 'red reaches further than blue');
  near(r.sum(0), 1.0, 0.07, 'red energy'); near(r.sum(2), 1.0, 0.07, 'blue energy');
}
// 9. lens iris makes a bladed shape, still unit energy
{
  const r = run({...G, radius:28, glow_mode:2, iris_blades:6, iris_iterations:12}, 161, 161, dot(1));
  near(r.sum(1), 1.0, 0.12, 'iris energy');
  const c = 80, ring = a => r.at(Math.round(c+20*Math.cos(a)), Math.round(c+20*Math.sin(a)))[1];
  let mn = 1e9, mx = -1e9;
  for (let k = 0; k < 60; k++) { const v = ring(k*Math.PI/30); mn = Math.min(mn,v); mx = Math.max(mx,v); }
  ok(mx > mn * 1.15, `iris is not a plain circle (min ${mn} max ${mx})`);
  const circ = run({...G, radius:28, glow_mode:2, iris_blades:0, iris_iterations:12}, 161, 161, dot(1));
  let cn = 1e9, cx = -1e9;
  for (let k = 0; k < 60; k++) { const a=k*Math.PI/30; const v = circ.at(Math.round(c+20*Math.cos(a)), Math.round(c+20*Math.sin(a)))[1]; cn=Math.min(cn,v); cx=Math.max(cx,v); }
  ok(cx < cn * 1.3, `iris blades 0 is round (min ${cn} max ${cx})`);
}
// 10. custom quality, half float, tint, tone map, blend modes all change something and stay finite
{
  const ref = run({...G, radius:30}, 128, 128, dot(1));
  const variants = {
    'custom quality': {quality:6, q_downsample:1, q_steps:2, q_iterations:9},
    'half float': {half_float:1},
    'tint inner/outer': {tint_style:1, tint_strength:100, tint_blend:0},
    'tone map': {tonemap:3, intensity:800},
    'pixel aberration': {chroma:60, ca_mode:1, pixel_channels:1},
    'lens dirt': {dirt_amount:80, intensity:400},
  };
  for (const [name, v] of Object.entries(variants)) {
    const r = run({...G, radius:30, ...v}, 128, 128, dot(1));
    ok(r.out.every(Number.isFinite), `${name} stays finite`);
    let diff = 0; for (let i = 0; i < r.out.length; i++) diff = Math.max(diff, Math.abs(r.out[i] - ref.out[i]));
    ok(diff > 1e-7, `${name} changes the result (max diff ${diff})`);
  }
  for (let b = 0; b < 8; b++) {
    const r = run({...G, radius:30, blend:b, src_opacity:100}, 96, 96, dot(1));
    ok(r.out.every(Number.isFinite), `blend ${b} stays finite`);
  }
}
// 11. degenerate inputs
{
  for (const [W,H] of [[1,1],[1,64],[64,1],[3,5]])
    ok(run({...G, radius:400}, W, H, dot(1)).out.every(Number.isFinite), `${W}x${H} stays finite`);
  ok(run({...G, radius:5000, aspect:10, angle_enable:1, angle:33}, 64, 64, dot(1)).out.every(Number.isFinite), 'extreme params stay finite');
  ok(run({...G, radius:30}, 64, 64, (W,H)=>new Float32Array(W*H*4)).out.every(v=>v===0), 'an empty frame stays empty');
}

// 12. v4.1: dither monochromatic, Overlay / Soft Light tints, Iris Opacity / Aspect / Continuous Rasterization
{
  const base = {...G, radius:30, intensity:300};
  const d1 = run({...base, dither:100, dither_mono:1}, 128, 128, dot(1));
  let monoOk = true, anyDiff = false;
  for (let i = 0; i < 128*128; i++) { const r=d1.out[i*4], g=d1.out[i*4+1], b=d1.out[i*4+2]; if (g > 0 && (Math.abs(r-g) > 1e-6 || Math.abs(g-b) > 1e-6)) monoOk = false; }
  ok(monoOk, 'mono dither adds the same noise to every channel');
  const d0 = run({...base, dither:100, dither_mono:0}, 128, 128, dot(1));
  for (let i = 0; i < 128*128; i++) { const r=d0.out[i*4], g=d0.out[i*4+1]; if (g > 0 && Math.abs(r-g) > 1e-6) { anyDiff = true; break; } }
  ok(anyDiff, 'per-channel dither differs between channels');
  const off = run({...base, dither:0}, 128, 128, dot(1));
  let worst = 0; for (let i = 0; i < 128*128*4; i++) worst = Math.max(worst, Math.abs(d1.out[i]-off.out[i]));
  ok(worst > 0 && worst < 0.2, `dither is small noise (${worst})`);

  for (const blend of [4, 5]) {
    const t = run({...base, tint_style:1, tint_blend:blend, tint_strength:100, tint_inner:[0.5,0.5,0.5,1], tint_outer:[0.5,0.5,0.5,1]}, 128, 128, dot(1));
    const p = run({...base, tint_style:1, tint_strength:0}, 128, 128, dot(1));
    let w = 0; for (let i = 0; i < 128*128*4; i++) w = Math.max(w, Math.abs(t.out[i]-p.out[i]));
    ok(w < 1e-4, `tint blend ${blend}: a mid-grey tint changes nothing (${w})`);
    const red = run({...base, tint_style:1, tint_blend:blend, tint_strength:100, tint_inner:[1,0,0,1], tint_outer:[1,0,0,1]}, 128, 128, dot(1));
    ok(red.out.every(Number.isFinite) && red.at(64,80)[0] > red.at(64,80)[2], `tint blend ${blend}: a red tint reddens the glow`);
  }

  const sq = {...G, glow_mode:2, radius:60, iris_blades:6, iris_iterations:20, intensity:2000};
  const full = run(sq, 241, 241, dot(1)), round = run({...sq, iris_blades:0}, 241, 241, dot(1));
  const none = run({...sq, iris_opacity:0}, 241, 241, dot(1)), half = run({...sq, iris_opacity:50}, 241, 241, dot(1));
  let dz = 0; for (let i = 0; i < 241*241*4; i++) dz = Math.max(dz, Math.abs(none.out[i]-round.out[i]));
  ok(dz < 1e-6, `iris opacity 0 is the round glow (${dz})`);
  let df = 0; for (let i = 0; i < 241*241*4; i++) df = Math.max(df, Math.abs(full.out[i]-round.out[i]));
  ok(df > 1e-6, 'a six-blade iris differs from the round one');
  let dh = 0, dn = 0; for (let i = 0; i < 241*241*4; i++) { dh += Math.abs(half.out[i]-round.out[i]); dn += Math.abs(full.out[i]-round.out[i]); }
  ok(dh > 0 && dh < dn, 'iris opacity 50 lies between round and full');
  const wide = run({...sq, iris_aspect:3}, 241, 241, dot(1));
  ok(wide.at(120+40,120)[1] > 2*wide.at(120,120+40)[1], 'iris aspect 3 is wider than tall');
  near(wide.sum(1), 20.0, 2.5, 'iris aspect energy (gain 20)');
  const stepped = run({...sq, iris_continuous:0}, 241, 241, dot(1));
  let ds = 0; for (let i = 0; i < 241*241*4; i++) ds = Math.max(ds, Math.abs(stepped.out[i]-full.out[i]));
  ok(ds > 1e-7, 'stepped rasterization differs');
  near(stepped.sum(1), 20.0, 2.5, 'stepped iris energy (gain 20)');
  ok(stepped.out.every(Number.isFinite), 'stepped iris is finite');
}
// 13. debug-pass regressions (v4.1.1)
{
  // every quality keeps the impulse's light; Draft used to start its sigma ladder at 7.4 px instead of 3.6 (+5 % light)
  for (const q of [0, 1, 2, 3]) near(run({...G, radius:40, quality:q}, 256, 256, dot(1)).sum(1), 1.0, 0.03, `energy quality ${q}`);
  // absurd parameter values (presets, expressions) never give NaN, Inf or negative pixels
  for (const [k, v] of [['radius', 1e300], ['intensity', -1e30], ['intensity', 1e300], ['exposure', 1e30], ['threshold', -1e300],
                        ['angle', 1e30], ['iris_rotation', -1e30], ['dirt_scale', 1e300], ['radius_r', 1e300], ['tint_inner', [1e300, -1, NaN, 1]]]) {
    const extra = k === 'angle' ? {aspect:3, angle_enable:1} : (k.startsWith('iris') ? {glow_mode:2} : (k === 'dirt_scale' ? {dirt_amount:100} : (k.startsWith('tint') ? {tint_style:1} : {})));
    const r = run({...G, ...extra, [k]: v}, 64, 48, dot(5));
    ok(r.out.every(x => Number.isFinite(x) && x >= 0), `${k} = ${v}: finite, non-negative output`);
  }
}
console.log(fail ? `\n${fail} of ${n} checks FAILED` : `all ${n} checks passed`);
process.exit(fail ? 1 : 0);
