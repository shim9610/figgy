struct VsIn {
    @location(0) x_a: vec2<f32>,
    @location(1) y_a: vec2<f32>,
    @location(2) x_b: vec2<f32>,
    @location(3) y_b: vec2<f32>,
};

// Line-only extra instance inputs (outside the common block): cumulative
// arc length at the segment's two endpoints, from the GPU-computed prefix.
struct VsArc {
    @location(4) arc_a: f32,
    @location(5) arc_b: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Arc length (px) from the polyline start: arc_a at the two A-side
    // corners, arc_b at the two B-side corners — interpolation yields the
    // per-fragment arc position the dash pattern walks.
    @location(0) dist_px: f32,
    // Local segment-space coordinates in pixels. The fragment stage treats
    // the whole segment stroke as one square-cap rectangle:
    //   along_px ∈ [-half_w, len + half_w], side_px ∈ [-half_w, half_w].
    @location(1) along_px: f32,
    @location(2) side_px: f32,
    @location(3) segment_len_px: f32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
};


// ───── shared line stroke (SHADER_COMMON.md §5.6; `shader_consistency`
// compares these function bodies and constants with the pick shader) ─────

// Solid strokes grow their quad by this anti-aliasing fringe on every side;
// dashed strokes keep the bare stroke so the measured dash lengths hold.
const LINE_AA_EXTENT_PX: f32 = 0.5;
// Subdivision count S per segment instance: the sketch pipeline draws
// 2*(S+1) strip vertices per instance. The CPU-side draw count lives in
// mod.rs (`LINE_SKETCH_VERTICES_PER_INSTANCE`) — keep the two in sync.
const SKETCH_SUBDIV: u32 = 8u;

/// Signed pixel distance from a stroke fragment to the edge of its piece's
/// square-capped rectangle. `along` runs from the piece start along its
/// direction, `side` across it; the rectangle is
/// `[-half_w, len + half_w] x [-half_w, half_w]`. The stroke covers a pixel
/// where this is below its anti-aliasing width.
fn line_stroke_distance(along: f32, side: f32, len: f32, half_w: f32) -> f32 {
    let p = vec2<f32>(along - len * 0.5, side);
    let half_rect = vec2<f32>(len * 0.5 + half_w, half_w);
    let q = abs(p) - half_rect;
    let outside = max(q, vec2<f32>(0.0, 0.0));
    return length(outside) + min(max(q.x, q.y), 0.0);
}

/// Whether the dash pattern is "on" at arc position `dist_px`. `dash` packs
/// up to eight sequential `[on, off, ...]` pixel lengths (`dash[0].xyzw`
/// first) and `count` says how many are used; zero is a solid line, and so is
/// a pattern whose spans are all zero or negative.
fn dash_pattern_on(dist_px: f32, dash: array<vec4<f32>, 2>, count: u32) -> bool {
    if count == 0u {
        return true;
    }
    // A local copy, so the spans can be indexed at run time on every
    // backend. Clamp to the 8-scalar capacity so a bad CPU-side count cannot
    // index past the array.
    var spans = dash;
    let n = min(count, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(spans[i / 4u][i % 4u], 0.0);
    }
    if period <= 0.0 {
        return true;
    }
    // The square-cap extension can push dist_px slightly negative at the
    // polyline start — wrap into [0, period) with the sign-safe form.
    let phase = ((dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        let end = acc + max(spans[i / 4u][i % 4u], 0.0);
        acc = end;
        if phase <= end || i == n - 1u {
            // Even spans are "on", odd spans are "off". Dash membership is
            // intentionally binary so the requested pattern length stays
            // stable; antialiasing is applied to the stroke outline itself.
            return (i & 1u) == 0u;
        }
    }
    // phase == period can occur from float rounding; that point is the
    // start of the next repetition, which always opens with an "on" span.
    return true;
}
