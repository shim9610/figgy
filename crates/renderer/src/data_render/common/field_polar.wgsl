// The polar half of the field grid's cell lookup (#208): `field_lookup`
// (`field_grid.wgsl`) hands a polar chart's fragment here. The full set
// of a module that splices `field_grid.wgsl` splices it;
// `field_polar_basic.wgsl` is its twin for a renderer that has drawn no polar
// chart (#299), with the same signature.

/// `field_lookup` on a polar chart, given the rectangular answer at
/// chart-area `t`.
fn field_polar_lookup(t: vec2<f32>, lattice: u32, cartesian: FieldLookup) -> FieldLookup {
    let tf = transform;
    var out = cartesian;
    let px = polar_clip_offset(tf, t * 2.0 - vec2<f32>(1.0, 1.0));
    if (!polar_fragment_visible(px, polar_clip_limits(tf), polar_clip_sector(tf))) {
        out.hit = false;
        return out;
    }
    let u = polar_unproject(tf, px);
    // The angle axis is linear on a polar chart: one angle unit is `kx` of
    // `t`, and a turn `turn_t`.
    let range_x = (transform.data_max.x - transform.data_min.x)
        + (transform.data_max_lo.x - transform.data_min_lo.x);
    let kx = transform.data_to_panel_scale.x / range_x;
    let turn_t = abs(6.283185307179586 / transform.polar_angle.z * kx);
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let count = quad_count(along_cells, lattice);
    let b0 = boundary_t(field.x_base, field.x_len, 0u, 0u, lattice);
    let b1 = boundary_t(field.x_base, field.x_len, count, 0u, lattice);
    var tx = axis_t(vec2<f32>(u.angle, 0.0), 0u);
    if (!f32_is_finite(tx) || !f32_is_finite(turn_t) || turn_t <= 0.0
        || !f32_is_finite(b0) || !f32_is_finite(b1)) {
        out.hit = false;
        return out;
    }
    let low = min(b0, b1);
    tx = tx - floor((tx - low) / turn_t) * turn_t;
    out.t = vec2<f32>(tx, u.ty);
    out.jac_x = u.d_angle * kx;
    out.jac_y = u.d_ty;
    return out;
}

// ── The polar contour distance's foot-point walk (#208) ───────────────────
//
// `field_columnar.wgsl`'s contour paint and pick start a walk only on a polar
// chart (`contour_foot_begin`). Kept here so the basic set, which never draws
// a polar chart, compiles none of it into its contour entries (#453). The
// walk reads `field_grid.wgsl` alone, so every module that splices this
// fragment compiles it.

/// Foot-point steps of the polar distance (`contour_level_distance_at`).
/// Each step re-linearizes the level set at the current point; the
/// iteration converges quadratically to the closest point, so four reach
/// f32 precision within a stroke's reach. A fixed iteration count, not a
/// data count.
const CONTOUR_POLAR_FOOT_STEPS: u32 = 4u;

/// The foot-point iteration from `contour_foot_begin`'s `q`, advanced one
/// iteration of its caller's loop at a time by `contour_foot_advance`.
///
/// A foot-point step from `q` (relative to the fragment at chart-area
/// `chart_t`) linearizes the level set at `q` (`z(q) + g(q) · (x - q) =
/// level`, `g` the pixel gradient through `field_lookup` and the located
/// cell, crossing cells as `q` moves) and moves `q` to the foot of the
/// perpendicular from the fragment onto that line. Locating the cell is a
/// binary search on each axis, so a step takes several iterations: one
/// looks `q` up, each further one begins or halves a search
/// (`locate_step`, one axis after the other), and the last evaluates the
/// cell and moves `q`. The walk
/// is `done` after `CONTOUR_POLAR_FOOT_STEPS` steps or at a step that
/// cannot be taken, which leaves `q` where it was; the distance is then
/// `length(q)`. Run as a loop inside a loop, the search made Chromium's
/// SwiftShader compile each contour entry for 24 to 30 s.
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
    out.steps = 0u;
    out.searching = false;
    out.lookup_t = vec2<f32>(0.0, 0.0);
    out.jac_x = vec2<f32>(0.0, 0.0);
    out.jac_y = vec2<f32>(0.0, 0.0);
    out.axis = 0u;
    out.begun = false;
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
    if (!walk.searching) {
        let t_per_px = transform.pixel_to_ndc * 0.5;
        let lookup = field_lookup(chart_t + walk.q * t_per_px, LATTICE_SAMPLES);
        if (!lookup.hit) {
            out.done = true;
            return out;
        }
        out.lookup_t = lookup.t;
        out.jac_x = lookup.jac_x;
        out.jac_y = lookup.jac_y;
        out.searching = true;
        out.axis = 0u;
        out.begun = false;
        return out;
    }
    // One axis at a time through one call site of each search function, so
    // a compiler that inlines them does so once.
    if (walk.axis < 2u) {
        let on_x = walk.axis == 0u;
        let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
        let cells = select(select(field.cols, field.rows, columns_are_y),
            select(field.rows, field.cols, columns_are_y), !on_x);
        let base = select(field.y_base, field.x_base, on_x);
        let len = select(field.y_len, field.x_len, on_x);
        var search = walk.y;
        if (on_x) {
            search = walk.x;
        }
        if (!walk.begun) {
            search = locate_begin(base, len, quad_count(cells, LATTICE_SAMPLES), walk.axis,
                select(walk.lookup_t.y, walk.lookup_t.x, on_x), LATTICE_SAMPLES);
            out.begun = true;
        } else if (locate_searching(search)) {
            search = locate_step(base, len, walk.axis, LATTICE_SAMPLES, search);
        } else {
            out.axis = walk.axis + 1u;
            out.begun = false;
        }
        if (on_x) {
            out.x = search;
        } else {
            out.y = search;
        }
        return out;
    }
    out.searching = false;
    let at = contour_sample_cells(locate_end(walk.x), locate_end(walk.y));
    if (!at.hit) {
        out.done = true;
        return out;
    }
    let g = at.dz.x * walk.jac_x + at.dz.y * walk.jac_y;
    let g2 = dot(g, g);
    // The foot of the perpendicular from the fragment (the origin) onto the
    // level set linearized at `q`.
    let next = -((at.z - level) - dot(g, walk.q)) / g2 * g;
    if (!f32_is_finite(g2) || g2 <= 0.0 || !vec2_f32_is_finite(next)
        || length(next) > reach + 1.0) {
        out.done = true;
        return out;
    }
    out.q = next;
    out.steps = walk.steps + 1u;
    out.done = out.steps >= CONTOUR_POLAR_FOOT_STEPS;
    return out;
}
