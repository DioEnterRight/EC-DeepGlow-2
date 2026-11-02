#!/usr/bin/env python3
"""Measure a glow effect from the outside: radial profile, energy, exposure law, threshold curve,
anisotropy, grid phase, and a non-negative fit of the point-spread function to a Gaussian ladder.

Black-box only. It reads rendered frames (32-bit float EXR or TIFF) and never looks inside any plug-in.

    python3 analyze.py report   --renders renders [--json measured.json]
    python3 analyze.py compare  --renders renders --wasm ec_deep_glow_2.wasm
    python3 analyze.py selftest [--wasm ec_deep_glow_2.wasm]

`report` measures real renders (e.g. Deep Glow 2 from After Effects, made with `make_frames.py` + RUNS.md).
`compare` renders the same runs through the open plug-in and prints measured / plug-in side by side.
`selftest` runs the analyser on the open plug-in's own output and on a synthetic kernel, so you can trust the
numbers before you spend an afternoon in After Effects.
"""
import argparse, glob, json, os, subprocess, sys, tempfile
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import make_frames as mf  # noqa: E402

W, H, CX, CY = mf.W, mf.H, mf.CX, mf.CY


# ----------------------------------------------------------------------------- io
def load(path):
    """-> float32 (H, W, 3) from EXR or TIFF."""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".exr":
        import OpenEXR
        with OpenEXR.File(path) as f:
            ch = f.channels()
            for k in ("RGBA", "RGB"):
                if k in ch:
                    return np.asarray(ch[k].pixels, np.float32)[..., :3].copy()
            names = [n for n in ("R", "G", "B") if n in ch]
            if len(names) == 3:
                return np.stack([np.asarray(ch[n].pixels, np.float32) for n in names], -1)
            raise ValueError(f"{path}: no RGB channels in {list(ch)}")
    import tifffile
    a = tifffile.imread(path).astype(np.float32)
    if a.ndim == 2:
        a = np.repeat(a[..., None], 3, -1)
    return a[..., :3].copy()


def find_render(renders, run_id, suffix=""):
    for pat in (f"{run_id}{suffix}.exr", f"{run_id}{suffix}.tif", f"{run_id}{suffix}.tiff",
                f"{run_id}{suffix}_[0-9]*.exr", f"{run_id}{suffix}_[0-9]*.tif"):
        g = sorted(glob.glob(os.path.join(renders, pat)))
        if g:
            return g[0]
    return None


# ----------------------------------------------------------------------------- measurements
def lum(a):
    return a.mean(axis=2)


def radial_profile(a, cx=CX, cy=CY, nlog=90):
    """Azimuthal mean of the channel mean about pixel (cx, cy). -> r, mean, count, cropped
    r is the mean distance of the pixels in the bin. Only full circles inside the frame are used."""
    v = lum(a)
    hh, ww = v.shape
    rmax = min(cx, ww - 1 - cx, cy, hh - 1 - cy)
    y, x = np.mgrid[:hh, :ww]
    d = np.hypot(x - cx, y - cy)
    edges = np.concatenate([np.arange(-0.5, 8.5, 1.0), np.geomspace(8.5, max(rmax, 9), nlog)])
    sel = d <= rmax
    idx = np.digitize(d[sel], edges) - 1
    nb = len(edges) - 1
    cnt = np.bincount(idx, minlength=nb)[:nb].astype(np.float64)
    sv = np.bincount(idx, weights=v[sel], minlength=nb)[:nb]
    sr = np.bincount(idx, weights=d[sel], minlength=nb)[:nb]
    ok = cnt > 0
    return sr[ok] / cnt[ok], sv[ok] / cnt[ok], cnt[ok], rmax


def centroid(a, x0=0, y0=0):
    v = lum(a).astype(np.float64)
    s = v.sum()
    y, x = np.mgrid[:v.shape[0], :v.shape[1]]
    return (x * v).sum() / s, (y * v).sum() / s


def second_moments(a, cx, cy, rmax):
    """Second moments of the light inside a circle: -> (Ixx, Iyy, Ixy)."""
    v = lum(a).astype(np.float64)
    y, x = np.mgrid[:v.shape[0], :v.shape[1]]
    m = np.hypot(x - cx, y - cy) <= rmax
    w = np.where(m, v, 0)
    s = w.sum()
    return (w * (x - cx) ** 2).sum() / s, (w * (y - cy) ** 2).sum() / s, (w * (x - cx) * (y - cy)).sum() / s


def anisotropy(a, cx=CX, cy=CY):
    _, _, _, rmax = radial_profile(a, cx, cy)
    ixx, iyy, ixy = second_moments(a, cx, cy, rmax)
    tr, det = ixx + iyy, ixx * iyy - ixy * ixy
    disc = np.sqrt(max(tr * tr / 4 - det, 0))
    l1, l2 = tr / 2 + disc, max(tr / 2 - disc, 1e-30)
    ang = 0.5 * np.degrees(np.arctan2(2 * ixy, ixx - iyy))
    return dict(aspect=float(np.sqrt(l1 / l2)), major_angle_deg=float(ang), sigma_x=float(np.sqrt(ixx)), sigma_y=float(np.sqrt(iyy)))


def interp_cross(r, p, level):
    """First r where p falls below `level` (log-linear interpolation), else None."""
    below = np.where(p < level)[0]
    if len(below) == 0:
        return None
    i = below[0]
    if i == 0:
        return float(r[0])
    p0, p1 = max(p[i - 1], 1e-300), max(p[i], 1e-300)
    t = (np.log(level) - np.log(p0)) / (np.log(p1) - np.log(p0))
    return float(r[i - 1] + t * (r[i] - r[i - 1]))


def slope(r, p, r0, r1):
    s = (r >= r0) & (r <= r1) & (p > 0)
    if s.sum() < 4:
        return None
    return float(np.polyfit(np.log(r[s]), np.log(p[s]), 1)[0])


SLOPE_AT = (0.1, 0.25, 0.5, 1.0, 1.5)       # in units of Radius


def local_slopes(r, p, radius):
    """Local log-log slope d ln p / d ln r at r = f * Radius (window +-30%). None where the frame is too small."""
    return {f"{f}R": slope(r, p, f * radius / 1.3, f * radius * 1.3) for f in SLOPE_AT}


def psf_metrics(a, v_in, radius, cx=CX, cy=CY):
    """Everything we can say about a point-spread function from an impulse render."""
    r, p, cnt, rmax = radial_profile(a, cx, cy)
    glow = lum(a).astype(np.float64)
    total = float(glow.sum())
    pk = float(p.max())
    core = interp_cross(r[1:], p[1:], 0.5 * p[1]) if len(p) > 2 else None   # half of the 1-px ring
    cropped = bool(radius * 1.0 > rmax)
    return dict(
        total_out_per_in=total / v_in,
        peak=pk,
        p_at_1px=float(p[1]) if len(p) > 1 else None,
        r_half=core,
        r_tenth=interp_cross(r[1:], p[1:], 0.1 * p[1]),
        r_1e3=interp_cross(r[1:], p[1:], 1e-3 * p[1]),
        r_1e6=interp_cross(r[1:], p[1:], 1e-6 * p[1]),
        slopes=local_slopes(r, p, radius),
        frame_radius_px=float(rmax),
        radius_exceeds_frame=cropped,
        negative_pixels=int((glow < -1e-6).sum()),
        nan_pixels=int(np.isnan(a).sum()),
        profile=dict(r=r.tolist(), p=p.tolist()),
    )


# ----------------------------------------------------------------------------- Gaussian-ladder fit
def ladder(rmax, s0=0.5, ratio=2 ** 0.5):
    out, s = [], s0
    while s < rmax * 1.2:
        out.append(s)
        s *= ratio
    return np.array(out)


def fit_ladder(r, p, v_in=1.0, sigmas=None, floor=1e-6):
    """Non-negative fit of the azimuthal profile to unit-energy 2-D Gaussians, in relative error.
    -> dict(sigmas, weights (energy fractions of the input), rel_rms, max_rel)"""
    from scipy.optimize import nnls
    if sigmas is None:
        sigmas = ladder(r.max())
    keep = (p > floor * p.max()) & (r > 0.3)
    rr, pp = r[keep], p[keep] / v_in
    A = np.exp(-rr[:, None] ** 2 / (2 * sigmas[None, :] ** 2)) / (2 * np.pi * sigmas[None, :] ** 2)
    w, _ = nnls(A / pp[:, None], np.ones_like(pp), maxiter=20000)
    fit = A @ w
    rel = (fit - pp) / pp
    return dict(sigmas=sigmas.tolist(), weights=w.tolist(), rel_rms=float(np.sqrt((rel ** 2).mean())), max_rel=float(np.abs(rel).max()),
                total=float(w.sum()))



# ----------------------------------------------------------------------------- kernel constants
def fit_kernel(r, p, radius, cx_hint=None):
    """Fit one impulse response to K(r) = g * (1 + (r/a)^2)^(-q/2) * exp(-(r/(c*Radius))^2) in log space.
    -> dict(g, a_over_R, q, c, rms_log). q is the inverse-power exponent (2 = inverse square); c the Gaussian cutoff
    in units of Radius (large c = no visible cutoff in the frame)."""
    from scipy.optimize import least_squares
    keep = (p > 1e-7 * p.max()) & (r > 0.7)
    rr, lp = r[keep], np.log(p[keep])

    def model(t):
        lg, la, q, lc = t
        a, c = np.exp(la) * radius, np.exp(lc)
        return lg - 0.5 * q * np.log1p((rr / a) ** 2) - (rr / (c * radius)) ** 2

    best = None
    for la0 in (-3.0, -1.5, 0.0):
        for lc0 in (-0.5, 1.0, 3.0):
            x0 = [lp[0] + 0.5 * 2 * np.log1p((rr[0] / (np.exp(la0) * radius)) ** 2), la0, 2.0, lc0]
            try:
                sol = least_squares(lambda t: model(t) - lp, x0, bounds=([-50, -7, 0.2, -3], [50, 3, 6, 6]))
            except Exception:
                continue
            if best is None or sol.cost < best.cost:
                best = sol
    if best is None:
        return None
    lg, la, q, lc = best.x
    return dict(a_over_R=float(np.exp(la)), q=float(q), c=float(np.exp(lc)), rms_log=float(np.sqrt(2 * best.cost / len(rr))), n=int(len(rr)))


def cmd_fitkernel(a):
    """Per-Radius kernel constants from measured impulse renders."""
    rows = []
    for rid in [r["id"] for r in load_runs() if r["id"].startswith("radius_")]:
        pth = find_render(a.renders, rid) if a.renders else None
        if a.wasm and not pth:
            run = {x["id"]: x for x in load_runs()}[rid]
            arr = render_plugin(a.wasm, load(frame_path(run["frame"])), plugin_over(run, "linear"))
        elif pth:
            arr = load(pth)
        else:
            continue
        radius = float(rid.split("_")[1])
        r, p, _, rmax = radial_profile(arr)
        m = psf_metrics(arr, 100.0, radius)
        f = fit_kernel(r, p, radius)
        if f:
            f.update(radius=radius, cropped=m["radius_exceeds_frame"])
            rows.append(f)
    print(f"{'Radius':>7}{'a (px)':>9}{'a/R':>8}{'q':>7}{'cutoff c':>10}{'rms(log)':>10}  note")
    for f in rows:
        print(f"{f['radius']:>7g}{f['a_over_R'] * f['radius']:>9.2f}{f['a_over_R']:>8.3f}{f['q']:>7.2f}{f['c']:>10.2f}{f['rms_log']:>10.3f}  {'frame crops the tail' if f['cropped'] else ''}")
    if rows:
        ok = [f for f in rows if not f["cropped"]] or rows
        print("\nmedian over uncropped radii: a/R=%.3f  q=%.2f  c=%.2f" % tuple(np.median([f[k] for f in ok]) for k in ("a_over_R", "q", "c")))
        print("q ~ 2 and c ~ 1 across radii  => inverse-square tail with a Gaussian window at Radius (the model in use)")
        print("a (px) constant               => the core is a fixed few pixels wide;  a/R constant => the core scales with Radius")
        print("anything else (q or c drifting, rms > 0.1) => ship a weight table indexed by Radius (weights_by_radius.json)")
    return rows

# ----------------------------------------------------------------------------- threshold
def threshold_table(a, patch_inner=40):
    """Mean output over the middle of every patch of the `patches` frame -> list of (input value, output)."""
    v = lum(a)
    rows = []
    for i, val in enumerate(mf.PATCH_VALS):
        x0, y0 = mf.patch_origin(i)
        c = mf.PATCH // 2
        m = v[y0 + c - patch_inner // 2: y0 + c + patch_inner // 2, x0 + c - patch_inner // 2: x0 + c + patch_inner // 2]
        rows.append((float(val), float(m.mean())))
    return rows


def ramp_table(a, n=24):
    """Output along the `ramp_0_16` frame: input value of a column vs the mean output of that column."""
    v = lum(a)[200:880].mean(axis=0)
    xs = np.linspace(80, W - 81, n).astype(int)      # stay away from the frame edge (blur loses energy there)
    return [(float(x * 16.0 / (W - 1)), float(v[x])) for x in xs]


def threshold_summary(rows):
    xs = np.array([r[0] for r in rows])
    ys = np.array([r[1] for r in rows])
    on = np.where(ys > 1e-4 * max(ys.max(), 1e-12))[0]
    knee = float(xs[on[0]]) if len(on) else None
    hi = xs >= max(xs) * 0.4
    s = float(np.polyfit(xs[hi], ys[hi], 1)[0]) if hi.sum() >= 2 else None
    # x-intercept of the high end: where the straight part would reach zero
    icpt = float(-np.polyfit(xs[hi], ys[hi], 1)[1] / s) if s else None
    return dict(first_nonzero_input=knee, slope_high=s, x_intercept_high=icpt)


# ----------------------------------------------------------------------------- run list
def load_runs():
    return json.load(open(os.path.join(HERE, "runs.json")))["runs"]


def frame_path(name):
    for e in (".exr", ".tif"):
        p = os.path.join(HERE, "frames", name + e)
        if os.path.exists(p):
            return p
    raise FileNotFoundError(name)


def frame_value(name):
    return {"imp_V1": 1.0, "imp_V10": 10.0, "imp_V100": 100.0, "imp_V10_half2px": 10.0}.get(name, 1.0)


def numeric(v, default=None):
    try:
        return float(v)
    except (TypeError, ValueError):
        return default


def analyse_run(run, a):
    """a: rendered array. -> dict of metrics appropriate for the run kind."""
    kind, st = run["kind"], run["settings"]
    out = dict(id=run["id"], kind=kind, settings=st)
    radius = numeric(st.get("Radius"), 10.0)
    fr = run["frame"]
    if kind in ("psf", "exposure", "amplitude") or fr.startswith("imp"):
        cx = CX + (0.5 if fr.endswith("half2px") else 0.0)
        out.update(psf_metrics(a, frame_value(fr), radius, int(round(cx)), CY))
        out["centroid_dx"], out["centroid_dy"] = [float(c - o) for c, o in zip(centroid(a), (CX, CY))]
        if fr.startswith("imp"):
            r = np.array(out["profile"]["r"])
            p = np.array(out["profile"]["p"])
            out["ladder_fit"] = fit_ladder(r, p, frame_value(fr))
        if st.get("AspectRatio") or st.get("Angle") is not None:
            out["aniso"] = anisotropy(a)
    elif kind == "threshold":
        rows = ramp_table(a) if fr == "ramp_0_16" else threshold_table(a)
        out["table"] = rows
        out.update(threshold_summary(rows))
    elif kind == "edge":
        v = lum(a)
        row = v[CY]
        out["edge_row"] = row[::8].tolist()
        out["edge_center_over_far"] = float(row[CX] / max(row[CX - int(3 * (radius or 100))] if CX - int(3 * (radius or 100)) >= 0 else row[0], 1e-30))
    else:
        out["total"] = float(lum(a).sum())
        out["max"] = float(a.max())
    return out


def cross_run_laws(res):
    """Laws that need several runs: exposure transfer, amplitude linearity, radius scaling, self-similarity."""
    law = {}
    ex = sorted([(numeric(r["settings"].get("Exposure")), r["total_out_per_in"]) for r in res
                 if r["kind"] == "exposure" and "total_out_per_in" in r], key=lambda t: t[0])
    if len(ex) >= 3:
        e = np.array([t[0] for t in ex]); y = np.array([t[1] for t in ex])
        b, a0 = np.polyfit(np.log2(e), np.log2(y), 1)         # y = c * E^b
        res_pow = float(np.abs(np.log2(y) - (a0 + b * np.log2(e))).max())
        k, c0 = np.polyfit(e, np.log2(y), 1)                  # y = c * 2^(k E)
        res_exp = float(np.abs(np.log2(y) - (c0 + k * e)).max())
        law["exposure"] = dict(points=ex, power_law_exponent=float(b), power_law_residual_log2=res_pow,
                               stops_per_unit=float(k), exp_law_residual_log2=res_exp,
                               best="power law (linear gain)" if res_pow <= res_exp else "exponential (stops)")
    am = sorted([(frame_value(r["id"].replace("amp_V", "imp_V")) if r["id"].startswith("amp_V") else None, r["total_out_per_in"])
                 for r in res if r["kind"] == "amplitude" and "total_out_per_in" in r])
    if len(am) >= 2:
        y = np.array([t[1] for t in am])
        law["amplitude_linearity_max_dev"] = float(np.abs(y / y[0] - 1).max())
    rad = sorted([(numeric(r["settings"].get("Radius")), r) for r in res if r["id"].startswith("radius_") and "profile" in r],
                 key=lambda t: t[0])
    if len(rad) >= 2:
        law["radius"] = [dict(radius=rv, total=r["total_out_per_in"], r_half=r["r_half"], r_half_over_radius=(r["r_half"] / rv if r["r_half"] else None),
                              r_1e3_over_radius=(r["r_1e3"] / rv if r["r_1e3"] else None), **{f"slope@{k}": v for k, v in r["slopes"].items()},
                              cropped=r["radius_exceeds_frame"]) for rv, r in rad]
        # self-similarity: after scaling r by Radius and the profile by Radius^2 (energy-normalised), do the curves coincide?
        grid = np.geomspace(0.02, 0.5, 40)
        curves = []
        for rv, r in rad:
            if r["radius_exceeds_frame"]:
                continue
            rr, pp = np.array(r["profile"]["r"]), np.array(r["profile"]["p"]) * rv ** 2 / max(r["total_out_per_in"], 1e-30)
            ok = (rr / rv >= grid[0]) & (rr / rv <= grid[-1]) & (pp > 0)
            if ok.sum() > 5:
                curves.append(np.interp(np.log(grid), np.log(rr[ok] / rv), np.log(pp[ok]), left=np.nan, right=np.nan))
        if len(curves) >= 2:
            c = np.array(curves)
            good = np.isfinite(c).sum(0) >= 2
            if good.any():
                law["self_similarity_max_log_spread"] = float((np.nanmax(c[:, good], 0) - np.nanmin(c[:, good], 0)).max())
    ph = {r["id"]: r for r in res if r["id"] in ("phase_int", "phase_half") and "profile" in r}
    if len(ph) == 2:
        a_, b_ = ph["phase_int"], ph["phase_half"]
        law["phase"] = dict(total_ratio=b_["total_out_per_in"] / a_["total_out_per_in"] * (10 / 10),
                            centroid_shift_px=b_["centroid_dx"] - a_["centroid_dx"])
    return law


# ----------------------------------------------------------------------------- plug-in rendering
def to_rgba(a):
    h, w, _ = a.shape
    o = np.ones((h, w, 4), np.float32)
    o[..., :3] = a
    return o


def render_plugin(wasm, a, over):
    h, w, _ = a.shape
    with tempfile.TemporaryDirectory() as td:
        pi, po = os.path.join(td, "i.f32"), os.path.join(td, "o.f32")
        to_rgba(a).tofile(pi)
        r = subprocess.run(["node", os.path.join(HERE, "plugin_render.mjs"), wasm, pi, str(w), str(h), po, json.dumps(over)],
                           capture_output=True, text=True)
        if r.returncode:
            raise RuntimeError(r.stderr[-400:])
        return np.fromfile(po, np.float32).reshape(h, w, 4)[..., :3]


def plugin_over(run, exposure_mode="native"):
    """Map a run's settings to the open plug-in's parameters (glow only, everything neutral).
    Returns None if the plug-in has no equivalent."""
    st = run["settings"]
    o = dict(glow_only=True, unmult=False, dither=0, tonemap=0, tint_amount=0, threshold=0, softness=0,
             glow_mode=0, aspect=1, angle_enable=False, src_opacity=0, exposure=0)   # plug-in mode 0 = inverse square (the kernel DG2 calls Exponential)
    if "Radius" in st and numeric(st["Radius"]) is not None:
        o["radius"] = numeric(st["Radius"])
    elif st.get("Radius") == "minimum":
        o["radius"] = 1
    if "Exposure" in st:
        e = numeric(st["Exposure"])
        o["exposure"] = float(np.log2(e)) if exposure_mode == "linear" else e
    if "Threshold" in st:
        o["threshold"] = numeric(st["Threshold"])   # naive 1:1 (percent of 1.0); the real mapping is one of the things to measure
    if "ThresholdSmooth" in st:
        o["softness"] = numeric(st["ThresholdSmooth"])
    if "AspectRatio" in st:
        o["aspect"] = numeric(st["AspectRatio"])
    if st.get("EnableAngle"):
        o["angle_enable"] = True
        o["angle"] = numeric(st.get("Angle"), 0)
    if st.get("ThresholdMode") == "Chrominance":
        o["thr_mode"] = 1
    if "SaturationBias" in st:
        o["sat_bias"] = numeric(st["SaturationBias"])
    if st.get("GlowMode") == "Iris":
        return None
    if "LensDirtTexture" in st or "Quality" in st or "Multipliers" in st or "Downsampling" in st:
        return None
    return o


# ----------------------------------------------------------------------------- commands
def fmt(v, nd=4):
    if v is None:
        return "-"
    if isinstance(v, bool):
        return str(v)
    if isinstance(v, (int,)):
        return str(v)
    return f"{v:.{nd}g}"


def print_report(res, laws, title):
    print(f"\n=== {title} ===")
    print(f"{'run':<22}{'gain':>9}{'r_half':>9}{'r_1e-3':>9}{'r_1e-6':>9}{'slope@.5R':>10}{'slope@R':>9}{'ladder rms':>12}")
    for r in res:
        if "profile" in r:
            lf = r.get("ladder_fit", {})
            print(f"{r['id']:<22}{fmt(r['total_out_per_in']):>9}{fmt(r['r_half']):>9}{fmt(r['r_1e3']):>9}{fmt(r['r_1e6']):>9}"
                  f"{fmt(r['slopes'].get('0.5R'), 3):>10}{fmt(r['slopes'].get('1.0R'), 3):>9}{fmt(lf.get('rel_rms'), 3):>12}")
    for k, v in laws.items():
        print(f"\n[{k}]")
        if isinstance(v, list):
            for row in v:
                print("  ", {a: (round(b, 4) if isinstance(b, float) else b) for a, b in row.items()})
        else:
            print("  ", json.dumps(v, default=lambda o: round(float(o), 5)))
    for r in res:
        if r["kind"] == "threshold":
            print(f"\n[{r['id']}] first non-zero input={fmt(r.get('first_nonzero_input'))} slope(high)={fmt(r.get('slope_high'))} x-intercept={fmt(r.get('x_intercept_high'))}")
            print("   in : " + " ".join(f"{x:6.2f}" for x, _ in r["table"][:16]))
            print("   out: " + " ".join(f"{y:6.3f}" for _, y in r["table"][:16]))


def cmd_report(a):
    runs = load_runs()
    res, missing = [], []
    for run in runs:
        pth = find_render(a.renders, run["id"])
        if not pth:
            missing.append(run["id"])
            continue
        res.append(analyse_run(run, load(pth)))
    laws = cross_run_laws(res)
    print_report(res, laws, f"measured renders in {a.renders}")
    if missing:
        print(f"\nmissing renders ({len(missing)}): " + ", ".join(missing[:12]) + (" ..." if len(missing) > 12 else ""))
    if a.json:
        json.dump(dict(runs=res, laws=laws), open(a.json, "w"), indent=1, default=float)
        print("wrote", a.json)
    # weight table
    tab = [dict(radius=numeric(r["settings"]["Radius"]), sigmas=r["ladder_fit"]["sigmas"], weights=r["ladder_fit"]["weights"],
                rel_rms=r["ladder_fit"]["rel_rms"]) for r in res if r["id"].startswith("radius_") and "ladder_fit" in r]
    if tab and a.table:
        json.dump(tab, open(a.table, "w"), indent=1)
        print("wrote", a.table, "(Gaussian-ladder weights per Radius)")


def cmd_compare(a):
    runs = load_runs()
    rows = []
    for run in runs:
        pth = find_render(a.renders, run["id"])
        if not pth:
            continue
        o = plugin_over(run, a.exposure_mode)
        if o is None:
            continue
        meas = analyse_run(run, load(pth))
        mine = analyse_run(run, render_plugin(a.wasm, load(frame_path(run["frame"])), o))
        rows.append((run, meas, mine))
    print(f"{'run':<20}{'quantity':<14}{'measured':>12}{'plug-in':>12}{'ratio':>9}")
    for run, m, p in rows:
        if "profile" in m:
            for q in ("total_out_per_in", "r_half", "r_1e3", "slope@0.5R", "slope@1.0R"):
                mv, pv = (m.get(q), p.get(q)) if "@" not in q else (m["slopes"].get(q[6:]), p["slopes"].get(q[6:]))
                ratio = (pv / mv) if (mv and pv and mv != 0) else None
                print(f"{run['id']:<20}{q:<14}{fmt(mv):>12}{fmt(pv):>12}{fmt(ratio, 3):>9}")
            rm, rp = np.array(m["profile"]["r"]), np.array(m["profile"]["p"])
            rp2, pp2 = np.array(p["profile"]["r"]), np.array(p["profile"]["p"])
            n = np.interp(np.log(rm[rm > 0.9]), np.log(rp2[rp2 > 0.9]), pp2[rp2 > 0.9])
            lr = np.log(n / m["profile"]["p"][-len(n):]) if False else np.log(n / np.array(m["profile"]["p"])[rm > 0.9])
            print(f"{run['id']:<20}{'profile log-ratio rms':<14}{'':>12}{'':>12}{np.sqrt(np.mean(lr ** 2)):>9.3f}")
        elif m["kind"] == "threshold":
            print(f"{run['id']:<20}threshold table  measured: " + " ".join(f"{y:6.3f}" for _, y in m["table"]))
            print(f"{'':<20}                 plug-in : " + " ".join(f"{y:6.3f}" for _, y in p["table"]))


def cmd_selftest(a):
    ok = True

    def check(name, cond, info=""):
        nonlocal ok
        print(("PASS " if cond else "FAIL ") + name + (f"   [{info}]" if info else ""))
        ok &= bool(cond)

    # 1. the Gaussian-ladder fit recovers a known inverse-square-with-core kernel
    r = np.geomspace(0.5, 500, 200)
    aa, R = 8.0, 120.0
    k = (1 + (r / aa) ** 2) ** -1.0 * np.exp(-(r / R) ** 2)
    fit = fit_ladder(r, k)
    check("ladder fit reproduces a known kernel within 3% rms", fit["rel_rms"] < 0.03, f"rms {fit['rel_rms']:.4f}")
    # 2. slope / half-radius / crossing helpers
    check("log-log slope of 1/(1+(r/a)^2) between a and R is about -2", abs(slope(r, k, 16, 26) + 2) < 0.25, f"{slope(r, k, 16, 26):.2f}")
    # 3. radial profile of a synthetic isotropic image is flat in angle (profile of a Gaussian blob)
    y, x = np.mgrid[:H, :W]
    g = np.exp(-((x - CX) ** 2 + (y - CY) ** 2) / (2 * 30 ** 2)).astype(np.float32)
    gg = np.repeat(g[..., None], 3, -1)
    rr, pp, _, _ = radial_profile(gg)
    want = np.exp(-rr ** 2 / (2 * 30 ** 2))
    check("radial profile of a Gaussian blob matches the analytic one", np.abs(pp / want - 1)[rr < 100].max() < 0.02)
    an = anisotropy(np.repeat(np.exp(-((x - CX) ** 2 / (2 * 40 ** 2) + (y - CY) ** 2 / (2 * 20 ** 2))).astype(np.float32)[..., None], 3, -1))
    check("anisotropy of a 2:1 Gaussian reads 2", abs(an["aspect"] - 2) < 0.1 and abs(an["major_angle_deg"]) < 2, f"{an['aspect']:.2f} {an['major_angle_deg']:.1f}")

    # 4. the open plug-in, measured like a black box
    wasm = a.wasm
    if not os.path.exists(wasm):
        print("SKIP plug-in checks: wasm not found:", wasm)
        return 0 if ok else 1
    runs = {r["id"]: r for r in load_runs()}
    res = []
    for rid in ("radius_25", "radius_50", "radius_100", "radius_200", "exposure_0.5", "exposure_1", "exposure_2", "exposure_4",
                "amp_V1", "amp_V10", "amp_V100", "phase_int", "phase_half"):
        run = runs[rid]
        o = plugin_over(run, "linear")
        frame = load(frame_path(run["frame"]))
        # the plug-in's Exposure is in stops; the run list uses linear gain. plugin_over(.., "linear") maps it.
        res.append(analyse_run(run, render_plugin(wasm, frame, o)))
    laws = cross_run_laws(res)
    print_report(res, laws, "open plug-in (self-test)")
    byid = {r["id"]: r for r in res}
    for rid in ("radius_50", "radius_100"):
        s = byid[rid]
        check(f"{rid}: no NaN / negative pixels", s["nan_pixels"] == 0 and s["negative_pixels"] == 0)
        sl = s["slopes"]["0.5R"]
        check(f"{rid}: slope at 0.5 Radius is about -2 (inverse square, a little steeper from the cutoff)", sl is not None and -2.7 < sl < -1.5, fmt(sl, 3))
        check(f"{rid}: total out/in is 1 at 0 stops", abs(s["total_out_per_in"] - 1) < 0.03, fmt(s["total_out_per_in"], 4))
        check(f"{rid}: the fitted ladder describes the render within 10%", s["ladder_fit"]["rel_rms"] < 0.10, fmt(s["ladder_fit"]["rel_rms"], 3))
    check("exposure doubles the glow per stop (log2 mapping)", abs(laws["exposure"]["power_law_exponent"] - 1) < 0.03,
          f"exponent {laws['exposure']['power_law_exponent']:.3f}, best={laws['exposure']['best']}")
    check("glow is linear in the input amplitude", laws["amplitude_linearity_max_dev"] < 0.02, fmt(laws["amplitude_linearity_max_dev"], 3))
    check("radius grows the core: r_half rises with Radius", all(b["r_half"] >= a_["r_half"] * 0.9 for a_, b in zip(laws["radius"], laws["radius"][1:])),
          ", ".join(f"{x['r_half']:.1f}" for x in laws["radius"]))
    if "self_similarity_max_log_spread" in laws:
        print(f"info: self-similarity max log spread {laws['self_similarity_max_log_spread']:.2f} (0 = the kernel is a pure rescaling of one shape)")
    check("half-pixel impulse moves the centroid by 0.5 px", abs(laws["phase"]["centroid_shift_px"] - 0.5) < 0.1, fmt(laws["phase"]["centroid_shift_px"], 3))
    check("half-pixel impulse keeps the total", abs(laws["phase"]["total_ratio"] - 1) < 0.03, fmt(laws["phase"]["total_ratio"], 4))

    # kernel constants recovered from the plug-in's own renders
    r100 = np.array(byid["radius_100"]["profile"]["r"]); p100 = np.array(byid["radius_100"]["profile"]["p"])
    fk = fit_kernel(r100, p100, 100.0)
    check("fit-kernel finds an inverse-square tail (q about 2) in the plug-in", fk is not None and 1.6 < fk["q"] < 2.4, f"q={fk['q']:.2f} a/R={fk['a_over_R']:.3f} c={fk['c']:.2f} rms={fk['rms_log']:.3f}")
    check("fit-kernel describes the plug-in profile within 8% (log rms)", fk["rms_log"] < 0.08, f"{fk['rms_log']:.3f}")

    # threshold curve of the plug-in
    run = dict(id="thr_50", frame="patches", kind="threshold", settings=dict(Threshold=50, Radius="minimum"))
    o = plugin_over(run)
    tr = analyse_run(run, render_plugin(wasm, load(frame_path("patches")), o))
    ys = [y for _, y in tr["table"]]
    check("threshold curve is monotone non-decreasing", all(b >= a_ - 1e-4 for a_, b in zip(ys, ys[1:])))
    check("patches below the threshold stay dark", ys[0] < 1e-3 and ys[1] < 1e-3)
    print("threshold(plug-in, thr 200): in :", " ".join(f"{x:6.2f}" for x, _ in tr["table"]))
    print("                             out:", " ".join(f"{y:6.3f}" for _, y in tr["table"]))
    print("\nSELFTEST", "OK" if ok else "FAILED")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("report"); r.add_argument("--renders", default="renders"); r.add_argument("--json"); r.add_argument("--table", default="weights_by_radius.json")
    c = sub.add_parser("compare"); c.add_argument("--renders", default="renders"); c.add_argument("--wasm", default=os.path.join(HERE, "../../plugins/ec-deep-glow-2/ec_deep_glow_2.wasm"))
    c.add_argument("--exposure-mode", choices=["native", "linear"], default="linear", help="linear: DG2 Exposure E -> plug-in stops log2(E); native: pass through")
    k = sub.add_parser("fit-kernel", help="fit a/R, q, cutoff to the radius_* renders (or to the plug-in with --wasm)")
    k.add_argument("--renders", default=None); k.add_argument("--wasm", default=None)
    s = sub.add_parser("selftest"); s.add_argument("--wasm", default=os.path.join(HERE, "../../plugins/ec-deep-glow-2/ec_deep_glow_2.wasm"))
    a = ap.parse_args()
    if a.cmd == "report":
        cmd_report(a)
    elif a.cmd == "compare":
        cmd_compare(a)
    elif a.cmd == "fit-kernel":
        cmd_fitkernel(a)
    else:
        sys.exit(cmd_selftest(a))


if __name__ == "__main__":
    main()
