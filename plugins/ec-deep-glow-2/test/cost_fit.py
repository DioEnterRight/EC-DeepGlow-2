#!/usr/bin/env python3
"""Fit the cost-model coefficients (CK_*) in deep_glow_open.c to real wasmi fuel.

    python3 cost_fit.py <hostsim binary> <debug wasm built with -DEC_COST_DEBUG> [n]

hostsim mirrors EffectCraft's plug-in host (wasmi, fuel 50 M + 20 000 / pixel) and prints the
fuel used plus the plan features the debug build exports.  Prints the CK_* defines and the worst
under-estimate.
"""
import json, random, subprocess, sys
import numpy as np
from scipy.optimize import nnls

hs, wasm = sys.argv[1], sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 120
random.seed(7)
sizes = [(96, 54), (160, 90), (320, 180), (480, 270), (640, 360), (200, 300)]
rows = []
for i in range(n):
    o = {"quality": random.choice([0, 1, 2, 3, 4, 5, 6]), "radius": random.choice([5, 30, 120, 300, 900, 3000]),
         "glow_mode": random.choice([0, 0, 1, 2, 2])}
    if random.random() < 0.5:
        o.update(aspect=random.choice([0.2, 0.5, 2, 4, 10]))
        if random.random() < 0.6:
            o.update(angle_enable=True, angle=random.uniform(-90, 90))
    if o["quality"] == 6:
        o.update(q_downsample=random.randint(0, 3), q_steps=random.randint(1, 8), q_iterations=random.randint(3, 15))
    if o["glow_mode"] == 2:
        o.update(iris_quality=random.randint(0, 2), iris_iterations=random.choice([4, 16, 32, 64]), iris_aspect=random.choice([1, 1, 3, 0.4]))
    if random.random() < 0.3:
        o.update(chroma=random.choice([30, 100]))
    w, h = random.choice(sizes)
    p = subprocess.run([hs, wasm, str(w), str(h), json.dumps(o)], capture_output=True, text=True, timeout=600)
    try:
        j = json.loads(p.stdout)
    except ValueError:
        print("host error", o, w, h, p.stderr[-300:]); continue
    if "feat" not in j:
        continue
    rows.append((j["feat"][:9], j["fuel_used_M"] * 1e6, o, w, h))
F = np.array([[1.0] + r[0] for r in rows])
y = np.array([r[1] for r in rows])
# relative-error fit: divide by y
c, _ = nnls(F / y[:, None], np.ones(len(y)))
pred = F @ c
rel = pred / y - 1
names = ["CK_CONST", "CK_PX", "CK_ROT", "CK_DOWN", "CK_TAP", "CK_CONV", "CK_REC", "CK_CELL", "CK_FIT", "CK_KB"]
for nm, v in zip(names, c):
    print(f"#define {nm:<8}{v:.6g}")
print(f"n={len(y)}  rel err: median {np.median(np.abs(rel)):.3f}  worst under-estimate {rel.min():.3f}  worst over {rel.max():.3f}")
worst = np.argsort(rel)[:3]
for k in worst:
    print("  under:", round(rel[k], 3), rows[k][3], rows[k][4], rows[k][2])
