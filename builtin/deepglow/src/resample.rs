//! Separable resampling with tap tables and the image pyramids that sum filtered levels.
//!
//! Every pass is a sum of `k` taps per output sample (a [`Tab`]); the same two passes do the
//! 6-tap binomial down-sample, the cubic B-spline up-sample and the Gaussian blur of one pyramid
//! level. [`pyramid2d_sum`] has an independent level on each axis (anisotropic glow);
//! [`pyramid_sum`] has one level for both and may convolve with a kernel image (Lens Iris).

use std::collections::HashMap;

use effectcraft_raster::Image;
use rayon::prelude::*;

/// Taps of every output sample of one axis: `idx` / `w` hold `k` entries per output.
#[derive(Clone, Debug)]
pub struct Tab {
    pub dn: usize,
    pub k: usize,
    idx: Vec<u32>,
    w: Vec<f32>,
}

impl Tab {
    fn new(dn: usize, k: usize) -> Tab {
        Tab { dn, k, idx: vec![0; dn * k], w: vec![0.0; dn * k] }
    }

    fn set(&mut self, o: usize, k: usize, i: i64, sn: usize, w: f32) {
        let ok = i >= 0 && (i as usize) < sn;
        let at = o * self.k + k;
        self.idx[at] = if ok { i as u32 } else { 0 };
        self.w[at] = if ok { w } else { 0.0 };
    }

    /// Output = input.
    pub fn ident(n: usize) -> Tab {
        let mut t = Tab::new(n, 1);
        for o in 0..n {
            t.idx[o] = o as u32;
            t.w[o] = 1.0;
        }
        t
    }

    /// Gaussian of sigma `sl` samples, zero outside; the identity when `sl` is not positive.
    pub fn gauss(n: usize, sl: f32) -> Tab {
        if sl <= 0.0 {
            return Tab::ident(n);
        }
        let rad = ((3.3 * sl).ceil() as i64).clamp(1, 12);
        let k = (2 * rad + 1) as usize;
        let mut g: Vec<f32> = (0..k as i64).map(|j| (-((j - rad) as f32).powi(2) / (2.0 * sl * sl)).exp()).collect();
        let s: f32 = g.iter().sum();
        g.iter_mut().for_each(|v| *v /= s);
        let mut t = Tab::new(n, k);
        for o in 0..n {
            for (j, gv) in g.iter().enumerate() {
                t.set(o, j, o as i64 + j as i64 - rad, n, *gv);
            }
        }
        t
    }

    /// 6-tap binomial `[1 5 10 10 5 1] / 32`, cell centred; a plain `[1 1] / 2` when the source
    /// axis is tiny (it keeps the mass).
    pub fn down2(sn: usize, dn: usize) -> Tab {
        const B6: [f32; 6] = [1.0 / 32.0, 5.0 / 32.0, 10.0 / 32.0, 10.0 / 32.0, 5.0 / 32.0, 1.0 / 32.0];
        let mut t = Tab::new(dn, 6);
        for o in 0..dn {
            for k in 0..6 {
                let (i, w) =
                    if sn <= 8 { (2 * o as i64 + k as i64 - 2, if k == 2 || k == 3 { 0.5 } else { 0.0 }) } else { (2 * o as i64 - 2 + k as i64, B6[k]) };
                t.set(o, k, i, sn, w);
            }
        }
        t
    }

    /// Exact cubic B-spline sampling at the 1/4 and 3/4 offsets (2× refinement), clamped to the edge.
    pub fn up2(sn: usize, dn: usize) -> Tab {
        const WA: [f32; 4] = [1.0 / 384.0, 121.0 / 384.0, 235.0 / 384.0, 27.0 / 384.0];
        const WB: [f32; 4] = [27.0 / 384.0, 235.0 / 384.0, 121.0 / 384.0, 1.0 / 384.0];
        let mut t = Tab::new(dn, 4);
        for o in 0..dn {
            let (n, par) = ((o >> 1) as i64, o & 1);
            for k in 0..4 {
                let i = (if par == 1 { n - 1 } else { n - 2 } + k as i64).clamp(0, sn as i64 - 1);
                t.set(o, k, i, sn, if par == 1 { WB[k] } else { WA[k] });
            }
        }
        t
    }
}

/// Horizontal pass: `src` (w × h) → (`t.dn` × h).
pub fn pass_h(src: &Image, t: &Tab) -> Image {
    let sw = src.width as usize;
    let mut out = Image::new(t.dn as u32, src.height);
    out.rows_mut().for_each(|(y, row)| {
        let s = &src.data[y * sw..(y + 1) * sw];
        for (x, px) in row.iter_mut().enumerate() {
            let (idx, w) = (&t.idx[x * t.k..(x + 1) * t.k], &t.w[x * t.k..(x + 1) * t.k]);
            let mut a = [0.0f32; 4];
            for (i, wk) in idx.iter().zip(w) {
                if *wk != 0.0 {
                    let p = s[*i as usize];
                    a[0] += wk * p[0];
                    a[1] += wk * p[1];
                    a[2] += wk * p[2];
                    a[3] += wk * p[3];
                }
            }
            *px = a;
        }
    });
    out
}

/// Vertical pass: `src` (w × h) → (w × `t.dn`).
pub fn pass_v(src: &Image, t: &Tab) -> Image {
    let w = src.width as usize;
    let mut out = Image::new(src.width, t.dn as u32);
    out.rows_mut().for_each(|(y, row)| {
        let (idx, wt) = (&t.idx[y * t.k..(y + 1) * t.k], &t.w[y * t.k..(y + 1) * t.k]);
        for (i, wk) in idx.iter().zip(wt) {
            if *wk == 0.0 {
                continue;
            }
            let s = &src.data[*i as usize * w..(*i as usize + 1) * w];
            for (d, p) in row.iter_mut().zip(s) {
                d[0] += wk * p[0];
                d[1] += wk * p[1];
                d[2] += wk * p[2];
                d[3] += wk * p[3];
            }
        }
    });
    out
}

/// Both passes.
pub fn separable(src: &Image, tx: &Tab, ty: &Tab) -> Image {
    pass_v(&pass_h(src, tx), ty)
}

fn half(n: u32) -> u32 {
    n.div_ceil(2).max(1)
}

/// Half-width copy (6-tap binomial along x).
fn down_x(src: &Image) -> Image {
    pass_h(src, &Tab::down2(src.width as usize, half(src.width) as usize))
}

/// Half-height copy (6-tap binomial along y).
fn down_y(src: &Image) -> Image {
    pass_v(src, &Tab::down2(src.height as usize, half(src.height) as usize))
}

/// Half-size copy (6-tap binomial).
pub fn down2(src: &Image) -> Image {
    down_y(&down_x(src))
}

/// Cubic B-spline refinement to `w` wide.
fn up_x(src: &Image, w: u32) -> Image {
    pass_h(src, &Tab::up2(src.width as usize, w as usize))
}

/// Cubic B-spline refinement to `h` high.
fn up_y(src: &Image, h: u32) -> Image {
    pass_v(src, &Tab::up2(src.height as usize, h as usize))
}

/// Cubic B-spline refinement to `w` × `h` (at most twice the source).
pub fn up2(src: &Image, w: u32, h: u32) -> Image {
    up_y(&up_x(src, w), h)
}

/// Gaussian blur of sigma `sx` × `sy` (samples), zero outside.
pub fn gaussian(src: &Image, sx: f32, sy: f32) -> Image {
    separable(src, &Tab::gauss(src.width as usize, sx), &Tab::gauss(src.height as usize, sy))
}

/// `acc += w · img` per channel.
fn add_weighted(acc: &mut Image, img: &Image, w: [f32; 4]) {
    acc.data.par_iter_mut().zip(img.data.par_iter()).for_each(|(a, p)| {
        for c in 0..4 {
            a[c] += w[c] * p[c];
        }
    });
}

/// `acc += img`.
fn add(acc: &mut Image, img: &Image) {
    add_weighted(acc, img, [1.0; 4]);
}

/// `src` turned clockwise by `theta` radians about its centre onto an `ow` × `oh` canvas
/// (bilinear, transparent outside).
pub fn rotate(src: &Image, theta: f64, ow: u32, oh: u32) -> Image {
    let (scx, scy) = (f64::from(src.width) * 0.5, f64::from(src.height) * 0.5);
    let (ocx, ocy) = (f64::from(ow) * 0.5, f64::from(oh) * 0.5);
    let (s, c) = theta.sin_cos();
    crate::util::gen_image(ow, oh, |x, y| {
        let (dx, dy) = (x as f64 + 0.5 - ocx, y as f64 + 0.5 - ocy);
        src.sample_bilinear(scx + dx * c + dy * s, scy - dx * s + dy * c)
    })
}

/// A small convolution kernel (row-major, centred, sums to 1).
#[derive(Clone, Debug)]
pub struct Kernel {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

/// `src ⊛ k` with zero outside.
pub fn convolve(src: &Image, k: &Kernel) -> Image {
    let (sw, sh) = (src.width as i64, src.height as i64);
    let (cx, cy) = ((k.w / 2) as i64, (k.h / 2) as i64);
    let taps: Vec<(i64, i64, f32)> = (0..k.h)
        .flat_map(|j| (0..k.w).map(move |i| (i, j)))
        .filter_map(|(i, j)| {
            let v = k.data.get(j * k.w + i).copied().unwrap_or(0.0);
            (v != 0.0).then_some((i as i64 - cx, j as i64 - cy, v))
        })
        .collect();
    let mut out = Image::new(src.width, src.height);
    out.rows_mut().for_each(|(y, row)| {
        let y = y as i64;
        for (x, px) in row.iter_mut().enumerate() {
            let x = x as i64;
            let mut a = [0.0f32; 4];
            for (dx, dy, w) in &taps {
                let (xx, yy) = (x + dx, y + dy);
                if xx >= 0 && yy >= 0 && xx < sw && yy < sh {
                    let p = src.data[(yy * sw + xx) as usize];
                    a[0] += w * p[0];
                    a[1] += w * p[1];
                    a[2] += w * p[2];
                    a[3] += w * p[3];
                }
            }
            *px = a;
        }
    });
    out
}

/// What a [`Term`] does to its pyramid level.
#[derive(Clone, Debug)]
pub enum Filter {
    /// The level as it is.
    Identity,
    /// Convolution with a kernel image (Lens Iris).
    Kernel(Kernel),
}

/// One filtered pyramid level added, per channel weighted, to the glow.
#[derive(Clone, Debug)]
pub struct Term {
    pub lvl: usize,
    pub filter: Filter,
    pub w: [f32; 4],
}

/// `Σ_t w_t · filter_t(level_t(src))`, the levels summed top-down through B-spline up-sampling to
/// the full size of `src`. `base` is the finest level any term may use.
pub fn pyramid_sum(src: Image, base: usize, terms: &[Term]) -> Image {
    let (w0, h0) = (src.width, src.height);
    let top = terms.iter().map(|t| t.lvl).max().unwrap_or(base).max(base);
    let mut pyr: Vec<Image> = Vec::with_capacity(top + 1);
    pyr.push(src);
    for i in 1..=top {
        let next = down2(&pyr[i - 1]);
        pyr.push(next);
    }
    let mut cur: Option<Image> = None;
    for i in (base..=top).rev() {
        let (w, h) = (pyr[i].width, pyr[i].height);
        let mut acc = match cur.take() {
            Some(c) => up2(&c, w, h),
            None => Image::new(w, h),
        };
        for t in terms.iter().filter(|t| t.lvl == i) {
            match &t.filter {
                Filter::Identity => add_weighted(&mut acc, &pyr[i], t.w),
                Filter::Kernel(k) => add_weighted(&mut acc, &convolve(&pyr[i], k), t.w),
            }
        }
        cur = Some(acc);
    }
    let mut acc = cur.unwrap_or_else(|| Image::new(w0, h0));
    for i in (0..base).rev() {
        acc = up2(&acc, pyr[i].width, pyr[i].height);
    }
    acc
}

/// A Gaussian on the anisotropic pyramid: levels `lx` / `ly` (halvings along each axis),
/// sigma `sx` / `sy` inside them (samples of that level), and per-channel weights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gauss2 {
    pub lx: usize,
    pub ly: usize,
    pub sx: f32,
    pub sy: f32,
    pub w: [f32; 4],
}

/// `Σ_t w_t · G(σ_t)(cell_t(src))` where cell `(ix, iy)` is `src` halved `ix` times along x and
/// `iy` times along y. Each cell's sum is brought back to full size by refining x then y, so a
/// Gaussian is as wide on each axis as its two levels and sigmas say, whatever the other axis does.
pub fn pyramid2d_sum(src: Image, terms: &[Gauss2]) -> Image {
    let (w0, h0) = (src.width, src.height);
    if terms.is_empty() {
        return Image::new(w0, h0);
    }
    let (lx_max, ly_max) = (terms.iter().map(|t| t.lx).max().unwrap_or(0), terms.iter().map(|t| t.ly).max().unwrap_or(0));
    // Sizes of the cells' axes.
    let (mut dw, mut dh) = (vec![w0], vec![h0]);
    for i in 0..lx_max {
        dw.push(half(dw[i]));
    }
    for i in 0..ly_max {
        dh.push(half(dh[i]));
    }
    // Accumulators of the cells that have terms.
    let mut acc: HashMap<(usize, usize), Image> = HashMap::new();
    let mut col = src;
    for ix in 0..=lx_max {
        // `col` = src halved ix times along x; walk down it along y.
        let mut cell = col.clone();
        let ly_here = terms.iter().filter(|t| t.lx == ix).map(|t| t.ly).max();
        for iy in 0..=ly_here.unwrap_or(0) {
            if iy > 0 {
                cell = down_y(&cell);
            }
            for t in terms.iter().filter(|t| t.lx == ix && t.ly == iy) {
                let blurred = gaussian(&cell, t.sx, t.sy);
                let a = acc.entry((ix, iy)).or_insert_with(|| Image::new(dw[ix], dh[iy]));
                add_weighted(a, &blurred, t.w);
            }
        }
        if ix < lx_max {
            col = down_x(&col);
        }
    }
    // Refine along x within each row of cells, then along y.
    let mut rows: Vec<Option<Image>> = Vec::with_capacity(ly_max + 1);
    for iy in 0..=ly_max {
        let mut cur: Option<Image> = None;
        for ix in (0..=lx_max).rev() {
            let mut img = match cur.take() {
                Some(c) => up_x(&c, dw[ix]),
                None => Image::new(dw[ix], dh[iy]),
            };
            let has = acc.remove(&(ix, iy));
            if let Some(a) = &has {
                add(&mut img, a);
            }
            cur = (has.is_some() || ix < lx_max).then_some(img).or(None);
        }
        rows.push(cur);
    }
    let mut cur: Option<Image> = None;
    for iy in (0..=ly_max).rev() {
        let mut img = match cur.take() {
            Some(c) => up_y(&c, dh[iy]),
            None => Image::new(w0, dh[iy]),
        };
        if let Some(Some(r)) = rows.get(iy) {
            add(&mut img, r);
        }
        cur = Some(img);
    }
    cur.unwrap_or_else(|| Image::new(w0, h0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deepglow::kernel::{axis_plan, fit_weights, min_sigma, sigma_ladder};

    fn sum(img: &Image, c: usize) -> f64 {
        img.data.iter().map(|p| f64::from(p[c])).sum()
    }

    /// The anisotropic ladder for an isotropic or stretched kernel of outer radius `r`.
    fn ladder(r: f32, spread: [f32; 2], base: usize) -> Vec<Gauss2> {
        let sig = sigma_ladder(2, min_sigma(base), 1.6 * r);
        let w = fit_weights(&sig, r, 1.0, 2.0);
        sig.iter()
            .zip(&w)
            .filter_map(|(s, w)| {
                let (lx, sx) = axis_plan(s * spread[0], base, 12)?;
                let (ly, sy) = axis_plan(s * spread[1], base, 12)?;
                Some(Gauss2 { lx, ly, sx, sy, w: [*w; 4] })
            })
            .collect()
    }

    #[test]
    fn passes_keep_the_mass() {
        let mut img = Image::new(64, 48);
        img.set(20, 20, [10.0, 5.0, 1.0, 1.0]);
        let g = gaussian(&img, 2.0, 2.0);
        for c in 0..4 {
            assert!((sum(&g, c) - sum(&img, c)).abs() < 1e-3 * sum(&img, c).max(1.0), "gauss c{c}");
        }
        // down2 averages (a quarter of the mass in 2-D), up2 interpolates (four times it): the
        // round trip keeps the mass.
        let d = down2(&img);
        assert!((sum(&d, 0) - 2.5).abs() < 0.02, "down2 mass {}", sum(&d, 0));
        let u = up2(&d, 64, 48);
        assert!((sum(&u, 0) - 10.0).abs() < 0.05, "up2 mass {}", sum(&u, 0));
    }

    #[test]
    fn identity_tabs_copy() {
        let mut img = Image::new(7, 5);
        img.set(3, 2, [1.0, 2.0, 3.0, 4.0]);
        let o = separable(&img, &Tab::ident(7), &Tab::ident(5));
        assert_eq!(o.data, img.data);
        assert_eq!(gaussian(&img, 0.0, 0.0).data, img.data);
    }

    #[test]
    fn rotation_turns_clockwise_and_back() {
        let mut img = Image::new(41, 41);
        img.set(30, 20, [1.0, 1.0, 1.0, 1.0]);
        let r = rotate(&img, std::f64::consts::FRAC_PI_2, 41, 41);
        // A point right of the centre moves to below it.
        let (mut bx, mut by, mut best) = (0, 0, 0.0f32);
        for y in 0..41 {
            for x in 0..41 {
                if r.get(x, y)[0] > best {
                    best = r.get(x, y)[0];
                    (bx, by) = (x, y);
                }
            }
        }
        assert!((bx - 20).abs() <= 1 && (by - 30).abs() <= 1, "({bx},{by})");
        let back = rotate(&r, -std::f64::consts::FRAC_PI_2, 41, 41);
        assert!(back.get(30, 20)[0] > 0.9);
    }

    /// An impulse through the whole ladder is a unit-mass, symmetric, centre-peaked glow.
    #[test]
    fn ladder_of_an_impulse_is_unit_energy_and_symmetric() {
        let n = 257u32;
        let mut img = Image::new(n, n);
        img.set(128, 128, [1.0, 1.0, 1.0, 1.0]);
        let g = pyramid2d_sum(img, &ladder(24.0, [1.0, 1.0], 0));
        assert_eq!((g.width, g.height), (n, n));
        let mass = sum(&g, 1);
        assert!((mass - 1.0).abs() < 0.03, "energy {mass}");
        let (l, r) = (g.get(128 - 9, 128)[1], g.get(128 + 9, 128)[1]);
        assert!((l - r).abs() < 0.05 * l.max(1e-9), "asymmetric {l} {r}");
        let (u, d) = (g.get(128, 128 - 9)[1], g.get(128, 128 + 9)[1]);
        assert!((u - d).abs() < 0.05 * u.max(1e-9) && (u - l).abs() < 0.06 * u.max(1e-9), "not round {u} {d} {l}");
        assert!(g.get(128, 128)[1] > g.get(128 + 9, 128)[1] && g.get(128 + 9, 128)[1] > g.get(128 + 30, 128)[1]);
    }

    /// A stretched kernel is the isotropic one with its axes scaled: the profile along x at distance
    /// `d` equals the isotropic profile at `d / sx` (up to the mass the narrow axis concentrates).
    #[test]
    fn anisotropic_ladder_is_the_scaled_kernel() {
        let n = 301u32;
        let mut img = Image::new(n, n);
        img.set(150, 150, [1.0, 1.0, 1.0, 1.0]);
        let iso = pyramid2d_sum(img.clone(), &ladder(20.0, [1.0, 1.0], 0));
        let stretched = pyramid2d_sum(img, &ladder(20.0, [1.0, 0.25], 0));
        // Mass is kept.
        assert!((sum(&stretched, 1) - 1.0).abs() < 0.04, "energy {}", sum(&stretched, 1));
        // Along x the kernel is the same shape but concentrated four times as much (the y axis
        // is a quarter as wide): the profile ratio is 4 at every distance.
        for d in [3, 8, 16, 30] {
            let ratio = stretched.get(150 + d, 150)[1] / iso.get(150 + d, 150)[1];
            assert!((ratio / 4.0 - 1.0).abs() < 0.12, "d={d}: ratio {ratio}");
        }
        // Along y at distance d it equals the isotropic profile at 4d, times 4.
        let (a, b) = (stretched.get(150, 150 + 3)[1], iso.get(150, 150 + 12)[1] * 4.0);
        assert!((a / b - 1.0).abs() < 0.15, "{a} vs {b}");
    }

    /// A stretch that leaves the narrow axis below a pixel makes a pure line spread: nothing
    /// leaks off the row, and the row's profile still falls off.
    #[test]
    fn extreme_stretch_is_a_line() {
        let mut img = Image::new(201, 41);
        img.set(100, 20, [1.0, 1.0, 1.0, 1.0]);
        let g = pyramid2d_sum(img, &ladder(24.0, [1.0, 1.0 / 64.0], 0));
        let row: f64 = (0..201).map(|x| f64::from(g.get(x, 20)[1])).sum();
        assert!((sum(&g, 1) - 1.0).abs() < 0.04 && row > 0.95 * sum(&g, 1), "row {row} of {}", sum(&g, 1));
        assert!(g.get(110, 20)[1] > g.get(130, 20)[1] && g.get(130, 20)[1] > g.get(170, 20)[1]);
        assert!(g.get(110, 22)[1] < 0.01 * g.get(110, 20)[1], "off the row");
    }

    #[test]
    fn base_level_keeps_size_and_energy() {
        let mut img = Image::new(200, 120);
        for y in 50..70 {
            for x in 90..110 {
                img.set(x, y, [1.0, 1.0, 1.0, 1.0]);
            }
        }
        let g = pyramid2d_sum(img, &ladder(60.0, [1.0, 1.0], 2));
        assert_eq!((g.width, g.height), (200, 120));
        assert!(g.data.iter().all(|p| p[1] > -0.02));
        let mass = sum(&g, 1);
        assert!((mass / 400.0 - 1.0).abs() < 0.08, "energy ratio {}", mass / 400.0);
    }

    #[test]
    fn kernel_pyramid_conserves_mass() {
        let mut img = Image::new(64, 64);
        img.set(32, 32, [1.0, 1.0, 1.0, 1.0]);
        let k = Kernel { w: 3, h: 3, data: vec![1.0 / 9.0; 9] };
        let g = pyramid_sum(img, 0, &[Term { lvl: 1, filter: Filter::Kernel(k), w: [1.0; 4] }]);
        assert!((sum(&g, 0) - 1.0).abs() < 0.03, "{}", sum(&g, 0));
    }
}
