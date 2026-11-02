#!/usr/bin/env python3
"""Random settings through a copy of EffectCraft's real plug-in host (wasmi, fuel 50 M + 20 000 / px, 1 GiB):
no trap, rc 0, no NaN/Inf/negative pixels, and fuel under 85 % of the budget.

    python3 host_fuzz.py <hostsim binary> <wasm> [n]
"""
import json, random, subprocess, sys
hs, wasm = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 100
random.seed(11)
man = json.load(open(__file__.rsplit('/', 2)[0] + '/manifest.json'))
fails, worst = 0, 0
for i in range(n):
    o = {}
    for p in man['params']:
        if random.random() > 0.35:
            continue
        t = p['type']
        if t == 'checkbox': o[p['id']] = random.random() < 0.5
        elif t == 'popup': o[p['id']] = random.randrange(len(p['options']))
        elif t == 'slider': o[p['id']] = random.choice([p['min'], p['max'], random.uniform(p['min'], p['max'])])
        elif t == 'color': o[p['id']] = [random.uniform(0, 4) for _ in range(3)] + [1]
    w, h = random.choice([(1, 1), (3, 200), (96, 54), (320, 180), (640, 360), (500, 700)])
    p = subprocess.run([hs, wasm, str(w), str(h), json.dumps(o)], capture_output=True, text=True, timeout=900)
    try:
        j = json.loads(p.stdout)
    except ValueError:
        print('HOST ERROR', w, h, o, p.stderr[-200:]); fails += 1; continue
    ok = 'trap' not in j and j.get('rc') == 0 and j.get('bad', 1) == 0 and j.get('pct', 100) <= 85
    worst = max(worst, j.get('pct', 100))
    if not ok:
        fails += 1; print('FAIL', w, h, json.dumps(o), p.stdout.strip()[:160])
print(f'{n} runs, {fails} failures, worst fuel {worst}% of budget')
sys.exit(1 if fails else 0)
