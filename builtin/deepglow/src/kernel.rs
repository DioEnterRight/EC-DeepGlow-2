//! The radial glow kernel and its fit by a ladder of Gaussians.
//!
//! The glow is `input ⊛ K` where `K(r)` is an inverse-square falloff with a soft core and a
//! Gaussian outer window (`K = (1 + (r/a)²)^(-p/2) · exp(-(r/R)²)`). A convolution that wide is
//! far too slow to do directly, so `K` is written as a sum of Gaussians (each cheap on a
//! down-sampled pyramid level): the weights are found by a regularised non-negative
//! least-squares fit on log-spaced radii (relative error, so the faint far tail counts as much
//! as the bright core) and then normalised, which keeps the glow exactly energy preserving.
//!
//! The fit is done for an isotropic kernel. An anisotropic one (Aspect Ratio) is the same kernel
//! with its axes scaled, which turns every Gaussian of the ladder into one with a sigma per axis
//! (`σ·sx`, `σ·sy`): the weights stay the same.

/// Largest ladder size.
pub const MAX_ITEMS: usize = 64;
/// Radii the fit samples.
const NR: usize = 96;
const TAU: f32 = std::f32::consts::TAU;
const LN2: f32 = std::f32::consts::LN_2;

/// Variance (px²) of a 6-tap down-sample followed by a cubic B-spline up-sample, from level `i`
/// back to full resolution (along one axis).
pub fn chain_var(i: usize) -> f32 {
    0.861_111_1 * (((1u64 << (2 * i.min(30))) as f32) - 1.0)
}

/// Smallest sigma of the ladder when the finest pyramid level is `base`.
pub fn min_sigma(base: usize) -> f32 {
    if base == 0 { 0.8 } else { chain_var(base).sqrt() }
}

/// The ladder: `m` Gaussians per octave from `s_min` up to (and including the first at or above)
/// `sigma_max`.
pub fn sigma_ladder(m: usize, s_min: f32, sigma_max: f32) -> Vec<f32> {
    let m = m.max(1) as f32;
    let mut v = Vec::new();
    for k in 0..MAX_ITEMS {
        let s = s_min * (k as f32 / m).exp2();
        v.push(s);
        if s >= sigma_max {
            break;
        }
    }
    v
}

/// Where a Gaussian of total sigma `sigma` along one axis lives: `(pyramid level, sigma inside
/// that level)`. The level is the deepest one whose own softening (`chain_var`) is within the
/// sigma, at least `base` and at most `max_level`; `None` when even the deepest level would need
/// a wider Gaussian than it can hold.
pub fn axis_plan(sigma: f32, base: usize, max_level: usize) -> Option<(usize, f32)> {
    let mut i = base;
    while i < max_level && chain_var(i + 1) <= sigma * sigma {
        i += 1;
    }
    let v = (sigma * sigma - chain_var(i)) / (1u64 << (2 * i)) as f32;
    let sl = if v > 0.0 { v.sqrt() } else { 0.0 };
    if sl > 3.0 {
        return None;
    }
    Some((i, if sl < 0.30 { 0.0 } else { sl }))
}

/// `K(r)` for outer radius `r_out`, core size `a` and power `p` (2 = inverse square).
pub fn kern(r: f32, r_out: f32, a: f32, p: f32) -> f32 {
    let q = 1.0 + (r * r) / (a * a);
    let u = r / r_out;
    (-0.5 * p * q.log2() - std::f32::consts::LOG2_E * u * u).exp2()
}

/// `∫ 2π r K(r) dr` on a log-spaced grid.
fn kernel_energy(r_out: f32, a: f32, p: f32) -> f32 {
    const NI: usize = 400;
    let r0 = 1e-3 * a.min(r_out);
    let rm = 14.0 * r_out;
    let dln = (rm.log2() - r0.log2()) * LN2 / NI as f32;
    let mut integ = 0.0;
    for j in 0..=NI {
        let r = r0 * (j as f32 * dln * std::f32::consts::LOG2_E).exp2();
        let wt = if j == 0 || j == NI { 0.5 } else { 1.0 };
        integ += wt * TAU * r * r * kern(r, r_out, a, p) * dln;
    }
    integ + std::f32::consts::PI * r0 * r0
}

/// Weights `w[k]` (summing to 1) so that `Σ w_k G(r; σ_k) ≈ K(r) / ∫K`.
pub fn fit_weights(sigmas: &[f32], r_out: f32, a: f32, p: f32) -> Vec<f32> {
    let n = sigmas.len();
    if n == 0 {
        return Vec::new();
    }
    let inv = 1.0 / kernel_energy(r_out, a, p).max(1e-30);
    let rmin = 0.35f32;
    let rmax = (6.0 * r_out).max(8.0);
    let lstep = (rmax / rmin).log2() / (NR - 1) as f32;
    let k0 = kern(rmin, r_out, a, p) * inv;
    let (mut rr, mut tg, mut rho) = ([0.0f32; NR], [0.0f32; NR], [0.0f32; NR]);
    for j in 0..NR {
        let r = rmin * (lstep * j as f32).exp2();
        let kt = kern(r, r_out, a, p) * inv;
        rr[j] = r;
        rho[j] = 1.0 / kt.max(1e-5 * k0);
        tg[j] = kt * rho[j];
    }
    // Target mass per ladder step: the fit solves for weights relative to it, so that the
    // smoothness prior is relative as well.
    let wh: Vec<f32> = (0..n)
        .map(|k| {
            let s = sigmas[k];
            let sa = sigmas[k.saturating_sub(1)];
            let sb = sigmas[(k + 1).min(n - 1)];
            let mut dln = 0.5 * (sb.log2() - sa.log2()) * LN2;
            if dln < 0.05 {
                dln = 0.35;
            }
            (kern(s, r_out, a, p) * inv * TAU * s * s * dln).max(1e-9)
        })
        .collect();
    let mut am = vec![0.0f32; NR * n];
    for j in 0..NR {
        for k in 0..n {
            let s = sigmas[k];
            let g = (-std::f32::consts::LOG2_E * rr[j] * rr[j] / (2.0 * s * s)).exp2() / (TAU * s * s);
            am[j * n + k] = g * rho[j] * wh[k];
        }
    }
    let z = solve_nnls(&am, &tg, n);
    let mut w: Vec<f32> = (0..n).map(|k| wh[k] * z[k]).collect();
    let sum: f32 = w.iter().sum();
    if sum < 1e-20 {
        w.iter_mut().enumerate().for_each(|(k, v)| *v = if k == 0 { 1.0 } else { 0.0 });
        return w;
    }
    w.iter_mut().for_each(|v| *v /= sum);
    w
}

/// Non-negative least squares `min |A z − t|²` (+ a second-difference prior on `z`) by projected
/// Gauss-Seidel on the normal equations. `a` is `NR × n`, row-major.
fn solve_nnls(a: &[f32], t: &[f32; NR], n: usize) -> Vec<f32> {
    let mut m = vec![0.0f32; n * n];
    let mut bv = vec![0.0f32; n];
    let mut tr = 0.0;
    for i in 0..n {
        for k in 0..n {
            m[i * n + k] = (0..NR).map(|j| a[j * n + i] * a[j * n + k]).sum();
        }
        bv[i] = (0..NR).map(|j| a[j * n + i] * t[j]).sum();
        tr += m[i * n + i];
    }
    let lam = 0.03 * tr / n as f32;
    let c = [1.0f32, -2.0, 1.0];
    for r in 0..n.saturating_sub(2) {
        for (ai, ca) in c.iter().enumerate() {
            for (bi, cb) in c.iter().enumerate() {
                m[(r + ai) * n + r + bi] += lam * ca * cb;
            }
        }
    }
    let mut z = vec![1.0f32; n];
    for i in 0..n {
        m[i * n + i] += 1e-9 * tr / n as f32 + 1e-20;
    }
    for _ in 0..400 {
        for k in 0..n {
            let mut g = bv[k];
            for j in 0..n {
                if j != k {
                    g -= m[k * n + j] * z[j];
                }
            }
            z[k] = (g / m[k * n + k]).max(0.0);
        }
    }
    z
}

/// The model the weights realise at radius `r`: `Σ w_k G(r; σ_k)`.
#[cfg(test)]
pub fn model_at(sigmas: &[f32], w: &[f32], r: f32) -> f32 {
    sigmas.iter().zip(w).map(|(s, w)| w * (-0.5 * r * r / (s * s)).exp() / (TAU * s * s)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fit(r_out: f32, m: usize) -> (Vec<f32>, Vec<f32>, f32) {
        let a = (0.004 * r_out).max(1.0);
        let sigmas = sigma_ladder(m, min_sigma(0), 1.6 * r_out);
        let w = fit_weights(&sigmas, r_out, a, 2.0);
        (sigmas, w, a)
    }

    #[test]
    fn weights_are_energy_preserving_and_non_negative() {
        for r in [4.0, 40.0, 500.0] {
            let (s, w, _) = fit(r, 2);
            assert!(!s.is_empty());
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4, "R={r}");
            assert!(w.iter().all(|v| *v >= 0.0 && v.is_finite()));
        }
    }

    #[test]
    fn the_ladder_reaches_the_radius_and_is_ordered() {
        let (s, _, _) = fit(300.0, 2);
        assert!(s.last().is_some_and(|v| *v >= 1.6 * 300.0), "{:?}", s.last());
        assert!(s.windows(2).all(|p| p[1] > p[0]), "sigma must grow");
        assert!(s.len() <= MAX_ITEMS);
    }

    /// The Gaussian sum follows the target kernel (relative error on the visible range).
    #[test]
    fn the_fit_follows_the_kernel() {
        let r_out = 120.0;
        let (s, w, a) = fit(r_out, 3);
        let inv = 1.0 / kernel_energy(r_out, a, 2.0);
        let mut worst = 0.0f32;
        for k in 0..30 {
            let r = 1.0 + k as f32 * 4.0;
            let want = kern(r, r_out, a, 2.0) * inv;
            let have = model_at(&s, &w, r);
            worst = worst.max((have / want - 1.0).abs());
        }
        assert!(worst < 0.35, "worst relative error {worst}");
    }

    #[test]
    fn chain_variance_grows_by_four() {
        assert_eq!(chain_var(0), 0.0);
        assert!((chain_var(2) / chain_var(1) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn axis_plans_pick_the_deepest_level_that_fits() {
        // Small sigma: level 0, inside sigma as asked (or none when under 0.3).
        assert_eq!(axis_plan(0.8, 0, 12).map(|(l, _)| l), Some(0));
        assert_eq!(axis_plan(0.1, 0, 12), Some((0, 0.0)));
        // Large sigma goes deep, and the realised variance matches the request.
        let (l, sl) = axis_plan(200.0, 0, 12).expect("plan");
        assert!(l >= 5);
        let realised = chain_var(l) + (1u64 << (2 * l)) as f32 * sl * sl;
        assert!((realised / (200.0 * 200.0) - 1.0).abs() < 1e-3, "{realised}");
        // No level deep enough: refuse.
        assert_eq!(axis_plan(5000.0, 0, 3), None);
        // The base level is a floor.
        assert!(axis_plan(1.0, 2, 12).map(|(l, _)| l) >= Some(2));
    }
}
