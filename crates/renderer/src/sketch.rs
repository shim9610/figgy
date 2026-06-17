//! Hand-drawn ("sketch") geometry helpers — deterministic polyline wobble.
//!
//! Implements the noise specification of `docs/SKETCH_DESIGN.md` §3 (PCG-style
//! integer hash → 1D smoothstep value noise) and the subdivision rules of §5a.
//! The noise/geometry layer ([`hash01`] … [`sketch_rect_outline`]) is pure
//! functions over primitive parameters — no wgpu, tiny-skia, or model types.
//! Coordinates are pixel-space (y-down), matching the raster/deco consumers.
//! Everything is `f32`, with no runtime randomness or clocks — identical
//! inputs yield bit-identical outputs (§3 determinism contract).
//!
//! [`DecoStroker`] (STYLE_REGISTRY.md §4) is the deco layer's entry point: a
//! per-raster-pass stroke strategy that routes each decoration stroke either
//! to the plain [`Canvas`] calls (`Precise`) or through the wobble functions
//! above (`Sketch`). It is the one raster-facing piece of this module.

use crate::config::DrawStyle;
use crate::raster::{Canvas, Paint};

/// Spacing between subdivision points never exceeds this many pixels
/// (design §5a: "min 2 subdivisions, at most ~16 px per subdivision").
const MAX_SUBDIV_SPACING_PX: f32 = 16.0;

/// Hard safety cap on subdivisions per input segment — guards pathological
/// `len / wavelength` ratios against unbounded vertex allocation. At the
/// 16 px spacing rule this still covers segments up to ~65 k px, far beyond
/// any chart canvas.
const MAX_SUBDIV_PER_SEGMENT: f32 = 4096.0;

/// PCG-style integer hash (design §3) mapping `(i, seed)` into `[0, 1)`.
///
/// Spec formula (all integer arithmetic wrapping mod 2³²):
///
/// ```text
/// h = i*0x9E3779B9 ^ seed*0x85EBCA6B
/// h = (h ^ (h >> 16)) * 0x45D9F3B
/// h = h ^ (h >> 16)
/// hash01 = f32(h) / 4294967296.0
/// ```
///
/// Caveat inherited from the spec formula: for the 128 inputs whose final
/// `h ≥ 2³² − 128`, the `u32 → f32` conversion rounds up to 2³² and the
/// result is exactly 1.0 (probability ≈ 2⁻²⁵). Downstream math ([`noise1`])
/// still honours its `[-1, 1]` contract, so this is documented, not
/// special-cased.
pub(crate) fn hash01(i: u32, seed: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    h = (h ^ (h >> 16)).wrapping_mul(0x045D_9F3B);
    h ^= h >> 16;
    h as f32 / 4_294_967_296.0
}

/// 1D value noise (design §3): smoothstep-interpolated lattice hashes —
/// C1-continuous in `t`, range `[-1, 1]`.
///
/// `t` is non-negative in practice (`arc_px / wavelength_px`), but any finite
/// `t` stays deterministic: the lattice index wraps mod 2³², and the wrap is
/// consistent between adjacent cells (`i + 1` uses `wrapping_add`).
pub(crate) fn noise1(t: f32, seed: u32) -> f32 {
    let cell = t.floor();
    let f = t - cell; // fract(t) ∈ [0, 1)
    let u = f * f * (3.0 - 2.0 * f); // smoothstep
    // `as i64` first: a direct f32→u32 cast saturates negatives to 0, which
    // would break lattice continuity for t < 0.
    let i = cell as i64 as u32;
    let a = hash01(i, seed);
    let b = hash01(i.wrapping_add(1), seed);
    (a + (b - a) * u) * 2.0 - 1.0 // mix(a, b, u) * 2 - 1
}

/// FNV-1a 32-bit hash of `tag`'s UTF-8 bytes — derives per-element seeds for
/// the deco layer (design §5a: `seed' = seed ^ fnv1a(tag)`, with stable tags
/// like `"axis_left"`, `"tick_x_3"`, `"legend_box"`).
pub(crate) fn fnv1a(tag: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5; // FNV offset basis
    for &byte in tag.as_bytes() {
        h ^= u32::from(byte);
        h = h.wrapping_mul(0x0100_0193); // FNV prime
    }
    h
}

/// Perturb a pixel-space polyline into a hand-drawn squiggle (design §3/§5a).
///
/// Each segment of length `L` is subdivided into
/// `ceil(L / (wavelength_px / 4))` pieces (at least 2, spacing capped at
/// ~16 px), and every subdivision point — including the very first and last
/// point of the polyline; hand-drawn ends wobble too — is displaced
/// perpendicular to its segment's travel direction by
/// `amplitude_px * noise1(arc / wavelength_px, seed)`. `arc` is the
/// cumulative arc length measured along the **original** (undisplaced) path,
/// so displacement never feeds back into the noise parameter.
///
/// Interior vertices are emitted once, displaced along the **incoming**
/// segment's perpendicular; the noise phase carries straight through corners
/// (this is what makes [`sketch_rect_outline`] arc-continuous).
///
/// Degenerate handling:
/// - `amplitude_px == 0.0`, `wavelength_px <= 0.0`, non-finite parameters, or
///   fewer than 2 points: returns an exact copy of `points`.
/// - Non-finite input points pass through verbatim (upper layers render them
///   as gaps); the finite runs on either side are sketched normally — their
///   own endpoints still wobble — and the arc parameter carries on across
///   the gap.
/// - Zero-length segments have no direction to be perpendicular to: they
///   contribute no arc and repeat the previously emitted point.
pub(crate) fn sketch_polyline(
    points: &[(f32, f32)],
    amplitude_px: f32,
    wavelength_px: f32,
    seed: u32,
) -> Vec<(f32, f32)> {
    if amplitude_px == 0.0
        || wavelength_px <= 0.0
        || !amplitude_px.is_finite()
        || !wavelength_px.is_finite()
        || points.len() < 2
    {
        return points.to_vec();
    }

    let finite = |p: &(f32, f32)| p.0.is_finite() && p.1.is_finite();
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len() * 4);
    let mut arc_px = 0.0_f32; // cumulative arc along the ORIGINAL path
    let mut idx = 0;
    while idx < points.len() {
        if !finite(&points[idx]) {
            out.push(points[idx]); // gap sentinel: pass through untouched
            idx += 1;
            continue;
        }
        // Maximal run of finite points.
        let start = idx;
        while idx < points.len() && finite(&points[idx]) {
            idx += 1;
        }
        let run = &points[start..idx];
        if run.len() == 1 {
            out.push(run[0]); // isolated point: no travel direction
        } else {
            sketch_finite_run(
                run,
                amplitude_px,
                wavelength_px,
                seed,
                &mut arc_px,
                &mut out,
            );
        }
    }
    out
}

/// Subdivide and displace one maximal run of finite points (§5a rules).
/// `arc_px` accumulates across runs so the noise phase continues over gaps.
fn sketch_finite_run(
    run: &[(f32, f32)],
    amplitude_px: f32,
    wavelength_px: f32,
    seed: u32,
    arc_px: &mut f32,
    out: &mut Vec<(f32, f32)>,
) {
    let mut emitted_start = false;
    for pair in run.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 || !len.is_finite() {
            // Zero-length (or magnitude-overflowed) segment: no direction.
            // Keep the endpoint, repeat the last emitted position, add no arc.
            if !emitted_start {
                out.push(a);
                emitted_start = true;
            }
            let last = *out.last().expect("emitted_start guarantees non-empty");
            out.push(last);
            continue;
        }
        // Unit perpendicular to the travel direction (pixel space, y-down;
        // the sign is immaterial — noise is symmetric about 0).
        let (nx, ny) = (-dy / len, dx / len);
        // §5a subdivision count: ceil(L / (wavelength/4)), at least 2,
        // spacing never above ~16 px, plus a hard safety cap.
        let n = (len / (wavelength_px / 4.0))
            .ceil()
            .max((len / MAX_SUBDIV_SPACING_PX).ceil())
            .max(2.0)
            .min(MAX_SUBDIV_PER_SEGMENT) as u32;
        // Shared interior vertices were already emitted by the previous
        // segment (displaced along its perpendicular) — skip j = 0 then.
        let first_j = u32::from(emitted_start);
        for j in first_j..=n {
            let t = j as f32 / n as f32;
            let arc = *arc_px + len * t;
            let d = amplitude_px * noise1(arc / wavelength_px, seed);
            out.push((a.0 + dx * t + nx * d, a.1 + dy * t + ny * d));
        }
        emitted_start = true;
        *arc_px += len;
    }
}

/// Two-point convenience wrapper over [`sketch_polyline`].
pub(crate) fn sketch_line(
    p0: (f32, f32),
    p1: (f32, f32),
    amplitude_px: f32,
    wavelength_px: f32,
    seed: u32,
) -> Vec<(f32, f32)> {
    sketch_polyline(&[p0, p1], amplitude_px, wavelength_px, seed)
}

/// Closed rectangle outline as one continuous sketched polyline.
///
/// The four edges are chained into a single path that revisits the starting
/// corner to close, so the arc parameter — and with it the noise phase —
/// carries across every corner (design §5a). One shared `seed`; deliberately
/// **no** per-edge seed mixing, continuity wins. Like a real hand-drawn box,
/// the closing point is displaced at a different noise phase than the opening
/// point and need not coincide with it exactly.
pub(crate) fn sketch_rect_outline(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    amplitude_px: f32,
    wavelength_px: f32,
    seed: u32,
) -> Vec<(f32, f32)> {
    let corners = [
        (x, y),
        (x + w, y),
        (x + w, y + h),
        (x, y + h),
        (x, y), // revisit start: closes the outline
    ];
    sketch_polyline(&corners, amplitude_px, wavelength_px, seed)
}

// Decoration stroke strategy — STYLE_REGISTRY.md §4.

/// Minimum stroke width (px) for sketch-mode decoration strokes. A wobbled
/// 1 px hairline drifts across pixel rows and alternates between crisp and
/// 50/50-blurred AA coverage — it reads as a broken line. Hand-drawn pen
/// strokes have body; this floor restores it without touching user widths
/// that are already thicker.
const SKETCH_MIN_STROKE_PX: f32 = 1.4;

/// Decoration-layer stroke strategy, derived once per raster pass from the
/// chart's [`DrawStyle`]. `Precise` is a pure passthrough to the plain canvas
/// calls — byte-identical to pre-stroker rendering. `Sketch` replaces each
/// stroke with its seeded wobble path, drawn as a smooth Catmull–Rom spline
/// ([`Canvas::draw_polyline_smooth`]) with a [`SKETCH_MIN_STROKE_PX`] width
/// floor; color and dash come through unchanged, so dashes run along the
/// wobbled curve.
///
/// `tag` is a stable element tag (`"axis_left"`, `"tick_left_3"`,
/// `"grid_major_x_2"`, `"legend_box"`) mixed into the global seed as
/// `seed ^ fnv1a(tag)` (design §5a), so every element wobbles differently but
/// identically across re-rasters.
pub(crate) enum DecoStroker {
    Precise,
    Sketch {
        amplitude_px: f32,
        wavelength_px: f32,
        seed: u32,
    },
}

impl DecoStroker {
    /// Derive the stroker for a chart style: `Sketch` copies the wobble
    /// parameters, every other style strokes precisely.
    pub(crate) fn from_style(style: &DrawStyle) -> Self {
        match style {
            DrawStyle::Sketch(s) => DecoStroker::Sketch {
                amplitude_px: s.amplitude_px,
                wavelength_px: s.wavelength_px,
                seed: s.seed,
            },
            _ => DecoStroker::Precise,
        }
    }

    /// Stroke one straight deco segment. `Precise` is a single
    /// [`Canvas::draw_line`], identical to pre-stroker rendering; `Sketch`
    /// replaces the segment with its seeded wobble polyline.
    pub(crate) fn stroke_segment(
        &self,
        canvas: &mut Canvas,
        p0: (f32, f32),
        p1: (f32, f32),
        paint: &Paint,
        tag: &str,
    ) {
        match self {
            DecoStroker::Precise => canvas.draw_line(p0, p1, paint),
            DecoStroker::Sketch {
                amplitude_px,
                wavelength_px,
                seed,
            } => {
                let pts = sketch_line(p0, p1, *amplitude_px, *wavelength_px, seed ^ fnv1a(tag));
                let paint = paint.clone().with_min_stroke_width(SKETCH_MIN_STROKE_PX);
                canvas.draw_polyline_smooth(&pts, &paint);
            }
        }
    }

    /// Stroke a rectangle outline (e.g. the legend border). `Precise` is a
    /// single [`Canvas::draw_rect`] with the given (stroke) paint; `Sketch`
    /// chains the four edges into one arc-continuous wobble polyline via
    /// [`sketch_rect_outline`].
    #[allow(clippy::too_many_arguments)] // signature fixed by STYLE_REGISTRY §4
    pub(crate) fn stroke_rect_outline(
        &self,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        paint: &Paint,
        tag: &str,
    ) {
        match self {
            DecoStroker::Precise => canvas.draw_rect(x, y, w, h, paint),
            DecoStroker::Sketch {
                amplitude_px,
                wavelength_px,
                seed,
            } => {
                let pts = sketch_rect_outline(
                    x,
                    y,
                    w,
                    h,
                    *amplitude_px,
                    *wavelength_px,
                    seed ^ fnv1a(tag),
                );
                let paint = paint.clone().with_min_stroke_width(SKETCH_MIN_STROKE_PX);
                canvas.draw_polyline_smooth(&pts, &paint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (a) Determinism: identical inputs → identical outputs.
    #[test]
    fn determinism_same_input_same_output() {
        let pts = [(0.0, 0.0), (40.0, 10.0), (90.0, -5.0), (160.0, 30.0)];
        assert_eq!(
            sketch_polyline(&pts, 1.5, 60.0, 7),
            sketch_polyline(&pts, 1.5, 60.0, 7)
        );
        assert_eq!(
            sketch_line((0.0, 0.0), (100.0, 0.0), 2.0, 50.0, 3),
            sketch_line((0.0, 0.0), (100.0, 0.0), 2.0, 50.0, 3)
        );
        assert_eq!(
            sketch_rect_outline(10.0, 20.0, 200.0, 100.0, 1.5, 60.0, 0),
            sketch_rect_outline(10.0, 20.0, 200.0, 100.0, 1.5, 60.0, 0)
        );
    }

    // (b) Seed separation: different seeds → different wobble.
    #[test]
    fn seed_changes_output() {
        let s0 = sketch_line((0.0, 0.0), (200.0, 0.0), 1.5, 60.0, 0);
        let s1 = sketch_line((0.0, 0.0), (200.0, 0.0), 1.5, 60.0, 1);
        assert_eq!(s0.len(), s1.len(), "seed must not change subdivision");
        assert_ne!(s0, s1, "seed 0 vs 1 produced identical geometry");
    }

    // (c) Amplitude bound: |Δy| ≤ amplitude on a horizontal line (the
    // perpendicular is exactly vertical, so x must survive untouched too).
    #[test]
    fn horizontal_line_displacement_bounded() {
        let amp = 1.5;
        for seed in 0..32u32 {
            let out = sketch_line((0.0, 50.0), (300.0, 50.0), amp, 60.0, seed);
            for &(px, py) in &out {
                assert!(
                    (py - 50.0).abs() <= amp + 1e-4,
                    "seed {seed}: |Δy| = {} exceeds amplitude {amp}",
                    (py - 50.0).abs()
                );
                assert!(
                    (-1e-4..=300.0 + 1e-4).contains(&px),
                    "seed {seed}: x = {px} drifted along the travel direction"
                );
            }
        }
    }

    // (d) Amplitude 0 (and wavelength ≤ 0) → exact input copy, count included.
    #[test]
    fn zero_amplitude_returns_input_copy() {
        let pts = vec![(0.0, 0.0), (40.0, 10.0), (90.0, -5.0)];
        assert_eq!(sketch_polyline(&pts, 0.0, 60.0, 5), pts);
        assert_eq!(sketch_polyline(&pts, 1.5, 0.0, 5), pts);
        assert_eq!(sketch_polyline(&pts, 1.5, -3.0, 5), pts);
    }

    // (e) Subdivision density: L = 100, wavelength = 60 →
    // ceil(100 / 15) = 7 segments → at least 8 points.
    #[test]
    fn subdivision_density_meets_minimum() {
        let out = sketch_line((0.0, 0.0), (100.0, 0.0), 1.0, 60.0, 0);
        assert!(out.len() >= 8, "expected ≥ 8 points, got {}", out.len());
        // 16 px spacing rule kicks in when wavelength/4 is coarser:
        // L = 100, wavelength = 400 → ceil(100/16) = 7 segments → 8 points.
        let coarse = sketch_line((0.0, 0.0), (100.0, 0.0), 1.0, 400.0, 0);
        assert!(
            coarse.len() >= 8,
            "spacing cap violated: {} points",
            coarse.len()
        );
    }

    // (f) noise1 is C1-continuous: adjacent samples differ by ≪ 0.05
    // (max analytic slope is 3, so Δt = 1e-3 → Δ ≤ ~0.003).
    #[test]
    fn noise1_is_continuous_and_bounded() {
        for seed in [0u32, 1, 99] {
            for k in 0..20_000u32 {
                let t = k as f32 * 1e-3;
                let v0 = noise1(t, seed);
                let v1 = noise1(t + 1e-3, seed);
                assert!(
                    (-1.0..=1.0).contains(&v0),
                    "noise1({t}) = {v0} out of range"
                );
                assert!(
                    (v1 - v0).abs() < 0.05,
                    "seed {seed}: jump {} at t = {t}",
                    (v1 - v0).abs()
                );
            }
        }
    }

    // (g) hash01 stays in [0, 1) over many spread-out samples.
    #[test]
    fn hash01_in_unit_interval() {
        for seed in [0u32, 1, 0xDEAD_BEEF] {
            for k in 0..10_000u32 {
                let i = k.wrapping_mul(2_654_435_761); // spread inputs over u32
                let h = hash01(i, seed);
                assert!((0.0..1.0).contains(&h), "hash01({i}, {seed}) = {h}");
            }
        }
    }

    // FNV-1a 32-bit reference vectors (Noll's published test set).
    #[test]
    fn fnv1a_known_vectors() {
        assert_eq!(fnv1a(""), 0x811C_9DC5);
        assert_eq!(fnv1a("a"), 0xE40C_292C);
        assert_eq!(fnv1a("foobar"), 0xBF9C_F968);
    }

    // Non-finite points pass through verbatim; surrounding finite runs still
    // wobble (their endpoints included), and the result stays deterministic
    // bit-for-bit even with NaN present.
    #[test]
    fn non_finite_points_pass_through() {
        let pts = [
            (0.0, 0.0),
            (50.0, 0.0),
            (f32::NAN, f32::NAN),
            (100.0, 0.0),
            (150.0, 0.0),
        ];
        let out = sketch_polyline(&pts, 1.5, 60.0, 0);
        let nan_count = out.iter().filter(|p| p.0.is_nan() || p.1.is_nan()).count();
        assert_eq!(nan_count, 1, "gap sentinel must survive exactly once");
        for p in out.iter().filter(|p| p.0.is_finite()) {
            assert!(
                p.1.abs() <= 1.5 + 1e-4,
                "finite point escaped amplitude: {p:?}"
            );
        }
        // Bitwise determinism (NaN breaks PartialEq comparison).
        let again = sketch_polyline(&pts, 1.5, 60.0, 0);
        assert_eq!(out.len(), again.len());
        for (p, q) in out.iter().zip(&again) {
            assert_eq!(p.0.to_bits(), q.0.to_bits());
            assert_eq!(p.1.to_bits(), q.1.to_bits());
        }
    }

    // Rect outline: starts and ends near the origin corner, never escapes the
    // rect inflated by the amplitude, and is dense enough for four edges.
    #[test]
    fn rect_outline_closed_and_bounded() {
        let (x, y, w, h, amp) = (10.0, 20.0, 200.0, 120.0, 1.5);
        let out = sketch_rect_outline(x, y, w, h, amp, 60.0, 3);
        assert!(out.len() > 8, "four edges should subdivide: {}", out.len());
        let eps = amp + 1e-3;
        for &(px, py) in &out {
            assert!((x - eps..=x + w + eps).contains(&px), "x escaped: {px}");
            assert!((y - eps..=y + h + eps).contains(&py), "y escaped: {py}");
        }
        let first = out[0];
        let last = *out.last().unwrap();
        assert!((first.0 - x).abs() <= eps && (first.1 - y).abs() <= eps);
        assert!((last.0 - x).abs() <= eps && (last.1 - y).abs() <= eps);
    }
}
