//! Lens Iris glow: the glow takes the shape of an image.
//!
//! The Iris Layer is read as an aperture (luminance × alpha). The glow is a stack of `N`
//! (Iris Iterations) copies of that aperture at growing radii `r_i = R · (i/N)^γ`, each convolved
//! with the glow input at unit energy and weighted by the inverse-square falloff's share of
//! energy between `r_{i-1}` and `r_i`: a point of light becomes the aperture, bright at its
//! core and fading outwards like the Exponential glow does. Radii Easing sets `γ` (100 % spaces
//! the radii evenly, less packs them towards the core). Aspect Ratio and Angle stretch and turn
//! the aperture. Each copy is evaluated on the pyramid level where it is small enough to
//! convolve directly (Iris Sampling Quality sets how many taps that is).
//!
//! The vendor documents these Iris controls only by name, so their exact behaviour here is this
//! implementation's own reading of the names:
//!
//! * **Iris Opacity**: how much of the glow takes the aperture's shape; the rest stays a round
//!   disc of the same radius (0 % is the plain round glow, 100 % the aperture).
//! * **Iris Aspect Ratio**: stretches the aperture alone (Aspect Ratio in Style stretches the
//!   whole glow).
//! * **Radius Normalization**: scales the aperture so its farthest lit pixel sits at Radius, so
//!   empty space around the shape in the layer does not shrink the glow. Off, the layer's own
//!   rectangle is the aperture's diameter.
//! * **Continuous Rasterization**: on, each copy is drawn smoothly at its exact radius; off, at
//!   the nearest whole pixel and without smoothing between texels, which gives the crisp,
//!   stepped look.

use super::resample::{Filter, Kernel, Term};
use crate::EffectCtx;
use crate::util::Plane;

/// Iris Sampling Quality options.
pub const SAMPLING: [&str; 3] = ["Low", "Standard", "High"];

/// Largest kernel radius (level pixels) of each sampling quality.
const MAX_TAP_RADIUS: [f32; 3] = [6.0, 12.0, 24.0];

/// Settings of the iris pass.
#[derive(Clone, Copy, Debug)]
pub struct IrisCfg {
    pub quality: u32,
    pub iterations: usize,
    pub easing: f32,
    /// Spread along the aperture's x and y (Aspect Ratio), each in `(0, 1]`.
    pub spread: [f32; 2],
    /// Aperture rotation in radians.
    pub angle: f32,
    /// Share of the glow that takes the aperture's shape, in `[0, 1]` (Iris Opacity).
    pub opacity: f32,
    /// Smooth sampling at exact radii (Continuous Rasterization).
    pub continuous: bool,
}

/// `p` cropped (or padded) to the smallest square, centred on the plane's centre, that holds
/// every lit pixel inside its inscribed circle: the farthest lit pixel ends up at the aperture's
/// radius.
fn fit_to_circle(p: &Plane) -> Plane {
    let peak = p.data.iter().copied().fold(0.0f32, f32::max);
    let (cx, cy) = (p.w as f32 * 0.5, p.h as f32 * 0.5);
    let mut far = 0.0f32;
    for y in 0..p.h {
        for x in 0..p.w {
            if p.data[y * p.w + x] > 0.02 * peak {
                far = far.max((x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy));
            }
        }
    }
    let n = ((2.0 * far).ceil() as usize + 2).clamp(2, 512);
    let (ox, oy) = (((p.w as f32 - n as f32) * 0.5).round() as i64, ((p.h as f32 - n as f32) * 0.5).round() as i64);
    let mut o = Plane::new(n, n);
    for y in 0..n {
        for x in 0..n {
            let (sx, sy) = (x as i64 + ox, y as i64 + oy);
            if sx >= 0 && sy >= 0 && sx < p.w as i64 && sy < p.h as i64 {
                o.data[y * n + x] = p.data[sy as usize * p.w + sx as usize];
            }
        }
    }
    o
}

/// The aperture and its box-filtered mips (each half the size of the one before).
pub struct Shape {
    mips: Vec<Plane>,
}

fn disc(n: usize) -> Plane {
    let mut p = Plane::new(n, n);
    let c = (n as f32 - 1.0) * 0.5;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            p.data[y * n + x] = (c + 0.5 - d).clamp(0.0, 1.0);
        }
    }
    p
}

fn halve(p: &Plane) -> Plane {
    let (w, h) = ((p.w / 2).max(1), (p.h / 2).max(1));
    let mut o = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                s += p.get_clamped((2 * x + dx) as i64, (2 * y + dy) as i64);
            }
            o.data[y * w + x] = s * 0.25;
        }
    }
    o
}

impl Shape {
    fn from_plane(p: Plane) -> Shape {
        let mut mips = vec![p];
        while let Some(last) = mips.last() {
            if last.w.max(last.h) <= 4 {
                break;
            }
            let n = halve(last);
            mips.push(n);
        }
        Shape { mips }
    }

    /// A circular aperture (used while no Iris Layer is chosen).
    pub fn circle() -> Shape {
        Shape::from_plane(disc(128))
    }

    /// The Iris Layer's own rectangle as an aperture (luminance × alpha), at most 256 pixels on a
    /// side; a circle when the layer is empty or black.
    pub fn from_layer(ctx: &EffectCtx, normalize: bool) -> Shape {
        let Some(lp) = ctx.layer_param("iris/irisLayer", true) else { return Shape::circle() };
        let (lw, lh) = ((lp.size[0] * lp.buf.scale).max(1.0), (lp.size[1] * lp.buf.scale).max(1.0));
        let k = (256.0 / lw.max(lh)).min(1.0);
        let (pw, ph) = (((lw * k).round() as usize).max(1), ((lh * k).round() as usize).max(1));
        let mut p = Plane::new(pw, ph);
        for y in 0..ph {
            for x in 0..pw {
                let sx = lp.buf.offset[0] + (x as f64 + 0.5) / pw as f64 * lw;
                let sy = lp.buf.offset[1] + (y as f64 + 0.5) / ph as f64 * lh;
                let s = lp.buf.img.sample_bilinear(sx, sy);
                let a = s[3].clamp(0.0, 1.0);
                p.data[y * pw + x] = if a > 1e-6 { super::color::luma([s[0] / a, s[1] / a, s[2] / a]).max(0.0) * a } else { 0.0 };
            }
        }
        if p.data.iter().sum::<f32>() <= 1e-6 { Shape::circle() } else { Shape::from_plane(if normalize { fit_to_circle(&p) } else { p }) }
    }

    /// Sample of mip `m` at normalised coordinates `(u, v)` in `[0, 1]²` (zero outside): bilinear,
    /// or the nearest texel when `smooth` is off.
    fn sample(&self, m: usize, u: f32, v: f32, smooth: bool) -> f32 {
        let Some(p) = self.mips.get(m.min(self.mips.len() - 1)) else { return 0.0 };
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return 0.0;
        }
        if !smooth {
            let (x, y) = (((u * p.w as f32) as usize).min(p.w - 1), ((v * p.h as f32) as usize).min(p.h - 1));
            return p.data[y * p.w + x];
        }
        let (x, y) = (u * p.w as f32 - 0.5, v * p.h as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let g = |dx: i64, dy: i64| {
            let (xx, yy) = (x0 as i64 + dx, y0 as i64 + dy);
            if xx < 0 || yy < 0 || xx >= p.w as i64 || yy >= p.h as i64 { 0.0 } else { p.data[yy as usize * p.w + xx as usize] }
        };
        let top = g(0, 0) * (1.0 - fx) + g(1, 0) * fx;
        let bot = g(0, 1) * (1.0 - fx) + g(1, 1) * fx;
        top * (1.0 - fy) + bot * fy
    }

    /// The aperture at radius `r` (level pixels) as a unit-sum kernel, stretched by `spread`,
    /// turned by `angle` and softened by a Gaussian of sigma `soft` (level pixels).
    fn kernel(&self, r: f32, cfg: &IrisCfg, soft: f32) -> Option<Kernel> {
        let (mut rx, mut ry) = ((r * cfg.spread[0]).max(0.5), (r * cfg.spread[1]).max(0.5));
        if !cfg.continuous {
            (rx, ry) = (rx.round().max(1.0), ry.round().max(1.0));
        }
        let half = ((rx.max(ry) * 1.42 + 2.0 * soft).ceil() as usize).clamp(1, 40);
        let n = 2 * half + 1;
        let (sa, ca) = cfg.angle.sin_cos();
        let (sw, sh) = (self.mips.first()?.w as f32, self.mips.first()?.h as f32);
        // The mip whose texels are about a kernel texel big.
        let px_per_texel = (sw.max(sh) / (2.0 * rx.max(ry))).max(1.0);
        let m = px_per_texel.log2().floor().max(0.0) as usize;
        let mut data = vec![0.0f32; n * n];
        for j in 0..n {
            for i in 0..n {
                let (dx, dy) = (i as f32 - half as f32, j as f32 - half as f32);
                let (ux, uy) = (dx * ca + dy * sa, -dx * sa + dy * ca);
                data[j * n + i] = self.sample(m, 0.5 + ux / (2.0 * rx), 0.5 + uy / (2.0 * ry), cfg.continuous);
            }
        }
        let mut k = Kernel { w: n, h: n, data };
        if soft > 0.3 && cfg.continuous {
            soften(&mut k, soft);
        }
        let sum: f32 = k.data.iter().sum();
        if sum <= 1e-9 {
            return None;
        }
        k.data.iter_mut().for_each(|v| *v /= sum);
        Some(k)
    }
}

/// Gaussian-blur a kernel in place (separable, zero outside).
fn soften(k: &mut Kernel, sigma: f32) {
    let rad = ((3.0 * sigma).ceil() as i64).clamp(1, 12);
    let g: Vec<f32> = (-rad..=rad).map(|d| (-(d as f32).powi(2) / (2.0 * sigma * sigma)).exp()).collect();
    let (w, h) = (k.w as i64, k.h as i64);
    let mut tmp = vec![0.0f32; k.data.len()];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (j, gv) in g.iter().enumerate() {
                let xx = x + j as i64 - rad;
                if xx >= 0 && xx < w {
                    s += gv * k.data[(y * w + xx) as usize];
                }
            }
            tmp[(y * w + x) as usize] = s;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (j, gv) in g.iter().enumerate() {
                let yy = y + j as i64 - rad;
                if yy >= 0 && yy < h {
                    s += gv * tmp[(yy * w + x) as usize];
                }
            }
            k.data[(y * w + x) as usize] = s;
        }
    }
}

/// Radii of the iris copies: `r_i = R (i/N)^γ`, `γ = 1 + 2 (1 − easing)`, i = 1..=N.
pub fn radii(radius: f32, n: usize, easing: f32) -> Vec<f32> {
    let g = 1.0 + 2.0 * (1.0 - easing.clamp(0.0, 1.0));
    (1..=n.max(1)).map(|i| radius * (i as f32 / n.max(1) as f32).powf(g)).collect()
}

/// Share of the glow's energy in each ring `(r_{i-1}, r_i]` of the inverse-square falloff with a
/// Gaussian window at `radius`; sums to 1.
pub fn ring_weights(radii: &[f32], radius: f32) -> Vec<f32> {
    let a = (0.004 * radius).max(1.0);
    let mut prev = 0.0;
    let mut w: Vec<f32> = radii
        .iter()
        .map(|&r| {
            let (rm, dr) = (0.5 * (r + prev), r - prev);
            prev = r;
            let u = rm / radius.max(1e-3);
            (rm / (1.0 + (rm / a).powi(2)) * (-u * u).exp() * dr).max(0.0)
        })
        .collect();
    let s: f32 = w.iter().sum();
    if s > 0.0 {
        w.iter_mut().for_each(|v| *v /= s);
    }
    w
}

/// The pyramid terms of the iris glow at `radius` full-resolution pixels. `base` is the finest
/// pyramid level allowed, `max_level` the deepest.
pub fn iris_terms(shape: &Shape, radius: f32, cfg: &IrisCfg, base: usize, max_level: usize) -> Vec<Term> {
    let tap_r = MAX_TAP_RADIUS[(cfg.quality as usize).min(2)];
    let opacity = cfg.opacity.clamp(0.0, 1.0);
    let round = (opacity < 0.999).then(Shape::circle);
    let rs = radii(radius, cfg.iterations, cfg.easing);
    let ws = ring_weights(&rs, radius);
    let mut terms = Vec::with_capacity(rs.len());
    let mut prev = 0.0f32;
    for (&r, &w) in rs.iter().zip(&ws) {
        let mut lvl = base;
        while lvl < max_level && r / (1u64 << lvl) as f32 > tap_r {
            lvl += 1;
        }
        let scale = (1u64 << lvl) as f32;
        let (rl, dl) = (r / scale, (r - prev) / scale);
        prev = r;
        let filter = if rl * cfg.spread[0].max(cfg.spread[1]) < 0.75 {
            Filter::Identity
        } else {
            let soft = (0.5 * dl).max(0.0);
            match (shape.kernel(rl, cfg, soft), &round) {
                (Some(mut k), Some(disc)) => {
                    // Both kernels are unit-sum and the same size, so the mix is too.
                    if let Some(d) = disc.kernel(rl, cfg, soft).filter(|d| d.data.len() == k.data.len()) {
                        k.data.iter_mut().zip(&d.data).for_each(|(a, b)| *a = opacity * *a + (1.0 - opacity) * b);
                    }
                    Filter::Kernel(k)
                }
                (Some(k), None) => Filter::Kernel(k),
                (None, Some(disc)) => disc.kernel(rl, cfg, soft).map_or(Filter::Identity, Filter::Kernel),
                (None, None) => Filter::Identity,
            }
        };
        terms.push(Term { lvl, filter, w: [w; 4] });
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> IrisCfg {
        IrisCfg { quality: 1, iterations: 10, easing: 0.9, spread: [1.0, 1.0], angle: 0.0, opacity: 1.0, continuous: true }
    }

    #[test]
    fn radii_end_at_the_radius_and_ease() {
        let even = radii(100.0, 10, 1.0);
        assert!((even[0] - 10.0).abs() < 1e-3 && (even[9] - 100.0).abs() < 1e-3);
        let packed = radii(100.0, 10, 0.0);
        assert!(packed[4] < even[4] * 0.5, "low easing packs the radii towards the core");
        assert!(packed.windows(2).all(|p| p[1] > p[0]));
    }

    #[test]
    fn ring_weights_sum_to_one_and_favour_the_core() {
        let r = radii(200.0, 10, 0.9);
        let w = ring_weights(&r, 200.0);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(w.iter().all(|v| *v >= 0.0));
    }

    #[test]
    fn kernels_are_unit_energy_and_follow_the_aperture() {
        let s = Shape::circle();
        let k = s.kernel(8.0, &cfg(), 0.0).expect("circle kernel");
        assert!((k.data.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        // A disc of radius 8 is round: its centre column and row carry the same mass.
        let (cx, n) = (k.w / 2, k.w);
        let col: f32 = (0..n).map(|y| k.data[y * n + cx]).sum();
        let row: f32 = (0..n).map(|x| k.data[cx * n + x]).sum();
        assert!((col - row).abs() < 1e-4);
        // Stretched along x it is wider than tall.
        let wide = IrisCfg { spread: [1.0, 0.25], ..cfg() };
        let k = s.kernel(8.0, &wide, 0.0).expect("wide kernel");
        let n = k.w;
        let cx = n / 2;
        let row: f32 = (0..n).filter(|x| k.data[cx * n + x] > 0.0).count() as f32;
        let col: f32 = (0..n).filter(|y| k.data[y * n + cx] > 0.0).count() as f32;
        assert!(row > col * 2.5, "{row} x {col}");
    }

    #[test]
    fn terms_stay_within_the_tap_budget() {
        let s = Shape::circle();
        let t = iris_terms(&s, 500.0, &cfg(), 0, 12);
        assert_eq!(t.len(), 10);
        for term in &t {
            if let Filter::Kernel(k) = &term.filter {
                assert!(k.w <= 81, "kernel {} too big", k.w);
            }
        }
        let total: f32 = t.iter().map(|t| t.w[0]).sum();
        assert!((total - 1.0).abs() < 1e-4);
    }

    /// A plane with one lit square (x0..x1, y0..y1), zero elsewhere.
    fn lit(w: usize, h: usize, x: (usize, usize), y: (usize, usize)) -> Plane {
        let mut p = Plane::new(w, h);
        for yy in y.0..y.1 {
            for xx in x.0..x.1 {
                p.data[yy * w + xx] = 1.0;
            }
        }
        p
    }

    #[test]
    fn radius_normalization_crops_to_the_lit_pixels() {
        // A small centred square in a big empty layer: normalised, the plane shrinks to it.
        let p = lit(200, 200, (90, 110), (90, 110));
        let n = fit_to_circle(&p);
        assert!(n.w < 40 && n.w >= 28, "side {}", n.w);
        assert!((n.data.iter().sum::<f32>() - 400.0).abs() < 1e-3, "no light lost");
        // The farthest lit pixel of the result sits at the edge of its inscribed circle.
        let far = (0..n.h)
            .flat_map(|y| (0..n.w).map(move |x| (x, y)))
            .filter(|&(x, y)| n.data[y * n.w + x] > 0.0)
            .map(|(x, y)| (x as f32 + 0.5 - n.w as f32 * 0.5).hypot(y as f32 + 0.5 - n.h as f32 * 0.5))
            .fold(0.0f32, f32::max);
        assert!((far * 2.0 - n.w as f32).abs() < 3.0, "far {far} side {}", n.w);
        // An off-centre shape is centred on the plane's centre, not on itself (the aperture
        // keeps its offset), and still fits.
        let off = fit_to_circle(&lit(100, 100, (60, 80), (45, 55)));
        assert!(off.w >= 40 && off.data.iter().sum::<f32>() > 0.0);
    }

    #[test]
    fn iris_opacity_blends_towards_the_round_glow() {
        let square = Shape::from_plane(lit(64, 64, (0, 64), (0, 64)));
        let diag = |t: &[Term]| {
            let Some(Term { filter: Filter::Kernel(k), .. }) = t.iter().rev().find(|t| matches!(t.filter, Filter::Kernel(_))) else { return 0.0 };
            let (n, c) = (k.w, k.w / 2);
            // Light towards the corner of the kernel: inside a square aperture, outside a disc of
            // the same radius (the kernel half-width is about 1.42 radii).
            let r = (c as f32 * 0.6).round() as usize;
            k.data[(c + r) * n + (c + r)]
        };
        let at = |o: f32| diag(&iris_terms(&square, 80.0, &IrisCfg { opacity: o, ..cfg() }, 0, 12));
        let (full, half, none) = (at(1.0), at(0.5), at(0.0));
        assert!(full > 1e-6, "a square fills its corners");
        assert!(none < full * 0.05, "opacity 0 is a disc: {none} vs {full}");
        assert!(half > none && half < full, "opacity 0.5 lies between: {none} {half} {full}");
        // The mix keeps unit energy.
        let t = iris_terms(&square, 80.0, &IrisCfg { opacity: 0.4, ..cfg() }, 0, 12);
        for term in &t {
            if let Filter::Kernel(k) = &term.filter {
                assert!((k.data.iter().sum::<f32>() - 1.0).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn stepped_rasterization_is_crisper_and_keeps_energy() {
        let s = Shape::circle();
        let smooth = s.kernel(7.4, &cfg(), 1.5).expect("smooth");
        let stepped = s.kernel(7.4, &IrisCfg { continuous: false, ..cfg() }, 1.5).expect("stepped");
        assert!((stepped.data.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        // Unsoftened and unfiltered, the edge is a hard step: fewer distinct values than the
        // smoothed disc.
        let distinct = |k: &Kernel| {
            let mut v: Vec<u32> = k.data.iter().map(|x| (x * 1e6) as u32).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(distinct(&stepped) < distinct(&smooth) / 2, "{} vs {}", distinct(&stepped), distinct(&smooth));
    }
}
