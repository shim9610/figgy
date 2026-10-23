// Glyph layer: the arrow.
//
// Everything here is in a 2D pixel space with a tail and a tip already
// projected; it knows nothing about axes, grids or cameras. The shaft and the
// head are screen-facing, so the same glyph serves a 3D projection as
// billboarded arrows.
//
// One oriented quad per arrow (`ARROW_VERTICES` vertices) covers shaft, head
// and an anti-aliasing margin. The fragment stage evaluates the exact signed
// distance to the union of the shaft (a segment with half-width `width / 2`)
// and the head (a triangle) and turns it into coverage, so edges are analytic
// at any zoom and the quad's size does not change the drawing.

const ARROW_VERTICES: u32 = 6u;
const ARROW_PIVOT_TAIL: u32 = 0u;
const ARROW_PIVOT_MIDDLE: u32 = 1u;
const ARROW_PIVOT_TIP: u32 = 2u;
// A shaft shorter than this (pixels) is no shaft. The head takes the whole
// length of an arrow shorter than the head; the fragment stage recomputes
// that length from the pivot-shifted tail and tip, and the rounding must not
// turn the missing shaft into a round dot at the tail.
const ARROW_SHAFT_MIN_PX: f32 = 0.01;

/// Pixel-space arrow style. Sizes are already multiplied by the render scale.
struct ArrowStyle {
    width_px: f32,
    head_length_px: f32,
    head_width_px: f32,
    min_length_px: f32,
    pivot: u32,
};

/// An arrow placed on its anchor: tail and tip after the pivot shift, and the
/// head dimensions after scaling down for a short arrow.
struct ArrowPlacement {
    tail: vec2<f32>,
    tip: vec2<f32>,
    head_length_px: f32,
    head_width_px: f32,
    valid: bool,
};

fn arrow_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// Apply the pivot and the minimum length, and size the head to the arrow.
/// A zero-length arrow, one shorter than `min_length_px`, or one with a
/// non-finite end is not drawn.
fn arrow_place(tail: vec2<f32>, tip: vec2<f32>, style: ArrowStyle) -> ArrowPlacement {
    var out: ArrowPlacement;
    out.tail = tail;
    out.tip = tip;
    out.head_length_px = 0.0;
    out.head_width_px = 0.0;
    out.valid = false;
    let delta = tip - tail;
    let len = length(delta);
    if (!arrow_finite(len) || len <= 0.0 || len < style.min_length_px) {
        return out;
    }
    var shift = 0.0;
    if (style.pivot == ARROW_PIVOT_MIDDLE) {
        shift = -0.5;
    } else if (style.pivot == ARROW_PIVOT_TIP) {
        shift = -1.0;
    }
    out.tail = tail + delta * shift;
    out.tip = tip + delta * shift;
    let head_length = min(max(style.head_length_px, 0.0), len);
    out.head_length_px = head_length;
    if (style.head_length_px > 0.0) {
        out.head_width_px = max(style.head_width_px, 0.0) * (head_length / style.head_length_px);
    }
    out.valid = true;
    return out;
}

/// Corner `vertex_index` (0..ARROW_VERTICES) of the quad that covers the
/// placed arrow plus `margin_px` on every side, in the glyph's pixel space.
/// The shaft's round end reaches half its width behind the tail, so the quad
/// reaches that far too: a quad cut at the tail plus the margin clipped the
/// end the coverage draws (and the picker measures).
fn arrow_quad_corner(vertex_index: u32, placed: ArrowPlacement, width_px: f32, margin_px: f32) -> vec2<f32> {
    let delta = placed.tip - placed.tail;
    let len = length(delta);
    let dir = delta / max(len, 1e-12);
    let nrm = vec2<f32>(-dir.y, dir.x);
    let center = (placed.tail + placed.tip) * 0.5;
    let half_len = len * 0.5 + max(width_px, 0.0) * 0.5 + margin_px;
    let half_wid = max(width_px * 0.5, placed.head_width_px * 0.5) + margin_px;
    var cx = -1.0;
    var cy = -1.0;
    // Two triangles sharing the (-1,-1) → (1,1) diagonal.
    switch (vertex_index) {
        case 1u: { cx = 1.0; cy = -1.0; }
        case 2u, 4u: { cx = 1.0; cy = 1.0; }
        case 5u: { cx = -1.0; cy = 1.0; }
        default: { cx = -1.0; cy = -1.0; }
    }
    return center + dir * (cx * half_len) + nrm * (cy * half_wid);
}

/// Unsigned distance from `p` to the segment `ab`; a zero-length segment is a
/// point.
fn arrow_sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

/// Signed distance from `p` to the triangle `p0 p1 p2` (negative inside).
fn arrow_sd_triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 1e-12), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 1e-12), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 1e-12), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d = min(
        min(
            vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x)),
            vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x)),
        ),
        vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x)),
    );
    return -sqrt(d.x) * sign(d.y);
}

/// Signed distance from pixel `p` to the placed arrow: the union of the
/// shaft (a segment `width_px` wide) and the head (a triangle). Negative
/// inside; `1e20` for an arrow with neither.
fn arrow_distance(p: vec2<f32>, placed: ArrowPlacement, width_px: f32) -> f32 {
    let delta = placed.tip - placed.tail;
    let len = length(delta);
    let dir = delta / max(len, 1e-12);
    let nrm = vec2<f32>(-dir.y, dir.x);
    let head_base = placed.tip - dir * placed.head_length_px;
    var distance = 1e20;
    // The shaft stops where the head begins. A shaft of zero length draws
    // nothing rather than a dot.
    if (len - placed.head_length_px > ARROW_SHAFT_MIN_PX && width_px > 0.0) {
        distance = min(distance, arrow_sd_segment(p, placed.tail, head_base) - width_px * 0.5);
    }
    if (placed.head_length_px > 0.0 && placed.head_width_px > 0.0) {
        let half = nrm * (placed.head_width_px * 0.5);
        distance = min(distance, arrow_sd_triangle(p, placed.tip, head_base + half, head_base - half));
    }
    return distance;
}

/// Coverage of pixel `p` by the placed arrow: `arrow_distance`
/// anti-aliased over `aa_px`.
fn arrow_coverage(p: vec2<f32>, placed: ArrowPlacement, width_px: f32, aa_px: f32) -> f32 {
    let distance = arrow_distance(p, placed, width_px);
    let aa = max(aa_px, 1e-3);
    return clamp(0.5 - distance / aa, 0.0, 1.0);
}

/// `pair + s` as a `(hi, lo)` pair: the error-free sum keeps the low part, so a
/// large-offset coordinate still resolves a small data-space step.
fn pair_add(pair: vec2<f32>, s: f32) -> vec2<f32> {
    let sum = fma(1.0, pair.x, s);
    let b_virtual = fma(1.0, sum, -pair.x);
    let a_virtual = fma(1.0, sum, -b_virtual);
    let b_roundoff = fma(1.0, s, -b_virtual);
    let a_roundoff = fma(1.0, pair.x, -a_virtual);
    return vec2<f32>(sum, fma(1.0, a_roundoff, b_roundoff) + pair.y);
}

/// Pixels per data unit along one axis whose range is `range` (the hi/lo
/// sum), at `pair`: the derivative of `axis_pair_to_t` mapped through NDC.
/// Exact on a linear axis; the local direction on a logarithmic one, the way
/// the contour labels orient themselves.
fn axis_px_per_unit_at(range: f32, panel_scale: f32, is_log: f32, pixel_to_ndc: f32, pair: vec2<f32>) -> f32 {
    let linear = 2.0 * panel_scale / (range * pixel_to_ndc);
    // d(log10 v)/dv = 1 / (v ln 10); `sample_valid` has already excluded v <= 0.
    let value = pair.x + pair.y;
    let log_slope = linear / (value * log(10.0));
    if is_log > 0.5 {
        return log_slope;
    }
    return mix(linear, log_slope, is_log);
}
