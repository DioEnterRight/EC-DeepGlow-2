# EffectCraft Plugins

Free, open glow effect for [EffectCraft](https://getartcraft.com/apps/effectcraft), inspired by the After Effects plugin *Deep Glow 2*.

**EC Deep Glow 2** makes bright parts of your picture glow like real light: a hot core, a long soft halo, in HDR (values brighter than white keep glowing instead of clipping).

> **Honest status:** every control Deep Glow 2 documents is here, except the few listed under "Not yet". But nobody has compared our numbers with real Deep Glow 2 renders yet, so it is **not proven 1:1**. The plan to get there is below.

---

## Install (2 minutes)

1. Download **[`plugins/ec-deep-glow-2/ec_deep_glow_2.wasm`](plugins/ec-deep-glow-2/ec_deep_glow_2.wasm)** (click the file, then the download button).
2. Put it where EffectCraft loads plugins. If you have an older `deep_glow_open_*.wasm`, delete it so you don't see two glows.
3. Restart EffectCraft. Find **EC Deep Glow 2** under **Stylize**.

That's all. Nothing to build.

---

## What's in this repo

| Folder | What it is | Do you need it? |
|---|---|---|
| `plugins/ec-deep-glow-2/` | The plugin (`.wasm`) and its source code | **Yes**, this is the effect |
| `tools/deepglow-calibration/` | Test images and a measuring script that fit the effect to real Deep Glow 2 renders | Only to help make it 1:1 |
| `builtin/deepglow/` | Rust version of the same effect, for people who build EffectCraft from source | No, the launcher app can't use it |

---

## Features: what works and how close it is

**Legend**
- **Solid**: works as documented and rests on plain maths, so it should already behave like the real thing.
- **Close**: has every documented control, but the exact numbers are our best estimate.
- **Not yet**: missing or limited.

### Solid
| Feature | Notes |
|---|---|
| Keeps all the light | The glow only spreads light around, it never adds or loses any (checked by tests) |
| HDR-safe | Super-bright pixels keep glowing; nothing clips at white |
| Aspect Ratio + Angle | Stretch the glow wide or tall and rotate it to any angle, smoothly |
| Source Opacity, Glow Only view, Unmult | Glow-only and threshold-matte views, alpha handling |
| 8 blend modes | Screen (HDR-safe), Add, Normal, Lighten, Color Dodge, Linear Dodge, Overlay, Soft Light |
| Never crashes | Tested with random and extreme settings, 4K frames, and memory checks |

### Close (all controls present, numbers still estimated)
| Feature | What you get |
|---|---|
| Radius, Exposure, Intensity | Glow size and strength. Exposure works in stops |
| Glow shape (Falloff) | Inverse-square halo with a bright core, like real light |
| Threshold, Smooth, Mode | Which pixels glow. Luminance or Chrominance, soft knee |
| Saturation Bias | Colourful light glows, grey light doesn't |
| Chromatic Aberration | Colour fringing: per-channel radius, pixel shift, 6 channel pairings, channel order |
| Tint | 3-tone or Inner/Outer, 6 blend modes (Multiply, Normal, Add, Screen, Overlay, Soft Light) |
| Tone Mapping | 6 curves, hue-preserving |
| Dither | Removes banding, with a Monochromatic switch |
| Quality | Draft to Extreme, plus Custom (downsampling, steps, iterations) |
| Lens Iris | Glow shaped like a camera aperture: blades, rotation, easing, opacity, aspect, smooth or stepped edge |
| Lens Dirt | Dirty-lens look: amount, scale, contrast, matte exposure and gamma |

### Not yet
| Missing | Why |
|---|---|
| **Exact match to real Deep Glow 2 numbers** | Needs real Deep Glow 2 renders to measure (see below) |
| Mask, Iris and Lens Dirt as **picture layers** | EffectCraft plugin API v1 can't take layer inputs. Iris and dirt are drawn by the plugin instead. (The built-in version does take layers.) |
| Buffer Expansion, grouped panels, buttons | Same API limit |
| Popup lists in the real UI | We only know the names the docs give; the rest are our own |
| Speed | CPU only. About 1 second for HD, 6-8 seconds for 4K with stretch + angle |
| Frames bigger than 4K with a rotated glow | Needs more than the 1 GB the plugin sandbox allows |

---

## Will it become 1:1?

That's the plan, and here is exactly what's needed:

1. Someone with **After Effects and Deep Glow 2** renders a handful of simple test frames. Everything is ready in `tools/deepglow-calibration/` with step-by-step instructions, and the first group of runs (P1) is enough to start.
2. The included script **measures** those renders (glow shape, brightness vs Exposure, threshold curve) and fits our numbers to them.
3. We update the plugin and re-check side by side.

We never open, copy or decompile Deep Glow 2. Only pictures it produces are measured. Please check Deep Glow 2's licence before rendering.

When the real renders arrive, the numbers can be matched. Until then, treat "Close" as "right idea, estimated numbers".

---

## Build it yourself

```
cd plugins/ec-deep-glow-2
./build.sh        # needs clang with wasm32 support
```

Tests: `node test/v4_test.mjs ec_deep_glow_2.wasm` (97 checks). See the plugin's own README for the rest.

## License

No license file yet. Until one is added, the code is all rights reserved. Open an issue if you'd like to reuse it.

## Credits

Clean-room: built only from public documentation and observed behaviour. Not affiliated with Deep Glow 2 or its makers.
