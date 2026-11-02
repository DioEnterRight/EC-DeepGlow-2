# EC Deep Glow 2 — v4.1.1 (EffectCraft wasm plugin)

Drop-in replacement for `deep_glow_open_3.wasm`: same ABI (`ec_api_version / ec_manifest_ptr / ec_manifest_len /
ec_alloc / ec_render`), same plugin id, same first 39 flat params. Thirty params appended (flat 39–68) in v4.0 and
four more (flat 69–72) in v4.1, so saved presets from v2/v3/v4.0 still load — anything missing falls back to its default.

`deep_glow_open.c` is the whole source: freestanding C, no libc, compiled to wasm32.

## Fixed in 4.1.1 (debug pass against EffectCraft's real plug-in host)

Tested through a copy of the app's host (`test/hostsim`: wasmi, instruction budget 50 M + 20 000 per pixel, 1 GiB memory),
not only Node. Bugs found and fixed:

| Bug | Effect in the app | Fix |
|---|---|---|
| Lens Iris at High sampling, or with many iterations, or with Aspect + Angle, ran out of the host's instruction budget | the frame came back **with no glow** (the host swallows the error) | a cost model of the plan (fitted to wasmi fuel, ±9 %) keeps every render under 70 % of the budget: it trims the iris kernel, then moves the finest work one pyramid level down, then (tiny frames) uses fewer Gaussians per octave |
| `memory.grow` failure was tested with `< 0`, but it returns an unsigned value | 4K with a rotated glow **trapped** (out-of-bounds) instead of failing cleanly | test against `(size_t)-1`; arena sizes kept in 64-bit |
| peak memory of a rotated glow | 4K + Aspect + Angle needed 1.19 GB, over the 1 GiB cap | reconstruction reuses the spent staircase and source buffers; `tmp` sized to the largest cell: 4K rotated now 0.93 GB and renders |
| Draft quality passed the ladder's start sigma (float) into an `int` | Draft's smallest Gaussian was 7.4 px instead of 3.6 px: softer core and +5 % light | the start sigma is passed as a float |
| parameters were not range-checked (presets/expressions can send anything) | `radius = 1e300` ran out of budget; negative or huge Intensity gave negative / Inf pixels | every parameter clamped in double precision; angles reduced in double; final glow forced finite and non-negative |

New tests: `test/host_fuzz.py` (random settings through the real host), `test/native_fuzz.py` (native build with
AddressSanitizer + UBSan, every buffer its own heap block), `test/cost_fit.py` (refits the cost model), and 14 more checks in
`v4_test.mjs` (97 in all).

## What changed in 4.1 (documented Deep Glow 2 controls that were missing)

| | 4.1 |
|---|---|
| Threshold Mode | the popup now reads **Luminance / Chrominance** (the second entry is the max-channel test, as before; only the label changed). Default stays Luminance. |
| Tint Blend | 6 modes: adds **Overlay** and **Soft Light** (standard formulas; above 1.0 the excess is carried through scaled by 2c so HDR glow is not clipped). |
| **Dither Monochromatic** (69) | on = one noise value for all three channels (as before); off = independent noise per channel. |
| **Iris Opacity** (70) | 0–100 %, 100. Mixes the aperture-shaped glow with the round glow. |
| **Iris Aspect Ratio** (71) | 0.1–10, 1.0. Stretches the aperture on its own; Aspect Ratio still stretches the whole glow. |
| **Iris Continuous Rasterization** (72) | on (default) = smooth aperture. off = ring radii snapped to whole level pixels with a narrow edge, the crisp stepped look. |

Meaning of the three Iris controls is **inferred from their names**: I could not check them against the real panel.
Not possible in plug-in API v1 (no layer parameters, groups, buttons or padding): the Mask / Iris / Lens Dirt *layer* inputs
(this plug-in draws its aperture and dirt procedurally), Buffer Expansion, Iris Radius Normalization (needs the aperture layer),
Tone Mapping presets beyond the six curves below, and the grouped UI. The built-in effect has the layer inputs.

## What changed from v3

| v3 | v4 |
|---|---|
| Aspect Ratio squeezed the image along the wide axis, glowed it isotropically, then un-squeezed. Above about 3:1 the kernel stopped being resolved and the profile went flat and blocky. | **Anisotropic pyramid**: each Gaussian of the ladder gets its own pyramid level *per axis* (`lx` halvings in x, `ly` in y) and its own sigma per axis. The kernel is resolved on both axes at any ratio; cells are rebuilt by cubic B-spline refinement in x, then in y. |
| no Angle | **Enable Angle / Angle**: the frame is turned by −angle onto a larger canvas, glowed, and turned back, so a stretched glow can point anywhere. Resampling uses a cubic B-spline, which is a partition of unity — plain bilinear loses 35 % of an impulse's energy at 45°. Measured energy error over 0–90°: ≤ 1 %. |
| Glow Mode: Inverse Square / Exponential | adds **Lens Iris**: nested procedural aperture rings (blade count, rotation, radii easing, iterations), each drawn on the pyramid level that holds it, the whole bank normalised to unit energy together. |
| threshold only | adds **Saturation Bias**: grey light is faded out of the glow source, saturated light is kept. |
| Glow Only checkbox | **View**: Composite / Glow Only / Threshold Matte, plus **Unmult** for the output alpha. |
| chroma: radius split + one fixed lateral pair | **Pixel Aberration Channels**: six channel pairings, and **Glow Aberration Order** to permute which multiplier drives which channel. |
| 3-tone tint (low / mid / high) | keeps it, and adds **Tint Style = Inner / Outer** with Multiply / Normal / Add / Screen blends and a Strength, matching the built-in effect. |
| 6 quality presets | adds **Custom**: Downsampling, Steps Multiplier, Iterations. |
| 2 blend modes | 8: Screen (HDR-safe), Add, Normal, Lighten, Colour Dodge, Linear Dodge, Overlay, Soft Light. |
| 5 tone curves | 6 (adds Lottes). |
| lens dirt amount / scale / contrast / seed | adds **Inherit Glow Colour**, **Matte Exposure**, **Matte Gamma**. |
| — | **Half Float**: rounds the glow buffer to IEEE half precision, to preview what a 16-bit pipeline would give. |

Also: one staircase walk builds the cells (`lx_max + ly_max` passes instead of rebuilding a chain per column),
and the reconstruction buffer reuses the thresholded source when no rotation is needed. 4K Medium is now faster
and lighter than v3 despite the extra generality.

## Kernel model
* Inverse Square: `K(r) = (1 + (r/a)^2)^(-p/2) * exp(-(r/R)^2)`, p = Falloff (2.0 = the inverse-square law),
  a = soft core (>= 1.5 px), Gaussian window at R so the energy is finite. Exponential: `K(r) = exp(-3 r / R)`.
* The fit is done once for the isotropic kernel: a smoothness-regularised non-negative least-squares sum of the
  ladder Gaussians, relative error on log-spaced radii, renormalised to unit energy. An anisotropic kernel is the
  same kernel with its axes scaled, so Aspect Ratio changes where each Gaussian is placed but not its weight —
  Radius, Falloff, chroma and the radius multipliers only move the fit weights, so every control is continuous.
* Aspect Ratio is area preserving: `aspect = 2` is twice as wide and half as tall, same total size.
* Measured against the analytic target (impulse, R = 100, Standard): median error 1.6–4.3 %, max 21 % for the
  shallowest falloff (p = 1) in the far tail where K is already tiny. Energy is 1.0000 in every case.

## Lens Iris
Rings at `r_i = R (i/N)^g`, `g = 1 + 2(1 - easing)`, weighted `1/(r_i^2 + a^2)` so the density follows the same
inverse-square law as the fitted kernel, each softened by `sigma = max(dr/3, 0.5)` and drawn with the aperture's
outline (`blades` sides, `rotation`, stretched by Aspect Ratio). Every ring goes on the coarsest pyramid level
that still holds it within the sampling quality's kernel half-width (5 / 7 / 11 level pixels), and the whole bank
is normalised to unit energy per channel.

**Known limit:** the blade shape is only resolved while the blade bulge spans more than a level pixel. The
aperture reads clearly over most of the glow at Standard and High; at Low, and in the far tail of very large
radii, it fades to a circle. Making it exact everywhere needs a kernel half-width of ~10 level pixels at every
level, which costs 16x per level finer — a GPU job, not a scalar wasm one. Glow Aberration shifts the ring radii
per channel; Pixel Aberration works as usual.

## Sliders (flat idx = position in the params array; colours take 4 slots)
| param | range / default | mapping |
|---|---|---|
| Threshold (0) | 0–400 %, 35 | 100 % = luma 1.0; > 100 % isolates HDR super-whites. Quadratic soft knee, keeps hue |
| Threshold Softness (1) | 0–100 %, 50 | knee = softness x (thr + 0.05): never zero, so no stair-stepping |
| Radius (2) | 1–5000 px, 300 | pixels x host resolution scale; the outer extent R of the kernel |
| Exposure (3) / Intensity (4) | stops / % | gain = intensity/100 x 2^exposure (the kernel is energy-normalised) |
| Glow Mode (5) | Inverse Square / Exponential / Lens Iris | |
| Falloff (6) | 0.25–4, 2.0 | exponent p of the inverse-power law |
| Scene Gamma (7) | 0.2–5, 1.0 | threshold in display space, blur in linear, convert back; 1.0 = off |
| Glow Radius R/G/B (8–10) | 0.01–8, 1.0 | per-channel kernel width |
| Aspect Ratio (11) | 0.1–10, 1.0 | kernel w/h, area preserving (x radius `R*sqrt(a)`, y radius `R/sqrt(a)`) |
| Chromatic Aberration (12) | 0–100 % | eased `c^1.25`: radius split `2^(+-1.5 c^1.25)`, lateral `+-5 % c^1.25` |
| Tint Amount / Low / Mid / High (13–25) | | the 3-tone tint, used when Tint Style = 3-Tone |
| Lens Dirt Amount / Scale / Contrast / Seed (26–29) | | procedural dirt in the glare |
| Source Opacity (30) | 0–100 %, 100 | |
| Blend Mode (31) | 8 modes | Screen is HDR-safe: `s + g(1 - clamp(s))` |
| Tone Mapping (32) / Mix (37) | 6 curves | hue preserving: RGB scaled by `TM(max)/max` |
| Quality (33) | Draft..Extreme, Custom | base pyramid level 2/1/0/0/0/0, Gaussians per octave 1/1/2/3/4/4 |
| Glow Only (34) | checkbox | legacy; View supersedes it |
| Threshold Mode (35) | Luminance / Chrominance | Chrominance = max-channel test |
| Aberration Mode (36) | Radius / Pixel / Both | |
| Dither (38) | 0–100, 100 | TPDF, mono, 1/255 step in a gamma-2.2 display, glow only |
| **Saturation Bias (39)** | 0–100 %, 0 | 100 % drops unsaturated light from the glow entirely |
| **Enable Angle (40) / Angle (41)** | checkbox / ±360°, 0 | turns the stretched kernel; ignored when Aspect Ratio is 1 |
| **View (42)** | Composite / Glow Only / Threshold Matte | |
| **Unmult (43)** | checkbox, on | on: the glow contributes alpha. off: the source alpha is kept |
| **Pixel Aberration Channels (44)** | 6 pairings | the first channel moves out, the second moves in |
| **Glow Aberration Order (45)** | RGB..BGR | permutes which radius multiplier drives which channel |
| **Tint Style (46)** | 3-Tone / Inner-Outer | |
| **Tint Blend (47) / Inner (48) / Outer (52) / Strength (56)** | Multiply, Normal, Add, Screen, Overlay, Soft Light | inner where the glow is bright, outer where it is dim |
| **Custom Downsampling (57) / Steps Multiplier (58) / Iterations (59)** | 0–6 / 1–8 / 1–15 | active when Quality = Custom |
| **Half Float (60)** | checkbox, off | rounds the glow to IEEE half precision |
| **Iris Blades (61)** | 0–16, 6 | 0 or <3 = circular aperture |
| **Iris Rotation (62) / Iterations (63) / Radii Easing (64) / Sampling Quality (65)** | | Lens Iris only |
| **Dirt Inherit Glow Colour (66) / Matte Exposure (67) / Matte Gamma (68)** | 0–100 % / ±6 / 0.1–6 | |
| **Dither Monochromatic (69)** | checkbox, on | off = independent noise per channel |
| **Iris Opacity (70) / Iris Aspect Ratio (71) / Iris Continuous Rasterization (72)** | 0–100 % / 0.1–10 / checkbox | Lens Iris only; see 4.1 notes |

## Cost (node, scalar wasm, 1080p neon scene)
Draft 0.37 s, Medium 0.76 s, Standard 0.79 s (default 1080p run in 4.1: 0.88 s). Aspect 4 + Angle 30: 1.9 s (the rotated canvas is larger).
Lens Iris: Low 1.36 s, Standard 1.42 s, High 4.8 s. 4K Medium: 1.7 s, ~560 MB wasm memory
(v3: 2.9 s, 470 MB). SIMD gave no measurable gain, so none is shipped.

## Build / test
    python3 gen_manifest.py     # manifest.json -> manifest.h
    ./build.sh                  # clang --target=wasm32, freestanding, no libc
    node test/v4_test.mjs                       # 97 behavioural checks of the v4 engine
    (cd test/hostsim && cargo build --release) && python3 test/host_fuzz.py test/hostsim/target/release/hostsim ec_deep_glow_2.wasm
    python3 test/native_fuzz.py                 # needs gcc with ASan/UBSan
    cd test && python3 kern_test.py && python3 unit_test.py && python3 fuzz.py \
            && python3 fuzz_v4.py && python3 continuity.py

`v4_test.mjs` covers energy conservation, radial falloff and roundness, aspect width/height and decay, the angle
at 0/30/45/60/90°, threshold and saturation bias, the three views, unmult, exposure linearity, glow aberration,
the iris shape against a circular aperture, custom quality, half float, tint, tone mapping, every blend mode,
the 4.1 controls (Overlay / Soft Light tints, per-channel dither, iris opacity / aspect / stepped rasterization), and
degenerate sizes. `fuzz_v4.py` throws 70 random parameter sets at sizes from 1x1 up, at scales 0.25–2, and
checks for traps, NaN, Inf and negatives.

## How close to Deep Glow 2 is this?
Not proven 1:1. The controls now cover what the public documentation lists (within API v1), but the kernel constants,
the Exposure and Threshold curves and the popup lists were never compared with real Deep Glow 2 output. `deepglow_calibration.zip`
holds the black-box way to close that: test frames, an After Effects run list, and an analyser that measures renders and fits
the constants. Note the plug-in's **Inverse Square** mode is the one that corresponds to Deep Glow 2's default glow.
