/*
 * EC Deep Glow 2 - v4.1.1 - EffectCraft wasm plugin (freestanding, no libc)
 *
 * ABI (unchanged):
 *   ec_api_version() -> 1          ec_manifest_ptr()/ec_manifest_len()
 *   ec_alloc(n)      -> 65536      ec_render(buf,w,h,params,nparams,time,scale) -> 0 ok | 1 bad args | 2 OOM
 *   buf    : w*h*4 f32, premultiplied linear RGBA, rendered IN PLACE
 *   params : f64 array in manifest order (colour params occupy 4 slots); flat indices 0..38 are v3's, 39..68 v4, 69..72 v4.1
 *   scale  : host resolution scale (radius is multiplied by it)
 *
 * PIPELINE
 *   1. input       un-premultiply, threshold (quadratic knee, hue preserving), saturation bias, scene-gamma
 *                  linearise, re-premultiply
 *   2. angle       when the kernel is anisotropic and Enable Angle is on, the frame is turned by -angle onto a
 *                  larger canvas, glowed, and turned back (so Aspect Ratio can point anywhere)
 *   3. ladder      one isotropic kernel K(r) is fitted by a ladder of Gaussians (sigma_k = s0 * 2^(k/m));
 *                  Aspect Ratio then scales each Gaussian per axis, which leaves the fit weights untouched
 *   4. kernel fit  per channel, smoothness-regularised non-negative least squares on log-spaced radii with
 *                  relative error, renormalised to unit energy -> continuous controls, no popping, no light lost
 *   5. anisotropic every Gaussian is placed on its own pyramid cell (lx halvings in x, ly in y) and blurred with
 *      pyramid    a sigma per axis; cells are rebuilt to full size by cubic B-spline refinement in x, then in y.
 *                 A kernel much wider than it is tall is therefore resolved on both axes (v3 squeezed the image
 *                 instead, which flattened the profile at large Aspect Ratio).
 *   6. lens iris   Glow Mode = Lens Iris: nested procedural iris rings (blade count, rotation, easing) convolved
 *                 on the pyramid level that holds each ring, normalised to unit energy together
 *   7. post        pixel aberration, glow gamma, exposure/intensity, lens dirt, tint, hue-preserving tone map,
 *                 dither, half-float rounding, HDR-safe composite, unmult
 */
#include <stdint.h>
#include <stddef.h>

#ifdef EC_NATIVE
#define EC_EXPORT(n)
#else
#define EC_EXPORT(n) __attribute__((export_name(n)))
#endif
typedef float f32;

#include "manifest.h"

/* ------------------------------------------------------------------ libc stubs (compiled with -fno-builtin) */
#ifndef EC_NATIVE
void *memset(void *d, int c, size_t n) { unsigned char *p = (unsigned char *)d; while (n--) *p++ = (unsigned char)c; return d; }
void *memcpy(void *d, const void *s, size_t n) { unsigned char *p = (unsigned char *)d; const unsigned char *q = (const unsigned char *)s; while (n--) *p++ = *q++; return d; }
#endif

/* ------------------------------------------------------------------ math */
static inline f32 minf_(f32 a, f32 b) { return a < b ? a : b; }
static inline f32 maxf_(f32 a, f32 b) { return a > b ? a : b; }
static inline f32 clampf_(f32 x, f32 lo, f32 hi) { return x < lo ? lo : (x > hi ? hi : x); }
static inline f32 sqrtf_(f32 x) { return __builtin_sqrtf(x); }
static inline f32 floorf_(f32 x) { return __builtin_floorf(x); }
static inline f32 ceilf_(f32 x) { return __builtin_ceilf(x); }
static inline f32 absf_(f32 x) { return __builtin_fabsf(x); }
static inline f32 lerpf_(f32 a, f32 b, f32 t) { return a + (b - a) * t; }
/* sanitise: NaN -> 0, clamp to [0, 1e5] (keeps float sums finite) */
static inline f32 san_(f32 x) { return (x > 0.f) ? (x < 1e5f ? x : 1e5f) : 0.f; }

static f32 exp2_(f32 x) {
    x = clampf_(x, -125.f, 125.f);
    f32 fl = floorf_(x), f = x - fl;
    f32 p = 1.f + f * (0.69314718f + f * (0.24022651f + f * (0.05550411f + f * (0.00961813f +
            f * (0.00133336f + f * (0.00015404f + f * (0.0000152527f + f * 0.0000013215f)))))));
    union { f32 f; uint32_t u; } v; v.u = (uint32_t)((int32_t)fl + 127) << 23;
    return p * v.f;
}
static f32 log2_(f32 x) {              /* x > 0 */
    union { f32 f; uint32_t u; } v; v.f = x;
    int e = (int)((v.u >> 23) & 255) - 127;
    v.u = (v.u & 0x7fffffu) | 0x3f800000u;
    f32 m = v.f;
    if (m > 1.41421356f) { m *= 0.5f; e++; }
    f32 z = (m - 1.f) / (m + 1.f), z2 = z * z;
    f32 ln = 2.f * z * (1.f + z2 * (1.f / 3 + z2 * (1.f / 5 + z2 * (1.f / 7 + z2 * (1.f / 9 + z2 * (1.f / 11))))));
    return (f32)e + ln * 1.44269504f;
}
static f32 powf_(f32 x, f32 y) { return x > 0.f ? exp2_(y * log2_(x)) : 0.f; }
static inline f32 hypotf_(f32 x, f32 y) { return sqrtf_(x * x + y * y); }
/* sin/cos by range reduction + minimax polynomials (|err| < 2e-7 on the reduced range) */
static f32 sinf_(f32 x) {
    const f32 TWO_PI = 6.2831853f, PI = 3.14159265f, HALF_PI = 1.5707963f;
    if (!(absf_(x) < 1e5f)) x = 0.f;                     /* NaN / absurd angles (callers reduce first) */
    x = x - TWO_PI * floorf_(x / TWO_PI + 0.5f);          /* -pi .. pi */
    if (x > HALF_PI) x = PI - x; else if (x < -HALF_PI) x = -PI - x;
    f32 x2 = x * x;
    return x * (1.f + x2 * (-0.16666667f + x2 * (0.00833333f + x2 * (-0.00019841f + x2 * 2.7557e-6f))));
}
static inline f32 cosf_(f32 x) { return sinf_(x + 1.5707963f); }
static f32 atan2f_(f32 y, f32 x) {
    if (x == 0.f && y == 0.f) return 0.f;
    f32 ax = absf_(x), ay = absf_(y), a = (ax > ay ? ay / ax : ax / ay), s = a * a;
    f32 r = ((-0.0464964749f * s + 0.15931422f) * s - 0.327622764f) * s * a + a;
    if (ay > ax) r = 1.5707963f - r;
    if (x < 0.f) r = 3.14159265f - r;
    return y < 0.f ? -r : r;
}
/* IEEE half precision rounding (10 mantissa bits, +-65504), value kept in f32 storage */
static f32 half_round(f32 v) {
    if (!(v == v)) return v;
    f32 a = absf_(v);
    if (a < 5.96e-8f) return 0.f;
    if (a >= 65520.f) return v > 0.f ? 65504.f : -65504.f;
    union { f32 f; uint32_t u; } p; p.f = v;
    uint32_t r = p.u + 0x0FFFu + ((p.u >> 13) & 1u);
    p.u = r & ~0x1FFFu;
    return p.f;
}

/* ------------------------------------------------------------------ parameters */
typedef struct {
    f32 thr, soft, radius, expo, inten, falloff, gamma, rmul[3], aspect, chroma, tint_amt, tint[3][3];
    f32 dirt_amt, dirt_scale, dirt_con, src_op, tm_mix, dither;
    int gmode, dirt_seed, blend, tonemap, quality, thr_mode, ca_mode;
    /* v4 */
    f32 sat_bias, angle, tint_in[3], tint_out[3], tint_str, iris_rot, iris_ease, dirt_inherit, dirt_expo, dirt_gamma;
    int view, unmult, px_ch, ab_order, tint_style, tint_blend, q_down, q_steps, q_iter, half, iris_blades, iris_iter, iris_q;
    /* v4.1 */
    int dither_mono, iris_cont; f32 iris_opacity, iris_aspect;
} Params;

/* every parameter read: NaN -> default, then clamped to [lo, hi] in double (so +-Inf and 1e300 are safe) */
static f32 gpc(const double *q, int n, int i, double def, double lo, double hi) {
    double v = (i < n) ? q[i] : def;
    if (!(v == v)) v = def;
    v = v < lo ? lo : (v > hi ? hi : v);
    return (f32)v;
}
static f32 gp(const double *q, int n, int i, double def) { return gpc(q, n, i, def, -1e6, 1e6); }
static int gpi(const double *q, int n, int i, double def, int lo, int hi) {
    double v = (i < n) ? q[i] : def;
    if (!(v == v)) v = def;
    v += 0.5;
    if (v < (double)lo) return lo;
    if (v > (double)hi) return hi;
    int r = (int)v; if ((double)r > v) r--;            /* floor */
    return r < lo ? lo : (r > hi ? hi : r);
}
/* an angle in degrees, reduced to (-360, 360) in double precision, returned in radians */
static f32 gpdeg(const double *q, int n, int i) {
    double v = (i < n) ? q[i] : 0.0;
    if (!(v == v) || !(v < 1e12 && v > -1e12)) return 0.f;
    long long t = (long long)(v / 360.0);
    return (f32)((v - 360.0 * (double)t) * 0.017453292519943295);
}
/* glow-aberration channel orders, index into the popup */
static const unsigned char AB_ORDER[6][3] = {{0,1,2},{0,2,1},{1,0,2},{1,2,0},{2,0,1},{2,1,0}};

static void read_params(Params *P, const double *q, int n) {
    P->thr = gpc(q, n, 0, 35, 0, 10000) * 0.01f;       /* 100 % == luminance 1.0, up to 400 % for HDR */
    P->soft = clampf_(gp(q, n, 1, 50) * 0.01f, 0.f, 1.f);
    P->radius = gpc(q, n, 2, 300, 0, 100000);
    P->expo = gpc(q, n, 3, 1.5, -40, 40);
    P->inten = gpc(q, n, 4, 100, 0, 1e5) * 0.01f;
    P->gmode = gpi(q, n, 5, 0, 0, 2);
    P->falloff = clampf_(gp(q, n, 6, 2), 0.25f, 4.f);
    P->gamma = clampf_(gp(q, n, 7, 1), 0.2f, 5.f);
    P->ab_order = gpi(q, n, 45, 0, 0, 5);
    { f32 rm[3]; for (int c = 0; c < 3; c++) rm[c] = gpc(q, n, 8 + c, 1, 0.01, 100);
      for (int c = 0; c < 3; c++) P->rmul[c] = rm[AB_ORDER[P->ab_order][c]]; }
    P->aspect = clampf_(gp(q, n, 11, 1), 0.1f, 10.f);
    P->chroma = clampf_(gp(q, n, 12, 0) * 0.01f, 0.f, 1.f);
    P->tint_amt = clampf_(gp(q, n, 13, 0) * 0.01f, 0.f, 1.f);
    static const double tdef[3][3] = {{1, .45, .1}, {1, .75, .45}, {1, .95, .85}};
    for (int s = 0; s < 3; s++) for (int c = 0; c < 3; c++) P->tint[s][c] = gpc(q, n, 14 + 4 * s + c, tdef[s][c], 0, 1000);
    P->dirt_amt = clampf_(gp(q, n, 26, 0) * 0.01f, 0.f, 1.f);
    P->dirt_scale = gpc(q, n, 27, 1, 0.05, 1000);
    P->dirt_con = gpc(q, n, 28, 1, 0.05, 100);
    P->dirt_seed = gpi(q, n, 29, 1, 0, 1 << 24);
    P->src_op = clampf_(gp(q, n, 30, 100) * 0.01f, 0.f, 1.f);
    P->blend = gpi(q, n, 31, 0, 0, 7);
    P->tonemap = gpi(q, n, 32, 0, 0, 5);
    P->quality = gpi(q, n, 33, 2, 0, 6);
    P->thr_mode = gpi(q, n, 35, 0, 0, 1);
    P->ca_mode = gpi(q, n, 36, 0, 0, 2);
    P->tm_mix = clampf_(gp(q, n, 37, 100) * 0.01f, 0.f, 1.f);
    P->dither = clampf_(gp(q, n, 38, 100) * 0.01f, 0.f, 1.f);
    /* v4 */
    P->sat_bias = clampf_(gp(q, n, 39, 0) * 0.01f, 0.f, 1.f);
    P->angle = (gp(q, n, 40, 0) > 0.5f) ? gpdeg(q, n, 41) : 0.f;
    P->view = gpi(q, n, 42, 0, 0, 2);
    if (gp(q, n, 34, 0) > 0.5f && P->view == 0) P->view = 1;        /* legacy Glow Only checkbox */
    P->unmult = gp(q, n, 43, 1) > 0.5f;
    P->px_ch = gpi(q, n, 44, 0, 0, 5);
    P->tint_style = gpi(q, n, 46, 0, 0, 1);
    P->tint_blend = gpi(q, n, 47, 0, 0, 5);
    static const double idef[3] = {1, .9, .7}, odef[3] = {.6, .4, 1};
    for (int c = 0; c < 3; c++) { P->tint_in[c] = gpc(q, n, 48 + c, idef[c], 0, 1000); P->tint_out[c] = gpc(q, n, 52 + c, odef[c], 0, 1000); }
    P->tint_str = clampf_(gp(q, n, 56, 100) * 0.01f, 0.f, 1.f);
    P->q_down = gpi(q, n, 57, 0, 0, 6);
    P->q_steps = gpi(q, n, 58, 3, 1, 8);
    P->q_iter = gpi(q, n, 59, 15, 1, 15);
    P->half = gp(q, n, 60, 0) > 0.5f;
    P->iris_blades = gpi(q, n, 61, 6, 0, 16);
    P->iris_rot = gpdeg(q, n, 62);
    P->iris_iter = gpi(q, n, 63, 16, 1, 64);
    P->iris_ease = clampf_(gp(q, n, 64, 50) * 0.01f, 0.f, 1.f);
    P->iris_q = gpi(q, n, 65, 1, 0, 2);
    P->dirt_inherit = clampf_(gp(q, n, 66, 100) * 0.01f, 0.f, 1.f);
    P->dirt_expo = clampf_(gp(q, n, 67, 0), -6.f, 6.f);
    P->dirt_gamma = clampf_(gp(q, n, 68, 1), 0.1f, 6.f);
    P->dither_mono = gp(q, n, 69, 1) > 0.5f;
    P->iris_opacity = clampf_(gp(q, n, 70, 100) * 0.01f, 0.f, 1.f);
    P->iris_aspect = clampf_(gp(q, n, 71, 1), 0.1f, 10.f);
    P->iris_cont = gp(q, n, 72, 1) > 0.5f;
    if (P->iris_blades > 0 && P->iris_blades < 3) P->iris_blades = 3;
}
/* ------------------------------------------------------------------ separable resampling via tap tables */
typedef struct { int dn, K; int *idx; f32 *w; int lo, hi; } Tab;   /* [lo,hi): outputs whose taps are contiguous & in range */

/* contiguous-run detection: lets pass_h skip per-tap index loads and bounds logic in the interior */
static void tab_meta(Tab *t, int sn) {
    int K = t->K, first = -1, end = 0;
    for (int o = 0; o < t->dn; o++) {
        int b0 = t->idx[o * K], ok = (b0 + K <= sn);
        for (int k = 0; ok && k < K; k++) if (t->idx[o * K + k] != b0 + k) ok = 0;
        if (ok) { if (first < 0) first = o; end = o + 1; }
        else if (first >= 0) break;                    /* interior run is contiguous: stop at its end */
    }
    if (first < 0) { t->lo = t->hi = t->dn; } else { t->lo = first; t->hi = end; }
}
static void tab_ident(Tab *t, int n) { t->dn = n; t->K = 1; for (int o = 0; o < n; o++) { t->idx[o] = o; t->w[o] = 1.f; } t->lo = 0; t->hi = n; }
static void tab_gauss(Tab *t, int n, f32 sl) {
    int rad = (int)ceilf_(3.3f * sl); if (rad < 1) rad = 1; if (rad > 12) rad = 12;
    int K = 2 * rad + 1; f32 g[25], s = 0.f;
    for (int k = 0; k < K; k++) { f32 d = (f32)(k - rad); g[k] = exp2_(-1.4426950f * d * d / (2.f * sl * sl)); s += g[k]; }
    for (int k = 0; k < K; k++) g[k] /= s;
    t->dn = n; t->K = K;
    for (int o = 0; o < n; o++) for (int k = 0; k < K; k++) {
        int i = o + k - rad; int ok = (i >= 0 && i < n);
        t->idx[o * K + k] = ok ? i : 0; t->w[o * K + k] = ok ? g[k] : 0.f;
    }
    tab_meta(t, n);
}
/* 6-tap binomial [1 5 10 10 5 1]/32 (cell centred); box [1 1]/2 when the source axis is tiny (mass preserving) */
static void tab_down2(Tab *t, int sn, int dn) {
    static const f32 b6[6] = {1.f / 32, 5.f / 32, 10.f / 32, 10.f / 32, 5.f / 32, 1.f / 32};
    t->dn = dn; t->K = 6;
    for (int o = 0; o < dn; o++) for (int k = 0; k < 6; k++) {
        int i = 2 * o - 2 + k; f32 w = b6[k];
        if (sn <= 8) { i = 2 * o + (k - 2); w = (k == 2 || k == 3) ? 0.5f : 0.f; }
        int ok = (i >= 0 && i < sn);
        t->idx[o * 6 + k] = ok ? i : 0; t->w[o * 6 + k] = ok ? w : 0.f;
    }
    tab_meta(t, sn);
}
/* exact cubic B-spline sampling, offsets 1/4 and 3/4 (2x refinement), clamp to edge */
static void tab_up2(Tab *t, int sn, int dn) {
    static const f32 wa[4] = {0.00260417f, 0.31510417f, 0.61197917f, 0.0703125f};   /* even: n-2..n+1 */
    static const f32 wb[4] = {0.0703125f, 0.61197917f, 0.31510417f, 0.00260417f};   /* odd : n-1..n+2 */
    t->dn = dn; t->K = 4;
    for (int o = 0; o < dn; o++) {
        int n = o >> 1, par = o & 1;
        for (int k = 0; k < 4; k++) {
            int i = (par ? n - 1 : n - 2) + k; i = i < 0 ? 0 : (i > sn - 1 ? sn - 1 : i);
            t->idx[o * 4 + k] = i; t->w[o * 4 + k] = par ? wb[k] : wa[k];
        }
    }
    tab_meta(t, sn);
}
/* horizontal pass: src (sw x rows) -> dst (t->dn x rows), RGB interleaved */
static void pass_h_(const f32 *src, int sw, int rows, f32 *dst, const Tab *t, int add) {
    int K = t->K, dn = t->dn, lo = t->lo, hi = t->hi;
    for (int y = 0; y < rows; y++) {
        const f32 *s = src + (size_t)y * sw * 3; f32 *d = dst + (size_t)y * dn * 3;
        for (int x = 0; x < dn; x++) {
            f32 r = 0, g = 0, b = 0; const f32 *wt = t->w + x * K;
            if (x >= lo && x < hi) {
                const f32 *p = s + t->idx[x * K] * 3;
                for (int k = 0; k < K; k++, p += 3) { f32 w = wt[k]; r += w * p[0]; g += w * p[1]; b += w * p[2]; }
            } else {
                const int *ix = t->idx + x * K;
                for (int k = 0; k < K; k++) { const f32 *p = s + ix[k] * 3; f32 w = wt[k]; r += w * p[0]; g += w * p[1]; b += w * p[2]; }
            }
            if (add) { d[0] += r; d[1] += g; d[2] += b; } else { d[0] = r; d[1] = g; d[2] = b; }
            d += 3;
        }
    }
}
static void pass_h(const f32 *src, int sw, int rows, f32 *dst, const Tab *t) { pass_h_(src, sw, rows, dst, t, 0); }
/* vertical pass: src (w x sh) -> dst (w x t->dn).  cw == NULL: overwrite; else dst += cw[c] * result */
static void pass_v(const f32 *src, int w, f32 *dst, const Tab *t, const f32 *cw, f32 *row) {
    int K = t->K, dn = t->dn, n3 = w * 3;
    for (int y = 0; y < dn; y++) {
        for (int j = 0; j < n3; j++) row[j] = 0.f;
        for (int k = 0; k < K; k++) {
            f32 wk = t->w[y * K + k]; if (wk == 0.f) continue;
            const f32 *s = src + (size_t)t->idx[y * K + k] * n3;
            for (int j = 0; j < n3; j++) row[j] += wk * s[j];
        }
        f32 *d = dst + (size_t)y * n3;
        if (!cw) { for (int j = 0; j < n3; j++) d[j] = row[j]; }
        else { f32 c0 = cw[0], c1 = cw[1], c2 = cw[2]; for (int x = 0; x < w; x++) { d[3 * x] += c0 * row[3 * x]; d[3 * x + 1] += c1 * row[3 * x + 1]; d[3 * x + 2] += c2 * row[3 * x + 2]; } }
    }
}


/* ------------------------------------------------------------------ kernel model + fit */
#define MAXI 64
#define NR 96
#define MAXCELL 64
/* cost-model coefficients (wasmi fuel per unit), fitted by test/cost_fit.py */
#ifndef CK_PX
#define CK_CONST 958617.0
#define CK_PX    4660.06
#define CK_ROT   1859.16
#define CK_DOWN  0.0
#define CK_TAP   48.0108
#define CK_CONV  44.7812
#define CK_REC   251.886
#define CK_CELL  739.456
#define CK_FIT   12936.0
#define CK_KB    390.327
#endif

/* variance of a down(6-tap)+up(cubic B-spline) chain from level i back to full res, one axis (px^2) */
static inline f32 chain_var(int i) { return 0.8611111f * ((f32)(1 << (2 * (i < 15 ? i : 15))) - 1.f); }

/* the isotropic sigma ladder: m Gaussians per octave from s0 up to the first one at or above sigmax */
static int sigma_ladder(f32 *st, int m, f32 s0, f32 sigmax) {
    int n = 0;
    for (int k = 0; k < MAXI; k++) {
        f32 s = s0 * exp2_((f32)k / (f32)m);
        st[n++] = s;
        if (s >= sigmax) break;
    }
    return n;
}

/* where a Gaussian of total sigma `sigma` lives on one axis: pyramid level + the sigma inside that level.
 * Past the deepest level allowed, the widest Gaussian that level can hold is used, so the glow is a touch
 * narrower than asked for but never lost. */
static void axis_plan(f32 sigma, int base, int maxl, int *lvl, f32 *sl) {
    int i = base;
    while (i < maxl && chain_var(i + 1) <= sigma * sigma) i++;
    f32 v = (sigma * sigma - chain_var(i)) / (f32)(1 << (2 * i));
    f32 s = v > 0.f ? sqrtf_(v) : 0.f;
    if (s > 3.f) s = 3.f;
    if (s < 0.30f) s = 0.f;
    *lvl = i; *sl = s;
}

static f32 kern(int mode, f32 r, f32 R, f32 a, f32 p) {
    if (mode == 1) return exp2_(-4.328085f * r / R);     /* exp(-3 r / R) : 5 % at r = R */
    f32 q = 1.f + (r * r) / (a * a), u = r / R;          /* inverse-square^(p/2), soft core, Gaussian window */
    return exp2_(-0.5f * p * log2_(q) - 1.4426950f * u * u);
}
/* fit unit-energy weights w[k] so that sum_k w_k G(r; sigma_k) ~= K(r)/int(K) (relative error, log radii) */
static void fit_weights(const f32 *st, int n, int mode, f32 R, f32 a, f32 p, f32 *w, f32 *scr) {
    f32 *A = scr, *M = A + NR * n, *bv = M + n * n, *z = bv + n, *wh = z + n, *tg = wh + n, *rr = tg + NR, *rho = rr + NR;
    /* normalisation  int 2 pi r K dr  (log-spaced trapezoid) */
    f32 integ; {
        const int NI = 400; f32 r0 = 1e-3f * minf_(a, R), rm = 14.f * R, dln = (log2_(rm) - log2_(r0)) * 0.69314718f / (f32)NI; integ = 0.f;
        for (int j = 0; j <= NI; j++) { f32 r = r0 * exp2_((f32)j * dln * 1.4426950f); f32 wt = (j == 0 || j == NI) ? 0.5f : 1.f; integ += wt * 6.2831853f * r * r * kern(mode, r, R, a, p) * dln; }
        integ += 3.1415927f * r0 * r0;
    }
    f32 inv = 1.f / integ;
    f32 rmin = 0.35f, rmax = maxf_(6.f * R, 8.f);
    f32 lstep = log2_(rmax / rmin) / (f32)(NR - 1);
    f32 k0 = kern(mode, rmin, R, a, p) * inv;
    for (int j = 0; j < NR; j++) {
        f32 r = rmin * exp2_(lstep * (f32)j); rr[j] = r;
        f32 kt = kern(mode, r, R, a, p) * inv;
        rho[j] = 1.f / maxf_(kt, 1e-5f * k0); tg[j] = kt * rho[j];
    }
    for (int k = 0; k < n; k++) {                         /* target mass per ladder step (for relative smoothness) */
        f32 s = st[k];
        f32 sa = st[k > 0 ? k - 1 : k], sb = st[k < n - 1 ? k + 1 : k];
        f32 dln = 0.5f * (log2_(sb) - log2_(sa)) * 0.69314718f; if (dln < 0.05f) dln = 0.35f;
        wh[k] = maxf_(kern(mode, s, R, a, p) * inv * 6.2831853f * s * s * dln, 1e-9f);
    }
    for (int j = 0; j < NR; j++) for (int k = 0; k < n; k++) {
        f32 s = st[k], g = exp2_(-1.4426950f * rr[j] * rr[j] / (2.f * s * s)) / (6.2831853f * s * s);
        A[j * n + k] = g * rho[j] * wh[k];
    }
    f32 tr = 0.f;
    for (int i = 0; i < n; i++) {
        for (int k = 0; k < n; k++) { f32 s = 0.f; for (int j = 0; j < NR; j++) s += A[j * n + i] * A[j * n + k]; M[i * n + k] = s; }
        f32 s = 0.f; for (int j = 0; j < NR; j++) s += A[j * n + i] * tg[j]; bv[i] = s; tr += M[i * n + i];
    }
    f32 lam = 0.03f * tr / (f32)n;                        /* second-difference smoothness prior on relative weights */
    for (int r = 0; r + 2 < n; r++) { int id[3] = {r, r + 1, r + 2}; f32 c[3] = {1.f, -2.f, 1.f}; for (int a_ = 0; a_ < 3; a_++) for (int b_ = 0; b_ < 3; b_++) M[id[a_] * n + id[b_]] += lam * c[a_] * c[b_]; }
    for (int i = 0; i < n; i++) { M[i * n + i] += 1e-9f * tr / (f32)n + 1e-20f; z[i] = 1.f; }
    for (int sweep = 0; sweep < 400; sweep++)
        for (int k = 0; k < n; k++) {
            f32 g = bv[k]; for (int j = 0; j < n; j++) if (j != k) g -= M[k * n + j] * z[j];
            f32 v = g / M[k * n + k]; z[k] = v > 0.f ? v : 0.f;
        }
    f32 sum = 0.f; for (int k = 0; k < n; k++) { w[k] = wh[k] * z[k]; sum += w[k]; }
    if (sum < 1e-20f) { for (int k = 0; k < n; k++) w[k] = (k == 0); sum = 1.f; }
    for (int k = 0; k < n; k++) w[k] /= sum;               /* exactly energy preserving */
}
/* ------------------------------------------------------------------ colour helpers */
static f32 hable_(f32 x) { return (x * (0.15f * x + 0.05f) + 0.004f) / (x * (0.15f * x + 0.5f) + 0.06f) - 0.0666667f; }
static f32 tm_curve(f32 x, int mode) {
    x = maxf_(x, 0.f);
    switch (mode) {
    case 1: return x / (1.f + x);
    case 2: return clampf_((x * (2.51f * x + 0.03f)) / (x * (2.43f * x + 0.59f) + 0.14f), 0.f, 1.f);
    case 3: return hable_(2.f * x) / hable_(11.2f);
    case 4: return minf_(x * (1.f + x * (1.f / 16.f)) / (1.f + x), 1.f);     /* extended Reinhard, white point 4 */
    default: return x;
    }
}
static inline uint32_t hash_u(uint32_t h) { h = (h ^ (h >> 16)) * 0x7feb352du; h = (h ^ (h >> 15)) * 0x846ca68bu; return h ^ (h >> 16); }
static inline f32 hash3(int x, int y, int s) {
    uint32_t h = (uint32_t)x * 374761393u + (uint32_t)y * 668265263u + (uint32_t)s * 1442695041u;
    h = (h ^ (h >> 13)) * 1274126177u; h ^= h >> 16; return (f32)(h & 0xFFFFFFu) * (1.f / 16777216.f);
}
static f32 vnoise(f32 x, f32 y, int s) {
    f32 fx0 = floorf_(x), fy0 = floorf_(y); int ix = (int)fx0, iy = (int)fy0; f32 fx = x - fx0, fy = y - fy0;
    fx = fx * fx * (3.f - 2.f * fx); fy = fy * fy * (3.f - 2.f * fy);
    return lerpf_(lerpf_(hash3(ix, iy, s), hash3(ix + 1, iy, s), fx), lerpf_(hash3(ix, iy + 1, s), hash3(ix + 1, iy + 1, s), fx), fy);
}
static f32 dirt_mask(f32 u, f32 v, f32 asp, f32 sc, f32 con, int seed) {
    f32 x = u * asp * sc * 6.f, y = v * sc * 6.f;
    f32 n = (0.5f * vnoise(x, y, seed) + 0.25f * vnoise(2 * x, 2 * y, seed + 1) + 0.125f * vnoise(4 * x, 4 * y, seed + 2) + 0.0625f * vnoise(8 * x, 8 * y, seed + 3)) / 0.9375f;
    f32 blot = clampf_((n - 0.5f) * con * 4.f, 0.f, 1.f);
    f32 spk = clampf_((vnoise(x * 9.f + 37.f, y * 9.f + 11.f, seed + 7) - 0.82f) * 8.f, 0.f, 1.f);
    return clampf_(blot * 0.8f + spk, 0.f, 1.f);
}
/* Catmull-Rom sample of one channel, clamp to edge (pixel-index coordinates) */
static f32 sample_cr(const f32 *G, int W, int H, int c, f32 x, f32 y) {
    f32 fx = floorf_(x), fy = floorf_(y); int ix = (int)fx, iy = (int)fy; f32 tx = x - fx, ty = y - fy;
    f32 wx[4], wy[4];
    f32 t2 = tx * tx, t3 = t2 * tx; wx[0] = -0.5f * t3 + t2 - 0.5f * tx; wx[1] = 1.5f * t3 - 2.5f * t2 + 1.f; wx[2] = -1.5f * t3 + 2.f * t2 + 0.5f * tx; wx[3] = 0.5f * t3 - 0.5f * t2;
    t2 = ty * ty; t3 = t2 * ty; wy[0] = -0.5f * t3 + t2 - 0.5f * ty; wy[1] = 1.5f * t3 - 2.5f * t2 + 1.f; wy[2] = -1.5f * t3 + 2.f * t2 + 0.5f * ty; wy[3] = 0.5f * t3 - 0.5f * t2;
    f32 s = 0.f;
    for (int j = 0; j < 4; j++) {
        int yy = iy - 1 + j; yy = yy < 0 ? 0 : (yy > H - 1 ? H - 1 : yy); f32 r = 0.f;
        for (int i = 0; i < 4; i++) { int xx = ix - 1 + i; xx = xx < 0 ? 0 : (xx > W - 1 ? W - 1 : xx); r += wx[i] * G[((size_t)yy * W + xx) * 3 + c]; }
        s += wy[j] * r;
    }
    return maxf_(s, 0.f);
}

/* cubic B-spline sample of one RGB triple, zero outside (pixel centres at +0.5).  The B-spline basis is a
 * partition of unity and its spectrum falls as sinc^4, so resampling onto a rotated grid keeps the light:
 * plain bilinear leaks over 30 % of an impulse's energy at 45 degrees. */
static inline void sample_b3(const f32 *S, int W, int H, f32 x, f32 y, f32 *out) {
    out[0] = out[1] = out[2] = 0.f;
    f32 fx = floorf_(x - 0.5f), fy = floorf_(y - 0.5f);
    f32 tx = x - 0.5f - fx, ty = y - 0.5f - fy;
    int ix = (int)fx - 1, iy = (int)fy - 1;
    f32 wx[4], wy[4];
    for (int p = 0; p < 2; p++) {
        f32 t = p ? ty : tx, u = 1.f - t, t2 = t * t, t3 = t2 * t, *w = p ? wy : wx;
        w[0] = u * u * u * (1.f / 6.f);
        w[1] = (3.f * t3 - 6.f * t2 + 4.f) * (1.f / 6.f);
        w[2] = (-3.f * t3 + 3.f * t2 + 3.f * t + 1.f) * (1.f / 6.f);
        w[3] = t3 * (1.f / 6.f);
    }
    for (int j = 0; j < 4; j++) {
        int yy = iy + j; if (yy < 0 || yy >= H || wy[j] == 0.f) continue;
        for (int i = 0; i < 4; i++) {
            int xx = ix + i; if (xx < 0 || xx >= W) continue;
            f32 w = wy[j] * wx[i]; const f32 *p = S + ((size_t)yy * W + xx) * 3;
            out[0] += w * p[0]; out[1] += w * p[1]; out[2] += w * p[2];
        }
    }
}
/* src turned clockwise by theta about its centre onto a dw x dh canvas (zero outside) */
static void rot_img(const f32 *S, int sw, int sh, f32 *D, int dw, int dh, f32 theta) {
    f32 scx = 0.5f * (f32)sw, scy = 0.5f * (f32)sh, ocx = 0.5f * (f32)dw, ocy = 0.5f * (f32)dh;
    f32 s = sinf_(theta), c = cosf_(theta);
    for (int y = 0; y < dh; y++) for (int x = 0; x < dw; x++) {
        f32 dx = (f32)x + 0.5f - ocx, dy = (f32)y + 0.5f - ocy;
        sample_b3(S, sw, sh, scx + dx * c + dy * s, scy - dx * s + dy * c, D + ((size_t)y * dw + x) * 3);
    }
}

/* Overlay (standard formula, glow as the base) with the part of the glow above 1 added back scaled by 2c:
 * a mid-grey tint of 0.5 leaves any glow unchanged, HDR light keeps its energy. */
static f32 overlay_hdr(f32 g, f32 c) {
    f32 u = clampf_(g, 0.f, 1.f);
    f32 o = (u < 0.5f) ? 2.f * u * c : 1.f - 2.f * (1.f - u) * (1.f - c);
    return o + maxf_(g - u, 0.f) * 2.f * c;
}
/* Soft Light (W3C compositing formula) with the same HDR rule */
static f32 soft_light_hdr(f32 g, f32 c) {
    f32 u = clampf_(g, 0.f, 1.f), o;
    if (c <= 0.5f) o = u - (1.f - 2.f * c) * u * (1.f - u);
    else { f32 d = (u <= 0.25f) ? ((16.f * u - 12.f) * u + 4.f) * u : sqrtf_(u); o = u + (2.f * c - 1.f) * (d - u); }
    return o + maxf_(g - u, 0.f) * 2.f * c;
}

/* inner/outer tint of a glow colour (mirrors the built-in effect): inner where the glow is bright, outer
 * where it is dim, applied with one of four blends and mixed back by strength. */
static void apply_tint(f32 *g, const Params *P) {
    if (P->tint_str <= 0.f) return;
    f32 l = 0.2126f * g[0] + 0.7152f * g[1] + 0.0722f * g[2];
    if (l < 0.f) l = 0.f;
    f32 u = l / (l + 0.25f), col[3], out[3];
    for (int c = 0; c < 3; c++) col[c] = lerpf_(P->tint_out[c], P->tint_in[c], u);
    switch (P->tint_blend) {
    case 1: {                                                   /* Normal: take the hue, keep the brightness */
        f32 lc = maxf_(0.2126f * col[0] + 0.7152f * col[1] + 0.0722f * col[2], 1e-4f);
        for (int c = 0; c < 3; c++) out[c] = col[c] * l / lc;
        break;
    }
    case 2: for (int c = 0; c < 3; c++) out[c] = g[c] + col[c] * l; break;                                   /* Add    */
    case 3: for (int c = 0; c < 3; c++) out[c] = g[c] + col[c] * l * (1.f - clampf_(g[c], 0.f, 1.f)); break; /* Screen */
    case 4: for (int c = 0; c < 3; c++) out[c] = overlay_hdr(g[c], col[c]); break;                            /* Overlay */
    case 5: for (int c = 0; c < 3; c++) out[c] = soft_light_hdr(g[c], col[c]); break;                         /* Soft Light */
    default: for (int c = 0; c < 3; c++) out[c] = g[c] * col[c]; break;                                      /* Multiply */
    }
    for (int c = 0; c < 3; c++) g[c] = lerpf_(g[c], out[c], P->tint_str);
}

/* iris outline radius factor at angle th for a `blades`-sided aperture (0 = circle); 1.0 = inscribed radius */
static f32 iris_shape(f32 th, int blades, f32 rot) {
    if (blades < 3) return 1.f;
    f32 seg = 6.2831853f / (f32)blades;
    f32 a = th + rot;
    a = a - seg * floorf_(a / seg) - 0.5f * seg;
    f32 c = cosf_(a);
    return c > 0.08f ? 1.f / c : 12.5f;
}

/* ------------------------------------------------------------------ memory plan */
typedef struct { uint8_t *base; uint64_t off; } Arena;   /* 64-bit offset: a huge plan must not wrap round */
#ifdef EC_NATIVE   /* sanitizer builds (test/native_fuzz.sh): every buffer is its own heap block */
void *ec_native_alloc(uint64_t bytes);
#define EC_NATIVE_HEAP ((uint8_t *)16)
#endif
static void *ar_alloc(Arena *a, uint64_t bytes) {
#ifdef EC_NATIVE
    if (a->base == EC_NATIVE_HEAP) return ec_native_alloc(bytes);
#endif
    bytes = (bytes + 15u) & ~(uint64_t)15u; void *p = a->base ? (void *)(a->base + (size_t)a->off) : (void *)0; a->off += bytes; return p; }

EC_EXPORT("ec_api_version") int ec_api_version(void) { return 1; }
EC_EXPORT("ec_manifest_ptr") int ec_manifest_ptr(void) { return (int)(intptr_t)MANIFEST; }
EC_EXPORT("ec_manifest_len") int ec_manifest_len(void) { return MANIFEST_LEN; }
#ifndef EC_NATIVE
EC_EXPORT("ec_alloc") int ec_alloc(int n) {
    int need = (int)(((long long)65536 + n + 65535) >> 16), have = (int)__builtin_wasm_memory_size(0);
    if (need > have) __builtin_wasm_memory_grow(0, need - have);
    return 65536;
}
#endif

/* one Gaussian (or iris kernel) of the ladder, placed on its own pyramid cell */
typedef struct { int lx, ly; f32 sx, sy; f32 w[3]; int kb; } G2;   /* kb < 0: separable Gaussian, else kernel bank slot */
typedef struct { int ix, iy; f32 *acc; } Cell;

/* add a (2kr+1)^2 convolution of src into dst; the kernel holds one plane per channel */
static void conv_add(const f32 *S, int w, int h, const f32 *K, int kr, f32 *D) {
    int kw = 2 * kr + 1;
    for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
        f32 a0 = 0.f, a1 = 0.f, a2 = 0.f;
        int j0 = y - kr < 0 ? kr - y : 0, j1 = (y + kr >= h) ? (h - 1 - y + kr) : (kw - 1);
        int i0 = x - kr < 0 ? kr - x : 0, i1 = (x + kr >= w) ? (w - 1 - x + kr) : (kw - 1);
        for (int j = j0; j <= j1; j++) {
            const f32 *s = S + ((size_t)(y + j - kr) * w + (x + i0 - kr)) * 3;
            const f32 *k = K + ((size_t)j * kw + i0) * 3;
            for (int i = i0; i <= i1; i++, s += 3, k += 3) { a0 += k[0] * s[0]; a1 += k[1] * s[1]; a2 += k[2] * s[2]; }
        }
        f32 *d = D + ((size_t)y * w + x) * 3; d[0] += a0; d[1] += a1; d[2] += a2;
    }
}

typedef struct {
    f32 *TH, *T, *wkA, *wkB, *tmp, *xA, *xB, *vA, *vB, *row, *fit, *kb[16];
    f32 *acc[MAXCELL]; int *tabi; f32 *tabw;
} Mem;

static void layout(Mem *m, Arena *ar, int W, int H, int GW, int GH, const int *lw, const int *lh,
                   int rot, const Cell *cl, int ncell, int lxmax, int lymax, int nfit, const int *kr) {
    m->TH = (f32 *)ar_alloc(ar, (uint64_t)W * H * 12);
    m->T = rot ? (f32 *)ar_alloc(ar, (uint64_t)GW * GH * 12) : m->TH;
    for (int i = 0; i < ncell; i++) m->acc[i] = (f32 *)ar_alloc(ar, (uint64_t)lw[cl[i].ix] * lh[cl[i].iy] * 12);
    {   /* the staircase walk needs two buffers, each big enough for the first halving on either axis */
        uint64_t half = (uint64_t)lw[1] * GH; uint64_t h2 = (uint64_t)GW * lh[1]; if (h2 > half) half = h2;
        m->wkA = (lxmax + lymax >= 1) ? (f32 *)ar_alloc(ar, half * 12) : 0;
        m->wkB = (lxmax + lymax >= 2) ? (f32 *)ar_alloc(ar, half * 12) : 0;
    }
    /* the staircase buffers are spent once the cells are filled, so the reconstruction borrows them:
     * xA (lw[1] x GH) fits in wkA, vB (GW x lh[1]) in wkB.  Halves the peak memory of a rotated 4K glow. */
    m->xA = (lxmax >= 1) ? m->wkA : 0;
    m->xB = (lxmax >= 2) ? (f32 *)ar_alloc(ar, (uint64_t)lw[2] * GH * 12) : 0;
    /* the glow source (thresholded, or its rotated copy) is spent once the cells are filled, so the
     * full-size reconstruction buffer reuses it */
    m->vA = m->T;
    m->vB = (lymax >= 1) ? (m->wkB ? m->wkB : (f32 *)ar_alloc(ar, (uint64_t)GW * lh[1] * 12)) : 0;
    {   /* tmp holds one cell's horizontal pass: as big as the largest cell */
        uint64_t big = 1;
        for (int i = 0; i < ncell; i++) { uint64_t c = (uint64_t)lw[cl[i].ix] * lh[cl[i].iy]; if (c > big) big = c; }
        m->tmp = (f32 *)ar_alloc(ar, big * 12);
    }
    int mdim = W > H ? W : H; if (GW > mdim) mdim = GW; if (GH > mdim) mdim = GH;
    m->row = (f32 *)ar_alloc(ar, (uint64_t)mdim * 12 + 64);
    m->tabi = (int *)ar_alloc(ar, (uint64_t)mdim * 26 * 4);
    m->tabw = (f32 *)ar_alloc(ar, (uint64_t)mdim * 26 * 4);
    m->fit = (f32 *)ar_alloc(ar, ((uint64_t)NR * nfit + (uint64_t)nfit * nfit + 4 * (uint64_t)nfit + 3 * NR + 16) * 4);
    for (int i = 0; i < 16; i++) {
        int k = kr[i];
        m->kb[i] = (k >= 0) ? (f32 *)ar_alloc(ar, (uint64_t)(2 * k + 1) * (2 * k + 1) * 12) : 0;
    }
}

static inline void mk(Tab *t, const Mem *m) { t->idx = m->tabi; t->w = m->tabw; t->lo = t->hi = 0; t->dn = 0; t->K = 1; }
static void zero3(f32 *p, size_t n3) { for (size_t i = 0; i < n3; i++) p[i] = 0.f; }

/* ------------------------------------------------------------------ cost model
 * Instructions the render will execute, from the plan alone.  Coefficients are wasmi fuel per unit,
 * fitted on measured renders (test/cost_fit.py); the model only has to be right within ~20 %. */
#ifdef EC_COST_DEBUG
static double ec_dbg_f[10];
EC_EXPORT("ec_dbg") int ec_dbg(void) { return (int)(intptr_t)ec_dbg_f; }
#endif
static int gtaps(f32 s) { if (!(s > 0.f)) return 1; int r = (int)ceilf_(3.3f * s); if (r < 1) r = 1; if (r > 12) r = 12; return 2 * r + 1; }
static double est_cost(int W, int H, int GW, int GH, int rot, const int *lw, const int *lh, const G2 *tm, int nt,
                       const int *kr, int ncell, const Cell *cl, int lymax, double fitwork, double kbuild) {
    double f[9] = {(double)W * H, rot ? (double)GW * GH : 0.0, 0, 0, 0, 0, 0};
    int cx = 0, cy = 0;
    for (int k = 0; k < nt; k++) {
        while (cx < tm[k].lx) { f[2] += (double)lw[cx + 1] * lh[cy] * 6.0; cx++; }
        while (cy < tm[k].ly) { f[2] += (double)lw[cx] * lh[cy + 1] * 6.0; cy++; }
        double cp = (double)lw[cx] * lh[cy];
        if (tm[k].kb >= 0) { double kw = 2.0 * kr[tm[k].kb] + 1.0; f[4] += cp * kw * kw; }
        else f[3] += cp * (double)(gtaps(tm[k].sx) + gtaps(tm[k].sy));
    }
    for (int iy = 0; iy <= lymax; iy++) f[5] += (double)GW * lh[iy];
    for (int i = 0; i < ncell; i++) f[6] += (double)lw[cl[i].ix] * lh[cl[i].iy];
    f[7] = fitwork; f[8] = kbuild;
#ifdef EC_COST_DEBUG
    for (int i = 0; i < 9; i++) ec_dbg_f[i] = f[i];
#endif
    return CK_CONST + CK_PX * f[0] + CK_ROT * f[1] + CK_DOWN * f[2] + CK_TAP * f[3] + CK_CONV * f[4] + CK_REC * f[5]
         + CK_CELL * f[6] + CK_FIT * f[7] + CK_KB * f[8];
}

/* ------------------------------------------------------------------ render */
EC_EXPORT("ec_render") int ec_render(f32 *buf, int W, int H, const double *params, int np, double time, double scale) {
    if (np < 30 || W < 1 || H < 1) return 1;
    Params P; read_params(&P, params, np);
    f32 sc = (scale == scale && scale > 0.0) ? clampf_((f32)scale, 0.01f, 64.f) : 1.f;

    /* ---- 1. input: threshold, saturation bias, scene-gamma linearise (always at host resolution) */
    f32 *TH0 = 0; /* set after the layout; the pass itself runs below */

    /* ---- anisotropy (area preserving) and the optional rotation of the whole glow */
    f32 spx = sqrtf_(P.aspect), spy = 1.f / sqrtf_(P.aspect);
    int aniso = absf_(P.aspect - 1.f) > 0.002f;
    f32 th = aniso ? P.angle : 0.f;
    int rot = absf_(th) > 1e-4f;
    int GW = W, GH = H;
    if (rot) {
        f32 cc = absf_(cosf_(th)), ss = absf_(sinf_(th));
        GW = (int)ceilf_((f32)W * cc + (f32)H * ss) + 2;
        GH = (int)ceilf_((f32)W * ss + (f32)H * cc) + 2;
        if (GW < 1) GW = 1; if (GH < 1) GH = 1;
    }
    int lw[18], lh[18]; lw[0] = GW; lh[0] = GH;
    for (int i = 1; i <= 17; i++) { lw[i] = (lw[i - 1] + 1) >> 1; lh[i] = (lh[i - 1] + 1) >> 1; }

    /* ---- per-channel radii: user multipliers x chromatic dispersion */
    f32 cam = P.chroma > 0.f ? powf_(P.chroma, 1.25f) : 0.f;
    f32 disp = (P.ca_mode == 0 || P.ca_mode == 2) ? 1.5f * cam : 0.f;
    f32 lat = (P.ca_mode >= 1) ? 0.05f * cam : 0.f;
    f32 R0 = maxf_(P.radius * sc, 1.f), Ric[3], rmax = 0.f;
    const f32 dsp[3] = {disp, 0.f, -disp};
    for (int c = 0; c < 3; c++) { Ric[c] = maxf_(R0 * P.rmul[c] * exp2_(dsp[c]), 1.f); rmax = maxf_(rmax, Ric[c]); }

    /* ---- quality -> base pyramid level, Gaussians per octave, deepest level */
    static const int QB[7] = {2, 1, 0, 0, 0, 0, 0}, QM[7] = {1, 1, 2, 3, 4, 4, 3};
    int base = (P.quality == 6) ? P.q_down : QB[P.quality];
    int mstep = (P.quality == 6) ? P.q_steps : QM[P.quality];
    int mxd = GW > GH ? GW : GH, L0 = 0; while ((1 << L0) < mxd) L0++;
    int maxl = L0 + 2; if (maxl > 15) maxl = 15;
    if (P.quality == 6 && maxl > P.q_iter) maxl = P.q_iter;
    double npx = (double)GW * (double)GH;
    int forced = npx > 24.0e6 ? 2 : (npx > 6.0e6 ? 1 : 0);
    if (base < forced) base = forced;
    if (base > L0) base = L0;
    if (maxl < base) maxl = base;

    /* ---- plan the terms (sizes only: no memory touched yet).  If the plan would cost more than the
     *       host's instruction budget (see est_cost), it is coarsened and planned again. */
    int capk_cut = 0, replans = 0;
    G2 tm[MAXI]; int nt;
replan:
    nt = 0;
    f32 st[MAXI]; int nlad = 0;
    int kr[16]; for (int i = 0; i < 16; i++) kr[i] = -1;
    int irisN = 0; int irisL[MAXI]; f32 irisR[MAXI], irisS[MAXI], irisW[MAXI];
    f32 iax = sqrtf_(P.iris_aspect), iay = 1.f / iax;   /* Iris Aspect Ratio stretches the aperture alone */
    f32 smaxs = maxf_(spx, spy);
    if (P.gmode == 2) smaxs = maxf_(spx * iax, spy * iay);

    if (P.gmode == 2) {                                   /* Lens Iris: nested procedural iris rings */
        int capk = ((P.iris_q == 0) ? 5 : (P.iris_q == 1 ? 7 : 11)) - capk_cut; if (capk < 2) capk = 2;   /* half-width of the iris kernel, in level pixels:
                                                                     * the aperture's outline is only resolved while the
                                                                     * blade bulge spans more than a level pixel */
        int N = P.iris_iter; if (N > MAXI) N = MAXI;
        f32 ge = 1.f + 2.f * (1.f - P.iris_ease), prev = 0.f;
        int ib = base;
        f32 core = maxf_(1.5f, 0.004f * rmax);   /* same soft core as the fitted kernel, so the two modes match */
        for (int i = 1; i <= N; i++) {
            f32 t = (f32)i / (f32)N, ri = rmax * powf_(t, ge);
            f32 sg = maxf_(0.3334f * (ri - prev), 0.5f); prev = ri;
            f32 reach = ri * smaxs + 3.f * sg;
            int L = ib; while (L < maxl && L < 15 && reach / (f32)(1 << L) > (f32)capk) L++;
            int k = (int)ceilf_(reach / (f32)(1 << L)); if (k > capk) k = capk; if (k < 1) k = 1;
            if (k > kr[L]) kr[L] = k;
            irisL[irisN] = L; irisR[irisN] = ri; irisS[irisN] = sg;
            irisW[irisN] = 1.f / (ri * ri + core * core);
            irisN++;
        }
        for (int L = 0; L < 16; L++) if (kr[L] >= 0) {
            tm[nt].lx = tm[nt].ly = L; tm[nt].sx = tm[nt].sy = 0.f;
            tm[nt].w[0] = tm[nt].w[1] = tm[nt].w[2] = 1.f; tm[nt].kb = L; nt++;
        }
    } else {                                              /* the fitted Gaussian ladder */
        f32 s0 = (base == 0) ? 0.80f : sqrtf_(chain_var(base));
        nlad = sigma_ladder(st, mstep, s0 / smaxs, 1.6f * rmax);
        int bx = (spx >= spy) ? base : 0, by = (spy >= spx) ? base : 0;
        for (int k = 0; k < nlad; k++) {
            int lx, ly; f32 sxk, syk;
            axis_plan(st[k] * spx, bx, maxl, &lx, &sxk);
            axis_plan(st[k] * spy, by, maxl, &ly, &syk);
            tm[nt].lx = lx; tm[nt].ly = ly; tm[nt].sx = sxk; tm[nt].sy = syk; tm[nt].kb = -1;
            tm[nt].w[0] = tm[nt].w[1] = tm[nt].w[2] = 0.f;   /* filled by the fit, after the layout */
            nt++;
        }
    }

    /* ---- cells: the distinct (lx, ly) pairs the terms land on */
    Cell cl[MAXCELL]; int ncell = 0, lxmax = 0, lymax = 0;
    for (int k = 0; k < nt; k++) {
        int f = -1;
        for (int c = 0; c < ncell; c++) if (cl[c].ix == tm[k].lx && cl[c].iy == tm[k].ly) { f = c; break; }
        if (f < 0 && ncell < MAXCELL) { cl[ncell].ix = tm[k].lx; cl[ncell].iy = tm[k].ly; cl[ncell].acc = 0; ncell++; }
        if (tm[k].lx > lxmax) lxmax = tm[k].lx;
        if (tm[k].ly > lymax) lymax = tm[k].ly;
    }

    /* ---- budget: EffectCraft runs plug-ins under an instruction budget of 50 M + 20 000 per pixel and
     *       leaves the frame untouched when it runs out.  Keep the estimate under 70 % of it: first trim the
     *       iris kernel, then move the finest work one pyramid level down. */
    {
        double budget = 0.70 * (50.0e6 + 20000.0 * (double)W * (double)H);
        double fitwork = 0.0, kbuild = 0.0;
        if (P.gmode != 2) {
            int nf = 1; if (absf_(Ric[1] - Ric[0]) > 1e-3f * Ric[0]) nf++; if (absf_(Ric[2] - Ric[0]) > 1e-3f * Ric[0] && absf_(Ric[2] - Ric[1]) > 1e-3f * Ric[1]) nf++;
            fitwork = (double)nf * nlad * nlad;
        } else for (int i = 0; i < irisN; i++) { double kw = 2.0 * kr[irisL[i]] + 1.0; kbuild += kw * kw; }
        double cost = est_cost(W, H, GW, GH, rot, lw, lh, tm, nt, kr, ncell, cl, lymax, fitwork, kbuild);
#ifdef EC_COST_DEBUG
        ec_dbg_f[9] = cost; budget = 1e300;
#endif
        if (cost > budget && replans < 24) {
            replans++;
            if (P.gmode == 2 && capk_cut < 8 && ((P.iris_q == 0) ? 5 : (P.iris_q == 1 ? 7 : 11)) - capk_cut > 3) capk_cut++;
            else if (base < L0) { base++; if (maxl < base) maxl = base; }
            else if (mstep > 1) mstep--;                     /* tiny frames: the fit itself is the cost */
            else replans = 99;
            if (replans < 99) goto replan;
        }
    }

    /* ---- memory (measure, grow, place) */
    Mem m; Arena ar = {0, 0};
    int nfit = nlad > 1 ? nlad : 1;
    uint8_t *mbase = (uint8_t *)(((uintptr_t)buf + (size_t)W * H * 16u + 15u) & ~(uintptr_t)15u);
    layout(&m, &ar, W, H, GW, GH, lw, lh, rot, cl, ncell, lxmax, lymax, nfit, kr);
#ifdef EC_NATIVE
    (void)mbase; ar.base = EC_NATIVE_HEAP; ar.off = 0;
#else
    uint64_t need64 = (uint64_t)(uintptr_t)mbase + ar.off;
    if (need64 > 0xFFFF0000ull) return 2;                   /* more than a 32-bit memory can hold */
    size_t need = (size_t)need64, have = (size_t)__builtin_wasm_memory_size(0) << 16;
    /* memory.grow returns size_t: failure is (size_t)-1, never < 0 (the old `< 0` test let a failed grow through,
     * and the render then trapped out of bounds at 4K with a rotated glow) */
    if (need > have) { size_t pg = (need - have + 65535u) >> 16; if (__builtin_wasm_memory_grow(0, (int)pg) == (size_t)-1) return 2; }
    ar.base = mbase; ar.off = 0;
#endif
    layout(&m, &ar, W, H, GW, GH, lw, lh, rot, cl, ncell, lxmax, lymax, nfit, kr);
    for (int c = 0; c < ncell; c++) cl[c].acc = m.acc[c];
    Tab tb; mk(&tb, &m);
    TH0 = m.TH;

    /* ---- 1. input pass */
    {
        f32 knee = maxf_(P.soft * (P.thr + 0.05f), 1e-4f), thr = P.thr;
        int gam = absf_(P.gamma - 1.f) > 1e-3f; f32 g = P.gamma;
        for (size_t i = 0, e = (size_t)W * H; i < e; i++) {
            const f32 *s = buf + i * 4; f32 *d = TH0 + i * 3;
            f32 a = clampf_(s[3], 0.f, 1.f);
            if (a <= 1e-6f) { d[0] = d[1] = d[2] = 0.f; continue; }
            f32 ia = 1.f / a, ur = san_(s[0]) * ia, ug = san_(s[1]) * ia, ub = san_(s[2]) * ia;
            f32 met = P.thr_mode ? maxf_(ur, maxf_(ug, ub)) : (0.2126f * ur + 0.7152f * ug + 0.0722f * ub);
            f32 x = minf_(maxf_(met - thr + knee, 0.f), 2.f * knee), q = x * x / (4.f * knee + 1e-5f);
            f32 mask = maxf_(maxf_(q, met - thr) / maxf_(met, 1e-5f), 0.f);
            if (mask > 1.f) mask = 1.f;
            if (P.sat_bias > 0.f) {                        /* fade grey light out, keep saturated light */
                f32 mxv = maxf_(ur, maxf_(ug, ub)), mnv = minf_(ur, minf_(ug, ub));
                f32 sat = mxv > 1e-6f ? (mxv - mnv) / mxv : 0.f;
                mask *= 1.f - P.sat_bias * (1.f - sat);
            }
            f32 cr = ur * mask, cg = ug * mask, cb = ub * mask;
            if (gam) { cr = powf_(cr, g); cg = powf_(cg, g); cb = powf_(cb, g); }
            d[0] = cr * a; d[1] = cg * a; d[2] = cb * a;
        }
    }
    if (P.view == 2) {                                     /* Threshold Matte: show the glow source */
        f32 invg = 1.f / P.gamma; int gam = absf_(P.gamma - 1.f) > 1e-3f;
        for (size_t i = 0, e = (size_t)W * H; i < e; i++) {
            f32 *o = buf + i * 4; const f32 *s = TH0 + i * 3;
            for (int c = 0; c < 3; c++) { f32 v = maxf_(s[c], 0.f); o[c] = gam ? powf_(v, invg) : v; }
        }
        return 0;
    }

    /* ---- 2. rotate into glow space */
    const f32 *SRC = TH0;
    if (rot) { rot_img(TH0, W, H, m.T, GW, GH, -th); SRC = m.T; }

    /* ---- 3. weights: one fit per distinct channel radius (the ladder is isotropic, so the
     *       anisotropic placement above does not change them) */
    if (P.gmode != 2) {
        f32 wts[3][MAXI];
        for (int c = 0; c < 3; c++) {
            int same = -1;
            for (int c2 = 0; c2 < c; c2++) if (absf_(Ric[c] - Ric[c2]) < 1e-3f * Ric[c]) { same = c2; break; }
            if (same >= 0) { for (int k = 0; k < nlad; k++) wts[c][k] = wts[same][k]; continue; }
            f32 a = maxf_(1.f, maxf_(1.5f * sc, 0.004f * Ric[c]));
            fit_weights(st, nlad, P.gmode == 1 ? 1 : 0, Ric[c], a, P.falloff, wts[c], m.fit);
        }
        for (int k = 0; k < nt; k++) for (int c = 0; c < 3; c++) tm[k].w[c] = wts[c][k];
    } else {
        /* iris kernel bank: nested rings, each drawn on the level that holds it, then the whole bank
         * normalised to unit energy per channel */
        f32 tot[3] = {0.f, 0.f, 0.f};
        for (int L = 0; L < 16; L++) if (kr[L] >= 0) {
            int k = kr[L], kw = 2 * k + 1; f32 *K = m.kb[L];
            zero3(K, (size_t)kw * kw * 3);
            f32 lvl = (f32)(1 << L), area = lvl * lvl;
            for (int i = 0; i < irisN; i++) {
                if (irisL[i] != L) continue;
                /* Continuous Rasterization off: the ring sits on a whole level pixel and has a hard edge */
                f32 ring = P.iris_cont ? irisR[i] : maxf_(floorf_(irisR[i] / lvl + 0.5f), 1.f) * lvl;
                f32 sg = P.iris_cont ? irisS[i] : 0.4f * lvl, w = irisW[i] * area, inv = -0.7213475f / (sg * sg);
                f32 op = P.iris_opacity;
                for (int dy = -k; dy <= k; dy++) for (int dx = -k; dx <= k; dx++) {
                    f32 u = (f32)dx * lvl / (spx * iax), v = (f32)dy * lvl / (spy * iay);
                    f32 rr = hypotf_(u, v), shp = iris_shape(atan2f_(v, u), P.iris_blades, P.iris_rot);
                    f32 *dst = K + ((size_t)(dy + k) * kw + (dx + k)) * 3;
                    for (int c = 0; c < 3; c++) {
                        f32 rc = ring * (Ric[c] / rmax);
                        f32 d = rr - rc * shp, e = exp2_(inv * d * d);
                        if (op < 0.999f) { f32 dr = rr - rc; e = op * e + (1.f - op) * exp2_(inv * dr * dr); }   /* Iris Opacity: the rest stays round */
                        dst[c] += w * e;
                    }
                }
            }
            for (int j = 0; j < kw * kw; j++) for (int c = 0; c < 3; c++) tot[c] += K[j * 3 + c];
        }
        for (int c = 0; c < 3; c++) if (!(tot[c] > 1e-20f)) tot[c] = 1.f;
        for (int L = 0; L < 16; L++) if (kr[L] >= 0) {
            int kw = 2 * kr[L] + 1; f32 *K = m.kb[L];
            for (int j = 0; j < kw * kw; j++) for (int c = 0; c < 3; c++) K[j * 3 + c] /= tot[c];
        }
    }

    /* ---- 4. anisotropic pyramid: walk the staircase of cells once.  lx and ly both grow with the
     *       ladder index, and the two half-size passes commute, so every cell is one halving away from
     *       the previous one: lxmax + lymax passes in all. */
    for (int c = 0; c < ncell; c++) zero3(cl[c].acc, (size_t)lw[cl[c].ix] * lh[cl[c].iy] * 3);
    {
        const f32 *cur = SRC; int cx = 0, cy = 0, par = 0;
        for (int k = 0; k < nt; k++) {
            while (cx < tm[k].lx) {
                f32 *dst = par ? m.wkB : m.wkA; par ^= 1;
                tab_down2(&tb, lw[cx], lw[cx + 1]);
                pass_h(cur, lw[cx], lh[cy], dst, &tb);
                cur = dst; cx++;
            }
            while (cy < tm[k].ly) {
                f32 *dst = par ? m.wkB : m.wkA; par ^= 1;
                tab_down2(&tb, lh[cy], lh[cy + 1]);
                pass_v(cur, lw[cx], dst, &tb, 0, m.row);
                cur = dst; cy++;
            }
            int ci = -1;
            for (int c = 0; c < ncell; c++) if (cl[c].ix == cx && cl[c].iy == cy) { ci = c; break; }
            if (ci < 0) continue;
            if (tm[k].kb >= 0) { conv_add(cur, lw[cx], lh[cy], m.kb[tm[k].kb], kr[tm[k].kb], cl[ci].acc); continue; }
            if (tm[k].sx > 0.f) tab_gauss(&tb, lw[cx], tm[k].sx); else tab_ident(&tb, lw[cx]);
            pass_h(cur, lw[cx], lh[cy], m.tmp, &tb);
            if (tm[k].sy > 0.f) tab_gauss(&tb, lh[cy], tm[k].sy); else tab_ident(&tb, lh[cy]);
            pass_v(m.tmp, lw[cx], cl[ci].acc, &tb, tm[k].w, m.row);
        }
    }

    /* ---- 5. reconstruction: refine x inside every row of cells, then y */
    const f32 *G = 0;
    {
        const f32 *vert = 0; int vh = 0;
        for (int iy = lymax; iy >= 0; iy--) {
            f32 *dst = (iy & 1) ? m.vB : m.vA;
            size_t cnt = (size_t)GW * lh[iy] * 3;
            if (vert) { tab_up2(&tb, vh, lh[iy]); pass_v(vert, GW, dst, &tb, 0, m.row); }
            else zero3(dst, cnt);
            int ixs = -1;
            for (int ix = lxmax; ix >= 1; ix--) {
                for (int c = 0; c < ncell; c++) if (cl[c].ix == ix && cl[c].iy == iy) { ixs = ix; break; }
                if (ixs >= 0) break;
            }
            const f32 *cur = 0;
            for (int ix = ixs; ix >= 1; ix--) {
                f32 *xb = (ix & 1) ? m.xA : m.xB;
                size_t xn = (size_t)lw[ix] * lh[iy] * 3;
                if (cur) { tab_up2(&tb, lw[ix + 1], lw[ix]); pass_h(cur, lw[ix + 1], lh[iy], xb, &tb); }
                else zero3(xb, xn);
                for (int c = 0; c < ncell; c++) if (cl[c].ix == ix && cl[c].iy == iy) {
                    const f32 *a = cl[c].acc; for (size_t j = 0; j < xn; j++) xb[j] += a[j];
                }
                cur = xb;
            }
            if (cur) {
                tab_up2(&tb, lw[1], GW); pass_h_(cur, lw[1], lh[iy], dst, &tb, 1);
            }
            for (int c = 0; c < ncell; c++) if (cl[c].ix == 0 && cl[c].iy == iy) {
                const f32 *a = cl[c].acc; for (size_t j = 0; j < cnt; j++) dst[j] += a[j];
            }
            vert = dst; vh = lh[iy];
        }
        G = vert;
    }
    /* ---- rotate the glow back */
    if (rot) { rot_img(G, GW, GH, TH0, W, H, th); G = TH0; }

    /* ---- 6. style and output */
    {
        f32 gain = P.inten * exp2_(P.expo), invg = 1.f / P.gamma;
        int gam = absf_(P.gamma - 1.f) > 1e-3f;
        f32 aspW = (f32)W / (f32)H, cx = 0.5f * (f32)(W - 1), cy = 0.5f * (f32)(H - 1);
        uint32_t fseed = (time == time && time > -1e9 && time < 1e9) ? (uint32_t)(int32_t)(time * 1000.0) : 0u;
        /* pixel aberration: the first channel of the pair moves out, the second moves in */
        static const signed char PXC[6][2] = {{0,2},{0,1},{1,2},{0,-1},{1,-1},{2,-1}};
        f32 pxs[3] = {0.f, 0.f, 0.f};
        if (lat > 0.f) { int a = PXC[P.px_ch][0], b = PXC[P.px_ch][1]; pxs[a] = lat; if (b >= 0) pxs[b] = -lat; }
        int glow_only = (P.view == 1);
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) {
            size_t pi = (size_t)y * W + x; f32 *o = buf + pi * 4; f32 gl[3];
            if (lat > 0.f) {
                for (int c = 0; c < 3; c++) {
                    if (pxs[c] == 0.f) { gl[c] = G[pi * 3 + c]; continue; }
                    f32 s = 1.f + pxs[c];
                    gl[c] = sample_cr(G, W, H, c, cx + ((f32)x - cx) * s, cy + ((f32)y - cy) * s);
                }
            } else { gl[0] = G[pi * 3]; gl[1] = G[pi * 3 + 1]; gl[2] = G[pi * 3 + 2]; }
            if (P.half) { for (int c = 0; c < 3; c++) gl[c] = half_round(gl[c]); }
            for (int c = 0; c < 3; c++) { f32 v = maxf_(gl[c], 0.f); if (gam) v = powf_(v, invg); gl[c] = v * gain; }
            if (P.dirt_amt > 0.f) {
                f32 dm = dirt_mask(((f32)x + 0.5f) / W, ((f32)y + 0.5f) / H, aspW, P.dirt_scale, P.dirt_con, P.dirt_seed);
                dm = clampf_(dm * exp2_(P.dirt_expo), 0.f, 1.f);
                if (absf_(P.dirt_gamma - 1.f) > 1e-3f) dm = powf_(dm, P.dirt_gamma);
                f32 l = 0.2126f * gl[0] + 0.7152f * gl[1] + 0.0722f * gl[2];
                f32 k = P.dirt_amt * dm * 4.f * (l / (l + 0.25f));
                for (int c = 0; c < 3; c++) gl[c] += k * lerpf_(l, gl[c], P.dirt_inherit);
            }
            if (P.tint_style == 1) apply_tint(gl, &P);
            else if (P.tint_amt > 0.f) {
                f32 lum = 0.2126f * gl[0] + 0.7152f * gl[1] + 0.0722f * gl[2], t = lum / (1.f + lum);
                for (int c = 0; c < 3; c++) {
                    f32 col = t < 0.5f ? lerpf_(P.tint[0][c], P.tint[1][c], t * 2.f) : lerpf_(P.tint[1][c], P.tint[2][c], (t - 0.5f) * 2.f);
                    gl[c] = lerpf_(gl[c], col * lum, P.tint_amt);
                }
            }
            if (P.tonemap > 0 && P.tm_mix > 0.f) {         /* hue preserving: RGB scaled by TM(max)/max */
                f32 mxc = maxf_(gl[0], maxf_(gl[1], gl[2]));
                if (mxc > 1e-9f) { f32 s = lerpf_(1.f, tm_curve(mxc, P.tonemap) / mxc, P.tm_mix); gl[0] *= s; gl[1] *= s; gl[2] *= s; }
            }
            if (P.dither > 0.f) {                          /* TPDF, monochrome, step = 1/255 in a gamma-2.2 display */
                uint32_t hsh = hash_u((uint32_t)x * 0x9E3779B1u ^ hash_u((uint32_t)y * 0x85EBCA77u + fseed));
                f32 n1 = (f32)(hsh & 0xFFFFu) * (1.f / 65536.f), n2 = (f32)(hsh >> 16) * (1.f / 65536.f);
                f32 tn = (n1 + n2 - 1.f) * P.dither;
                for (int c = 0; c < 3; c++) if (gl[c] > 0.f) {
                    f32 tc = tn;
                    if (!P.dither_mono) {                      /* a different noise value per channel */
                        uint32_t hc = hash_u(hsh + 0x6C8E9CF5u * (uint32_t)(c + 1));
                        tc = ((f32)(hc & 0xFFFFu) * (1.f / 65536.f) + (f32)(hc >> 16) * (1.f / 65536.f) - 1.f) * P.dither;
                    }
                    f32 stp = 0.0086274f * sqrtf_(gl[c]); gl[c] = maxf_(gl[c] + tc * stp, 0.f);
                }
            }
            for (int c = 0; c < 3; c++) gl[c] = san_(gl[c]);        /* NaN -> 0, never Inf or negative */
            f32 ag = clampf_(maxf_(gl[0], maxf_(gl[1], gl[2])), 0.f, 1.f);
            if (glow_only) { o[0] = gl[0]; o[1] = gl[1]; o[2] = gl[2]; o[3] = P.unmult ? ag : 1.f; continue; }
            f32 sa = clampf_(o[3], 0.f, 1.f) * P.src_op;
            for (int c = 0; c < 3; c++) {
                f32 s = san_(o[c]) * P.src_op, g = gl[c], u = clampf_(s, 0.f, 1.f), r;
                switch (P.blend) {
                case 1: case 5: r = s + g; break;                                   /* Add / Linear Dodge   */
                case 2: r = g + s * (1.f - ag); break;                              /* Normal (glow over)   */
                case 3: r = maxf_(s, g); break;                                     /* Lighten              */
                case 4: r = (u >= 0.999f) ? maxf_(s, g) : minf_(s / (1.f - clampf_(g, 0.f, 0.999f)), 1e5f); break;   /* Colour Dodge */
                case 6: r = (u < 0.5f) ? 2.f * s * g : s + g * (1.f - u);  break;   /* Overlay (HDR safe)   */
                case 7: { f32 gg = clampf_(g, 0.f, 1.f);                            /* Soft Light           */
                          r = (gg < 0.5f) ? s * (1.f - (1.f - 2.f * gg) * (1.f - u)) : s + (2.f * gg - 1.f) * (sqrtf_(u) - u); break; }
                default: r = s + g * (1.f - u); break;                              /* Screen (HDR safe)    */
                }
                o[c] = (r == r) ? r : s;
            }
            o[3] = P.unmult ? (sa + ag - sa * ag) : clampf_(o[3], 0.f, 1.f);
        }
    }
    return 0;
}
