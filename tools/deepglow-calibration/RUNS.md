# Deep Glow 2 measurement runs

32 bpc project. Run once with Project Settings > Linearize Working Space ON (primary) and, for the runs marked `both`, once OFF with the same settings (name the output `<run>_nolin`). Comp 1920x1080, source layer = the frame, comp-sized, Deep Glow 2 applied. Defaults except what the run says. Source Opacity 0 (glow only), Unmult OFF, Dither OFF, Tone Mapping None/off, Tint off, Aspect Ratio 1 (no angle), Threshold 0, Glow Mode Exponential, Quality = the default preset unless stated.

Render each run to 32-bit float EXR (or TIFF) named `<run id>_#####.exr` (or just `<run id>.exr`) into `renders/`.
Do the P1 runs first: they are enough to fit the kernel, the Exposure law and the Threshold curve.

| P | run id | source frame | settings | what it tells us |
|---|---|---|---|---|
| P1 | `radius_10` | `imp_V100` | Radius=10 | Radius sweep (impulse, V=100). |
| P1 | `radius_25` | `imp_V100` | Radius=25 | Radius sweep (impulse, V=100). |
| P1 | `radius_50` | `imp_V100` | Radius=50 | Radius sweep (impulse, V=100). |
| P1 | `radius_100` | `imp_V100` | Radius=100 | Radius sweep (impulse, V=100). |
| P1 | `radius_200` | `imp_V100` | Radius=200 | Radius sweep (impulse, V=100). |
| P1 | `radius_400` | `imp_V100` | Radius=400 | Radius sweep (impulse, V=100). |
| P1 | `radius_800` | `imp_V100` | Radius=800 | Radius sweep (impulse, V=100). |
| P1 | `exposure_0.25` | `imp_V100` | Radius=100, Exposure=0.25 | Exposure sweep at Radius 100. |
| P1 | `exposure_0.5` | `imp_V100` | Radius=100, Exposure=0.5 | Exposure sweep at Radius 100. |
| P1 | `exposure_1` | `imp_V100` | Radius=100, Exposure=1 | Exposure sweep at Radius 100. |
| P1 | `exposure_2` | `imp_V100` | Radius=100, Exposure=2 | Exposure sweep at Radius 100. |
| P1 | `exposure_4` | `imp_V100` | Radius=100, Exposure=4 | Exposure sweep at Radius 100. |
| P1 | `exposure_8` | `imp_V100` | Radius=100, Exposure=8 | Exposure sweep at Radius 100. |
| P1 | `amp_V1` | `imp_V1` | Radius=100 | Linearity in input amplitude, Radius 100. |
| P1 | `amp_V10` | `imp_V10` | Radius=100 | Linearity in input amplitude, Radius 100. |
| P1 | `amp_V100` | `imp_V100` | Radius=100 | Linearity in input amplitude, Radius 100. |
| P2 | `phase_int` | `imp_V10` | Radius=50 | Grid-phase test, integer position, Radius 50. |
| P2 | `phase_half` | `imp_V10_half2px` | Radius=50 | Grid-phase test, +0.5 px, Radius 50. |
| P2 | `edge_100` | `edge_V2` | Radius=100 | Half-plane: edge spread function. |
| P2 | `edge_400` | `edge_V2` | Radius=400 | Half-plane: edge spread function. |
| P2 | `disc_100` | `disc_r100_V4` | Radius=100 | Large disc, Radius 100. |
| P1 | `thr_0` | `patches` | Threshold=0, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P1 | `thr_10` | `patches` | Threshold=10, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P1 | `thr_25` | `patches` | Threshold=25, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P1 | `thr_50` | `patches` | Threshold=50, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P1 | `thr_75` | `patches` | Threshold=75, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P1 | `thr_100` | `patches` | Threshold=100, Radius=minimum | Threshold transfer: set Radius to its MINIMUM, Exposure 1 (or note it), View = final. |
| P2 | `thrsmooth_0` | `patches` | Threshold=50, ThresholdSmooth=0, Radius=minimum | Threshold 50, Threshold Smooth sweep, Radius minimum. |
| P2 | `thrsmooth_50` | `patches` | Threshold=50, ThresholdSmooth=50, Radius=minimum | Threshold 50, Threshold Smooth sweep, Radius minimum. |
| P2 | `thrsmooth_100` | `patches` | Threshold=50, ThresholdSmooth=100, Radius=minimum | Threshold 50, Threshold Smooth sweep, Radius minimum. |
| P2 | `thrmode_lum` | `patches` | Threshold=50, ThresholdMode=Luminance, Radius=minimum | Threshold 50, mode Luminance, Radius minimum. |
| P2 | `thrmode_chroma` | `patches` | Threshold=50, ThresholdMode=Chrominance, Radius=minimum | Threshold 50, mode Chrominance, Radius minimum. |
| P2 | `thr_ramp_50` | `ramp_0_16` | Threshold=50, Radius=minimum | Ramp, Threshold 50, Radius minimum. |
| P2 | `sat_bias_0` | `sat_discs` | Radius=100, SaturationBias=0 | Saturated discs, Radius 100, Saturation Bias sweep. |
| P2 | `sat_bias_100` | `sat_discs` | Radius=100, SaturationBias=100 | Saturated discs, Radius 100, Saturation Bias sweep. |
| P2 | `aspect_0.5` | `imp_V100` | Radius=100, AspectRatio=0.5 | Aspect Ratio sweep, Radius 100. |
| P2 | `aspect_2` | `imp_V100` | Radius=100, AspectRatio=2 | Aspect Ratio sweep, Radius 100. |
| P2 | `aspect_2_angle45` | `imp_V100` | Radius=100, AspectRatio=2, EnableAngle=True, Angle=45 | Aspect 2, Enable Angle, 45 degrees. |
| P2 | `rgb_default` | `imp_V100` | Radius=100, GlowAberration=True | True chromatic aberration (RGB radius multipliers) at their defaults, Radius 100. |
| P3 | `rgb_custom` | `imp_V100` | Radius=100, GlowAberration=True, Multipliers=50/100/200 | RGB multipliers R=50 G=100 B=200, Radius 100. |
| P2 | `quality_draft` | `imp_V100` | Radius=100, Quality=Draft | Quality preset ladder signature (look for ripple), Radius 100. |
| P2 | `quality_low` | `imp_V100` | Radius=100, Quality=Low | Quality preset ladder signature (look for ripple), Radius 100. |
| P2 | `quality_medium` | `imp_V100` | Radius=100, Quality=Medium | Quality preset ladder signature (look for ripple), Radius 100. |
| P2 | `quality_high` | `imp_V100` | Radius=100, Quality=High | Quality preset ladder signature (look for ripple), Radius 100. |
| P2 | `quality_(each` | `imp_V100` | Radius=100, Quality=(each remaining preset in the list) | Quality preset ladder signature (look for ripple), Radius 100. |
| P3 | `custom_d0_s1_i8` | `imp_V100` | Radius=100, Quality=Custom, Downsampling=0, StepsMultiplier=1, Iterations=8 | Custom quality. |
| P3 | `custom_d0_s4_i8` | `imp_V100` | Radius=100, Quality=Custom, Downsampling=0, StepsMultiplier=4, Iterations=8 | Custom quality. |
| P3 | `custom_d2_s2_i8` | `imp_V100` | Radius=100, Quality=Custom, Downsampling=2, StepsMultiplier=2, Iterations=8 | Custom quality. |
| P3 | `custom_d0_s2_i3` | `imp_V100` | Radius=100, Quality=Custom, Downsampling=0, StepsMultiplier=2, Iterations=3 | Custom quality. |
| P3 | `tonemap_each` | `imp_V100` | Radius=100 | One render per Tone Mapping operator (write down the names the popup shows, in order), Radius 100. |
| P3 | `tint_each` | `imp_V100` | Radius=100 | Tint inner=orange outer=blue, one per blend mode (write the popup names), Radius 100. |
| P3 | `dirt_dirt_flat_0.5` | `imp_V100` | Radius=100, LensDirtTexture=dirt_flat_0.5 | Lens Dirt with the `dirt_flat_0.5` frame as the texture layer; then repeat with Matte Exposure +2 and Matte Gamma 2. |
| P3 | `dirt_dirt_checker` | `imp_V100` | Radius=100, LensDirtTexture=dirt_checker | Lens Dirt with the `dirt_checker` frame as the texture layer; then repeat with Matte Exposure +2 and Matte Gamma 2. |
| P2 | `iris_disc_r100` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100 | Glow Mode Iris, Iris layer = iris_disc, Radius 100. |
| P2 | `iris_hex_r100` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_hex, Radius=100 | Glow Mode Iris, Iris layer = iris_hex, Radius 100. |
| P2 | `iris_ring_r100` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_ring, Radius=100 | Glow Mode Iris, Iris layer = iris_ring, Radius 100. |
| P2 | `iris_offdot_r100` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_offdot, Radius=100 | Glow Mode Iris, Iris layer = iris_offdot, Radius 100. |
| P3 | `iris_disc_it1` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, IrisIterations=1 | Iris disc, iterations sweep. |
| P3 | `iris_disc_it4` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, IrisIterations=4 | Iris disc, iterations sweep. |
| P3 | `iris_disc_it16` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, IrisIterations=16 | Iris disc, iterations sweep. |
| P3 | `iris_disc_it64` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, IrisIterations=64 | Iris disc, iterations sweep. |
| P3 | `iris_disc_ease0` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, RadiiEasing=0 | Iris disc, radii easing sweep. |
| P3 | `iris_disc_ease50` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, RadiiEasing=50 | Iris disc, radii easing sweep. |
| P3 | `iris_disc_ease100` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_disc, Radius=100, RadiiEasing=100 | Iris disc, radii easing sweep. |
| P3 | `iris_offdot_norm` | `imp_V100` | GlowMode=Iris, IrisLayer=iris_offdot, Radius=100 | Iris offdot with Radius Normalization ON vs OFF (render both, suffix _on/_off). |
