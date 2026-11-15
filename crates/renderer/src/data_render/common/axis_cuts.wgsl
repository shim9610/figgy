// The cut remap of a broken axis (#200): the `AxisCuts` of one axis and the
// functions that place data across its cuts. Spliced before
// `projection.wgsl` into the cut variant of every module that places data;
// `axis_cuts_identity.wgsl` is its twin for a renderer that has drawn no
// broken chart (#299), with the same signatures.

// One axis of a broken chart (#200): its cuts `(from, to, start, end)` in
// the axis' data t and `(count, piece scale)`. Count 0 is the identity.
// CPU twin: `layout::BreakMap`.
struct AxisCuts {
    k0: vec4<f32>,
    k1: vec4<f32>,
    k2: vec4<f32>,
    count_scale: vec2<f32>,
};

fn no_cuts() -> AxisCuts {
    return AxisCuts(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec2<f32>(0.0, 1.0));
}

// Axis `axis` (0 = x, 1 = y) of a transform's break table.
fn cuts_of(breaks: array<vec4<f32>, 6>, info: vec4<f32>, axis: u32) -> AxisCuts {
    if (axis == 0u) {
        return AxisCuts(breaks[0], breaks[1], breaks[2], vec2<f32>(info.x, info.z));
    }
    return AxisCuts(breaks[3], breaks[4], breaks[5], vec2<f32>(info.y, info.w));
}

// The scale of the piece from raw `a` (drawn at `da`) to raw `b` (drawn at
// `db`): its drawn length per raw length (#205), each piece its own. The
// common scale where the piece has no span.
fn piece_slope(a: f32, da: f32, b: f32, db: f32, s: f32) -> f32 {
    if (b - a > 1e-7) {
        return (db - da) / (b - a);
    }
    return s;
}

// The scale of piece `p` (0 to 3) of the cuts, from the knots that bound it.
fn cut_piece_slope(c: AxisCuts, p: u32) -> f32 {
    let n = c.count_scale.x;
    let s = c.count_scale.y;
    if (p == 0u) {
        return piece_slope(0.0, 0.0, c.k0.x, c.k0.z, s);
    }
    if (p == 1u) {
        if (n < 1.5) {
            return piece_slope(c.k0.y, c.k0.w, 1.0, 1.0, s);
        }
        return piece_slope(c.k0.y, c.k0.w, c.k1.x, c.k1.z, s);
    }
    if (p == 2u) {
        if (n < 2.5) {
            return piece_slope(c.k1.y, c.k1.w, 1.0, 1.0, s);
        }
        return piece_slope(c.k1.y, c.k1.w, c.k2.x, c.k2.z, s);
    }
    return piece_slope(c.k2.y, c.k2.w, 1.0, 1.0, s);
}

// Where data t is drawn: linear within each piece at that piece's scale,
// linear across each cut onto its gap, so the map is continuous, increasing
// and one-to-one. Written without a loop over the knots: a dynamic index
// into a value array is not portable.
fn cut_remap(c: AxisCuts, t: f32) -> f32 {
    let n = c.count_scale.x;
    if (n < 0.5) {
        return t;
    }
    if (t < c.k0.x) {
        return t * cut_piece_slope(c, 0u);
    }
    if (t < c.k0.y) {
        return c.k0.z + (t - c.k0.x) / (c.k0.y - c.k0.x) * (c.k0.w - c.k0.z);
    }
    if (n < 1.5 || t < c.k1.x) {
        return c.k0.w + (t - c.k0.y) * cut_piece_slope(c, 1u);
    }
    if (t < c.k1.y) {
        return c.k1.z + (t - c.k1.x) / (c.k1.y - c.k1.x) * (c.k1.w - c.k1.z);
    }
    if (n < 2.5 || t < c.k2.x) {
        return c.k1.w + (t - c.k1.y) * cut_piece_slope(c, 2u);
    }
    if (t < c.k2.y) {
        return c.k2.z + (t - c.k2.x) / (c.k2.y - c.k2.x) * (c.k2.w - c.k2.z);
    }
    return c.k2.w + (t - c.k2.y) * cut_piece_slope(c, 3u);
}

// The inverse of `cut_remap`: the data t drawn at `drawn`.
fn cut_unmap(c: AxisCuts, drawn: f32) -> f32 {
    let n = c.count_scale.x;
    if (n < 0.5) {
        return drawn;
    }
    if (drawn < c.k0.z) {
        return drawn / cut_piece_slope(c, 0u);
    }
    if (drawn < c.k0.w) {
        return c.k0.x + (drawn - c.k0.z) / (c.k0.w - c.k0.z) * (c.k0.y - c.k0.x);
    }
    if (n < 1.5 || drawn < c.k1.z) {
        return c.k0.y + (drawn - c.k0.w) / cut_piece_slope(c, 1u);
    }
    if (drawn < c.k1.w) {
        return c.k1.x + (drawn - c.k1.z) / (c.k1.w - c.k1.z) * (c.k1.y - c.k1.x);
    }
    if (n < 2.5 || drawn < c.k2.z) {
        return c.k1.y + (drawn - c.k1.w) / cut_piece_slope(c, 2u);
    }
    if (drawn < c.k2.w) {
        return c.k2.x + (drawn - c.k2.z) / (c.k2.w - c.k2.z) * (c.k2.y - c.k2.x);
    }
    return c.k2.y + (drawn - c.k2.w) / cut_piece_slope(c, 3u);
}

// The scale of the piece data t lies in: drawn length per data-t length, 1
// on an unbroken axis. Derivatives (pixels per data unit) multiply by it.
// Inside a cut, the piece after it.
fn cut_scale_at(c: AxisCuts, t: f32) -> f32 {
    let n = c.count_scale.x;
    if (n < 0.5) {
        return 1.0;
    }
    if (t < c.k0.x) {
        return cut_piece_slope(c, 0u);
    }
    if (n < 1.5 || t < c.k1.x) {
        return cut_piece_slope(c, 1u);
    }
    if (n < 2.5 || t < c.k2.x) {
        return cut_piece_slope(c, 2u);
    }
    return cut_piece_slope(c, 3u);
}
