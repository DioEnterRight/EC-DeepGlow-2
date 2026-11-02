#!/usr/bin/env python3
"""Build the plug-in natively with AddressSanitizer + UndefinedBehaviorSanitizer (every buffer its own heap
block, so an overrun between buffers is caught) and throw random settings at it.   python3 native_fuzz.py [n]"""
import json, os, random, subprocess, sys
here = os.path.dirname(os.path.abspath(__file__)); root = os.path.dirname(here)
exe = os.path.join(os.environ.get('TMPDIR', '/tmp'), 'dg_native_asan')
cc = [os.environ.get('CC', 'gcc'), '-O1', '-g', '-std=gnu11', '-DEC_NATIVE', '-D__builtin_wasm_memory_size(x)=0', '-D__builtin_wasm_memory_grow(x,y)=0',
      '-fsanitize=address,undefined,float-cast-overflow', '-fno-sanitize-recover=all', '-fno-omit-frame-pointer', '-I', root, os.path.join(root, 'deep_glow_open.c'), os.path.join(here, 'native_main.c'), '-lm', '-o', exe]
subprocess.run(cc, check=True)
man = json.load(open(os.path.join(root, 'manifest.json')))
idx, base = {}, []
for p in man['params']:
    idx[p['id']] = len(base)
    base += p['default'] if p['type'] == 'color' else [float(p['default'])]
n = int(sys.argv[1]) if len(sys.argv) > 1 else 200
random.seed(5)
fails = 0
for i in range(n):
    flat = list(base)
    for p in man['params']:
        if random.random() > 0.4: continue
        t, j = p['type'], idx[p['id']]
        if t == 'checkbox': flat[j] = float(random.random() < .5)
        elif t == 'popup': flat[j] = float(random.randrange(len(p['options'])))
        elif t == 'slider': flat[j] = random.choice([p['min'], p['max'], random.uniform(p['min'], p['max']), random.uniform(-1e4, 1e4)])
        else: flat[j:j + 3] = [random.uniform(-1, 5) for _ in range(3)]
    if random.random() < 0.1:
        flat[random.randrange(len(flat))] = random.choice([float('nan'), float('inf'), -float('inf'), 1e300, -1e300])
    if random.random() < 0.1:
        flat = flat[:random.randint(30, len(flat))]          # an older host sending fewer params
    w, h = random.choice([(1, 1), (2, 3), (17, 5), (64, 36), (150, 100), (240, 135), (7, 300)])
    r = subprocess.run([exe, str(w), str(h)] + [repr(v) for v in flat], capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        fails += 1
        print('FAIL', w, h, r.stdout.strip(), r.stderr[:1500])
        if fails > 3: break
print(f'{i + 1} native sanitizer runs, {fails} failures')
sys.exit(1 if fails else 0)
