// ─────────────────────────── error-bar geometry ───────────────────────────
// One error-bar instance draws six axis-aligned quads (SHADER_COMMON.md
// §5.11): 0 the Y stem, 1 the cap at y_lo, 2 the cap at y_hi, 3 the X stem,
// 4 the cap at x_lo, 5 the cap at x_hi. A stem runs between two data
// points; a cap spans a pixel length either side of one data point. Each
// quad is expanded perpendicular to its own axis by a half stroke width in
// pixels. Every error-bar entry of `errorbar_columnar.wgsl` and the pick's
// `gpu_pick/errorbar.wgsl` build the quads here, so a bar picks on the
// geometry it is drawn with.

// `Style.primitive_flags`: the directions the series draws.
const ERRORBAR_HAS_Y: u32 = 1u;
const ERRORBAR_HAS_X: u32 = 2u;

// Milkyway jets: the halo room past the core stroke (beams) and the cap
// size (knots) that the glow needs.
const JET_BEAM_HALO: f32 = 4.0;
const JET_KNOT_HALO: f32 = 1.7;

struct DataPoint {
    x: vec2<f32>,
    y: vec2<f32>,
};

fn pair_sum(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x + b.x, a.y + b.y);
}

fn pair_sub(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x - b.x, a.y - b.y);
}

fn select_point(b: DataPoint, a: DataPoint, pick_a: bool) -> DataPoint {
    return DataPoint(select(b.x, a.x, pick_a), select(b.y, a.y, pick_a));
}

// One instance's columns: the anchor and the four error values (hi/lo
// pairs), the lower ones subtracted from the anchor, the upper ones added.
struct ErrorBarSample {
    x: vec2<f32>,
    y: vec2<f32>,
    err_y_lo: vec2<f32>,
    err_y_hi: vec2<f32>,
    err_x_lo: vec2<f32>,
    err_x_hi: vec2<f32>,
};

// The pixel sizes of one bar's quads: half a stem's stroke width, half a
// cap's length and half a cap's stroke width.
struct ErrorBarStroke {
    stem_half: f32,
    cap_half: f32,
    cap_stroke_half: f32,
};

// A precise or sketch bar: the style's widths.
fn errorbar_stroke(line_width_px: f32, cap_half_px: f32, cap_width_px: f32) -> ErrorBarStroke {
    return ErrorBarStroke(line_width_px * 0.5, cap_half_px, cap_width_px * 0.5);
}

// A Milkyway jet: a beam of `JET_BEAM_HALO` times the core stroke, and a
// square knot of `JET_KNOT_HALO` times the cap size at each bound.
fn errorbar_jet_stroke(line_width_px: f32, cap_half_px: f32) -> ErrorBarStroke {
    let beam_half = max(line_width_px * 0.5, 1.2) * JET_BEAM_HALO;
    let knot_half = max(cap_half_px, 3.0) * JET_KNOT_HALO;
    return ErrorBarStroke(beam_half, knot_half, knot_half);
}

// Quad `seg`: the data-space ends A and B, their pixel offsets (y up), the
// pixel expansion axis and the half stroke along it.
struct ErrorBarSegment {
    a: DataPoint,
    b: DataPoint,
    a_px: vec2<f32>,
    b_px: vec2<f32>,
    perp: vec2<f32>,
    half_stroke: f32,
};

// A disabled direction collapses at the anchor, caps included. Direction
// presence is structural and must not depend on whether an error value is 0.
fn errorbar_segment_enabled(flags: u32, seg: u32) -> bool {
    let has_y = (flags & ERRORBAR_HAS_Y) != 0u;
    let has_x = (flags & ERRORBAR_HAS_X) != 0u;
    return select(has_x, has_y, seg < 3u);
}

// All segments are axis-aligned, so `perp` needs no normalization.
fn errorbar_segment(seg: u32, s: ErrorBarSample, stroke: ErrorBarStroke) -> ErrorBarSegment {
    let y_lo = pair_sub(s.y, s.err_y_lo);
    let y_hi = pair_sum(s.y, s.err_y_hi);
    let x_lo = pair_sub(s.x, s.err_x_lo);
    let x_hi = pair_sum(s.x, s.err_x_hi);

    var g: ErrorBarSegment;
    g.a_px = vec2<f32>(0.0, 0.0);
    g.b_px = vec2<f32>(0.0, 0.0);
    if (seg == 0u) {
        // Y stem: vertical in data space, expand along X.
        g.a = DataPoint(s.x, y_lo);
        g.b = DataPoint(s.x, y_hi);
        g.perp = vec2<f32>(1.0, 0.0);
        g.half_stroke = stroke.stem_half;
    } else if (seg == 1u) {
        // Cap @ y_lo: horizontal, expand along Y.
        g.a = DataPoint(s.x, y_lo);
        g.b = g.a;
        g.a_px = vec2<f32>(-stroke.cap_half, 0.0);
        g.b_px = vec2<f32>( stroke.cap_half, 0.0);
        g.perp = vec2<f32>(0.0, 1.0);
        g.half_stroke = stroke.cap_stroke_half;
    } else if (seg == 2u) {
        // Cap @ y_hi: horizontal, expand along Y.
        g.a = DataPoint(s.x, y_hi);
        g.b = g.a;
        g.a_px = vec2<f32>(-stroke.cap_half, 0.0);
        g.b_px = vec2<f32>( stroke.cap_half, 0.0);
        g.perp = vec2<f32>(0.0, 1.0);
        g.half_stroke = stroke.cap_stroke_half;
    } else if (seg == 3u) {
        // X stem: horizontal in data space, expand along Y.
        g.a = DataPoint(x_lo, s.y);
        g.b = DataPoint(x_hi, s.y);
        g.perp = vec2<f32>(0.0, 1.0);
        g.half_stroke = stroke.stem_half;
    } else if (seg == 4u) {
        // Cap @ x_lo: vertical, expand along X.
        g.a = DataPoint(x_lo, s.y);
        g.b = g.a;
        g.a_px = vec2<f32>(0.0, -stroke.cap_half);
        g.b_px = vec2<f32>(0.0,  stroke.cap_half);
        g.perp = vec2<f32>(1.0, 0.0);
        g.half_stroke = stroke.cap_stroke_half;
    } else {
        // Cap @ x_hi: vertical, expand along X.
        g.a = DataPoint(x_hi, s.y);
        g.b = g.a;
        g.a_px = vec2<f32>(0.0, -stroke.cap_half);
        g.b_px = vec2<f32>(0.0,  stroke.cap_half);
        g.perp = vec2<f32>(1.0, 0.0);
        g.half_stroke = stroke.cap_stroke_half;
    }
    return g;
}

// Triangle-list corner map [0,1,2, 2,1,3] over corners
// {0: A-, 1: A+, 2: B-, 3: B+} (-/+ = perpendicular half-stroke side).
struct ErrorBarCorner {
    at_a: bool,
    side: f32,
};

fn errorbar_corner(vi: u32) -> ErrorBarCorner {
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[vi % 6u];
    return ErrorBarCorner(corner < 2u, select(1.0, -1.0, (corner & 1u) == 0u));
}

// The corner at end A (or B) of quad `g`, `across_px` pixels along its
// expansion axis (the signed half stroke, plus a sketch displacement).
fn errorbar_corner_ndc(t: Transform, g: ErrorBarSegment, at_a: bool, across_px: f32) -> vec2<f32> {
    let end_data = select_point(g.b, g.a, at_a);
    let end_px = select(g.b_px, g.a_px, at_a);
    let offset_px = end_px + g.perp * across_px;
    return data_to_ndc(t, end_data.x, end_data.y) + offset_px * t.pixel_to_ndc;
}

// Sketch: end A or B of quad `seg` is displaced along its expansion axis by
// amplitude · noise(2·seg + end, seed), so each of the twelve quad ends has
// an independent deterministic offset that both its corners share.
fn errorbar_sketch_offset(amp: f32, seed: u32, seg: u32, at_a: bool) -> f32 {
    let lattice = f32(seg * 2u + select(1u, 0u, at_a));
    return amp * sketch_noise(lattice, seed);
}

// ───── per-point error-bar styles (Precise) ─────
// The style rows the mapped entry binds (group 2 bindings 5-6) and the pick
// reads after the marker rows: a slot's mask says which channels it sets.
struct ErrorBarStyleSlot {
    color_premul: vec4<f32>,
    // x = stem width, y = cap half-size, z = mask, w = cap width.
    params: vec4<f32>,
};

struct ResolvedErrorBarStyle {
    color_premul: vec4<f32>,
    line_width_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
};

const ERRORBAR_STYLE_MASK_COLOR: u32 = 1u;
const ERRORBAR_STYLE_MASK_STEM_WIDTH: u32 = 2u;
const ERRORBAR_STYLE_MASK_CAP_HALF: u32 = 4u;
const ERRORBAR_STYLE_MASK_CAP_WIDTH: u32 = 8u;

fn valid_errorbar_style_index(v: f32) -> bool {
    return v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn apply_errorbar_style_slot(
    base: ResolvedErrorBarStyle,
    slot: ErrorBarStyleSlot,
) -> ResolvedErrorBarStyle {
    let mask = u32(slot.params.z);
    var out = base;
    if ((mask & ERRORBAR_STYLE_MASK_COLOR) != 0u) {
        out.color_premul = slot.color_premul;
    }
    if ((mask & ERRORBAR_STYLE_MASK_STEM_WIDTH) != 0u) {
        out.line_width_px = max(slot.params.x, 0.0);
    }
    if ((mask & ERRORBAR_STYLE_MASK_CAP_HALF) != 0u) {
        out.cap_half_px = max(slot.params.y, 0.0);
    }
    if ((mask & ERRORBAR_STYLE_MASK_CAP_WIDTH) != 0u) {
        out.cap_width_px = max(slot.params.w, 0.0);
    }
    return out;
}
// ───────────────────────── end error-bar geometry ─────────────────────────
