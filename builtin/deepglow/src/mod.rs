//! Exponential Glow: a physically motivated glow with the control set of Deep Glow 2.
//!
//! (Deep Glow is a trademark of its vendor, so the effect is registered under a generic name;
//! `lookup("Deep Glow 2")` finds it through [`ALIASES`].)
//!
//! Pipeline, in the order the controls are grouped:
//!
//! 1. **Input**: the glow source is the layer's colour times the share of its brightness that
//!    passes Threshold (soft knee = Threshold Smooth), biased towards saturated pixels
//!    (Saturation Bias) and cut by the Mask Layer. The brightness measure is read from the
//!    colours as they are in the working space (so 50 % is mid grey), then the colours are
//!    linearised (Gamma Correction) because light adds in linear space.
//! 2. **Glow** (Main): *Exponential* spreads the source with an inverse-square falloff: a
//!    ladder of Gaussians on an image pyramid, weighted by a fitted kernel ([`kernel`]), exactly
//!    energy preserving, so Radius moves light around and Exposure alone sets how much of it
//!    there is. Aspect Ratio and Angle stretch the spread; Glow Aberration gives each colour
//!    channel its own radius. *Lens Iris* instead spreads it in the shape of the Iris Layer
//!    ([`iris`]).
//! 3. **Style**: Pixel Aberration, Lens Dirt, Tint, Tone Mapping ([`post`], [`color`]).
//! 4. **Output**: Blend Mode, Source Opacity, Unmult, Dither and the View.
//!
//! Quality chooses the pyramid depth and the number of Gaussians per octave; Half Float rounds
//! the glow buffer to 16-bit precision; Buffer Expansion grows the layer for the glow that
//! leaves it.

mod color;
mod iris;
mod kernel;
mod post;
mod resample;

use effectcraft_color::BlendMode;
use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_raster::Image;
use rayon::prelude::*;

use self::color::{TONE_OPS, half_round, luma, saturation, threshold_share, tone_map};
use self::kernel::{axis_plan, fit_weights, min_sigma, sigma_ladder};
use self::post::{
    Compose, Dirt, Dither, PIXEL_CHANNELS, TINT_BLENDS, Tint, apply_dirt, apply_tint, compose, dirt_matte, dither_glow, encode_premul, pixel_aberration,
};
use self::resample::{Gauss2, pyramid_sum, pyramid2d_sum, rotate};
use crate::{Buf, EffectCtx, EffectSpec, LAYER_SOURCE_OPTIONS, col, num, p, popup, slider};

/// Other names the effect answers to in [`crate::lookup`].
pub const ALIASES: &[&str] = &["Deep Glow 2", "Deep Glow"];

/// Display names of the twirl-down groups (by match id).
pub const GROUPS: &[(&str, &str)] = &[
    ("main", "Main"),
    ("input", "Input"),
    ("iris", "Iris"),
    ("style", "Style"),
    ("gamma", "Gamma Correction"),
    ("aspect", "Aspect Ratio"),
    ("aberration", "Chromatic Aberration"),
    ("tint", "Tint"),
    ("toneMapping", "Tone Mapping"),
    ("lensDirt", "Lens Dirt Texture"),
    ("quality", "Quality"),
    ("output", "Output"),
];

const BLEND_MODES: [&str; 10] = ["Screen", "Add", "Normal", "Multiply", "Overlay", "Soft Light", "Hard Light", "Color Dodge", "Lighten", "Difference"];
const QUALITY_PRESETS: [&str; 7] = ["Draft", "Low", "Medium", "High", "Very High", "Ultra", "Custom"];
/// Index of "Custom" in [`QUALITY_PRESETS`].
const CUSTOM_QUALITY: u32 = 6;
/// Pyramid base level and Gaussians per octave of the six presets.
const PRESET_BASE: [usize; 6] = [2, 1, 0, 0, 0, 0];
const PRESET_STEPS: [usize; 6] = [1, 1, 2, 3, 4, 4];
const CHANNEL_ORDERS: [&str; 6] = ["R G B", "R B G", "G R B", "G B R", "B R G", "B G R"];
/// Channel `c` of the glow takes the multiplier `ORDER[o][c]`.
const ORDER: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
/// The narrowest spread, relative to the widest, that Aspect Ratio allows.
const MIN_SPREAD: f32 = 1.0 / 64.0;

fn pct() -> ParamUi {
    slider(0.0, 100.0, 0.0, 100.0, 1)
}

fn layer_source() -> ParamUi {
    popup(&LAYER_SOURCE_OPTIONS)
}

fn params() -> Vec<crate::ParamSpec> {
    let layer = |id, name| p(id, name, Value::Layer(None), ParamUi::Layer);
    vec![
        // Main
        p("main/glowMode", "Glow Mode", Value::Enum(0), popup(&["Exponential", "Lens Iris"])),
        p("main/blendMode", "Blend Mode", Value::Enum(0), popup(&BLEND_MODES)),
        p("main/radius", "Radius", num(500.0), slider(0.0, 10000.0, 0.0, 2000.0, 2)),
        p("main/exposure", "Exposure", num(1.0), slider(0.0, 100.0, 0.0, 5.0, 2)),
        // Input
        p("input/threshold", "Threshold", num(0.0), slider(0.0, 1000.0, 0.0, 100.0, 2)),
        p("input/thresholdMode", "Threshold Mode", Value::Enum(0), popup(&["Luminance", "Chrominance"])),
        p("input/thresholdSmooth", "Threshold Smooth", num(0.0), pct()),
        p("input/saturationBias", "Saturation Bias", num(0.0), pct()),
        layer("input/maskLayer", "Mask Layer"),
        p("input/maskLayerSource", "Mask Layer Source", Value::Enum(0), layer_source()),
        p("input/maskMode", "Mask Mode", Value::Enum(0), popup(&["None", "Alpha", "Luminance"])),
        p("input/maskInvert", "Mask Invert", Value::Bool(false), ParamUi::Checkbox),
        // Iris
        layer("iris/irisLayer", "Iris Layer"),
        p("iris/irisLayerSource", "Iris Layer Source", Value::Enum(0), layer_source()),
        p("iris/samplingQuality", "Iris Sampling Quality", Value::Enum(1), popup(&iris::SAMPLING)),
        p("iris/iterations", "Iris Iterations", num(10.0), slider(1.0, 64.0, 1.0, 32.0, 0)),
        p("iris/radiiEasing", "Radii Easing", num(90.0), pct()),
        p("iris/opacity", "Iris Opacity", num(100.0), pct()),
        p("iris/aspectRatio", "Iris Aspect Ratio", num(1.0), slider(0.0, 2.0, 0.0, 2.0, 2)),
        p("iris/radiusNormalization", "Radius Normalization", Value::Bool(false), ParamUi::Checkbox),
        p("iris/continuousRasterization", "Continuous Rasterization", Value::Bool(true), ParamUi::Checkbox),
        // Style: Gamma Correction
        p("style/gamma/autoDetect", "Auto Detect Gamma", Value::Bool(true), ParamUi::Checkbox),
        p("style/gamma/sceneGamma", "Scene Gamma", num(2.2), slider(0.1, 5.0, 0.5, 3.0, 2)),
        // Style: Aspect Ratio
        p("style/aspect/ratio", "Aspect Ratio", num(1.0), slider(0.0, 2.0, 0.0, 2.0, 2)),
        p("style/aspect/enableAngle", "Enable Angle", Value::Bool(false), ParamUi::Checkbox),
        p("style/aspect/angle", "Angle", num(0.0), ParamUi::Angle),
        // Style: Chromatic Aberration
        p("style/aberration/pixelEnable", "Enable Pixel Aberration", Value::Bool(false), ParamUi::Checkbox),
        p("style/aberration/pixelChannels", "Aberration Channels", Value::Enum(0), popup(&PIXEL_CHANNELS)),
        p("style/aberration/pixelOffset", "Pixel Offset", num(1.0), slider(0.0, 1000.0, 0.0, 50.0, 2)),
        p("style/aberration/glowEnable", "Enable Glow Aberration", Value::Bool(false), ParamUi::Checkbox),
        p("style/aberration/multiplyRed", "Multiply Red", num(80.0), slider(0.0, 1000.0, 0.0, 300.0, 2)),
        p("style/aberration/multiplyGreen", "Multiply Green", num(100.0), slider(0.0, 1000.0, 0.0, 300.0, 2)),
        p("style/aberration/multiplyBlue", "Multiply Blue", num(120.0), slider(0.0, 1000.0, 0.0, 300.0, 2)),
        p("style/aberration/channelOrder", "Shuffle Channels", Value::Enum(0), popup(&CHANNEL_ORDERS)),
        // Style: Tint
        p("style/tint/mode", "Tint Mode", Value::Enum(0), popup(&["Off", "Single Color", "Inner & Outer"])),
        p("style/tint/blend", "Tint Blend Mode", Value::Enum(0), popup(&TINT_BLENDS)),
        p("style/tint/inner", "Color Inner", col(0.2, 0.6, 1.0), ParamUi::Color),
        p("style/tint/outer", "Color Outer", col(0.7, 0.2, 1.0), ParamUi::Color),
        p("style/tint/swap", "Swap Colors", Value::Bool(false), ParamUi::Checkbox),
        p("style/tint/strength", "Tint Strength", num(100.0), pct()),
        // Style: Tone Mapping
        p("style/toneMapping/operation", "Operation", Value::Enum(0), popup(&TONE_OPS)),
        p("style/toneMapping/mix", "Mix", num(100.0), pct()),
        // Style: Lens Dirt Texture
        layer("style/lensDirt/texture", "Texture"),
        p("style/lensDirt/textureSource", "Texture Layer Source", Value::Enum(0), layer_source()),
        p("style/lensDirt/opacity", "Opacity", num(100.0), pct()),
        p("style/lensDirt/inheritGlowColor", "Inherit Glow Color", num(100.0), pct()),
        p("style/lensDirt/matteExposure", "Matte Exposure", num(0.0), slider(-10.0, 10.0, -5.0, 5.0, 2)),
        p("style/lensDirt/matteGamma", "Matte Gamma", num(1.0), slider(0.1, 5.0, 0.1, 3.0, 2)),
        p("style/lensDirt/repeatEdges", "Repeat Edges", Value::Bool(true), ParamUi::Checkbox),
        // Quality
        p("quality/preset", "Quality", Value::Enum(2), popup(&QUALITY_PRESETS)),
        p("quality/downsampling", "Downsampling", num(0.0), slider(0.0, 6.0, 0.0, 6.0, 0)),
        p("quality/stepsMultiplier", "Glow Steps Multiplier", num(2.0), slider(1.0, 8.0, 1.0, 8.0, 0)),
        p("quality/iterations", "Iterations", num(12.0), slider(1.0, 15.0, 1.0, 15.0, 0)),
        p("quality/halfFloat", "Half Float", Value::Bool(true), ParamUi::Checkbox),
        p("quality/bufferExpansionMode", "Buffer Expansion Mode", Value::Enum(0), popup(&["Auto", "Manual", "Off"])),
        p("quality/auxBufferExpansion", "Aux Buffer Expansion", num(0.0), slider(0.0, 4000.0, 0.0, 1000.0, 0)),
        // Output
        p("output/view", "View", Value::Enum(0), popup(&["Final Result", "Glow Input", "Glow Only"])),
        p("output/sourceOpacity", "Source Opacity", num(100.0), pct()),
        p("output/unmult", "Unmult (Required for Alpha)", Value::Bool(true), ParamUi::Checkbox),
        p("output/dither", "Dither", Value::Bool(true), ParamUi::Checkbox),
        p("output/ditherMono", "Monochromatic", Value::Bool(true), ParamUi::Checkbox),
    ]
}

pub fn specs() -> Vec<EffectSpec> {
    vec![EffectSpec { id: "ec.stylize.deepglow", name: "Exponential Glow", category: "Stylize", params: params(), render, gpu: false, float: true }]
}

/// Whether Effect Controls shows `param` (a spec id path, `group/param`) given the instance's
/// popups (`e`) and checkboxes (`b`): Iris controls only in Lens Iris mode, per-channel glow
/// radii only in Exponential mode, tint colours only when tinting, and so on.
pub fn shown(param: &str, e: &dyn Fn(&str) -> u32, b: &dyn Fn(&str) -> bool) -> bool {
    let iris = e("main/glowMode") == 1;
    let ab = "style/aberration/";
    match param {
        "iris" => iris,
        _ if param.starts_with("iris/") => iris,
        "style/aberration/glowEnable" => !iris,
        _ if param.starts_with(ab) && ["multiply", "channelOrder"].iter().any(|k| param[ab.len()..].starts_with(k)) => {
            !iris && b("style/aberration/glowEnable")
        }
        "style/aberration/pixelChannels" | "style/aberration/pixelOffset" => b("style/aberration/pixelEnable"),
        "style/aspect/angle" => b("style/aspect/enableAngle"),
        "output/ditherMono" => b("output/dither"),
        "style/gamma/sceneGamma" => !b("style/gamma/autoDetect"),
        "style/tint/blend" | "style/tint/inner" | "style/tint/strength" => e("style/tint/mode") != 0,
        "style/tint/outer" | "style/tint/swap" => e("style/tint/mode") == 2,
        "style/toneMapping/mix" => e("style/toneMapping/operation") != 0,
        "quality/downsampling" | "quality/stepsMultiplier" | "quality/iterations" => e("quality/preset") == CUSTOM_QUALITY,
        "quality/auxBufferExpansion" => e("quality/bufferExpansionMode") != 2,
        _ => true,
    }
}

/// Everything the controls say, read once.
struct Cfg {
    iris: bool,
    blend: u32,
    radius: f64,
    exposure: f32,
    thr: f32,
    thr_max: bool,
    knee: f32,
    sat_bias: f32,
    mask_mode: u32,
    mask_invert: bool,
    gamma: f32,
    ratio: f32,
    angle: Option<f32>,
    pixel_ab: Option<(u32, f32)>,
    glow_ab: Option<[f32; 3]>,
    tint: Tint,
    tone_op: u32,
    tone_mix: f32,
    dirt: Dirt,
    base: usize,
    steps: usize,
    max_level: usize,
    half: bool,
    expand: u32,
    aux: f64,
    view: u32,
    src_opacity: f32,
    unmult: bool,
    dither: Dither,
    iris_normalize: bool,
    iris_cfg: iris::IrisCfg,
}

fn gamma_of(ctx: &EffectCtx) -> f32 {
    let pr = ctx.params;
    let g = if pr.b("style/gamma/autoDetect") { if ctx.env.working_linear { 1.0 } else { 2.2 } } else { pr.f("style/gamma/sceneGamma") as f32 };
    g.clamp(0.1, 5.0)
}

fn linear_color(ctx: &EffectCtx, id: &str, gamma: f32) -> [f32; 3] {
    let c = ctx.params.color(id);
    [0, 1, 2].map(|i| c[i].max(0.0).powf(gamma))
}

/// Directional spread `(x, y)` of Aspect Ratio: 0 spreads vertically only, 1 equally, 2
/// horizontally only (the widest axis is always 1).
fn spread_of(ratio: f32) -> [f32; 2] {
    let r = if ratio.is_finite() { ratio.clamp(0.0, 2.0) } else { 1.0 };
    [r.clamp(MIN_SPREAD, 1.0), (2.0 - r).clamp(MIN_SPREAD, 1.0)]
}

/// Spread of the Lens Iris aperture: Aspect Ratio (the whole glow) times Iris Aspect Ratio (the
/// aperture alone), scaled so the widest axis is 1.
fn iris_spread(ratio: f32, iris_ratio: f32) -> [f32; 2] {
    let (a, b) = (spread_of(ratio), spread_of(iris_ratio));
    let (x, y) = (a[0] * b[0], a[1] * b[1]);
    let m = x.max(y).max(1e-6);
    [(x / m).max(MIN_SPREAD), (y / m).max(MIN_SPREAD)]
}

/// Dither noise pattern of this frame of this instance.
fn dither_seed(ctx: &EffectCtx) -> u32 {
    ctx.seed.wrapping_add((ctx.time * 1000.0).round() as i64 as u32)
}

impl Cfg {
    fn read(ctx: &EffectCtx) -> Cfg {
        let pr = ctx.params;
        let gamma = gamma_of(ctx);
        let thr = (pr.f("input/threshold") / 100.0).max(0.0) as f32;
        let smooth = (pr.f("input/thresholdSmooth") / 100.0).clamp(0.0, 1.0) as f32;
        let ratio = pr.f("style/aspect/ratio") as f32;
        let angle = (pr.b("style/aspect/enableAngle") && (ratio - 1.0).abs() > 0.002).then(|| pr.f("style/aspect/angle").to_radians() as f32);
        let mult = |id: &str| (pr.f(id) / 100.0).max(0.0) as f32;
        let mut m = [mult("style/aberration/multiplyRed"), mult("style/aberration/multiplyGreen"), mult("style/aberration/multiplyBlue")];
        let order = ORDER.get(pr.e("style/aberration/channelOrder") as usize).copied().unwrap_or([0, 1, 2]);
        m = [m[order[0]], m[order[1]], m[order[2]]];
        let swap = pr.b("style/tint/swap");
        let (mut inner, mut outer) = (linear_color(ctx, "style/tint/inner", gamma), linear_color(ctx, "style/tint/outer", gamma));
        if swap {
            std::mem::swap(&mut inner, &mut outer);
        }
        let preset = pr.e("quality/preset");
        let (base, steps, max_level) = quality_of(pr, preset);
        let iris_cfg = iris::IrisCfg {
            quality: pr.e("iris/samplingQuality"),
            iterations: pr.f("iris/iterations").round().clamp(1.0, 64.0) as usize,
            easing: (pr.f("iris/radiiEasing") / 100.0).clamp(0.0, 1.0) as f32,
            spread: iris_spread(ratio, pr.f("iris/aspectRatio") as f32),
            angle: angle.unwrap_or(0.0),
            opacity: (pr.f("iris/opacity") / 100.0).clamp(0.0, 1.0) as f32,
            continuous: pr.b("iris/continuousRasterization"),
        };
        Cfg {
            iris: pr.e("main/glowMode") == 1,
            blend: pr.e("main/blendMode"),
            radius: pr.f("main/radius").max(0.0),
            exposure: pr.f("main/exposure").max(0.0) as f32,
            thr,
            thr_max: pr.e("input/thresholdMode") == 1,
            knee: smooth * (thr + 0.05),
            sat_bias: (pr.f("input/saturationBias") / 100.0).clamp(0.0, 1.0) as f32,
            mask_mode: pr.e("input/maskMode"),
            mask_invert: pr.b("input/maskInvert"),
            gamma,
            ratio,
            angle,
            pixel_ab: pr.b("style/aberration/pixelEnable").then(|| (pr.e("style/aberration/pixelChannels"), pr.f("style/aberration/pixelOffset") as f32)),
            glow_ab: pr.b("style/aberration/glowEnable").then_some(m),
            tint: Tint {
                mode: pr.e("style/tint/mode"),
                blend: pr.e("style/tint/blend"),
                inner,
                outer,
                strength: (pr.f("style/tint/strength") / 100.0).clamp(0.0, 1.0) as f32,
            },
            tone_op: pr.e("style/toneMapping/operation"),
            tone_mix: (pr.f("style/toneMapping/mix") / 100.0).clamp(0.0, 1.0) as f32,
            dirt: Dirt {
                opacity: (pr.f("style/lensDirt/opacity") / 100.0).clamp(0.0, 1.0) as f32,
                inherit: (pr.f("style/lensDirt/inheritGlowColor") / 100.0).clamp(0.0, 1.0) as f32,
                exposure: pr.f("style/lensDirt/matteExposure") as f32,
                gamma: pr.f("style/lensDirt/matteGamma") as f32,
                repeat_edges: pr.b("style/lensDirt/repeatEdges"),
            },
            base,
            steps,
            max_level,
            half: pr.b("quality/halfFloat"),
            expand: pr.e("quality/bufferExpansionMode"),
            aux: pr.f("quality/auxBufferExpansion").max(0.0),
            view: pr.e("output/view"),
            src_opacity: (pr.f("output/sourceOpacity") / 100.0).clamp(0.0, 1.0) as f32,
            unmult: pr.b("output/unmult"),
            dither: Dither { on: pr.b("output/dither"), mono: pr.b("output/ditherMono"), gamma, seed: dither_seed(ctx) },
            iris_normalize: pr.b("iris/radiusNormalization"),
            iris_cfg,
        }
    }

    /// Largest radius (px) any channel's glow reaches.
    fn reach(&self, scale: f64) -> f64 {
        let ch = self.glow_ab.map_or(1.0, |m| f64::from(m[0].max(m[1]).max(m[2])));
        let spread = f64::from(spread_of(self.ratio).iter().fold(0.0f32, |a, b| a.max(*b)));
        self.radius * scale * ch * spread
    }

    /// Transparent pixels to add on every side.
    fn expansion(&self, scale: f64) -> u32 {
        let aux = self.aux * scale;
        let want = match self.expand {
            0 => 2.0 * self.reach(scale) + aux + 2.0,
            1 => aux,
            _ => 0.0,
        };
        want.ceil().clamp(0.0, 16384.0) as u32
    }
}

/// `(base pyramid level, Gaussians per octave, deepest level)` of a quality preset (or Custom).
fn quality_of(pr: &crate::Params, preset: u32) -> (usize, usize, usize) {
    if preset >= CUSTOM_QUALITY {
        let base = pr.f("quality/downsampling").round().clamp(0.0, 6.0) as usize;
        let steps = pr.f("quality/stepsMultiplier").round().clamp(1.0, 8.0) as usize;
        return (base, steps, pr.f("quality/iterations").round().clamp(1.0, 15.0) as usize);
    }
    let i = (preset as usize).min(PRESET_BASE.len() - 1);
    (PRESET_BASE[i], PRESET_STEPS[i], 15)
}

/// The Mask Layer's matte over the buffer (`None` when masking is off or no layer is chosen).
fn input_mask(ctx: &EffectCtx, b: &Buf, c: &Cfg) -> Option<Vec<f32>> {
    if c.mask_mode == 0 {
        return None;
    }
    let lp = ctx.layer_param("input/maskLayer", false)?;
    let m = crate::util::fit_layer(ctx, b, &lp, true);
    Some(
        m.data
            .par_iter()
            .map(|px| {
                let v = if c.mask_mode == 1 { px[3] } else { luma([px[0], px[1], px[2]]) };
                let v = v.clamp(0.0, 1.0);
                if c.mask_invert { 1.0 - v } else { v }
            })
            .collect(),
    )
}

fn pow_pos(v: f32, g: f32) -> f32 {
    if (g - 1.0).abs() < 1e-4 { v.max(0.0) } else { v.max(0.0).powf(g) }
}

/// The glow source: linear, premultiplied colour of the part of each pixel that passes the
/// threshold (see the module docs).
fn glow_input(img: &Image, c: &Cfg, mask: Option<&[f32]>) -> Image {
    let mut out = Image::new(img.width, img.height);
    out.data.par_iter_mut().zip(img.data.par_iter()).enumerate().for_each(|(i, (o, s))| {
        let a = s[3].clamp(0.0, 1.0);
        if a <= 1e-6 {
            return;
        }
        let ia = 1.0 / a;
        let col = [s[0] * ia, s[1] * ia, s[2] * ia].map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 });
        let m = if c.thr_max { col[0].max(col[1]).max(col[2]) } else { luma(col) };
        let share = threshold_share(m, c.thr, c.knee).min(1.0);
        let sat = 1.0 - c.sat_bias * (1.0 - saturation(col));
        let k = share * sat * mask.and_then(|m| m.get(i)).copied().unwrap_or(1.0);
        let lin = col.map(|v| pow_pos(v, c.gamma));
        *o = [lin[0] * k * a, lin[1] * k * a, lin[2] * k * a, a * k];
    });
    out
}

/// Pyramid depth limits for an image of `w` × `h`: `(level count L0 with 2^L0 ≥ max side, base)`.
/// Large buffers start higher in the pyramid so memory stays bounded.
fn pyramid_limits(c: &Cfg, w: u32, h: u32) -> (usize, usize) {
    let l0 = (w.max(h).max(1) as f64).log2().ceil() as usize;
    let px = f64::from(w) * f64::from(h);
    let forced = if px > 24.0e6 {
        2
    } else if px > 6.0e6 {
        1
    } else {
        0
    };
    (l0, c.base.max(forced).min(l0))
}

/// Where a Gaussian of sigma `sigma` along one axis goes; past the deepest level allowed the
/// widest Gaussian that level holds is used (the glow is a little narrower than asked, never lost).
fn plan_axis(sigma: f32, base: usize, max_level: usize) -> (usize, f32) {
    axis_plan(sigma, base, max_level).unwrap_or((max_level.max(base), 3.0))
}

/// Exponential glow of the linear premultiplied `input`; each channel has its own radius when
/// Glow Aberration is on. The radial kernel is fitted once by a ladder of Gaussians; Aspect Ratio
/// scales each Gaussian's sigma per axis, and Angle turns the whole frame under the glow.
fn exponential(input: Image, c: &Cfg, scale: f64) -> Image {
    let (w, h) = (input.width, input.height);
    let spread = spread_of(c.ratio);
    let aniso = (spread[0] - spread[1]).abs() > 1e-3;
    let rotated = c.angle.filter(|_| aniso);
    let src = match rotated {
        Some(a) => {
            let (sa, ca) = (f64::from(a).sin().abs(), f64::from(a).cos().abs());
            let cw = (f64::from(w) * ca + f64::from(h) * sa).ceil() as u32 + 2;
            let ch = (f64::from(w) * sa + f64::from(h) * ca).ceil() as u32 + 2;
            rotate(&input, -f64::from(a), cw, ch)
        }
        None => input,
    };
    let mults = c.glow_ab.unwrap_or([1.0; 3]);
    let radii: [f32; 3] = mults.map(|k| ((c.radius * scale) as f32 * k).max(1.0));
    let rmax = radii.iter().fold(0.0f32, |a, b| a.max(*b));
    let (l0, base) = pyramid_limits(c, src.width, src.height);
    let max_level = (l0 + 2).min(c.max_level.min(15)).max(base);
    let sigmas = sigma_ladder(c.steps, min_sigma(base), 1.6 * rmax);
    let weights: Vec<Vec<f32>> = radii.iter().map(|&r| fit_weights(&sigmas, r, (1.5 * scale as f32).max(0.004 * r).max(1.0), 2.0)).collect();
    let terms: Vec<Gauss2> = sigmas
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let (lx, sx) = plan_axis(s * spread[0], base, max_level);
            let (ly, sy) = plan_axis(s * spread[1], base, max_level);
            Gauss2 { lx, ly, sx, sy, w: [weights[0][k], weights[1][k], weights[2][k], weights[1][k]] }
        })
        .collect();
    let glow = pyramid2d_sum(src, &terms);
    match rotated {
        Some(a) => rotate(&glow, f64::from(a), w, h),
        None => glow,
    }
}

/// Lens Iris glow of the linear premultiplied `input`.
fn iris_glow(ctx: &EffectCtx, scale: f64, input: Image, c: &Cfg) -> Image {
    let shape = iris::Shape::from_layer(ctx, c.iris_normalize);
    let (l0, base) = pyramid_limits(c, input.width, input.height);
    let r = (c.radius * scale) as f32;
    let terms = iris::iris_terms(&shape, r.max(1.0), &c.iris_cfg, base.min(l0), (l0 + 2).min(15));
    pyramid_sum(input, base.min(l0), &terms)
}

fn generate(ctx: &EffectCtx, b: &Buf, input: Image, c: &Cfg) -> Image {
    let mut glow = if c.iris { iris_glow(ctx, b.scale, input, c) } else { exponential(input, c, b.scale) };
    if let Some((choice, off)) = c.pixel_ab {
        glow = pixel_aberration(&glow, choice, off * b.scale as f32);
    }
    if c.half {
        glow.data.par_iter_mut().for_each(|p| {
            for v in p.iter_mut() {
                *v = half_round(*v);
            }
        });
    }
    glow
}

/// Blend Mode as the compositor wants it.
fn compose_of(c: &Cfg, opacity: f32) -> Compose {
    let label = BLEND_MODES.get(c.blend as usize).copied().unwrap_or("Screen");
    let mode = BlendMode::ALL.iter().copied().find(|m| m.label() == label).unwrap_or(BlendMode::Normal);
    Compose { mode, screen: c.blend == 0, add: c.blend == 1, opacity, unmult: c.unmult, gamma: c.gamma }
}

/// Style and output: exposure, Lens Dirt, Tint, Tone Mapping, then the composite with the source.
fn finish(ctx: &EffectCtx, b: &mut Buf, glow: &Image, c: &Cfg) {
    let matte = (c.dirt.opacity > 0.0).then(|| ctx.layer_param("style/lensDirt/texture", true)).flatten().map(|t| dirt_matte(ctx, b, &t, &c.dirt));
    let opacity = if c.view == 2 { 0.0 } else { c.src_opacity };
    let cmp = compose_of(c, opacity);
    let width = (b.img.width as usize).max(1);
    b.img.data.par_iter_mut().zip(glow.data.par_iter()).enumerate().for_each(|(i, (o, g))| {
        let mut gl = [g[0].max(0.0) * c.exposure, g[1].max(0.0) * c.exposure, g[2].max(0.0) * c.exposure];
        if let Some(m) = matte.as_ref().and_then(|m| m.get(i)) {
            gl = apply_dirt(gl, *m, &c.dirt);
        }
        gl = apply_tint(gl, &c.tint);
        gl = tone_map(gl, c.tone_op, c.tone_mix);
        gl = dither_glow(gl, (i % width) as u32, (i / width) as u32, &c.dither);
        *o = compose(*o, gl, &cmp);
    });
}

fn render(ctx: &EffectCtx, mut b: Buf) -> Buf {
    let c = Cfg::read(ctx);
    let pad = c.expansion(b.scale);
    if pad > 0 && !ctx.adjustment {
        b.pad(pad);
    }
    let mask = input_mask(ctx, &b, &c);
    let input = glow_input(&b.img, &c, mask.as_deref());
    if c.view == 1 {
        b.img.data.par_iter_mut().zip(input.data.par_iter()).for_each(|(o, i)| *o = encode_premul(*i, c.gamma));
        return b;
    }
    let glow = generate(ctx, &b, input, &c);
    finish(ctx, &mut b, &glow, &c);
    b
}

#[cfg(test)]
mod tests;
