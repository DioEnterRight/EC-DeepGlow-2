#!/usr/bin/env python3
"""Make the test frames and the run list for measuring Deep Glow 2 from the outside.

Black-box method only: you render these frames through the effect in After Effects, then
`analyze.py` measures the outputs. Nothing here touches the plugin binary.

    python3 make_frames.py [--out frames] [--exr]

Writes 32-bit float RGB TIFFs (and EXRs with --exr), `runs.json` (machine-readable) and `RUNS.md`
(what to set in After Effects for every run).

Frame geometry: every frame is 1920 x 1080 except the iris apertures (256 x 256). Pixel (960, 540)
is the centre of the comp; its centre is at (960.5, 540.5).
"""
import argparse, json, os
import numpy as np

W, H = 1920, 1080
CX, CY = 960, 540


PATCH_VALS = [0, 0.05, 0.1, 0.2, 0.3, 0.5, 0.7, 1, 1.5, 2, 3, 4, 6, 8, 12, 16]
PATCH = 120


def patch_origin(i):
    return 60 + (i % 8) * 230, 200 + (i // 8) * 400


def impulse(v, two_px=False):
    a = np.zeros((H, W, 3), np.float32)
    if two_px:                       # centroid half a pixel to the right of (CX, CY)
        a[CY, CX, :] = v / 2
        a[CY, CX + 1, :] = v / 2
    else:
        a[CY, CX, :] = v
    return a


def disc(a, cx, cy, r, rgb):
    y, x = np.ogrid[:a.shape[0], :a.shape[1]]
    d = np.hypot(x + 0.5 - cx, y + 0.5 - cy)
    m = np.clip(r + 0.5 - d, 0, 1)[..., None]       # 1-pixel anti-aliased edge
    a[:] = a * (1 - m) + np.array(rgb, np.float32) * m


def frames():
    f = {}
    for v in (1, 10, 100):
        f[f"imp_V{v}"] = impulse(v)
    f["imp_V10_half2px"] = impulse(10, two_px=True)

    e = np.zeros((H, W, 3), np.float32)
    e[:, :CX, :] = 2.0
    f["edge_V2"] = e

    d = np.zeros((H, W, 3), np.float32)
    disc(d, CX + 0.5, CY + 0.5, 100, (4, 4, 4))
    f["disc_r100_V4"] = d

    # Threshold: flat patches and a ramp.
    p = np.zeros((H, W, 3), np.float32)
    for i, v in enumerate(PATCH_VALS):
        x0, y0 = patch_origin(i)
        p[y0:y0 + 120, x0:x0 + 120, :] = v
    f["patches"] = p
    r = np.zeros((H, W, 3), np.float32)
    r[:] = (np.linspace(0, 16, W, dtype=np.float32))[None, :, None]
    f["ramp_0_16"] = r

    s = np.zeros((H, W, 3), np.float32)
    cols = [(4, 0, 0), (0, 4, 0), (0, 0, 4), (4, 4, 0), (0, 4, 4), (4, 0, 4),
            (4, 2, 2), (2, 4, 2), (2, 2, 4), (4, 4, 2), (2, 4, 4), (4, 2, 4)]
    for i, c in enumerate(cols):
        disc(s, 160 + (i % 6) * 320, 300 + (i // 6) * 480, 40, c)
    f["sat_discs"] = s

    # Iris apertures, 256 x 256, white on black.
    def ap(fn):
        a = np.zeros((256, 256, 3), np.float32)
        y, x = np.mgrid[:256, :256]
        m = fn(x + 0.5 - 128, y + 0.5 - 128)
        a[:] = m[..., None].astype(np.float32)
        return a
    f["iris_disc"] = ap(lambda x, y: (np.hypot(x, y) <= 100).astype(float))
    f["iris_ring"] = ap(lambda x, y: ((np.hypot(x, y) <= 100) & (np.hypot(x, y) >= 80)).astype(float))
    f["iris_offdot"] = ap(lambda x, y: (np.hypot(x - 60, y) <= 12).astype(float))

    def hexa(x, y):
        m = np.ones_like(x, dtype=bool)
        for k in range(6):
            t = k * np.pi / 3
            m &= (x * np.cos(t) + y * np.sin(t)) <= 100 * np.cos(np.pi / 6)
        return m.astype(float)
    f["iris_hex"] = ap(hexa)

    # Lens dirt mattes.
    f["dirt_flat_0.5"] = np.full((H, W, 3), 0.5, np.float32)
    c = np.zeros((H, W, 3), np.float32)
    yy, xx = np.mgrid[:H, :W]
    c[:] = (((xx // 120) + (yy // 120)) % 2).astype(np.float32)[..., None]
    f["dirt_checker"] = c
    return f


COMMON = ("32 bpc project. Run once with Project Settings > Linearize Working Space ON (primary) and, for the "
          "runs marked `both`, once OFF with the same settings (name the output `<run>_nolin`). Comp 1920x1080, "
          "source layer = the frame, comp-sized, Deep Glow 2 applied. Defaults except what the run says. "
          "Source Opacity 0 (glow only), Unmult OFF, Dither OFF, Tone Mapping None/off, Tint off, "
          "Aspect Ratio 1 (no angle), Threshold 0, Glow Mode Exponential, Quality = the default preset unless stated.")

RADII = [10, 25, 50, 100, 200, 400, 800]
EXPOSURES = [0.25, 0.5, 1, 2, 4, 8]


def runs():
    R = []

    def add(rid, frame, pri, kind, note, **settings):
        R.append(dict(id=rid, frame=frame, priority=pri, kind=kind, note=note, settings=settings))

    for r in RADII:
        add(f"radius_{r}", "imp_V100", "P1", "psf", "Radius sweep (impulse, V=100).", Radius=r)
    for x in EXPOSURES:
        add(f"exposure_{x}", "imp_V100", "P1", "exposure", "Exposure sweep at Radius 100.", Radius=100, Exposure=x)
    for v in (1, 10, 100):
        add(f"amp_V{v}", f"imp_V{v}", "P1", "amplitude", "Linearity in input amplitude, Radius 100.", Radius=100)
    add("phase_int", "imp_V10", "P2", "psf", "Grid-phase test, integer position, Radius 50.", Radius=50)
    add("phase_half", "imp_V10_half2px", "P2", "psf", "Grid-phase test, +0.5 px, Radius 50.", Radius=50)
    for r in (100, 400):
        add(f"edge_{r}", "edge_V2", "P2", "edge", "Half-plane: edge spread function.", Radius=r)
    add("disc_100", "disc_r100_V4", "P2", "psf", "Large disc, Radius 100.", Radius=100)
    for t in (0, 10, 25, 50, 75, 100):
        add(f"thr_{t}", "patches", "P1", "threshold",
            "Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final.", Threshold=t, Radius="minimum")
    for sm in (0, 50, 100):
        add(f"thrsmooth_{sm}", "patches", "P2", "threshold", "Threshold 50, Threshold Smooth sweep, Radius minimum.",
            Threshold=50, ThresholdSmooth=sm, Radius="minimum")
    add("thrmode_lum", "patches", "P2", "threshold", "Threshold 50, mode Luminance, Radius minimum.", Threshold=50, ThresholdMode="Luminance", Radius="minimum")
    add("thrmode_chroma", "patches", "P2", "threshold", "Threshold 50, mode Chrominance, Radius minimum.", Threshold=50, ThresholdMode="Chrominance", Radius="minimum")
    add("thr_ramp_50", "ramp_0_16", "P2", "threshold", "Ramp, Threshold 50, Radius minimum.", Threshold=50, Radius="minimum")
    for sb in (0, 100):
        add(f"sat_bias_{sb}", "sat_discs", "P2", "generic", "Saturated discs, Radius 100, Saturation Bias sweep.", Radius=100, SaturationBias=sb)
    for a in (0.5, 2):
        add(f"aspect_{a}", "imp_V100", "P2", "generic", "Aspect Ratio sweep, Radius 100.", Radius=100, AspectRatio=a)
    add("aspect_2_angle45", "imp_V100", "P2", "generic", "Aspect 2, Enable Angle, 45 degrees.", Radius=100, AspectRatio=2, EnableAngle=True, Angle=45)
    add("rgb_default", "imp_V100", "P2", "generic", "True chromatic aberration (RGB radius multipliers) at their defaults, Radius 100.", Radius=100, GlowAberration=True)
    add("rgb_custom", "imp_V100", "P3", "generic", "RGB multipliers R=50 G=100 B=200, Radius 100.", Radius=100, GlowAberration=True, Multipliers="50/100/200")
    for q in ("Draft", "Low", "Medium", "High", "(each remaining preset in the list)"):
        add(f"quality_{q.split()[0].lower()}", "imp_V100", "P2", "generic", "Quality preset ladder signature (look for ripple), Radius 100.", Radius=100, Quality=q)
    for ds, st, it in ((0, 1, 8), (0, 4, 8), (2, 2, 8), (0, 2, 3)):
        add(f"custom_d{ds}_s{st}_i{it}", "imp_V100", "P3", "generic", "Custom quality.", Radius=100, Quality="Custom", Downsampling=ds, StepsMultiplier=st, Iterations=it)
    add("tonemap_each", "imp_V100", "P3", "generic", "One render per Tone Mapping operator (write down the names the popup shows, in order), Radius 100.", Radius=100)
    add("tint_each", "imp_V100", "P3", "generic", "Tint inner=orange outer=blue, one per blend mode (write the popup names), Radius 100.", Radius=100)
    for dm, name in ((0, "dirt_flat_0.5"), (1, "dirt_checker")):
        add(f"dirt_{name}", "imp_V100", "P3", "generic",
            f"Lens Dirt with the `{name}` frame as the texture layer; then repeat with Matte Exposure +2 and Matte Gamma 2.", Radius=100, LensDirtTexture=name)
    for ap in ("iris_disc", "iris_hex", "iris_ring", "iris_offdot"):
        add(f"{ap}_r100", "imp_V100", "P2", "generic", f"Glow Mode Iris, Iris layer = {ap}, Radius 100.", GlowMode="Iris", IrisLayer=ap, Radius=100)
    for it in (1, 4, 16, 64):
        add(f"iris_disc_it{it}", "imp_V100", "P3", "generic", "Iris disc, iterations sweep.", GlowMode="Iris", IrisLayer="iris_disc", Radius=100, IrisIterations=it)
    for eas in (0, 50, 100):
        add(f"iris_disc_ease{eas}", "imp_V100", "P3", "generic", "Iris disc, radii easing sweep.", GlowMode="Iris", IrisLayer="iris_disc", Radius=100, RadiiEasing=eas)
    add("iris_offdot_norm", "imp_V100", "P3", "generic", "Iris offdot with Radius Normalization ON vs OFF (render both, suffix _on/_off).", GlowMode="Iris", IrisLayer="iris_offdot", Radius=100)
    return R


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="frames")
    ap.add_argument("--exr", action="store_true")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    import tifffile
    for name, img in frames().items():
        tifffile.imwrite(os.path.join(a.out, name + ".tif"), img, photometric="rgb")
        if a.exr:
            import OpenEXR
            with OpenEXR.File({"compression": OpenEXR.ZIP_COMPRESSION, "type": OpenEXR.scanlineimage}, {"RGB": img}) as f:
                f.write(os.path.join(a.out, name + ".exr"))
    R = runs()
    json.dump(dict(width=W, height=H, cx=CX, cy=CY, common=COMMON, runs=R), open("runs.json", "w"), indent=1)
    with open("RUNS.md", "w") as f:
        f.write("# Deep Glow 2 measurement runs\n\n" + COMMON + "\n\n")
        f.write("Render each run to 32-bit float EXR (or TIFF) named `<run id>_#####.exr` (or just `<run id>.exr`) into `renders/`.\n"
                "Do the P1 runs first: they are enough to fit the kernel, the Exposure law and the Threshold curve.\n\n")
        f.write("| P | run id | source frame | settings | what it tells us |\n|---|---|---|---|---|\n")
        for r in R:
            st = ", ".join(f"{k}={v}" for k, v in r["settings"].items())
            f.write(f"| {r['priority']} | `{r['id']}` | `{r['frame']}` | {st} | {r['note']} |\n")
    print(f"{len(frames())} frames in {a.out}/, {len(R)} runs in runs.json / RUNS.md")


if __name__ == "__main__":
    main()
