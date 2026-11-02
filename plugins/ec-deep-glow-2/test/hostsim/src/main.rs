// Mirrors crates/plugin/src/wasm.rs: wasmi 2, fuel 50M + 20k/px, 1 GiB memory cap, params then pixels at ec_alloc base.
use wasmi::*;
const FUEL_BASE: u64 = 50_000_000;
const FUEL_PER_PIXEL: u64 = 20_000;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let wasm = std::fs::read(&a[1]).unwrap();
    let (w, h): (usize, usize) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let over: serde_json::Map<String, serde_json::Value> = serde_json::from_str(a.get(4).map(|s| s.as_str()).unwrap_or("{}")).unwrap();
    let scene = a.get(5).map(|s| s.as_str()).unwrap_or("neon");
    let mut cfg = Config::default(); cfg.consume_fuel(true);
    let engine = Engine::new(&cfg);
    let module = Module::new(&engine, &wasm[..]).unwrap();
    let mut store = Store::new(&engine, StoreLimitsBuilder::new().memory_size(1 << 30).instances(1).build());
    store.limiter(|l| l);
    store.set_fuel(FUEL_BASE).unwrap();
    let inst = Linker::<StoreLimits>::new(&engine).instantiate_and_start(&mut store, &module).unwrap();
    let mem = inst.get_memory(&store, "memory").unwrap();
    let mp = inst.get_typed_func::<(), i32>(&store, "ec_manifest_ptr").unwrap().call(&mut store, ()).unwrap();
    let ml = inst.get_typed_func::<(), i32>(&store, "ec_manifest_len").unwrap().call(&mut store, ()).unwrap();
    let mut t = vec![0u8; ml as usize]; mem.read(&store, mp as usize, &mut t).unwrap();
    let man: serde_json::Value = serde_json::from_slice(&t).unwrap();
    let mut flat: Vec<f64> = vec![]; let mut idx = std::collections::HashMap::new();
    for p in man["params"].as_array().unwrap() {
        idx.insert(p["id"].as_str().unwrap().to_string(), flat.len());
        match p["type"].as_str().unwrap() {
            "color" => for v in p["default"].as_array().unwrap() { flat.push(v.as_f64().unwrap()) },
            "checkbox" => flat.push(if p["default"].as_bool().unwrap() { 1.0 } else { 0.0 }),
            _ => flat.push(p["default"].as_f64().unwrap()),
        }
    }
    for (k, v) in over { let i = idx[&k]; flat[i] = match v { serde_json::Value::Bool(b) => if b {1.0} else {0.0}, x => x.as_f64().unwrap_or(f64::NAN) }; }
    let n = w * h;
    let mut px = vec![[0f32; 4]; n];
    for y in 0..h { for x in 0..w { let v = 0.02 + 0.03 * y as f32 / h as f32; px[y*w+x] = [v*0.6, v*0.8, v, 1.0]; } }
    if scene == "neon" { for (cx, cy, r, c) in [(0.22,0.35,3,30.0),(0.5,0.5,10,12.0),(0.78,0.3,22,1.5)] { let (cx, cy) = ((cx*w as f64) as i64, (cy*h as f64) as i64);
        for y in -r..=r { for x in -r..=r { if x*x+y*y <= r*r { let (X, Y) = (cx+x, cy+y); if X>=0 && Y>=0 && (X as usize)<w && (Y as usize)<h { px[Y as usize*w+X as usize] = [c, c, c, 1.0]; } } } } } }
    let params_bytes = flat.len() * 8; let total = params_bytes + n * 16;
    let mult: u64 = std::env::var("FUEL_MULT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let fuel = (FUEL_BASE + FUEL_PER_PIXEL * n as u64) * mult;
    store.set_fuel(fuel).unwrap();
    let base = inst.get_typed_func::<i32, i32>(&store, "ec_alloc").unwrap().call(&mut store, total as i32).unwrap() as u32 as usize;
    let mut buf = Vec::with_capacity(total);
    for v in &flat { buf.extend_from_slice(&v.to_le_bytes()); }
    for p in &px { for c in p { buf.extend_from_slice(&c.to_le_bytes()); } }
    if let Err(e) = mem.write(&mut store, base, &buf) { println!("{{\"err\":\"write {e}\"}}"); return; }
    let render = inst.get_typed_func::<(i32,i32,i32,i32,i32,f64,f64), i32>(&store, "ec_render").unwrap();
    let t0 = std::time::Instant::now();
    let r = render.call(&mut store, ((base + params_bytes) as i32, w as i32, h as i32, base as i32, flat.len() as i32, 0.0, 1.0));
    let left = store.get_fuel().unwrap_or(0);
    let used = fuel - left;
    let memmb = mem.data(&store).len() as f64 / 1048576.0;
    let mut feat = String::new();
    store.set_fuel(1_000_000_000).unwrap();
    if let Ok(d) = inst.get_typed_func::<(), i32>(&store, "ec_dbg") { let p = d.call(&mut store, ()).unwrap() as usize; let mut b = [0u8; 80]; mem.read(&store, p, &mut b).unwrap();
        let v: Vec<String> = b.chunks(8).map(|c| format!("{}", f64::from_le_bytes(c.try_into().unwrap()))).collect(); feat = format!(",\"feat\":[{}]", v.join(",")); }
    match r {
        Ok(code) => {
            mem.read(&store, base + params_bytes, &mut buf[params_bytes..]).unwrap();
            let mut bad = 0usize; let mut mx = 0f32;
            for ch in buf[params_bytes..].chunks(4) { let v = f32::from_le_bytes([ch[0],ch[1],ch[2],ch[3]]); if !v.is_finite() || v < 0.0 { bad += 1 } else if v > mx { mx = v } }
            println!("{{\"rc\":{code},\"fuel_used_M\":{:.1},\"budget_M\":{:.1},\"pct\":{:.0},\"ms\":{},\"mem_MB\":{:.0},\"bad\":{bad},\"max\":{mx}{feat}}}", used as f64/1e6, fuel as f64/1e6, 100.0*used as f64/fuel as f64, t0.elapsed().as_millis(), memmb);
        }
        Err(e) => println!("{{\"trap\":\"{}\",\"fuel_used_M\":{:.1},\"budget_M\":{:.1},\"ms\":{},\"mem_MB\":{:.0}}}", e.to_string().replace('"',"'"), used as f64/1e6, fuel as f64/1e6, t0.elapsed().as_millis(), memmb),
    }
}
