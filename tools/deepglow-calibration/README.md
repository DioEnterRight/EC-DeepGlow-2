# Deep Glow 2 calibration kit (black-box)

Goal: measure how the real effect behaves from the outside so the open glow can be fitted to it.
Nothing here opens, decompiles or copies the plug-in; it only looks at rendered pixels.

Before you start, check Deep Glow 2's licence terms for anything about benchmarking or output
analysis, and only use renders you are allowed to make.

## 1. Make the frames (already done in this zip)

    python3 make_frames.py --out frames --exr      # 15 frames, runs.json, RUNS.md

`frames/*.exr` (also `.tif`) are 32-bit float. `RUNS.md` lists every run: which frame, which settings, why.

## 2. After Effects setup (once)

1. New project. **Project Settings > Color > Depth: 32 bits per channel (float)**.
2. Run everything with **Linearize Working Space ON** (primary). For the P1 runs, repeat with it OFF and add the suffix `_nolin` to the file name. The difference tells us what space the glow is computed in.
3. Comp 1920 x 1080, 1 fps, 1 frame. Import a frame from `frames/` (EXR), drop it in the comp, make it comp-sized.
4. Apply Deep Glow 2. Leave everything at default **except**, for every run:
   - Glow Mode: the default (the plain exponential-style glow). Quality: default preset unless the run says so.
   - Source Opacity (or equivalent "glow only") so that only the glow is output. If there is no such control, set View to the glow-only/final output and say so in the notes file.
   - Tone Mapping off/none, Dither off, Tint off, Unmult off, Aspect Ratio 1, Angle off, Threshold 0.
   - Then the run's own settings from `RUNS.md`.
5. **Render to 32-bit float OpenEXR** (File > Export > Add to Render Queue, Format OpenEXR, Channels RGBA, Depth Float 32). Name the file `<run id>.exr` (a sequence suffix like `_00000` is fine) and put it in `renders/`.
6. Keep a `notes.txt`: the exact names in every popup you touch (Tone Mapping, Tint Blend, Quality, Threshold Mode...), the DG2 version, the AE version, and anything that looked odd. The popup names are facts about the UI and are as useful as the numbers.

Do the **P1** runs first (radius sweep, exposure sweep, amplitude, and the threshold table). That is enough to
fit the kernel, the Exposure law and the Threshold curve. P2/P3 add anisotropy, grid phase, edge response, saturation bias,
chromatic aberration, quality presets, iris and lens dirt.

For `Radius = minimum` runs set Radius to the lowest value the slider allows; for the others the numbers are pixels as shown in the UI.
If a Radius is larger than the comp can show (the tail is clipped), still render it: `analyze.py` marks those profiles as cropped and does not use them for the kernel constants.

## 3. Analyse

    pip install numpy scipy tifffile OpenEXR
    python3 analyze.py selftest                       # trust the tools first (runs on the open plug-in, needs node)
    python3 analyze.py report  --renders renders --json measured.json   # radial profiles, laws, ladder-weight table
    python3 analyze.py fit-kernel --renders renders   # inverse-power exponent q, core a, Gaussian cutoff c per Radius
    python3 analyze.py compare --renders renders --wasm ec_deep_glow_2.wasm   # real vs open plug-in, side by side

What each answer means:

| Output | Reads as |
|---|---|
| `total_out_per_in` | glow energy per unit of input at this setting (the Exposure/Intensity scale) |
| exposure law | power law with exponent 1 = Exposure is a linear gain; best = exponential = Exposure is in stops |
| `slope@0.5R`, `slope@1.0R` | local log-log slope of the profile; -2 = inverse square; steeper = cutoff |
| `fit-kernel`: q, c, a | tail exponent, Gaussian cutoff in Radii, core size in px |
| self-similarity spread | 0 means one rescaled shape for all Radii; large means the shape changes with Radius |
| threshold table | output vs input on flat patches at minimum Radius: shows the knee, soft knee and any scale |
| `weights_by_radius.json` | non-negative Gaussian-ladder weights per Radius: the table to ship if the closed form does not hold |

## 4. Turn the numbers into the effect

- Kernel: if `q` and `c` are steady across Radii, put them (and the measured `a`) into `kernel.rs` / the plug-in `kern()`; otherwise ship the weight table.
- Exposure: replace the gain law with the measured one.
- Threshold: replace the knee with the measured curve (and fit the soft-knee width).
- Check with `compare`; the target is a profile log-ratio rms under about 0.05 across radii, with the same total.

`runs.json` / `RUNS.md` are generated, so edit `make_frames.py` and rerun it rather than editing them.
