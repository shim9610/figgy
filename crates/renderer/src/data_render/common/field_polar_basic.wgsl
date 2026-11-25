// The basic twin of `field_polar.wgsl` (#299): the same signature with no
// polar lookup, for a renderer that has drawn no polar chart, so its field
// pipelines do not compile it. A polar chart is never drawn or picked
// through this fragment (`shader_assembly::ShaderSet`).

/// Unreached on a rectangular chart (`field_lookup` tests the polar flag):
/// a miss, so a polar field draws nothing here.
fn field_polar_lookup(t: vec2<f32>, lattice: u32, cartesian: FieldLookup) -> FieldLookup {
    var out = cartesian;
    out.hit = false;
    return out;
}

/// The basic twin of the polar foot-point walk (#453): the same walk
/// state, and a walk that ends at once. Unreached: `contour_foot_begin`
/// starts a walk only on a polar chart, which the basic set never draws.
struct ContourFootWalk {
    q: vec2<f32>,
    /// Foot-point steps taken.
    steps: u32,
    /// Whether the current step has looked `q` up and is searching its cell.
    searching: bool,
    /// `field_lookup`'s point and its per-pixel derivatives.
    lookup_t: vec2<f32>,
    jac_x: vec2<f32>,
    jac_y: vec2<f32>,
    /// The axis being searched, 0 or 1; 2 once both cells are found.
    axis: u32,
    /// Whether `axis`'s search has begun.
    begun: bool,
    x: CellSearch,
    y: CellSearch,
    done: bool,
};

fn contour_foot_walk(q: vec2<f32>) -> ContourFootWalk {
    var out: ContourFootWalk;
    out.q = q;
    out.done = false;
    return out;
}

fn contour_foot_advance(
    chart_t: vec2<f32>,
    walk: ContourFootWalk,
    level: f32,
    reach: f32,
) -> ContourFootWalk {
    var out = walk;
    out.done = true;
    return out;
}
