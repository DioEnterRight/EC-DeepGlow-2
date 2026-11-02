//! Behavioural tests of the glow: every control has to change the picture the way it says.

use effectcraft_keyframe::Value;
use effectcraft_raster::Image;

use crate::{Buf, EffectEnv, EffectHost, LayerPixels, run_fx};

const ID: &str = "ec.stylize.deepglow";

fn n(v: f64) -> Value {
    Value::Scalar(v)
}
fn e(v: u32) -> Value {
    Value::Enum(v)
}
fn on(v: bool) -> Value {
    Value::Bool(v)
}

/// Other layers of the test composition: layer `i + 1` is `0[i]`.
struct Layers(Vec<Image>);

impl EffectHost for Layers {
    fn layer(&self, id: u64, _: bool) -> Option<LayerPixels> {
        let img = self.0.get(usize::try_from(id).ok()?.checked_sub(1)?)?;
        Some(LayerPixels { buf: Buf { img: img.clone(), offset: [0.0; 2], scale: 1.0 }, size: [f64::from(img.width), f64::from(img.height)] })
    }
    fn audio(&self, _: u64, _: f64, _: usize, _: u32) -> Option<Vec<f32>> {
        None
    }
}

fn scene(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> Image {
    let mut img = Image::new(w, h);
    for y in 0..h {
        for x in 0..w {
            img.set(x, y, f(x, y));
        }
    }
    img
}

/// Black, opaque, with a square of value `v` (x0..x1, y0..y1).
fn square(w: u32, h: u32, x: (u32, u32), y: (u32, u32), v: f32) -> Image {
    scene(w, h, |px, py| if (x.0..x.1).contains(&px) && (y.0..y.1).contains(&py) { [v, v, v, 1.0] } else { [0.0, 0.0, 0.0, 1.0] })
}

/// The standard test picture: a 6 × 6 white square in the middle of 128 × 128.
fn dot() -> Image {
    square(128, 128, (61, 67), (61, 67), 1.0)
}

/// Run the effect without buffer expansion (so pixel coordinates stay put).
fn go(vals: &[(&str, Value)], img: Image) -> Buf {
    go_env(vals, img, EffectEnv::default())
}

fn go_env(vals: &[(&str, Value)], img: Image, env: EffectEnv) -> Buf {
    // Dither is off unless a test asks for it: it is noise, and most tests measure smooth profiles.
    let mut all: Vec<(&str, Value)> = vec![("quality/bufferExpansionMode", e(2)), ("output/dither", on(false))];
    all.extend_from_slice(vals);
    run_fx(ID, &all, img, 0.0, env)
}

/// Linear light, Add: the output minus the source is the glow itself.
fn linear_add() -> Vec<(&'static str, Value)> {
    vec![("style/gamma/autoDetect", on(false)), ("style/gamma/sceneGamma", n(1.0)), ("main/blendMode", e(1))]
}

fn with(base: Vec<(&'static str, Value)>, extra: &[(&'static str, Value)]) -> Vec<(&'static str, Value)> {
    let mut v = base;
    for (k, x) in extra {
        v.retain(|(bk, _)| bk != k);
        v.push((*k, x.clone()));
    }
    v
}

fn at(b: &Buf, x: u32, y: u32) -> [f32; 4] {
    b.img.get(i64::from(x), i64::from(y))
}

fn total(b: &Buf, c: usize) -> f64 {
    b.img.data.iter().map(|p| f64::from(p[c])).sum()
}

#[test]
fn every_control_of_the_reference_is_registered() {
    let spec = crate::find(ID).expect("registered");
    let want = [
        "main/glowMode",
        "main/blendMode",
        "main/radius",
        "main/exposure",
        "input/threshold",
        "input/thresholdMode",
        "input/thresholdSmooth",
        "input/saturationBias",
        "input/maskLayer",
        "input/maskLayerSource",
        "input/maskMode",
        "input/maskInvert",
        "iris/irisLayer",
        "iris/irisLayerSource",
        "iris/samplingQuality",
        "iris/iterations",
        "iris/radiiEasing",
        "iris/opacity",
        "iris/aspectRatio",
        "iris/radiusNormalization",
        "iris/continuousRasterization",
        "style/gamma/autoDetect",
        "style/gamma/sceneGamma",
        "style/aspect/ratio",
        "style/aspect/enableAngle",
        "style/aspect/angle",
        "style/aberration/pixelEnable",
        "style/aberration/pixelChannels",
        "style/aberration/pixelOffset",
        "style/aberration/glowEnable",
        "style/aberration/multiplyRed",
        "style/aberration/multiplyGreen",
        "style/aberration/multiplyBlue",
        "style/aberration/channelOrder",
        "style/tint/mode",
        "style/tint/blend",
        "style/tint/inner",
        "style/tint/outer",
        "style/tint/swap",
        "style/tint/strength",
        "style/toneMapping/operation",
        "style/toneMapping/mix",
        "style/lensDirt/texture",
        "style/lensDirt/textureSource",
        "style/lensDirt/opacity",
        "style/lensDirt/inheritGlowColor",
        "style/lensDirt/matteExposure",
        "style/lensDirt/matteGamma",
        "style/lensDirt/repeatEdges",
        "quality/preset",
        "quality/downsampling",
        "quality/stepsMultiplier",
        "quality/iterations",
        "quality/halfFloat",
        "quality/bufferExpansionMode",
        "quality/auxBufferExpansion",
        "output/view",
        "output/sourceOpacity",
        "output/unmult",
        "output/dither",
        "output/ditherMono",
    ];
    for id in want {
        assert!(spec.params.iter().any(|p| p.id == id), "missing control {id}");
    }
    assert_eq!(spec.params.len(), want.len(), "a control nothing tests for");
    assert_eq!(crate::lookup("Deep Glow 2").map(|s| s.id), Some(ID));
    assert!(crate::SKIPS_COVER.contains(&ID));
}

#[test]
fn defaults_match_the_reference_panel() {
    let spec = crate::find(ID).expect("registered");
    let d = |id: &str| spec.params.iter().find(|p| p.id == id).map(|p| p.default.clone()).expect(id);
    assert_eq!(d("main/radius"), n(500.0));
    assert_eq!(d("main/exposure"), n(1.0));
    assert_eq!(d("main/blendMode"), e(0));
    assert_eq!(d("style/gamma/sceneGamma"), n(2.2));
    assert_eq!(d("style/gamma/autoDetect"), on(true));
    assert_eq!(d("style/aspect/ratio"), n(1.0));
    assert_eq!(d("style/aberration/multiplyRed"), n(80.0));
    assert_eq!(d("style/aberration/multiplyGreen"), n(100.0));
    assert_eq!(d("style/aberration/multiplyBlue"), n(120.0));
    assert_eq!(d("style/aberration/pixelOffset"), n(1.0));
    assert_eq!(d("style/tint/strength"), n(100.0));
    assert_eq!(d("style/toneMapping/mix"), n(100.0));
    assert_eq!(d("style/lensDirt/opacity"), n(100.0));
    assert_eq!(d("style/lensDirt/repeatEdges"), on(true));
    assert_eq!(d("quality/preset"), e(2));
    assert_eq!(d("quality/halfFloat"), on(true));
    assert_eq!(d("output/sourceOpacity"), n(100.0));
    assert_eq!(d("output/unmult"), on(true));
    assert_eq!(d("iris/iterations"), n(10.0));
    assert_eq!(d("iris/radiiEasing"), n(90.0));
    assert_eq!(d("iris/opacity"), n(100.0));
    assert_eq!(d("iris/aspectRatio"), n(1.0));
    assert_eq!(d("iris/radiusNormalization"), on(false));
    assert_eq!(d("iris/continuousRasterization"), on(true));
    assert_eq!(d("output/dither"), on(true));
    assert_eq!(d("output/ditherMono"), on(true));
}

#[test]
fn a_bright_square_glows_and_the_glow_falls_off() {
    let o = go(&[("main/radius", n(14.0))], dot());
    assert!(at(&o, 64, 64)[0] >= 0.999, "the source stays");
    let (near, mid, far) = (at(&o, 64, 74)[0], at(&o, 64, 84)[0], at(&o, 64, 120)[0]);
    assert!(near > 0.01, "glow next to the square: {near}");
    assert!(near > mid && mid > far, "falloff {near} {mid} {far}");
    // Symmetric around the square.
    assert!((at(&o, 53, 64)[0] - at(&o, 74, 64)[0]).abs() < 0.01 * at(&o, 53, 64)[0].max(0.01));
    assert!((at(&o, 64, 53)[0] - at(&o, 64, 74)[0]).abs() < 0.01 * at(&o, 64, 53)[0].max(0.01));
}

#[test]
fn exposure_zero_is_the_source_and_exposure_is_linear() {
    let o = go(&[("main/exposure", n(0.0)), ("main/radius", n(14.0))], dot());
    assert!(o.img.data.iter().zip(&dot().data).all(|(a, b)| (0..4).all(|c| (a[c] - b[c]).abs() < 1e-5)));
    let base = linear_add();
    let one = go(&with(base.clone(), &[("main/radius", n(14.0)), ("main/exposure", n(1.0))]), dot());
    let two = go(&with(base, &[("main/radius", n(14.0)), ("main/exposure", n(2.0))]), dot());
    for (x, y) in [(64, 72), (64, 80), (50, 64)] {
        let (a, b) = (at(&one, x, y)[0], at(&two, x, y)[0]);
        assert!((b / a - 2.0).abs() < 0.02, "({x},{y}): {a} -> {b}");
    }
}

#[test]
fn radius_moves_light_it_does_not_add_light() {
    let big = square(256, 256, (124, 132), (124, 132), 1.0);
    let base = linear_add();
    let small = go(&with(base.clone(), &[("main/radius", n(10.0))]), big.clone());
    let wide = go(&with(base, &[("main/radius", n(40.0))]), big.clone());
    let src = total(&go(&with(linear_add(), &[("main/exposure", n(0.0))]), big), 0);
    for o in [&small, &wide] {
        let glow = total(o, 0) - src;
        assert!((glow / 64.0 - 1.0).abs() < 0.06, "glow energy {glow} for an 8 x 8 white square");
    }
    assert!(at(&small, 128, 133)[0] > at(&wide, 128, 133)[0] * 1.25, "near the source the small radius is brighter");
    assert!(at(&wide, 128, 190)[0] > at(&small, 128, 190)[0], "far from it the wide one is");
}

#[test]
fn radius_zero_does_not_blow_up() {
    let o = go(&[("main/radius", n(0.0))], dot());
    assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite() && *v >= 0.0)));
}

#[test]
fn animating_the_radius_changes_the_glow_smoothly() {
    let mut worst = 0.0f32;
    let a = go(&[("main/radius", n(20.0))], dot());
    for step in 1..=5 {
        let b = go(&[("main/radius", n(20.0 + 0.2 * f64::from(step)))], dot());
        let d = a.img.data.iter().zip(&b.img.data).map(|(p, q)| (p[0] - q[0]).abs()).fold(0.0, f32::max);
        worst = worst.max(d);
    }
    assert!(worst < 0.02, "a 1 % radius step moved a pixel by {worst}");
}

#[test]
fn threshold_keeps_dim_things_from_glowing() {
    let dim = square(128, 128, (61, 67), (61, 67), 0.4);
    let o = go(&[("input/threshold", n(60.0)), ("main/radius", n(14.0))], dim.clone());
    assert!(o.img.data.iter().zip(&dim.data).all(|(a, b)| (a[0] - b[0]).abs() < 1e-5), "0.4 is under a 60 % threshold");
    let o = go(&[("input/threshold", n(60.0)), ("main/radius", n(14.0))], dot());
    assert!(at(&o, 64, 74)[0] > 0.005, "white passes");
    let all = go(&[("main/radius", n(14.0))], dim.clone());
    assert!(at(&all, 64, 74)[0] > 0.003, "no threshold: grey glows too");
}

#[test]
fn threshold_smooth_lets_values_just_under_the_threshold_through() {
    let s = square(128, 128, (61, 67), (61, 67), 0.55);
    let hard = go(&[("input/threshold", n(60.0)), ("main/radius", n(14.0))], s.clone());
    let soft = go(&[("input/threshold", n(60.0)), ("input/thresholdSmooth", n(100.0)), ("main/radius", n(14.0))], s.clone());
    assert!(at(&hard, 64, 74)[0] < 1e-5);
    assert!(at(&soft, 64, 74)[0] > 1e-4, "{}", at(&soft, 64, 74)[0]);
    // The glow grows monotonically as the square gets brighter: no popping.
    let mut last = 0.0;
    for i in 0..60 {
        let v = 0.015 * i as f32;
        let o = go(&[("input/threshold", n(60.0)), ("input/thresholdSmooth", n(100.0)), ("main/radius", n(14.0))], square(128, 128, (61, 67), (61, 67), v));
        let g = at(&o, 64, 74)[0];
        assert!(g >= last - 1e-6, "{v}: {g} after {last}");
        assert!(g - last < 0.01, "{v}: jumped {last} -> {g}");
        last = g;
    }
}

#[test]
fn threshold_mode_chooses_luminance_or_the_brightest_channel() {
    let blue = scene(128, 128, |x, y| if (61..67).contains(&x) && (61..67).contains(&y) { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] });
    let lum = go(&[("input/threshold", n(50.0)), ("input/thresholdMode", e(0)), ("main/radius", n(14.0))], blue.clone());
    let chroma = go(&[("input/threshold", n(50.0)), ("input/thresholdMode", e(1)), ("main/radius", n(14.0))], blue);
    assert!(at(&lum, 64, 74)[2] < 1e-5, "pure blue is dark in luminance");
    assert!(at(&chroma, 64, 74)[2] > 0.005, "but its brightest channel is full");
}

#[test]
fn hdr_values_pass_a_threshold_over_one() {
    let hot = square(128, 128, (61, 67), (61, 67), 5.0);
    let grey = square(128, 128, (61, 67), (61, 67), 0.9);
    let v = |img: Image| go(&[("input/threshold", n(100.0)), ("main/radius", n(14.0)), ("main/blendMode", e(1))], img);
    assert!(at(&v(hot), 64, 74)[0] > 0.01);
    assert!(at(&v(grey.clone()), 64, 74)[0] < 1e-5);
    // The glow of a 5.0 source passes 1.0 (HDR is not clipped on the way).
    let o = go(
        &[("main/radius", n(6.0)), ("main/blendMode", e(1)), ("style/gamma/autoDetect", on(false)), ("style/gamma/sceneGamma", n(1.0))],
        square(128, 128, (61, 67), (61, 67), 5.0),
    );
    assert!(at(&o, 64, 64)[0] > 5.0);
}

#[test]
fn saturation_bias_prefers_coloured_light() {
    let img = scene(192, 128, |x, y| {
        if (30..36).contains(&x) && (61..67).contains(&y) {
            [1.0, 1.0, 1.0, 1.0]
        } else if (150..156).contains(&x) && (61..67).contains(&y) {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    });
    let plain = go(&[("main/radius", n(10.0))], img.clone());
    let biased = go(&[("main/radius", n(10.0)), ("input/saturationBias", n(100.0))], img);
    assert!(at(&plain, 33, 72)[1] > 0.005 && at(&plain, 153, 72)[0] > 0.005);
    assert!(at(&biased, 33, 72)[1] < 1e-4, "white no longer glows");
    assert!(at(&biased, 153, 72)[0] > 0.005, "red still does");
}

#[test]
fn the_mask_layer_decides_where_the_glow_comes_from() {
    let img =
        scene(
            192,
            128,
            |x, y| if ((30..36).contains(&x) || (150..156).contains(&x)) && (61..67).contains(&y) { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] },
        );
    // Mask: white on the right half only (opaque).
    let mask = scene(192, 128, |x, _| if x >= 96 { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] });
    let host = Layers(vec![mask]);
    let env = EffectEnv { host: Some(&host), ..Default::default() };
    let run = |mode: u32, invert: bool| {
        go_env(
            &[("main/radius", n(10.0)), ("input/maskLayer", Value::Layer(Some(1))), ("input/maskMode", e(mode)), ("input/maskInvert", on(invert))],
            img.clone(),
            env,
        )
    };
    let (left, right) = ((33, 72), (153, 72));
    // None: the layer is ignored.
    let none = run(0, false);
    assert!(at(&none, left.0, left.1)[0] > 0.005 && at(&none, right.0, right.1)[0] > 0.005);
    // Luminance: only the right half glows (the mask's alpha is 1 everywhere).
    let lum = run(2, false);
    assert!(at(&lum, left.0, left.1)[0] < 1e-5 && at(&lum, right.0, right.1)[0] > 0.005);
    // Invert flips it.
    let inv = run(2, true);
    assert!(at(&inv, left.0, left.1)[0] > 0.005 && at(&inv, right.0, right.1)[0] < 1e-5);
    // Alpha: the mask is opaque everywhere, so everything glows.
    let alpha = run(1, false);
    assert!(at(&alpha, left.0, left.1)[0] > 0.005);
    // No layer chosen: nothing is masked.
    let missing = go(&[("main/radius", n(10.0)), ("input/maskMode", e(2))], img);
    assert!(at(&missing, left.0, left.1)[0] > 0.005);
}

#[test]
fn unmult_generates_alpha_and_without_it_the_glow_sits_on_black() {
    let img = scene(128, 128, |x, y| if (61..67).contains(&x) && (61..67).contains(&y) { [1.0, 1.0, 1.0, 1.0] } else { [0.0; 4] });
    let on_ = go(&[("main/radius", n(14.0))], img.clone());
    let p = at(&on_, 64, 72);
    assert!(p[3] > 0.01 && p[3] < 1.0, "glow over transparency has alpha {p:?}");
    assert!(p[0] <= p[3] + 1e-4, "premultiplied: {p:?}");
    assert!(at(&on_, 2, 2)[3] < at(&on_, 64, 72)[3]);
    let off = go(&[("main/radius", n(14.0)), ("output/unmult", on(false))], img);
    assert_eq!(at(&off, 64, 72)[3], 1.0);
    assert!(at(&off, 64, 72)[0] > 0.0);
}

#[test]
fn source_opacity_fades_the_original_out() {
    let full = go(&[("main/radius", n(14.0))], dot());
    let half = go(&[("main/radius", n(14.0)), ("output/sourceOpacity", n(50.0))], dot());
    let none = go(&[("main/radius", n(14.0)), ("output/sourceOpacity", n(0.0)), ("output/unmult", on(true))], dot());
    let (a, b, c) = (at(&full, 64, 64)[0], at(&half, 64, 64)[0], at(&none, 64, 64)[0]);
    assert!(a > b && b > c, "{a} {b} {c}");
    assert!(c < 0.7, "glow alone at the core is dim: {c}");
}

#[test]
fn views_show_the_glow_input_and_the_glow_alone() {
    let input = go(&[("output/view", e(1)), ("input/threshold", n(50.0)), ("main/radius", n(14.0))], square(128, 128, (61, 67), (61, 67), 1.0));
    assert!(at(&input, 64, 64)[0] > 0.45 && at(&input, 64, 64)[0] < 0.55, "threshold 50 % leaves half: {}", at(&input, 64, 64)[0]);
    assert_eq!(at(&input, 64, 72)[0], 0.0, "the input is not blurred");
    let only = go(&[("output/view", e(2)), ("main/radius", n(14.0))], dot());
    let full = go(&[("main/radius", n(14.0))], dot());
    assert!(at(&only, 64, 64)[0] < at(&full, 64, 64)[0]);
    assert!(at(&only, 64, 74)[0] > 0.005);
}

#[test]
fn aspect_ratio_spreads_the_glow_along_one_axis() {
    let horizontal = go(&[("style/aspect/ratio", n(2.0)), ("main/radius", n(24.0))], dot());
    let vertical = go(&[("style/aspect/ratio", n(0.0)), ("main/radius", n(24.0))], dot());
    let equal = go(&[("main/radius", n(24.0))], dot());
    let d = 22;
    assert!(at(&horizontal, 64 + d, 64)[0] > 3.0 * at(&horizontal, 64, 64 + d)[0], "ratio 2 spreads sideways");
    assert!(at(&vertical, 64, 64 + d)[0] > 3.0 * at(&vertical, 64 + d, 64)[0], "ratio 0 spreads up and down");
    assert!((at(&equal, 64 + d, 64)[0] - at(&equal, 64, 64 + d)[0]).abs() < 0.02 * at(&equal, 64 + d, 64)[0]);
}

#[test]
fn the_angle_turns_the_aspect_ratio() {
    let base = [("style/aspect/ratio", n(2.0)), ("main/radius", n(24.0)), ("style/aspect/enableAngle", on(true))];
    let turned = go(&[base[0].clone(), base[1].clone(), base[2].clone(), ("style/aspect/angle", n(90.0))], dot());
    let d = 22;
    assert!(at(&turned, 64, 64 + d)[0] > 2.0 * at(&turned, 64 + d, 64)[0], "90 degrees: along y");
    let diag = go(&[base[0].clone(), base[1].clone(), base[2].clone(), ("style/aspect/angle", n(45.0))], dot());
    let (a, b) = (at(&diag, 64 + 16, 64 + 16)[0], at(&diag, 64 + 16, 64 - 16)[0]);
    assert!(a > 1.5 * b, "45 degrees clockwise favours the down-right diagonal: {a} vs {b}");
    // Angle disabled: ignored.
    let off = go(&[base[0].clone(), base[1].clone(), ("style/aspect/angle", n(90.0))], dot());
    assert!(at(&off, 64 + d, 64)[0] > at(&off, 64, 64 + d)[0]);
}

#[test]
fn glow_aberration_gives_each_channel_its_own_radius() {
    let off = go(&[("main/radius", n(20.0))], dot());
    let ab = go(&[("main/radius", n(20.0)), ("style/aberration/glowEnable", on(true))], dot());
    let p = at(&off, 64, 96);
    assert!((p[0] - p[1]).abs() < 1e-4 && (p[1] - p[2]).abs() < 1e-4, "no aberration: grey {p:?}");
    let q = at(&ab, 64, 96);
    assert!(q[2] > q[1] && q[1] > q[0], "far out blue (120 %) beats green (100 %) beats red (80 %): {q:?}");
    let near = at(&ab, 64, 70);
    assert!(near[0] > near[2], "near the source the narrow red channel is brighter: {near:?}");
    // Shuffle Channels swaps who gets which multiplier.
    let sh = go(&[("main/radius", n(20.0)), ("style/aberration/glowEnable", on(true)), ("style/aberration/channelOrder", e(5))], dot());
    let s = at(&sh, 64, 96);
    assert!(s[0] > s[1] && s[1] > s[2], "B G R: {s:?}");
}

#[test]
fn pixel_aberration_separates_the_channels_in_space() {
    let img = square(128, 128, (100, 112), (61, 67), 1.0);
    let off = go(&[("main/radius", n(6.0))], img.clone());
    let on_ = go(&[("main/radius", n(6.0)), ("style/aberration/pixelEnable", on(true)), ("style/aberration/pixelOffset", n(12.0))], img.clone());
    let centroid = |b: &Buf, c: usize| {
        let (mut s, mut w) = (0.0f64, 0.0f64);
        for x in 70..128 {
            let v = f64::from(at(b, x, 64)[c]);
            s += f64::from(x) * v;
            w += v;
        }
        s / w
    };
    assert!((centroid(&off, 0) - centroid(&off, 2)).abs() < 1e-3);
    assert!(centroid(&on_, 0) - centroid(&on_, 2) > 0.5, "red outward, blue inward: {} {}", centroid(&on_, 0), centroid(&on_, 2));
    // "Red" alone leaves green and blue where they were.
    let one = go(
        &[
            ("main/radius", n(6.0)),
            ("style/aberration/pixelEnable", on(true)),
            ("style/aberration/pixelOffset", n(12.0)),
            ("style/aberration/pixelChannels", e(3)),
        ],
        img,
    );
    assert!((centroid(&one, 1) - centroid(&off, 1)).abs() < 1e-3 && (centroid(&one, 2) - centroid(&off, 2)).abs() < 1e-3);
    assert!(centroid(&one, 0) > centroid(&off, 0) + 0.2);
}

#[test]
fn tint_recolours_the_glow_not_the_source() {
    let plain = go(&[("main/radius", n(14.0))], dot());
    let red = go(&[("main/radius", n(14.0)), ("style/tint/mode", e(1)), ("style/tint/inner", Value::Color([1.0, 0.0, 0.0, 1.0]))], dot());
    let p = at(&red, 64, 74);
    assert!(p[0] > 0.005 && p[1] < 1e-4 && p[2] < 1e-4, "red glow {p:?}");
    assert!(at(&plain, 64, 74)[1] > 0.005);
    assert!(at(&red, 64, 64)[1] > 0.99, "the white source is untouched");
    // Strength 0 is no tint at all.
    let zero = go(&[("main/radius", n(14.0)), ("style/tint/mode", e(1)), ("style/tint/strength", n(0.0))], dot());
    assert!((at(&zero, 64, 74)[1] - at(&plain, 64, 74)[1]).abs() < 1e-6);
    // Inner & Outer: near the source it is the inner colour, far from it the outer.
    let io = go(
        &[
            ("main/radius", n(24.0)),
            ("main/exposure", n(3.0)),
            ("style/tint/mode", e(2)),
            ("style/tint/inner", Value::Color([1.0, 0.0, 0.0, 1.0])),
            ("style/tint/outer", Value::Color([0.0, 0.0, 1.0, 1.0])),
        ],
        dot(),
    );
    let (near, far) = (at(&io, 64, 68), at(&io, 64, 112));
    assert!(near[0] > near[2], "inner is red {near:?}");
    assert!(far[2] > far[0], "outer is blue {far:?}");
    // Swap Colors exchanges them.
    let sw = go(
        &[
            ("main/radius", n(24.0)),
            ("main/exposure", n(3.0)),
            ("style/tint/mode", e(2)),
            ("style/tint/swap", on(true)),
            ("style/tint/inner", Value::Color([1.0, 0.0, 0.0, 1.0])),
            ("style/tint/outer", Value::Color([0.0, 0.0, 1.0, 1.0])),
        ],
        dot(),
    );
    assert!(at(&sw, 64, 68)[2] > at(&sw, 64, 68)[0] && at(&sw, 64, 112)[0] > at(&sw, 64, 112)[2]);
}

#[test]
fn tone_mapping_tames_hdr_glow_and_mix_blends_it_back() {
    let strong = |extra: &[(&'static str, Value)]| {
        go(&with(linear_add(), &[("main/radius", n(10.0)), ("main/exposure", n(60.0))]).into_iter().chain(extra.iter().cloned()).collect::<Vec<_>>(), dot())
    };
    let none = strong(&[]);
    let aces = strong(&[("style/toneMapping/operation", e(3))]);
    let half = strong(&[("style/toneMapping/operation", e(3)), ("style/toneMapping/mix", n(50.0))]);
    // Next to the source, outside it: above 1 untouched, at most 1 mapped.
    let (a, b, c) = (at(&none, 64, 70)[0], at(&aces, 64, 70)[0], at(&half, 64, 70)[0]);
    assert!(a > 1.5, "HDR glow is kept: {a}");
    assert!(b <= 1.0 + 1e-3, "tone mapped: {b}");
    assert!(c > b && c < a, "mix 50 % lies between: {b} {c} {a}");
    for op in 1..6 {
        let o = strong(&[("style/toneMapping/operation", e(op))]);
        assert!(at(&o, 64, 70)[0] <= 1.0 + 1e-3, "operation {op}");
    }
}

#[test]
fn lens_dirt_textures_the_superbright_glow() {
    // A texture that is white on the left half and black on the right.
    let tex = scene(128, 128, |x, _| if x < 64 { [1.0, 1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0, 1.0] });
    let host = Layers(vec![tex]);
    let env = EffectEnv { host: Some(&host), ..Default::default() };
    let img = square(128, 128, (61, 67), (61, 67), 1.0);
    let args = [("main/radius", n(30.0)), ("main/exposure", n(4.0)), ("style/lensDirt/texture", Value::Layer(Some(1)))];
    let dirty = go_env(&args, img.clone(), env);
    let clean = go(&[("main/radius", n(30.0)), ("main/exposure", n(4.0))], img.clone());
    let (l, r) = (at(&dirty, 58, 56)[0], at(&dirty, 69, 56)[0]);
    assert!(l > r * 1.2, "dirt on the white half only: {l} vs {r}");
    assert!((at(&clean, 58, 56)[0] - at(&clean, 69, 56)[0]).abs() < 0.01 * l);
    // Opacity 0 and no texture are both plain glow.
    let zero = go_env(&[args[0].clone(), args[1].clone(), args[2].clone(), ("style/lensDirt/opacity", n(0.0))], img, env);
    assert!((at(&zero, 54, 52)[0] - at(&clean, 54, 52)[0]).abs() < 1e-5);
    // Matte Exposure brightens the matte, Matte Gamma > 1 darkens its mid tones.
    let grey = scene(128, 128, |_, _| [0.25, 0.25, 0.25, 1.0]);
    let host = Layers(vec![grey]);
    let env = EffectEnv { host: Some(&host), ..Default::default() };
    let at_x = |extra: &[(&'static str, Value)]| {
        let mut v = args.to_vec();
        v.extend_from_slice(extra);
        at(&go_env(&v, square(128, 128, (61, 67), (61, 67), 1.0), env), 54, 52)[0]
    };
    let base = at_x(&[]);
    assert!(at_x(&[("style/lensDirt/matteExposure", n(2.0))]) > base);
    assert!(at_x(&[("style/lensDirt/matteGamma", n(3.0))]) < base);
}

#[test]
fn lens_dirt_repeat_edges_extends_the_texture_past_the_layer() {
    // A small layer whose glow is padded: outside the layer the texture's edge repeats or is absent.
    let tex = scene(32, 32, |_, _| [1.0, 1.0, 1.0, 1.0]);
    let host = Layers(vec![tex]);
    let env = EffectEnv { host: Some(&host), ..Default::default() };
    let img = square(32, 32, (14, 18), (14, 18), 1.0);
    let run = |repeat: bool| {
        let o = run_fx(
            ID,
            &[
                ("main/radius", n(12.0)),
                ("main/exposure", n(4.0)),
                ("style/lensDirt/texture", Value::Layer(Some(1))),
                ("style/lensDirt/repeatEdges", on(repeat)),
            ],
            img.clone(),
            0.0,
            env,
        );
        // A pixel in the padding, left of the layer.
        let x = (o.offset[0] as i64 - 6).max(0);
        o.img.get(x, o.offset[1] as i64 + 16)[0]
    };
    assert!(run(true) > run(false), "{} vs {}", run(true), run(false));
}

#[test]
fn quality_presets_all_conserve_the_light() {
    let big = square(192, 192, (92, 100), (92, 100), 1.0);
    let mut sums = Vec::new();
    for preset in 0..7 {
        let o = go(&with(linear_add(), &[("main/radius", n(30.0)), ("quality/preset", e(preset))]), big.clone());
        assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite() && *v >= -1e-3)), "preset {preset}");
        sums.push(total(&o, 0));
    }
    let mean = sums.iter().sum::<f64>() / sums.len() as f64;
    for (i, s) in sums.iter().enumerate() {
        assert!((s / mean - 1.0).abs() < 0.03, "preset {i}: {s} vs mean {mean}");
    }
}

#[test]
fn custom_quality_follows_its_own_controls() {
    let img = square(192, 192, (92, 100), (92, 100), 1.0);
    let run = |d: f64, s: f64, it: f64| {
        go(
            &with(
                linear_add(),
                &[
                    ("main/radius", n(30.0)),
                    ("quality/preset", e(6)),
                    ("quality/downsampling", n(d)),
                    ("quality/stepsMultiplier", n(s)),
                    ("quality/iterations", n(it)),
                ],
            ),
            img.clone(),
        )
    };
    let fine = run(0.0, 4.0, 12.0);
    let coarse = run(3.0, 1.0, 12.0);
    let diff = fine.img.data.iter().zip(&coarse.img.data).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0, f32::max);
    assert!(diff > 1e-4, "downsampling and steps change the picture");
    assert!(total(&coarse, 0) > 0.0 && (total(&coarse, 0) / total(&fine, 0) - 1.0).abs() < 0.08);
}

#[test]
fn buffer_expansion_modes() {
    let img = dot();
    let size = |vals: &[(&str, Value)]| {
        let o = run_fx(ID, vals, img.clone(), 0.0, EffectEnv::default());
        (o.img.width, o.img.height)
    };
    let auto = size(&[("main/radius", n(20.0))]);
    assert!(auto.0 > 128 && auto.1 > 128, "Auto grows the buffer for the glow that leaves the layer: {auto:?}");
    assert_eq!(size(&[("main/radius", n(20.0)), ("quality/bufferExpansionMode", e(2))]), (128, 128));
    let manual = size(&[("main/radius", n(20.0)), ("quality/bufferExpansionMode", e(1)), ("quality/auxBufferExpansion", n(10.0))]);
    assert_eq!(manual, (148, 148));
    let both = size(&[("main/radius", n(20.0)), ("quality/auxBufferExpansion", n(10.0))]);
    assert!(both.0 > auto.0, "Auto adds the aux expansion on top");
    // The padding really holds glow.
    // (a wider glow, so the far side of the padding is lit)
    let o = run_fx(ID, &[("main/radius", n(60.0))], dot(), 0.0, EffectEnv::default());
    assert!(o.img.get(o.offset[0] as i64 - 1, o.img.height as i64 / 2)[0] > 0.0);
}

#[test]
fn half_float_rounds_the_glow_only() {
    let a = go(&[("main/radius", n(14.0)), ("quality/halfFloat", on(true))], dot());
    let b = go(&[("main/radius", n(14.0)), ("quality/halfFloat", on(false))], dot());
    let worst = a.img.data.iter().zip(&b.img.data).map(|(p, q)| (p[0] - q[0]).abs()).fold(0.0, f32::max);
    assert!(worst < 2e-3, "half precision stays within 11 bits: {worst}");
    assert!((at(&a, 64, 64)[0] - at(&b, 64, 64)[0]).abs() < 2e-3, "the source is not rounded");
}

#[test]
fn lens_iris_takes_the_shape_of_the_iris_layer() {
    // A point of light. The aperture is a filled square (a circle when no layer is chosen).
    let point = square(160, 160, (79, 81), (79, 81), 6.0);
    let aperture = scene(32, 32, |_, _| [1.0, 1.0, 1.0, 1.0]);
    let host = Layers(vec![aperture]);
    let env = EffectEnv { host: Some(&host), ..Default::default() };
    let args = [("main/glowMode", e(1)), ("main/radius", n(40.0)), ("main/blendMode", e(1)), ("main/exposure", n(2.0))];
    let sq = go_env(&[args[0].clone(), args[1].clone(), args[2].clone(), args[3].clone(), ("iris/irisLayer", Value::Layer(Some(1)))], point.clone(), env);
    let circle = go(&args, point);
    // The point 36 px out on the diagonal is outside a circle of radius 40 but inside the square.
    let ratio = |b: &Buf| at(b, 80 + 36, 80 + 36)[0] / at(b, 80 + 36, 80)[0].max(1e-6);
    assert!(ratio(&sq) > 4.0 * ratio(&circle), "square {} vs circle {}", ratio(&sq), ratio(&circle));
    // The aperture has an edge: the glow stops within its radius, not past it.
    assert!(at(&sq, 80 + 55, 80)[0] < 0.05 * at(&sq, 80 + 20, 80)[0].max(1e-6), "{} vs {}", at(&sq, 80 + 55, 80)[0], at(&sq, 80 + 20, 80)[0]);
    // No iris layer: a circular aperture, still a glow.
    assert!(at(&circle, 80 + 20, 80)[0] > 1e-3);
}

#[test]
fn lens_iris_controls_do_something() {
    let point = square(160, 160, (79, 81), (79, 81), 6.0);
    let base = [("main/glowMode", e(1)), ("main/radius", n(40.0)), ("main/blendMode", e(1))];
    let run = |extra: &[(&'static str, Value)]| {
        let mut v = base.to_vec();
        v.extend_from_slice(extra);
        go(&v, point.clone())
    };
    let std = run(&[]);
    // Fewer iterations: fewer rings, a different (coarser) profile.
    let few = run(&[("iris/iterations", n(2.0))]);
    assert!((0..160).map(|x| (at(&std, x, 80)[0] - at(&few, x, 80)[0]).abs()).fold(0.0, f32::max) > 1e-4);
    // Radii Easing moves the rings.
    let eased = run(&[("iris/radiiEasing", n(10.0))]);
    assert!((0..160).map(|x| (at(&std, x, 80)[0] - at(&eased, x, 80)[0]).abs()).fold(0.0, f32::max) > 1e-4);
    // Sampling quality does not break energy.
    for q in 0..3 {
        let o = run(&[("iris/samplingQuality", e(q))]);
        let glow = total(&o, 0) - 36.0 * 0.0 - total(&run(&[("main/exposure", n(0.0))]), 0);
        assert!(glow > 0.0 && o.img.data.iter().all(|p| p[0].is_finite()), "quality {q}");
    }
    // Aspect ratio stretches the aperture.
    let wide = run(&[("style/aspect/ratio", n(2.0))]);
    assert!(at(&wide, 80 + 30, 80)[0] > 2.0 * at(&wide, 80, 80 + 30)[0]);
}

#[test]
fn lens_iris_conserves_the_light() {
    let point = square(256, 256, (126, 130), (126, 130), 1.0);
    let args = with(linear_add(), &[("main/glowMode", e(1)), ("main/radius", n(40.0))]);
    let src = total(&go(&with(args.clone(), &[("main/exposure", n(0.0))]), point.clone()), 0);
    let o = go(&args, point);
    assert!((((total(&o, 0) - src) / 16.0) - 1.0).abs() < 0.08, "{}", (total(&o, 0) - src) / 16.0);
}

#[test]
fn gamma_correction_changes_how_light_adds() {
    let img = square(128, 128, (61, 67), (61, 67), 0.5);
    let auto = go(&[("main/radius", n(14.0)), ("main/blendMode", e(1))], img.clone());
    let lin = go_env(&[("main/radius", n(14.0)), ("main/blendMode", e(1))], img.clone(), EffectEnv { working_linear: true, ..Default::default() });
    let manual =
        go(&[("main/radius", n(14.0)), ("main/blendMode", e(1)), ("style/gamma/autoDetect", on(false)), ("style/gamma/sceneGamma", n(1.0))], img.clone());
    // A linear working space needs no conversion, so auto detect behaves like gamma 1.
    assert!((at(&lin, 64, 74)[0] - at(&manual, 64, 74)[0]).abs() < 1e-6);
    // In a gamma-encoded space the same glow reads brighter at low levels.
    assert!(at(&auto, 64, 74)[0] > at(&lin, 64, 74)[0]);
    // Scene Gamma is used when auto detect is off.
    let g3 = go(&[("main/radius", n(14.0)), ("main/blendMode", e(1)), ("style/gamma/autoDetect", on(false)), ("style/gamma/sceneGamma", n(3.0))], img);
    assert!(at(&g3, 64, 74)[0] > at(&auto, 64, 74)[0]);
}

#[test]
fn blend_modes_all_run_and_screen_never_exceeds_add() {
    let img = square(128, 128, (61, 67), (61, 67), 0.8);
    let add = go(&[("main/radius", n(14.0)), ("main/blendMode", e(1))], img.clone());
    let screen = go(&[("main/radius", n(14.0)), ("main/blendMode", e(0))], img.clone());
    assert!(at(&screen, 64, 74)[0] <= at(&add, 64, 74)[0] + 1e-6);
    for m in 0..10 {
        let o = go(&[("main/radius", n(14.0)), ("main/blendMode", e(m))], img.clone());
        assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite())), "blend {m}");
    }
}

#[test]
fn degenerate_inputs_stay_finite() {
    for (w, h) in [(1, 1), (3, 2), (2, 40), (17, 1)] {
        let img = scene(w, h, |x, y| [1.0, 0.5, 0.25, 1.0 - 0.1 * ((x + y) % 3) as f32]);
        for extra in [
            vec![("main/radius", n(10000.0))],
            vec![("main/glowMode", e(1)), ("main/radius", n(300.0))],
            vec![("style/aspect/ratio", n(0.0)), ("style/aspect/enableAngle", on(true)), ("style/aspect/angle", n(33.0))],
        ] {
            let o = run_fx(ID, &extra, img.clone(), 0.0, EffectEnv::default());
            assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite())), "{w}x{h} {extra:?}");
        }
    }
    // Not-a-number and infinite input pixels do not spread.
    let mut img = dot();
    img.set(10, 10, [f32::NAN, f32::INFINITY, -3.0, 1.0]);
    let o = go(&[("main/radius", n(10.0))], img);
    assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite())));
    // Zero-size radius, huge threshold.
    let o = go(&[("input/threshold", n(1000.0))], dot());
    assert!(o.img.data.iter().all(|p| p.iter().all(|v| v.is_finite())));
}

#[test]
fn rendering_is_deterministic_and_adjustment_layers_do_not_grow() {
    let a = go(&[("main/radius", n(14.0)), ("style/aberration/glowEnable", on(true))], dot());
    let b = go(&[("main/radius", n(14.0)), ("style/aberration/glowEnable", on(true))], dot());
    assert_eq!(a.img.data, b.img.data);
    let s = crate::find(ID).expect("registered");
    let params = crate::Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
    let ctx = crate::EffectCtx { params: &params, time: 0.0, layer_size: [128.0, 128.0], seed: 0, adjustment: true, env: Default::default() };
    let o = crate::apply(s, &ctx, Buf { img: dot(), offset: [0.0; 2], scale: 1.0 });
    assert_eq!((o.img.width, o.img.height), (128, 128));
}

#[test]
fn the_glow_scales_with_the_render_resolution() {
    // Radius is in layer pixels: at half resolution the same effect looks the same.
    let full = go(&[("main/radius", n(24.0)), ("main/blendMode", e(1))], square(128, 128, (60, 68), (60, 68), 1.0));
    let s = crate::find(ID).expect("registered");
    let mut params = crate::Params { values: s.params.iter().map(|p| (p.id.to_string(), p.default.clone())).collect() };
    params.values.insert("main/radius".into(), n(24.0));
    params.values.insert("main/blendMode".into(), e(1));
    params.values.insert("quality/bufferExpansionMode".into(), e(2));
    let ctx = crate::EffectCtx { params: &params, time: 0.0, layer_size: [128.0, 128.0], seed: 0, adjustment: false, env: Default::default() };
    let half = crate::apply(s, &ctx, Buf { img: square(64, 64, (30, 34), (30, 34), 1.0), offset: [0.0; 2], scale: 0.5 });
    // Same relative position: 16 layer px below the square's edge.
    let (a, b) = (at(&full, 64, 68 + 16)[0], at(&half, 32, 34 + 8)[0]);
    assert!((b / a - 1.0).abs() < 0.35, "full {a} vs half {b}");
}

#[test]
fn controls_show_only_when_they_matter() {
    let shown = |param: &str, set: &'static [(&'static str, u32)]| {
        crate::catalog::param_shown(ID, param, &|id| set.iter().find(|(k, _)| *k == id).map(|(_, v)| Value::Enum(*v)))
    };
    // Iris controls: Lens Iris only.
    assert!(!shown("iris", &[]) && !shown("iris/iterations", &[]));
    assert!(shown("iris", &[("main/glowMode", 1)]) && shown("iris/radiiEasing", &[("main/glowMode", 1)]));
    // Glow Aberration is Exponential-only.
    assert!(!shown("style/aberration/glowEnable", &[("main/glowMode", 1)]));
    assert!(shown("style/aberration/glowEnable", &[]));
    // Tint.
    assert!(!shown("style/tint/inner", &[]) && shown("style/tint/inner", &[("style/tint/mode", 1)]));
    assert!(!shown("style/tint/outer", &[("style/tint/mode", 1)]) && shown("style/tint/outer", &[("style/tint/mode", 2)]));
    // Tone mapping mix.
    assert!(!shown("style/toneMapping/mix", &[]) && shown("style/toneMapping/mix", &[("style/toneMapping/operation", 2)]));
    // Custom quality.
    assert!(!shown("quality/downsampling", &[]) && shown("quality/downsampling", &[("quality/preset", 6)]));
    // Monochromatic follows Dither.
    assert!(!shown("output/ditherMono", &[]) && shown("output/ditherMono", &[("output/dither", 1)]));
    // Always shown.
    assert!(shown("main/radius", &[]) && shown("output/unmult", &[]) && shown("style", &[]));
}

#[test]
fn an_arbitrary_angle_puts_the_glow_along_that_line() {
    // Ratio 2 spreads horizontally only; turned 30 degrees clockwise the line runs down-right.
    let o = go(&[("style/aspect/ratio", n(2.0)), ("main/radius", n(24.0)), ("style/aspect/enableAngle", on(true)), ("style/aspect/angle", n(30.0))], dot());
    let (sa, ca) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let along = |d: f64| at(&o, (64.0 + d * ca) as u32, (64.0 + d * sa) as u32)[0];
    let (a, b, c) = (along(10.0), along(20.0), along(30.0));
    assert!(a > b && b > c && c > 0.0, "falls off along the line: {a} {b} {c}");
    // Across it there is far less than along it.
    let across = at(&o, (64.0 - 14.0 * sa) as u32, (64.0 + 14.0 * ca) as u32)[0];
    assert!(across < 0.2 * along(14.0), "across {across} vs along {}", along(14.0));
}

#[test]
fn dither_adds_bounded_zero_mean_noise_that_changes_every_frame() {
    let args = |extra: &[(&'static str, Value)]| {
        let mut v = vec![
            ("main/radius", n(30.0)),
            ("style/gamma/autoDetect", on(false)),
            ("style/gamma/sceneGamma", n(1.0)),
            ("main/blendMode", e(1)),
            ("output/sourceOpacity", n(0.0)),
        ];
        v.extend_from_slice(extra);
        v
    };
    let plain = go(&args(&[]), dot());
    let d = |t: f64, extra: &[(&'static str, Value)]| {
        let mut v = args(&[("output/dither", on(true))]);
        v.extend_from_slice(extra);
        let mut all: Vec<(&str, Value)> = vec![("quality/bufferExpansionMode", e(2))];
        all.extend_from_slice(&v);
        run_fx(ID, &all, dot(), t, EffectEnv::default())
    };
    let (a, b) = (d(0.0, &[]), d(0.5, &[]));
    // Over the lit area the noise is at most one 8-bit step and averages out.
    let (mut sum, mut cnt, mut worst) = (0.0f64, 0u32, 0.0f32);
    for y in 40..90 {
        for x in 40..90 {
            let (g, o) = (at(&plain, x, y)[1], at(&a, x, y)[1]);
            if g > 0.008 {
                let diff = o - g;
                worst = worst.max(diff.abs());
                sum += f64::from(diff);
                cnt += 1;
            }
        }
    }
    assert!(cnt > 300 && worst > 0.0, "dither did something: {cnt} {worst}");
    assert!(worst <= 1.0 / 255.0 * 1.01, "at most one step: {worst}");
    assert!((sum / f64::from(cnt)).abs() < 2e-4, "zero mean: {}", sum / f64::from(cnt));
    // Another frame, another pattern; the same frame, the same pattern.
    assert!(a.img.data != b.img.data);
    assert_eq!(a.img.data, d(0.0, &[]).img.data);
    // Mono: all three channels get the same noise (a grey glow stays grey); not mono: they differ.
    let mono = at(&a, 70, 64);
    assert!((mono[0] - mono[1]).abs() < 1e-6 && (mono[1] - mono[2]).abs() < 1e-6);
    let col = d(0.0, &[("output/ditherMono", on(false))]);
    let any = (0..128).any(|x| {
        let p = at(&col, x, 64);
        (p[0] - p[1]).abs() > 1e-6
    });
    assert!(any, "per-channel noise differs between channels");
    // No glow, no noise: black stays black.
    assert_eq!(at(&a, 2, 2)[0], 0.0);
}

#[test]
fn overlay_and_soft_light_tints_work_through_the_effect() {
    let base = |blend: u32, inner: [f32; 3]| {
        go(
            &[
                ("main/radius", n(20.0)),
                ("main/blendMode", e(1)),
                ("style/gamma/autoDetect", on(false)),
                ("style/gamma/sceneGamma", n(1.0)),
                ("style/tint/mode", e(1)),
                ("style/tint/blend", e(blend)),
                ("style/tint/inner", Value::Color([f64::from(inner[0]), f64::from(inner[1]), f64::from(inner[2]), 1.0])),
            ],
            dot(),
        )
    };
    let plain = go(&[("main/radius", n(20.0)), ("main/blendMode", e(1)), ("style/gamma/autoDetect", on(false)), ("style/gamma/sceneGamma", n(1.0))], dot());
    for blend in [4, 5] {
        // A mid-grey tint changes nothing; a saturated one does.
        let neutral = base(blend, [0.5, 0.5, 0.5]);
        assert!((at(&neutral, 64, 74)[0] - at(&plain, 64, 74)[0]).abs() < 1e-4, "blend {blend}");
        let red = base(blend, [1.0, 0.0, 0.0]);
        let p = at(&red, 64, 74);
        assert!(p.iter().all(|v| v.is_finite()) && p[0] > p[2], "blend {blend}: {p:?}");
    }
}

/// A point of light, Lens Iris, Add, and an aperture layer from `ap`.
fn iris_run(ap: Option<Image>, extra: &[(&'static str, Value)]) -> Buf {
    let point = square(160, 160, (79, 81), (79, 81), 6.0);
    let mut args = vec![("main/glowMode", e(1)), ("main/radius", n(40.0)), ("main/blendMode", e(1)), ("main/exposure", n(2.0))];
    args.extend_from_slice(extra);
    match ap {
        Some(img) => {
            let host = Layers(vec![img]);
            let env = EffectEnv { host: Some(&host), ..Default::default() };
            args.push(("iris/irisLayer", Value::Layer(Some(1))));
            go_env(&args, point, env)
        }
        None => go(&args, point),
    }
}

#[test]
fn iris_opacity_blends_the_aperture_with_a_round_glow() {
    let square_ap = || Some(scene(32, 32, |_, _| [1.0, 1.0, 1.0, 1.0]));
    // The diagonal point at 36 px is outside the round glow and inside the square aperture.
    let corner = |b: &Buf| at(b, 80 + 36, 80 + 36)[0];
    let (full, half, none) =
        (iris_run(square_ap(), &[]), iris_run(square_ap(), &[("iris/opacity", n(50.0))]), iris_run(square_ap(), &[("iris/opacity", n(0.0))]));
    let round = iris_run(None, &[]);
    assert!(corner(&full) > corner(&half) && corner(&half) > corner(&none), "{} {} {}", corner(&full), corner(&half), corner(&none));
    // Opacity 0 is the plain round glow.
    let worst = (0..160).map(|x| (at(&none, x, 80)[0] - at(&round, x, 80)[0]).abs()).fold(0.0, f32::max);
    assert!(worst < 1e-3, "opacity 0 equals the round glow: {worst}");
}

#[test]
fn iris_aspect_ratio_stretches_the_aperture_alone() {
    let ap = || Some(scene(32, 32, |_, _| [1.0, 1.0, 1.0, 1.0]));
    let base = iris_run(ap(), &[]);
    let wide = iris_run(ap(), &[("iris/aspectRatio", n(2.0))]);
    // Wider than tall, and wider than the unstretched aperture.
    assert!(at(&wide, 80 + 30, 80)[0] > 2.0 * at(&wide, 80, 80 + 30)[0], "wide along x");
    assert!(at(&wide, 80 + 30, 80)[0] > at(&base, 80 + 30, 80)[0], "wider than 1.0");
    // Style Aspect Ratio and Iris Aspect Ratio add up: both stretching the same way still keeps
    // the widest axis at the radius, and the glow finite.
    let both = iris_run(ap(), &[("iris/aspectRatio", n(2.0)), ("style/aspect/ratio", n(2.0))]);
    assert!(both.img.data.iter().all(|p| p.iter().all(|v| v.is_finite() && *v >= 0.0)));
    assert!(at(&both, 80 + 30, 80)[0] > 3.0 * at(&both, 80, 80 + 30)[0].max(1e-6));
}

#[test]
fn radius_normalization_makes_padding_around_the_aperture_irrelevant() {
    // A small lit square in a big empty layer: unnormalised it is a tiny aperture, normalised it
    // is scaled up so its farthest corner reaches the radius.
    let padded = || Some(scene(128, 128, |x, y| if (56..72).contains(&x) && (56..72).contains(&y) { [1.0; 4] } else { [0.0, 0.0, 0.0, 1.0] }));
    let tight = || Some(scene(24, 24, |_, _| [1.0; 4]));
    let reach = |b: &Buf| (0..80).rev().find(|d| at(b, 80 + *d, 80)[0] > 2e-3).unwrap_or(0);
    let off = iris_run(padded(), &[]);
    let on_ = iris_run(padded(), &[("iris/radiusNormalization", on(true))]);
    assert!(reach(&on_) > reach(&off) + 8, "normalised reaches further: {} vs {}", reach(&on_), reach(&off));
    // Normalised, the padded and the tight layer reach about as far (a square's corner defines the
    // radius either way) and carry the same light.
    let t = iris_run(tight(), &[("iris/radiusNormalization", on(true))]);
    assert!(reach(&on_).abs_diff(reach(&t)) <= 5, "padding does not matter: {} vs {}", reach(&on_), reach(&t));
    let (a, b) = (total(&on_, 0), total(&t, 0));
    assert!(((a - b) / a).abs() < 0.06, "same light: {a} vs {b}");
}

#[test]
fn stepped_rasterization_changes_the_iris_but_keeps_its_light() {
    let ap = || Some(scene(32, 32, |_, _| [1.0, 1.0, 1.0, 1.0]));
    let smooth = iris_run(ap(), &[]);
    let stepped = iris_run(ap(), &[("iris/continuousRasterization", on(false))]);
    let diff = (0..160).map(|x| (at(&smooth, x, 80)[0] - at(&stepped, x, 80)[0]).abs()).fold(0.0, f32::max);
    assert!(diff > 1e-4, "stepped differs: {diff}");
    let (a, b) = (total(&smooth, 0), total(&stepped, 0));
    assert!(((a - b) / a).abs() < 0.08, "light kept: {a} vs {b}");
}
