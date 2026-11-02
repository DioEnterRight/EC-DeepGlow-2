//! Colour maths of the glow: threshold knee, saturation, tone curves, half-float rounding.

/// Tone Mapping → Operation options. Deep Glow 2 ships six presets without publishing their
/// names, so these are the six standard operators.
pub const TONE_OPS: [&str; 6] = ["None", "Reinhard", "Reinhard Extended", "ACES", "Hable Filmic", "Lottes"];

/// Rec. 709 luma of linear or encoded RGB.
#[inline]
pub fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// HSV saturation `(max − min) / max` (0 for black).
#[inline]
pub fn saturation(c: [f32; 3]) -> f32 {
    let mx = c[0].max(c[1]).max(c[2]);
    if mx <= 1e-6 { 0.0 } else { (mx - c[0].min(c[1]).min(c[2])) / mx }
}

/// Share of a colour whose brightness measure is `m` that passes threshold `thr` with a soft
/// knee of half-width `knee` (quadratic, Karis / Unity style): 0 below `thr − knee`, rising
/// smoothly to `(m − thr) / m` above `thr + knee`. The glow input is `colour × share`, so it is
/// continuous at the threshold (no popping on animation) and keeps the hue.
#[inline]
pub fn threshold_share(m: f32, thr: f32, knee: f32) -> f32 {
    let x = (m - thr + knee).clamp(0.0, 2.0 * knee);
    let soft = x * x / (4.0 * knee + 1e-5);
    (soft.max(m - thr) / m.max(1e-5)).max(0.0)
}

fn hable(x: f32) -> f32 {
    (x * (0.15 * x + 0.05) + 0.004) / (x * (0.15 * x + 0.5) + 0.06) - 0.066_666_7
}

/// Tone curve `op` (an index into [`TONE_OPS`]) of a non-negative scalar.
pub fn tone_curve(x: f32, op: u32) -> f32 {
    let x = x.max(0.0);
    match op {
        1 => x / (1.0 + x),
        2 => (x * (1.0 + x / 16.0) / (1.0 + x)).min(1.0),
        3 => ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0),
        4 => (hable(2.0 * x) / hable(11.2)).min(1.0),
        5 => {
            // Lottes (GDC 2016): contrast 1.6, shoulder 0.977, hdr max 8, mid in/out 0.18.
            let (a, d, hdr_max, mid_in, mid_out) = (1.6f32, 0.977f32, 8.0f32, 0.18f32, 0.267f32);
            let b = (-mid_in.powf(a) + hdr_max.powf(a) * mid_out) / ((hdr_max.powf(a * d) - mid_in.powf(a * d)) * mid_out);
            let c = (hdr_max.powf(a * d) * mid_in.powf(a) - hdr_max.powf(a) * mid_in.powf(a * d) * mid_out)
                / ((hdr_max.powf(a * d) - mid_in.powf(a * d)) * mid_out);
            (x.powf(a) / (x.powf(a * d) * b + c)).min(1.0)
        }
        _ => x,
    }
}

/// Hue-preserving tone map of a linear colour: the colour is scaled by `curve(max) / max`, mixed
/// with the untouched colour by `mix`.
pub fn tone_map(c: [f32; 3], op: u32, mix: f32) -> [f32; 3] {
    if op == 0 || mix <= 0.0 {
        return c;
    }
    let mx = c[0].max(c[1]).max(c[2]);
    if mx <= 1e-9 {
        return c;
    }
    let s = 1.0 + (tone_curve(mx, op) / mx - 1.0) * mix.min(1.0);
    [c[0] * s, c[1] * s, c[2] * s]
}

/// `v` rounded to IEEE half precision (10 mantissa bits, range ±65504): the precision of a
/// half-float buffer, without changing the storage.
#[inline]
pub fn half_round(v: f32) -> f32 {
    if !v.is_finite() {
        return v;
    }
    let a = v.abs();
    if a < 5.96e-8 {
        return 0.0;
    }
    if a >= 65520.0 {
        return 65504.0f32.copysign(v);
    }
    let b = v.to_bits();
    let r = b.wrapping_add(0x0FFF + ((b >> 13) & 1));
    f32::from_bits(r & !0x1FFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_share_is_continuous_and_bounded() {
        // Hard threshold: nothing below, (m - thr) / m above.
        assert_eq!(threshold_share(0.4, 0.5, 0.0), 0.0);
        assert!((threshold_share(1.0, 0.5, 0.0) - 0.5).abs() < 1e-4);
        // No threshold: everything passes.
        assert!((threshold_share(0.3, 0.0, 0.0) - 1.0).abs() < 1e-3);
        // Soft knee: monotone in the measure and never above 1.
        let mut last = 0.0;
        for i in 0..200 {
            let s = threshold_share(i as f32 * 0.01, 0.8, 0.2);
            assert!(s >= last - 1e-6 && s <= 1.0 + 1e-6, "{i}: {s} after {last}");
            last = s;
        }
    }

    #[test]
    fn tone_curves_compress_highlights_and_keep_black() {
        for op in 1..6 {
            assert!(tone_curve(0.0, op) < 1e-3, "op {op}");
            let (a, b, c) = (tone_curve(0.5, op), tone_curve(2.0, op), tone_curve(50.0, op));
            assert!(a < b && b <= c && c <= 1.0 + 1e-4, "op {op}: {a} {b} {c}");
        }
        assert_eq!(tone_curve(3.0, 0), 3.0);
    }

    #[test]
    fn tone_map_keeps_hue_and_mixes() {
        let c = [4.0, 2.0, 1.0];
        let t = tone_map(c, 1, 1.0);
        assert!((t[0] / t[1] - 2.0).abs() < 1e-4 && (t[1] / t[2] - 2.0).abs() < 1e-4);
        assert!((t[0] - 0.8).abs() < 1e-4);
        assert_eq!(tone_map(c, 1, 0.0), c);
        let h = tone_map(c, 1, 0.5);
        assert!(h[0] > t[0] && h[0] < c[0]);
        assert_eq!(tone_map(c, 0, 1.0), c);
    }

    #[test]
    fn half_rounding_matches_f16_precision() {
        assert_eq!(half_round(1.0), 1.0);
        assert_eq!(half_round(0.0), 0.0);
        // 1 + 2^-11 is halfway between two halves: ties-to-even gives 1.
        assert_eq!(half_round(1.0 + 1.0 / 2048.0), 1.0);
        assert_eq!(half_round(1.0 + 1.5 / 1024.0), 1.0 + 2.0 / 1024.0);
        assert_eq!(half_round(1.0e6), 65504.0);
        assert!(half_round(f32::NAN).is_nan());
    }
}
