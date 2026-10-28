// Solid strokes grow their quad by this anti-aliasing fringe on every side;
// dashed strokes keep the bare stroke so the measured dash lengths hold.
const LINE_AA_EXTENT_PX: f32 = 0.5;
// Subdivision count S per segment instance: the sketch pipeline draws
// 2*(S+1) strip vertices per instance. The CPU-side draw count lives in
// data_render/line.rs (`LINE_SKETCH_VERTICES_PER_INSTANCE`) — keep the two in sync.
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
    let period = dash_period(dash, count);
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

/// The length of one repetition of the dash pattern, the sum of its
/// positive spans; zero for a solid line.
fn dash_period(dash: array<vec4<f32>, 2>, count: u32) -> f32 {
    var spans = dash;
    let n = min(count, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(spans[i / 4u][i % 4u], 0.0);
    }
    return period;
}

/// Arc pair `origin` modulo the dash `period` (zero for a solid line): the
/// dash position a segment's `dist_px` starts from (§5.6 arc pairs).
fn arc_dash_phase(origin: vec2<u32>, period: f32) -> f32 {
    if !(period > 0.0) {
        return 0.0;
    }
    return f32(arc_divide(origin, arc_divisor(period)).remainder) / ARC_UNITS_PER_PX;
}

/// Value noise `sketch_noise((origin + offset_px) / wavelength_px)` for an
/// arc pair `origin` and a small `offset_px` along the segment: the lattice
/// cell of the origin is taken from the pair, so the noise keeps its scale
/// past 2^24 px of arc. `wavelength_px` > 0.
fn arc_noise(origin: vec2<u32>, offset_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    let cell = arc_cell(origin, wavelength_px);
    let t = cell.fraction + offset_px / wavelength_px;
    let k = floor(t);
    return sketch_noise_lattice(cell.index + u32(i32(k)), t - k, seed);
}

/// The sketch line's perpendicular displacement at arc `origin + offset_px`
/// (an arc pair and a small offset): `amplitude_px · noise(arc /
/// wavelength)`. A wavelength <= 0 disables the wobble — mirrors the CPU
/// stroker's guard (sketch.rs) so GPU and deco layers degrade identically.
fn sketch_line_offset_from(origin: vec2<u32>, offset_px: f32, amplitude_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    if !(wavelength_px > 0.0) {
        return 0.0;
    }
    return max(amplitude_px, 0.0) * arc_noise(origin, offset_px, wavelength_px, seed);
}
