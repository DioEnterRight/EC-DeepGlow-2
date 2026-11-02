//! What happens to the glow after it is generated: Lens Dirt, Tint, Pixel Aberration, and the
//! composite with the source (Blend Mode, Source Opacity, Unmult, gamma).

use effectcraft_color::{BlendMode, blend_pixel};
use effectcraft_raster::{Image, Px};
use rayon::prelude::*;

use super::color::luma;
use crate::{Buf, EffectCtx, LayerPixels};

/// Tint Blend Mode options.
pub const TINT_BLENDS: [&str; 6] = ["Multiply", "Normal", "Add", "Screen", "Overlay", "Soft Light"];

/// Tint settings; colours are linear.
#[derive(Clone, Debug)]
pub struct Tint {
    pub mode: u32,
    pub blend: u32,
    pub inner: [f32; 3],
    pub outer: [f32; 3],
    pub strength: f32,
}

#[inline]
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Tint a straight linear glow colour `g`. Mode 1 (single) uses the inner colour everywhere;
/// mode 2 runs from the outer colour (faint, far from the source) to the inner colour (bright,
/// next to the source) by the glow's own brightness.
pub fn apply_tint(g: [f32; 3], t: &Tint) -> [f32; 3] {
    if t.mode == 0 || t.strength <= 0.0 {
        return g;
    }
    let l = luma(g);
    let u = l / (l + 0.25);
    let col = if t.mode == 1 { t.inner } else { mix3(t.outer, t.inner, u) };
    let tinted = match t.blend {
        // Replace the hue, keep the brightness.
        1 => {
            let lc = luma(col).max(1e-4);
            col.map(|c| c * l / lc)
        }
        2 => [g[0] + col[0] * l, g[1] + col[1] * l, g[2] + col[2] * l],
        3 => [0, 1, 2].map(|i| g[i] + col[i] * l * (1.0 - g[i].clamp(0.0, 1.0))),
        4 => [0, 1, 2].map(|i| overlay_hdr(g[i], col[i])),
        5 => [0, 1, 2].map(|i| soft_light_hdr(g[i], col[i])),
        _ => [g[0] * col[0], g[1] * col[1], g[2] * col[2]],
    };
    mix3(g, tinted, t.strength.min(1.0))
}

/// Overlay (the standard formula, glow `g` as the base, tint `c` as the blend colour) with the
/// part of `g` above 1 added back scaled by `2c`, so HDR light keeps its energy (a mid-grey tint
/// of 0.5 leaves any glow unchanged).
fn overlay_hdr(g: f32, c: f32) -> f32 {
    let u = g.clamp(0.0, 1.0);
    let o = if u < 0.5 { 2.0 * u * c } else { 1.0 - 2.0 * (1.0 - u) * (1.0 - c) };
    o + (g - u).max(0.0) * 2.0 * c
}

/// Soft Light (W3C compositing formula) with the same HDR rule as [`overlay_hdr`].
fn soft_light_hdr(g: f32, c: f32) -> f32 {
    let u = g.clamp(0.0, 1.0);
    let o = if c <= 0.5 {
        u - (1.0 - 2.0 * c) * u * (1.0 - u)
    } else {
        let d = if u <= 0.25 { ((16.0 * u - 12.0) * u + 4.0) * u } else { u.sqrt() };
        u + (2.0 * c - 1.0) * (d - u)
    };
    o + (g - u).max(0.0) * 2.0 * c
}

/// Dither settings: triangular-PDF noise of one 8-bit step of the *encoded* output.
#[derive(Clone, Copy, Debug)]
pub struct Dither {
    pub on: bool,
    /// One noise value for all three channels (no colour speckle).
    pub mono: bool,
    /// Scene gamma the glow is encoded with.
    pub gamma: f32,
    /// Changes every frame, so the noise does not sit still.
    pub seed: u32,
}

#[inline]
fn hash32(mut h: u32) -> u32 {
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

/// Triangular noise in `(-1, 1)` for pixel `(x, y)`, channel `c`.
#[inline]
fn tpdf(x: u32, y: u32, c: u32, seed: u32) -> f32 {
    let h = hash32(x.wrapping_mul(0x9e37_79b1) ^ hash32(y.wrapping_mul(0x85eb_ca77).wrapping_add(seed).wrapping_add(c.wrapping_mul(0xc2b2_ae3d))));
    let (a, b) = ((h & 0xffff) as f32 * (1.0 / 65536.0), (h >> 16) as f32 * (1.0 / 65536.0));
    a + b - 1.0
}

/// Add the dither to the linear glow `g` at pixel `(x, y)`. The noise is one 8-bit step of the
/// encoded value, expressed in linear light (`γ·g^(1−1/γ)/255`), so it is invisible in the dark
/// and never lifts black.
pub fn dither_glow(g: [f32; 3], x: u32, y: u32, d: &Dither) -> [f32; 3] {
    if !d.on {
        return g;
    }
    let gamma = d.gamma.max(0.1);
    let mut out = g;
    for (c, v) in out.iter_mut().enumerate() {
        if *v <= 0.0 {
            continue;
        }
        let step = gamma * v.max(1e-6).powf(1.0 - 1.0 / gamma) * (1.0 / 255.0);
        let n = tpdf(x, y, if d.mono { 0 } else { c as u32 }, d.seed);
        *v = (*v + n * step).max(0.0);
    }
    out
}

/// Lens Dirt settings.
#[derive(Clone, Copy, Debug)]
pub struct Dirt {
    pub opacity: f32,
    pub inherit: f32,
    pub exposure: f32,
    pub gamma: f32,
    pub repeat_edges: bool,
}

/// The dirt texture's matte over the buffer: texture luminance × alpha, scaled by
/// `2^Matte Exposure` and raised to Matte Gamma. The texture stretches over the layer; outside
/// it Repeat Edges extends its border pixels, otherwise there is no dirt.
pub fn dirt_matte(ctx: &EffectCtx, b: &Buf, tex: &LayerPixels, d: &Dirt) -> Vec<f32> {
    let ls = ctx.layer_size;
    let os = tex.size;
    let (sx, sy) = if ls[0] > 0.0 && ls[1] > 0.0 { (os[0] / ls[0], os[1] / ls[1]) } else { (1.0, 1.0) };
    let inv = 1.0 / b.scale.max(1e-9);
    let gain = d.exposure.exp2();
    let (w, h) = (b.img.width as usize, b.img.height as usize);
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w.max(1)).enumerate().for_each(|(y, row)| {
        for (x, v) in row.iter_mut().enumerate() {
            let px = (x as f64 + 0.5 - b.offset[0]) * inv * sx * tex.buf.scale + tex.buf.offset[0];
            let py = (y as f64 + 0.5 - b.offset[1]) * inv * sy * tex.buf.scale + tex.buf.offset[1];
            let s = if d.repeat_edges { tex.buf.img.sample_bilinear_clamped(px, py) } else { tex.buf.img.sample_bilinear(px, py) };
            let a = s[3].clamp(0.0, 1.0);
            let lum = if a > 1e-6 { luma([s[0] / a, s[1] / a, s[2] / a]) * a } else { 0.0 };
            *v = (lum.max(0.0) * gain).powf(d.gamma.max(0.01)).min(1.0);
        }
    });
    out
}

/// Add the dirt texture's light to the glow: only where the glow is superbright (the texture
/// lives in the glare, not over the whole frame), tinted by the glow's own colour by `inherit`.
pub fn apply_dirt(g: [f32; 3], matte: f32, d: &Dirt) -> [f32; 3] {
    if d.opacity <= 0.0 || matte <= 0.0 {
        return g;
    }
    let l = luma(g);
    let k = d.opacity * matte * 4.0 * (l / (l + 0.25));
    let base = mix3([l; 3], g, d.inherit.clamp(0.0, 1.0));
    [g[0] + k * base[0], g[1] + k * base[1], g[2] + k * base[2]]
}

/// Pixel Aberration channel choices: (first channel moves out, second moves in); `None` = unused.
pub const PIXEL_CHANNELS: [&str; 6] = ["Red & Blue", "Red & Green", "Green & Blue", "Red", "Green", "Blue"];

fn channel_pair(choice: u32) -> (Option<usize>, Option<usize>) {
    match choice {
        0 => (Some(0), Some(2)),
        1 => (Some(0), Some(1)),
        2 => (Some(1), Some(2)),
        3 => (Some(0), None),
        4 => (Some(1), None),
        _ => (Some(2), None),
    }
}

/// Pixel Aberration: the chosen channels of `g` shift radially from the buffer centre, the
/// first outward and the second inward, by `offset` pixels at the corners (nothing at the centre).
pub fn pixel_aberration(g: &Image, choice: u32, offset: f32) -> Image {
    let (w, h) = (g.width as f64, g.height as f64);
    let (cx, cy) = (w * 0.5, h * 0.5);
    let half_diag = (cx * cx + cy * cy).sqrt().max(1.0);
    let (up, down) = channel_pair(choice);
    let mut out = g.clone();
    out.rows_mut().for_each(|(y, row)| {
        for (x, px) in row.iter_mut().enumerate() {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let k = f64::from(offset) / half_diag;
            for (ch, sign) in [(up, 1.0), (down, -1.0)] {
                if let Some(c) = ch {
                    let s = g.sample_bicubic(x as f64 + 0.5 - dx * k * sign, y as f64 + 0.5 - dy * k * sign);
                    px[c] = s[c].max(0.0);
                }
            }
        }
    });
    out
}

/// How the glow meets the source.
#[derive(Clone, Copy, Debug)]
pub struct Compose {
    /// Used when neither `screen` nor `add` is set.
    pub mode: BlendMode,
    pub screen: bool,
    pub add: bool,
    pub opacity: f32,
    pub unmult: bool,
    pub gamma: f32,
}

#[inline]
fn pow_pos(v: f32, g: f32) -> f32 {
    if (g - 1.0).abs() < 1e-4 { v.max(0.0) } else { v.max(0.0).powf(g) }
}

/// Composite linear premultiplied glow `g` (RGB) with the encoded premultiplied source `s`,
/// returning the encoded result. Unmult generates the alpha of the glow from its brightness (a
/// glow over transparency is visible); without it the result is opaque, glow over black.
pub fn compose(s: Px, g: [f32; 3], c: &Compose) -> Px {
    // Not-a-number and infinite source values are treated as black.
    let s = s.map(|v| if v.is_finite() { v } else { 0.0 });
    let sa = (s[3].clamp(0.0, 1.0)) * c.opacity;
    // Source in linear light (premultiplied).
    let src = if s[3] > 1e-6 { [0, 1, 2].map(|i| pow_pos(s[i] / s[3], c.gamma) * sa) } else { [0.0; 3] };
    let ag = {
        let m = g[0].max(g[1]).max(g[2]);
        pow_pos(m, 1.0 / c.gamma).clamp(0.0, 1.0)
    };
    let (rgb, alpha) = if c.screen {
        ([0, 1, 2].map(|i| src[i] + g[i] * (1.0 - src[i].clamp(0.0, 1.0))), sa + ag - sa * ag)
    } else if c.add {
        ([src[0] + g[0], src[1] + g[1], src[2] + g[2]], sa + ag - sa * ag)
    } else {
        let o = blend_pixel(c.mode, [src[0], src[1], src[2], sa], [g[0], g[1], g[2], ag], 0.0);
        ([o[0], o[1], o[2]], o[3])
    };
    let alpha = if c.unmult { alpha.clamp(0.0, 1.0) } else { 1.0 };
    if alpha <= 1e-6 {
        return [0.0; 4];
    }
    let inv = 1.0 / alpha;
    let enc = [0, 1, 2].map(|i| pow_pos(rgb[i] * inv, 1.0 / c.gamma) * alpha);
    [enc[0], enc[1], enc[2], alpha]
}

/// Linear straight-to-encoded helper used for the Glow Input view: encode premultiplied linear.
pub fn encode_premul(p: Px, gamma: f32) -> Px {
    let a = p[3];
    if a <= 1e-6 {
        return [0.0; 4];
    }
    let inv = 1.0 / a;
    let e = [0, 1, 2].map(|i| pow_pos(p[i] * inv, 1.0 / gamma) * a);
    [e[0], e[1], e[2], a]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(screen: bool, add: bool) -> Compose {
        Compose { mode: BlendMode::Normal, screen, add, opacity: 1.0, unmult: true, gamma: 1.0 }
    }

    #[test]
    fn no_glow_leaves_the_source() {
        let s = [0.25, 0.5, 0.75, 1.0];
        for (sc, ad) in [(true, false), (false, true)] {
            let o = compose(s, [0.0; 3], &cmp(sc, ad));
            assert!((0..4).all(|i| (o[i] - s[i]).abs() < 1e-6), "{o:?}");
        }
        // Through a gamma the round trip stays exact enough.
        let c = Compose { gamma: 2.2, ..cmp(true, false) };
        let o = compose(s, [0.0; 3], &c);
        assert!((0..4).all(|i| (o[i] - s[i]).abs() < 1e-5), "{o:?}");
    }

    #[test]
    fn screen_never_clips_hdr_but_add_does_not_either() {
        let o = compose([0.5, 0.5, 0.5, 1.0], [3.0, 3.0, 3.0], &cmp(true, false));
        assert!(o[0] >= 0.5 + 3.0 * 0.5 - 1e-5, "{o:?}");
        let o = compose([0.5, 0.5, 0.5, 1.0], [3.0, 3.0, 3.0], &cmp(false, true));
        assert!((o[0] - 3.5).abs() < 1e-5);
    }

    #[test]
    fn unmult_makes_a_glow_over_transparency_visible() {
        let o = compose([0.0; 4], [0.4, 0.2, 0.1], &cmp(true, false));
        assert!(o[3] > 0.39 && (o[0] - 0.4).abs() < 1e-5, "{o:?}");
        let c = Compose { unmult: false, ..cmp(true, false) };
        assert_eq!(compose([0.0; 4], [0.4, 0.2, 0.1], &c)[3], 1.0);
    }

    #[test]
    fn source_opacity_removes_the_source() {
        let c = Compose { opacity: 0.0, ..cmp(true, false) };
        let o = compose([1.0, 1.0, 1.0, 1.0], [0.2, 0.2, 0.2], &c);
        assert!((o[0] - 0.2).abs() < 1e-5 && (o[3] - 0.2).abs() < 1e-5, "{o:?}");
    }

    #[test]
    fn tint_modes() {
        let g = [0.5, 0.5, 0.5];
        let red = [1.0, 0.0, 0.0];
        let mut t = Tint { mode: 1, blend: 0, inner: red, outer: [0.0, 0.0, 1.0], strength: 1.0 };
        assert_eq!(apply_tint(g, &t), [0.5, 0.0, 0.0]);
        t.strength = 0.5;
        assert!((apply_tint(g, &t)[0] - 0.5).abs() < 1e-6 && (apply_tint(g, &t)[1] - 0.25).abs() < 1e-6);
        t.mode = 0;
        assert_eq!(apply_tint(g, &t), g);
        // Two colours: a bright glow takes the inner colour, a faint one the outer.
        let t = Tint { mode: 2, blend: 0, inner: red, outer: [0.0, 0.0, 1.0], strength: 1.0 };
        let bright = apply_tint([50.0; 3], &t);
        let faint = apply_tint([0.001; 3], &t);
        assert!(bright[0] > bright[2] * 10.0 && faint[2] > faint[0] * 10.0, "{bright:?} {faint:?}");
        // Normal keeps the brightness.
        let t = Tint { mode: 1, blend: 1, inner: red, outer: red, strength: 1.0 };
        assert!((luma(apply_tint(g, &t)) - luma(g)).abs() < 1e-5);
    }

    #[test]
    fn dirt_only_lights_superbright_glow() {
        let d = Dirt { opacity: 1.0, inherit: 1.0, exposure: 0.0, gamma: 1.0, repeat_edges: true };
        let dim = apply_dirt([0.001; 3], 1.0, &d);
        let hot = apply_dirt([4.0; 3], 1.0, &d);
        assert!(dim[0] / 0.001 < 1.05, "{dim:?}");
        assert!(hot[0] / 4.0 > 2.5, "{hot:?}");
        assert_eq!(apply_dirt([4.0; 3], 0.0, &d), [4.0; 3]);
    }

    #[test]
    fn pixel_aberration_shifts_the_chosen_channels_only() {
        let mut img = Image::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let v = if (40..44).contains(&x) { 1.0 } else { 0.0 };
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let o = pixel_aberration(&img, 0, 6.0);
        // Green is untouched; red and blue move in opposite directions along x (right of centre).
        assert_eq!((0..64).map(|x| o.get(x, 32)[1]).collect::<Vec<_>>(), (0..64).map(|x| img.get(x, 32)[1]).collect::<Vec<_>>());
        let centroid = |c: usize| {
            let (s, w): (f32, f32) = (0..64).map(|x| (x as f32 * o.get(x, 32)[c], o.get(x, 32)[c])).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
            s / w
        };
        assert!(centroid(0) > centroid(1) + 0.3 && centroid(2) < centroid(1) - 0.3, "{} {} {}", centroid(0), centroid(1), centroid(2));
    }

    #[test]
    fn overlay_and_soft_light_with_a_mid_grey_tint_change_nothing() {
        for blend in [4, 5] {
            let t = Tint { mode: 1, blend, inner: [0.5; 3], outer: [0.5; 3], strength: 1.0 };
            for g in [[0.1, 0.3, 0.45], [0.6, 0.9, 1.0], [2.0, 5.0, 0.0]] {
                let o = apply_tint(g, &t);
                for i in 0..3 {
                    assert!((o[i] - g[i]).abs() < 1e-5, "blend {blend}: {g:?} -> {o:?}");
                }
            }
        }
    }

    #[test]
    fn overlay_and_soft_light_darken_and_lighten_with_the_tint() {
        for blend in [4, 5] {
            let dark = Tint { mode: 1, blend, inner: [0.1; 3], outer: [0.1; 3], strength: 1.0 };
            let light = Tint { mode: 1, blend, inner: [0.9; 3], outer: [0.9; 3], strength: 1.0 };
            let g = [0.4, 0.4, 0.4];
            assert!(apply_tint(g, &dark)[0] < g[0] && apply_tint(g, &light)[0] > g[0], "blend {blend}");
            // HDR light stays above 1 and is scaled by the tint.
            assert!(apply_tint([4.0; 3], &light)[0] > apply_tint([4.0; 3], &dark)[0]);
            assert!(apply_tint([4.0, 0.0, 0.0], &light).iter().all(|v| v.is_finite() && *v >= 0.0));
        }
    }

    #[test]
    fn dither_is_zero_mean_bounded_and_mono_when_asked() {
        let d = Dither { on: true, mono: true, gamma: 2.2, seed: 7 };
        let g = [0.2f32, 0.2, 0.2];
        let step = 2.2 * 0.2f32.powf(1.0 - 1.0 / 2.2) / 255.0;
        let (mut sum, mut worst) = (0.0f64, 0.0f32);
        let n = 20_000u32;
        for i in 0..n {
            let o = dither_glow(g, i % 200, i / 200, &d);
            assert_eq!(o[0], o[1]);
            assert_eq!(o[1], o[2]);
            sum += f64::from(o[0] - g[0]);
            worst = worst.max((o[0] - g[0]).abs());
        }
        assert!(worst <= step * 1.0001, "one step at most: {worst} vs {step}");
        assert!((sum / f64::from(n)).abs() < f64::from(step) * 0.02, "zero mean: {}", sum / f64::from(n));
        // Not mono: the channels differ.
        let c = Dither { mono: false, ..d };
        let o = dither_glow(g, 3, 4, &c);
        assert!(o[0] != o[1] || o[1] != o[2]);
        // Black stays black, off is exact, the seed moves the pattern.
        assert_eq!(dither_glow([0.0; 3], 1, 1, &d), [0.0; 3]);
        assert_eq!(dither_glow(g, 1, 1, &Dither { on: false, ..d }), g);
        assert!(dither_glow(g, 1, 1, &d) != dither_glow(g, 1, 1, &Dither { seed: 8, ..d }));
    }
}
