// Columnar field shader — heatmaps and band (filled-contour) fields.
//
// The grid is a bundle of pool columns declared by `MatrixRef`: one column per
// x (or y) position, each holding that position's values along the other axis.
// Coordinates come from the ordinary `x_column` / `y_column`; nothing here is
// stored twice.
//
// **One quad for the whole field, not one per cell.** The cell a fragment
// belongs to is found per fragment by binary search over the coordinate
// columns. Two reasons, both decisive:
//
//   1. A 5000 x 5000 grid is 25M cells. As instances that is 150M vertices a
//      frame; as one quad it is 6, and the cost becomes the data area's pixel
//      count — which is what a heatmap's cost should be.
//   2. Adjacent per-cell quads seam under MSAA. Two quads meeting on an edge
//      resolve as `bg*(1-cA)*(1-cB) + ...`, so up to 25% of the background
//      shows through along every shared edge. One quad cannot seam against
//      itself.
//
// The search runs in axis `t` space (`axis_pair_to_t`), so logarithmic and
// inverted axes need no special case: the same monotone comparison works for
// all four combinations.
//
// Cell geometry follows `GridLayout`. `Edges` reads the coordinate columns as
// cell boundaries (n + 1 for n cells); `Centers` reads them as cell midpoints
// and derives boundaries as the midpoints between them, mirroring the outermost
// half-cells. A single centre has no neighbour to measure against, so its cell
// has zero width and draws nothing — the data does not say how wide it is, and
// inventing a width would be a number nobody wrote.
//
// `Shading::Flat` gives each cell its own colour from its own z.
// `Shading::Interpolated` interpolates z bilinearly between the four
// surrounding sample points, so the quads span point-to-point rather than
// cell-to-cell (matplotlib's `flat` vs `gouraud`).
//
// `FillMode::Bands` quantizes z into the intervals between the contour levels,
// which makes exactly the filled-contour bands the contour lines will be drawn
// over. `Continuous` uses the ramp directly.
//
// The colour ramp is the colourmap's own control points, and the mapping is a
// byte-for-byte reimplementation of `model::colormap::sample` — see `ramp()`.
// That is what makes the CPU-drawn colourbar strip and this shader agree: same
// stops, same formula, same `t`.

// Label-gap inputs exist only in `fs_contour_labelled`'s call graph. The plain
// `fs_contour` entry therefore keeps the original three-group layout, while a
// labelled contour binds the exact selected-anchor buffers as group 3.
struct LabelGapAnchor {
    x: vec2<f32>,
    y: vec2<f32>,
    dir: vec2<f32>,
    level: u32,
    width_px: f32,
};

struct LabelGapParams {
    atlas_w: f32,
    atlas_h: f32,
    cell_stride_w: f32,
    cell_stride_h: f32,
    columns: u32,
    rows: u32,
    gutter: f32,
    level_count: u32,
};

@group(3) @binding(0) var<storage, read> label_gap_anchors: array<LabelGapAnchor>;
@group(3) @binding(1) var<storage, read> label_gap_draw_args: array<u32>;
@group(3) @binding(3) var<uniform> label_gap_params: LabelGapParams;

struct FieldOut {
    @builtin(position) pos: vec4<f32>,
    /// Axis `t` of this fragment, i.e. its position in the chart area — the
    /// same quantity `axis_pair_to_t` returns for a data value, which is what
    /// lets the cell search compare the two directly.
    @location(0) axis_t: vec2<f32>,
};

/// One full-clip-space quad. `Transform` already maps the data range onto
/// NDC +/-1 across the chart area, and the caller's scissor trims it to the
/// data area, so the quad needs no data-dependent extent — the fragment stage
/// reports a miss wherever the field is not.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> FieldOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let ndc = corners[vid];
    var out: FieldOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.axis_t = (ndc + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

// Field fills use one shade at the pixel center for all covered MSAA samples.
// Canonicalize the fullscreen varying so the two quad triangles and the
// bounded compute replay select the same cell at exactly aligned boundaries.
fn field_fragment_center_t(t: vec2<f32>) -> vec2<f32> {
    return field_pixel_center_t(t, round(vec2<f32>(2.0) / transform.pixel_to_ndc));
}

fn field_pixel_center_t(t: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    return (round(t * size - vec2<f32>(0.5)) + vec2<f32>(0.5)) / size;
}

@fragment
fn fs_main(in: FieldOut) -> @location(0) vec4<f32> {
    let lookup = field_lookup(field_fragment_center_t(in.axis_t), LATTICE_QUADS);
    if (!lookup.hit) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let s = field_cell_sample(lookup.t);
    if (!s.hit) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    // An image (#139): its three channels are its colour.
    if (field_flag(FIELD_RGB)) {
        return field_image_color(s.c, s.r, s.fc, s.fr);
    }
    return field_fill_color(s.z00, s.z10, s.z01, s.z11, s.fc, s.fr, style.color_premul);
}

/// The grid samples the fill shades at axis `t`: the located cell's value
/// and, for interpolated shading, its three neighbours with the in-cell
/// fractions. `hit` is false outside the grid.
struct FieldCellSample {
    hit: bool,
    z00: GridValue,
    z10: GridValue,
    z01: GridValue,
    z11: GridValue,
    fc: f32,
    fr: f32,
    /// The located constituent and in-column indices (an image reads its
    /// other channels at them).
    c: u32,
    r: u32,
};

/// The fill fragment's cell lookup at pixel-centre axis `t`
/// (`field_fragment_center_t`); the cell pick calls it at the cursor's pixel
/// centre, so a cell picks exactly where it paints.
fn field_cell_sample(t: vec2<f32>) -> FieldCellSample {
    var out: FieldCellSample;
    out.hit = false;
    out.fc = 0.0;
    out.fr = 0.0;
    out.c = 0u;
    out.r = 0u;
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    // A hexagonal lattice (#134): the nearest centre, shaded flat.
    if (field_flag(FIELD_HEXAGONAL)) {
        let cell = hex_cell(t);
        if (!cell.hit) {
            return out;
        }
        let xi = u32(cell.column);
        let yi = u32(cell.row);
        out.c = select(xi, yi, columns_are_y);
        out.r = select(yi, xi, columns_are_y);
        out.z00 = grid_value(out.c, out.r);
        out.hit = true;
        return out;
    }
    // The constituent index runs along x unless the declaration says otherwise;
    // the within-column index runs along the other axis. Never inferred from
    // the lengths.
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along_cells, LATTICE_QUADS), 0u, t.x, LATTICE_QUADS);
    let y = locate(field.y_base, field.y_len, quad_count(across_cells, LATTICE_QUADS), 1u, t.y, LATTICE_QUADS);
    if (!x.hit || !y.hit) {
        return out;
    }
    let c = select(x.index, y.index, columns_are_y);
    let r = select(y.index, x.index, columns_are_y);
    out.c = c;
    out.r = r;
    out.fc = select(x.frac, y.frac, columns_are_y);
    out.fr = select(y.frac, x.frac, columns_are_y);
    out.z00 = grid_value(c, r);
    if (field_flag(FIELD_INTERPOLATED)) {
        out.z10 = grid_value(c + 1u, r);
        out.z01 = grid_value(c, r + 1u);
        out.z11 = grid_value(c + 1u, r + 1u);
    }
    out.hit = true;
    return out;
}

/// Screen-space differential terms shared by contour painting and contour
/// picking. Keeping the quadratic distance root behind this one helper is what
/// prevents the visible stroke and its hit target from drifting apart.
struct ContourMetric {
    valid: bool,
    g_len: f32,
    kappa: f32,
    gradient_square: f32,
    /// The gradient's unit direction in the draw's pixel frame. Read by the
    /// polar distance (`contour_level_distance_at`) only.
    n: vec2<f32>,
};

fn contour_metric(s: ContourSample) -> ContourMetric {
    var out: ContourMetric;
    out.valid = false;
    out.g_len = 0.0;
    out.kappa = 0.0;
    out.gradient_square = 0.0;
    out.n = vec2<f32>(0.0, 0.0);
    if (!s.hit) {
        return out;
    }
    let g = grad_px(s.dz);
    let g_len = length(g);
    // The Hessian in pixels. `grad_px` scales each axis by `pixel_to_ndc / 2`, so
    // the mixed second derivative takes that factor once per axis.
    let hxy = s.d2 * transform.pixel_to_ndc.x * transform.pixel_to_ndc.y * 0.25;
    if (!vec2_f32_is_finite(g) || !f32_is_finite(g_len) || !f32_is_finite(hxy)) {
        return out;
    }
    // Normal curvature of the level set. With `H = [[0, hxy], [hxy, 0]]` the
    // quadratic form collapses to this.
    var kappa = 0.0;
    if (g_len > 0.0) {
        let n = g / g_len;
        kappa = 2.0 * hxy * n.x * n.y;
    }
    let gradient_square = g_len * g_len;
    if (!f32_is_finite(kappa) || !f32_is_finite(gradient_square)) {
        return out;
    }
    if (g_len <= 0.0 && kappa == 0.0) {
        // A plateau: no gradient and no curvature. Its level set is the whole
        // flat region, and painting/picking that solid would invent a line.
        return out;
    }
    out.valid = true;
    out.g_len = g_len;
    out.kappa = kappa;
    out.gradient_square = gradient_square;
    return out;
}

/// `contour_metric` at a lookup (#208). A rectangular chart takes it as it
/// is. On a polar chart the axis-`t` gradient and the mixed second
/// derivative are carried to pixels through the lookup's Jacobian
/// (`FieldLookup`): `g = dz.x · jac_x + dz.y · jac_y` and the bilinear
/// Hessian `J^T H J`. Along a pixel line the polar field is not a
/// quadratic, so this metric only starts the distance:
/// `contour_level_distance_at` solves it exactly.
fn contour_metric_at(s: ContourSample, lookup: FieldLookup) -> ContourMetric {
    if (transform.polar_angle.x < 0.5) {
        return contour_metric(s);
    }
    var out: ContourMetric;
    out.valid = false;
    out.g_len = 0.0;
    out.kappa = 0.0;
    out.gradient_square = 0.0;
    if (!s.hit || !lookup.hit) {
        return out;
    }
    let g = s.dz.x * lookup.jac_x + s.dz.y * lookup.jac_y;
    let g_len = length(g);
    if (!vec2_f32_is_finite(g) || !f32_is_finite(g_len) || !f32_is_finite(s.d2)) {
        return out;
    }
    var kappa = 0.0;
    if (g_len > 0.0) {
        let n = g / g_len;
        kappa = 2.0 * s.d2 * dot(n, lookup.jac_x) * dot(n, lookup.jac_y);
    }
    let gradient_square = g_len * g_len;
    if (!f32_is_finite(kappa) || !f32_is_finite(gradient_square)) {
        return out;
    }
    if (g_len <= 0.0 && kappa == 0.0) {
        return out;
    }
    out.valid = true;
    out.g_len = g_len;
    out.kappa = kappa;
    out.gradient_square = gradient_square;
    out.n = select(vec2<f32>(0.0, 0.0), g / g_len, g_len > 0.0);
    return out;
}

/// Distance in pixels to one level along the bilinear field's gradient normal.
/// Returns -1 for an unplaceable level/root.
fn contour_level_distance_px(s: ContourSample, metric: ContourMetric, level: f32) -> f32 {
    if (!metric.valid || !f32_is_finite(level)) {
        return -1.0;
    }
    // z is bilinear, so along a straight line it is exactly quadratic. Written
    // rationalized so it degrades to `f / |grad|` as curvature tends to zero.
    let f = s.z - level;
    let curvature_term = 2.0 * metric.kappa * f;
    let disc = metric.gradient_square - curvature_term;
    if (!f32_is_finite(f) || !f32_is_finite(curvature_term)
        || !f32_is_finite(disc) || disc < 0.0) {
        return -1.0;
    }
    let root = sqrt(disc);
    let denom = metric.g_len + root;
    if (!f32_is_finite(root) || !f32_is_finite(denom) || denom <= 0.0) {
        return -1.0;
    }
    let distance_to_level = abs((2.0 * f) / denom);
    if (!f32_is_finite(distance_to_level)) {
        return -1.0;
    }
    return distance_to_level;
}

/// Foot-point steps of the polar distance (`contour_level_distance_at`).
/// Each step re-linearizes the level set at the current point; the
/// iteration converges quadratically to the closest point, so four reach
/// f32 precision within a stroke's reach. A fixed iteration count, not a
/// data count.
const CONTOUR_POLAR_FOOT_STEPS: u32 = 4u;

/// The first estimate of `contour_level_distance_at` and, on a polar chart
/// within `reach`, the foot-point iteration's start (#208). `refine` is
/// false where `start` is the answer: a rectangular chart, an unplaceable
/// level, a fragment beyond `reach`, or no gradient to step along.
struct ContourFoot {
    start: f32,
    /// The current point relative to the fragment, in the draw's pixel
    /// frame. The first step uses the fragment's own sample.
    q: vec2<f32>,
    refine: bool,
};

fn contour_foot_begin(s: ContourSample, metric: ContourMetric, level: f32, reach: f32) -> ContourFoot {
    var out: ContourFoot;
    out.start = contour_level_distance_px(s, metric, level);
    out.q = vec2<f32>(0.0, 0.0);
    out.refine = false;
    if (transform.polar_angle.x < 0.5 || out.start < 0.0 || !f32_is_finite(reach)
        || out.start > reach || metric.g_len <= 0.0 || !vec2_f32_is_finite(metric.n)) {
        return out;
    }
    let q = -(s.z - level) / metric.g_len * metric.n;
    if (!vec2_f32_is_finite(q)) {
        return out;
    }
    out.q = q;
    out.refine = true;
    return out;
}

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

/// The distance in pixels from the fragment at chart-area `chart_t` to
/// `level` (#208). A rectangular chart returns `contour_level_distance_px`
/// unchanged. On a polar chart the field along a pixel line is the bilinear
/// field composed with the polar inverse, so a closed form along one line
/// is not the distance to the curve. Instead the closest point `q` on the
/// level set is found by foot-point iteration (`contour_foot_begin`, then
/// the walk, `contour_foot_advance`). The fixed point is the foot of the
/// perpendicular onto the curve itself, so the result is the Euclidean
/// distance to the level set. Only a fragment whose first estimate is
/// within `reach` pixels is refined; beyond it the stroke paints and picks
/// nothing. The selected-level highlight calls this; the paint
/// (`contour_paint`) and the pick (`field_pick_contour`) advance the same
/// walk in their one flat loop over the levels.
fn contour_level_distance_at(
    chart_t: vec2<f32>,
    s: ContourSample,
    metric: ContourMetric,
    level: f32,
    reach: f32,
) -> f32 {
    let begin = contour_foot_begin(s, metric, level, reach);
    if (!begin.refine) {
        return begin.start;
    }
    var walk = contour_foot_walk(begin.q);
    loop {
        if (walk.done) {
            break;
        }
        walk = contour_foot_advance(chart_t, walk, level, reach);
    }
    return length(walk.q);
}

/// Contour lines, drawn as the level set of the bilinear field.
///
/// Same quad and same fragment machinery as the fill, so the two agree by
/// construction — the band boundary a `FillMode::Bands` fill paints and the line
/// drawn here come out of one interpolation, including on a logarithmic axis
/// without materializing or tracing a marching-squares segment list.
///
/// Coverage uses the quadratic root obtained by restricting the current cell's
/// bilinear polynomial to the fragment's gradient-normal line. That root is
/// algebraically exact for that polynomial and line; it is not asserted to stay
/// inside the cell or to be the globally shortest Euclidean distance to the
/// piecewise-bilinear isoline. The analytic gradient is the bilinear form's own
/// derivative, not a `dpdx` finite difference.
///
/// A point with no gradient can still have mixed curvature, which the quadratic
/// handles. Only a plateau (`|grad z| == 0` and zero mixed curvature) is left
/// blank: its level set is the whole flat region, so painting it solid would
/// state a line position the data does not contain.
///
/// Cost is the data area's pixel count, not the cell count: a 5000 x 5000 grid
/// costs what a 100 x 100 one does. Per fragment, each 32-level block is searched
/// by z range and only reachable candidates run the coverage math.
fn contour_color(in: FieldOut) -> vec4<f32> {
    return contour_paint(in).color;
}

fn contour_paint(in: FieldOut) -> ContourPaint {
    var out = contour_paint_none();
    let lookup = field_lookup(in.axis_t, LATTICE_SAMPLES);
    if (!lookup.hit) {
        return out;
    }
    let s = contour_sample(lookup.t);
    if (!s.hit) {
        return out;
    }
    let metric = contour_metric_at(s, lookup);
    if (!metric.valid) {
        return out;
    }
    if (!f32_is_finite(field.line_width_px)) {
        return out;
    }
    let half_w = max(field.line_width_px, 0.0) * 0.5;
    // The reject below must keep a neighbouring cell's line, which legitimately
    // reaches into this fragment: `margin` is how much z changes over half a
    // stroke plus a pixel of antialiasing.
    let margin = (half_w + 1.0) * metric.g_len;
    let search_lo = s.z_lo - margin;
    let search_hi = s.z_hi + margin;
    if (!f32_is_finite(half_w) || !f32_is_finite(margin)
        || !f32_is_finite(search_lo) || !f32_is_finite(search_hi)) {
        return out;
    }
    // One flat loop takes every level the fragment can reach, in the order
    // the blocks and their masks give: an iteration either loads the next
    // block's candidates, begins the next candidate or advances the current
    // one's foot-point walk. Each candidate's distance is
    // `contour_level_distance_at`'s, taken as the pick takes it
    // (`field_pick_contour`). With the foot-point loop nested in the loops
    // over the blocks and candidates, Chromium's SwiftShader passed 12 GB
    // compiling this fragment at its first draw and lost the device.
    var block = 0u;
    var start = 0u;
    var candidates = 0u;
    var i = 0u;
    var lv = 0.0;
    var walking = false;
    var walk = contour_foot_walk(vec2<f32>(0.0, 0.0));
    loop {
        var distance_to_level = -1.0;
        if (!walking) {
            if (candidates == 0u) {
                if (block >= contour_level_block_count()) {
                    break;
                }
                start = block * CONTOUR_LEVEL_BLOCK_SIZE;
                candidates = contour_block_candidates(block, search_lo, search_hi);
                block = block + 1u;
                continue;
            }
            let original = firstTrailingBit(candidates);
            candidates = candidates & ~(1u << original);
            i = start + original;
            lv = levels[i];
            if (!f32_is_finite(lv)) {
                continue;
            }
            let begin = contour_foot_begin(s, metric, lv, half_w + 1.0);
            distance_to_level = begin.start;
            if (begin.refine) {
                walk = contour_foot_walk(begin.q);
                walking = true;
                continue;
            }
        } else {
            walk = contour_foot_advance(in.axis_t, walk, lv, half_w + 1.0);
            if (!walk.done) {
                continue;
            }
            distance_to_level = length(walk.q);
            walking = false;
        }
        let coverage_unclamped = half_w - distance_to_level + 0.5;
        if (distance_to_level < 0.0 || !f32_is_finite(coverage_unclamped)) {
            continue;
        }
        out = contour_level_over(out, i, coverage_unclamped);
    }
    return out;
}

// ── 3D floor (Config.view_3d) ──────────────────────────────────────────────
//
// On a 3D chart a contour draws on the box floor: the same quad, projected
// through the camera (`project_3d.wgsl`), with `axis_t` the floor point in
// the space the flat entries locate cells in. The unchanged `fs_contour`
// then finds the isolines in floor coordinates, so its stroke widths are
// floor-plane pixels and foreshorten with the floor (#49 D7).

@vertex
fn vs_floor(@builtin(vertex_index) vid: u32) -> FieldOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let t = corners[vid];
    var out: FieldOut;
    out.pos = project_box(vec3<f32>(t, 0.0));
    out.axis_t = t * transform.data_to_panel_scale + transform.data_to_panel_offset;
    return out;
}

/// The floor contours' ID pick: the series (`view3d.flags.w`, its pick
/// slot plus one, with the top bit marking a floor contour), the topmost
/// level painted at the pixel and the sample-lattice cell it lies in, as
/// the flat contour pick names it.
@fragment
fn fs_floor_contour_pick(in: FieldOut) -> @location(0) vec4<u32> {
    let paint = contour_paint(in);
    if (paint.level == 0xffffffffu) {
        discard;
    }
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along_cells, LATTICE_SAMPLES), 0u,
        in.axis_t.x, LATTICE_SAMPLES);
    let y = locate(field.y_base, field.y_len, quad_count(across_cells, LATTICE_SAMPLES), 1u,
        in.axis_t.y, LATTICE_SAMPLES);
    return vec4<u32>(u32(view3d.flags.w) | 0x80000000u, paint.level, x.index, y.index);
}

/// Whether this contour fragment sits under one of the exact label quads that
/// the next draw will composite. The count comes from the same indirect args
/// buffer as that draw, so automatic placement never needs a CPU readback and
/// the gap cannot disagree with the labels that actually appear.
fn contour_label_gap_contains(axis_t_value: vec2<f32>) -> bool {
    let count = min(label_gap_draw_args[1], 1024u);
    let fragment_ndc = axis_t_value * 2.0 - 1.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let anchor = label_gap_anchors[i];
        if (anchor.level >= label_gap_params.level_count) {
            continue;
        }
        let w = anchor.width_px;
        let h = label_gap_params.cell_stride_h - 2.0 * label_gap_params.gutter;
        if (!f32_is_finite(w) || !f32_is_finite(h) || w <= 0.0 || h <= 0.0) {
            continue;
        }
        // The label quad's own frame. The gap reaches `LABEL_GAP_EDGE_PX`
        // past the content where the quad reaches `LABEL_EDGE_PX`, so it lies
        // inside the quad. `bg_padding_px` is already included in w/h, even
        // when the label background itself is transparent.
        let frame = contour_label_frame(transform, anchor.x, anchor.y, anchor.dir);
        let content = vec2<f32>(w, h);
        if (contour_label_covers(
            frame, transform.pixel_to_ndc, fragment_ndc, content, LABEL_GAP_EDGE_PX,
        )) {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_contour(in: FieldOut) -> @location(0) vec4<f32> {
    return contour_color(in);
}

@fragment
fn fs_contour_labelled(in: FieldOut) -> @location(0) vec4<f32> {
    let color = contour_color(in);
    // The anchor scan is paid only on pixels where a contour actually has
    // coverage, not across the field's whole rectangle.
    if (color.a > 0.0 && contour_label_gap_contains(in.axis_t)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return color;
}

// ───── Exact data picking (compute entry) ───────────────────────────────────

const DATA_PICK_FLAG_FILL: u32 = 1u;
const DATA_PICK_FLAG_CONTOUR: u32 = 2u;
// Resident fills: pick the pixel-centre cell, and only where it paints.
const DATA_PICK_FLAG_PAINTED: u32 = 4u;
const DATA_PICK_KIND_MATRIX_CELL: u32 = 2u;
const DATA_PICK_KIND_CONTOUR_LEVEL: u32 = 3u;

@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

fn field_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
}

/// The cell painted at the cursor's pixel: the fill fragment evaluates the
/// pixel centre (`field_fragment_center_t`, `field_cell_sample`), so the pick
/// does too, and a pixel the fill leaves clear (a transparent ramp stop, a
/// zero opacity, a transparent `nan_color`; see `field_fill_paints`) picks
/// nothing.
/// `data_pick_query.baseline` carries the fill's premultiplied `nan_color`.
/// Streamed fields keep their own pick (rule 8) and do not set
/// `DATA_PICK_FLAG_PAINTED`: their cursor stays raw and ungated.
fn field_pick_cell(cursor_t: vec2<f32>) -> DataPickCandidate {
    var t = cursor_t;
    let painted = (data_pick_query.data.x & DATA_PICK_FLAG_PAINTED) != 0u;
    if (painted) {
        t = field_fragment_center_t(cursor_t);
    }
    // The polar lookup (#208): the cell under the cursor in (angle, radius).
    let lookup = field_lookup(t, LATTICE_QUADS);
    if (!lookup.hit) {
        return field_pick_invalid();
    }
    t = lookup.t;
    if (painted) {
        let s = field_cell_sample(t);
        if (!s.hit) {
            return field_pick_invalid();
        }
        if (field_flag(FIELD_RGB)) {
            if (!field_image_paints(s.c, s.r, s.fc, s.fr)) {
                return field_pick_invalid();
            }
        } else if (!field_fill_paints(
            field_fill_value(s.z00, s.z10, s.z01, s.z11, s.fc, s.fr),
            data_pick_query.baseline,
        )) {
            return field_pick_invalid();
        }
    }
    // A hexagonal lattice (#134): the cell the fill painted at this pixel.
    if (field_flag(FIELD_HEXAGONAL)) {
        let cell = hex_cell(t);
        if (!cell.hit) {
            return field_pick_invalid();
        }
        return DataPickCandidate(
            1u,
            data_pick_query.data.y,
            DATA_PICK_KIND_MATRIX_CELL,
            u32(cell.column),
            u32(cell.row),
            0u,
            0.0,
            0u,
        );
    }
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(
        field.x_base,
        field.x_len,
        quad_count(along_cells, LATTICE_QUADS),
        0u,
        t.x,
        LATTICE_QUADS,
    );
    let y = locate(
        field.y_base,
        field.y_len,
        quad_count(across_cells, LATTICE_QUADS),
        1u,
        t.y,
        LATTICE_QUADS,
    );
    if (!x.hit || !y.hit) {
        return field_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_MATRIX_CELL,
        x.index,
        y.index,
        0u,
        0.0,
        0u,
    );
}

fn field_pick_contour(cursor_t: vec2<f32>) -> DataPickCandidate {
    var best = field_pick_invalid();
    let lookup = field_lookup(cursor_t, LATTICE_SAMPLES);
    if (!lookup.hit) {
        return best;
    }
    let t = lookup.t;
    let s = contour_sample(t);
    let metric = contour_metric_at(s, lookup);
    if (!metric.valid || !f32_is_finite(data_pick_query.limits.x)
        || !f32_is_finite(data_pick_query.limits.z)) {
        return best;
    }
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(
        field.x_base,
        field.x_len,
        quad_count(along_cells, LATTICE_SAMPLES),
        0u,
        t.x,
        LATTICE_SAMPLES,
    );
    let y = locate(
        field.y_base,
        field.y_len,
        quad_count(across_cells, LATTICE_SAMPLES),
        1u,
        t.y,
        LATTICE_SAMPLES,
    );
    if (!x.hit || !y.hit) {
        return best;
    }
    let half_width = max(data_pick_query.limits.z, 0.0) * 0.5;
    let max_distance = max(data_pick_query.limits.x, 0.0);
    let reach = half_width + max_distance + 1.0;
    // Each level's distance is `contour_level_distance_at`'s, taken in one
    // flat loop: an iteration either begins a level or advances its
    // foot-point walk. Nesting the polar foot-point loop, and the binary
    // searches inside it, in a loop over the levels made a software GPU
    // compiler's compute path grow without bound (Chromium's SwiftShader
    // passed 12 GB building this pipeline; Windows WARP overflowed its
    // stack, #226). `contour_paint` takes its candidates the same way.
    var level_index = 0u;
    var walking = false;
    var walk = contour_foot_walk(vec2<f32>(0.0, 0.0));
    var center_distance = -1.0;
    loop {
        if (level_index >= field.level_count) {
            break;
        }
        let level = levels[level_index];
        if (!walking) {
            let begin = contour_foot_begin(s, metric, level, reach);
            center_distance = begin.start;
            if (begin.refine) {
                walk = contour_foot_walk(begin.q);
                walking = true;
                continue;
            }
        } else {
            walk = contour_foot_advance(cursor_t, walk, level, reach);
            if (!walk.done) {
                continue;
            }
            center_distance = length(walk.q);
            walking = false;
        }
        if (center_distance >= 0.0) {
            let hit_distance = max(center_distance - half_width, 0.0);
            if (f32_is_finite(hit_distance) && hit_distance <= max_distance
                && (best.valid == 0u || hit_distance < best.distance_px
                    || (hit_distance == best.distance_px && level_index > best.index0))) {
                best = DataPickCandidate(
                    1u,
                    data_pick_query.data.y,
                    DATA_PICK_KIND_CONTOUR_LEVEL,
                    level_index,
                    x.index,
                    y.index,
                    hit_distance,
                    level_index,
                );
            }
        }
        level_index = level_index + 1u;
    }
    return best;
}

// A field series picks in two dispatches of one pass, this entry and then
// `pick_field_cell`, with the same bind groups (`gpu_data_pick.rs`). On
// Windows WARP, one entry holding both overflowed the 1 MiB default stack of
// a thread WARP creates; each half fits (#222).
//
// Contours paint over their fill. A contour within the configured pick
// tolerance therefore wins before the containing heatmap cell. A label gap
// (`fs_contour_labelled`'s test; group 3 holds the series' label placement,
// or none) paints no contour, so it picks none.
@compute @workgroup_size(1)
fn pick_field_contour() {
    let t = data_pick_query.cursor_ndc_t.zw;
    if ((data_pick_query.data.x & DATA_PICK_FLAG_CONTOUR) != 0u) {
        let contour = field_pick_contour(t);
        if (contour.valid != 0u && !contour_label_gap_contains(t)) {
            data_pick_output = contour;
            return;
        }
    }
    data_pick_output = field_pick_invalid();
}

// The second dispatch: the cell under the cursor, unless the contour entry
// already wrote a candidate.
@compute @workgroup_size(1)
fn pick_field_cell() {
    if (data_pick_output.valid != 0u) {
        return;
    }
    if ((data_pick_query.data.x & DATA_PICK_FLAG_FILL) != 0u) {
        data_pick_output = field_pick_cell(data_pick_query.cursor_ndc_t.zw);
    }
}

// ───── Typed selection overlay (render entry) ───────────────────────────────

@group(1) @binding(4) var<uniform> data_selection: DataSelection;

fn selection_color(coverage: f32) -> vec4<f32> {
    if (!f32_is_finite(coverage) || coverage <= 0.0
        || !vec4_f32_is_finite(data_selection.color_premul)) {
        return vec4<f32>(0.0);
    }
    return data_selection.color_premul * clamp(coverage, 0.0, 1.0);
}

fn selected_cell_coverage(chart_t: vec2<f32>) -> f32 {
    // The polar lookup (#208): the fragment lies in the picked annular cell
    // in (angle, radius) `t`.
    let lookup = field_lookup(chart_t, LATTICE_QUADS);
    if (!lookup.hit) {
        return 0.0;
    }
    let t = lookup.t;
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x_count = quad_count(along_cells, LATTICE_QUADS);
    let y_count = quad_count(across_cells, LATTICE_QUADS);
    let x_index = data_selection.indices.z;
    let y_index = data_selection.indices.w;
    let width_px = data_selection.metrics.x;
    if (x_index >= x_count || y_index >= y_count
        || !f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    // A hexagonal lattice (#134): inside the picked hexagon, the stroke
    // runs along its six edges, the bisectors with its neighbours.
    if (field_flag(FIELD_HEXAGONAL)) {
        let cell = hex_cell(t);
        if (!cell.hit || u32(cell.column) != x_index || u32(cell.row) != y_index) {
            return 0.0;
        }
        return width_px - hex_edge_distance_px(t, cell.column, cell.row) + 0.5;
    }
    let x0 = boundary_t(field.x_base, field.x_len, x_index, 0u, LATTICE_QUADS);
    let x1 = boundary_t(field.x_base, field.x_len, x_index + 1u, 0u, LATTICE_QUADS);
    let y0 = boundary_t(field.y_base, field.y_len, y_index, 1u, LATTICE_QUADS);
    let y1 = boundary_t(field.y_base, field.y_len, y_index + 1u, 1u, LATTICE_QUADS);
    if (!f32_is_finite(x0) || !f32_is_finite(x1)
        || !f32_is_finite(y0) || !f32_is_finite(y1)
        || t.x < min(x0, x1) || t.x > max(x0, x1)
        || t.y < min(y0, y1) || t.y > max(y0, y1)) {
        return 0.0;
    }
    if (transform.polar_angle.x > 0.5) {
        return polar_cell_coverage(chart_t, vec4<f32>(x0, x1, y0, y1), width_px);
    }
    let pixel_t = transform.pixel_to_ndc * 0.5;
    if (!vec2_f32_is_finite(pixel_t) || pixel_t.x <= 0.0 || pixel_t.y <= 0.0) {
        return 0.0;
    }
    let edge_distance_px = min(
        min(abs(t.x - x0), abs(t.x - x1)) / pixel_t.x,
        min(abs(t.y - y0), abs(t.y - y1)) / pixel_t.y,
    );
    return width_px - edge_distance_px + 0.5;
}

/// The picked polar cell's outline (#208): the annular sector between the
/// cell's angle and radius boundaries (`bounds`, lookup `t`: x0, x1, y0, y1)
/// as a `radial_sector.wgsl` `Sector`, built as the rose builds a wedge
/// (`category_wedge`), and its `footprint` distance in pixels. The outline
/// lies inside the cell, `width_px` wide, as the rectangular one does.
fn polar_cell_coverage(chart_t: vec2<f32>, bounds: vec4<f32>, width_px: f32) -> f32 {
    let p = transform.polar_angle;
    let ring = transform.polar_extent.w;
    // The angle axis is linear on a polar chart: `t` back to an angle value.
    let range_x = (transform.data_max.x - transform.data_min.x)
        + (transform.data_max_lo.x - transform.data_min_lo.x);
    let min_x = transform.data_min.x + transform.data_min_lo.x;
    let v = (bounds.xy - vec2<f32>(transform.data_to_panel_offset.x))
        / transform.data_to_panel_scale.x * range_x + vec2<f32>(min_x);
    let a = vec2<f32>(p.y) + p.z * v;
    // Radius `t` to pixels, as `category_radius_px` reads a radius.
    let rt = (bounds.zw - vec2<f32>(transform.data_to_panel_offset.y))
        / transform.data_to_panel_scale.y;
    let r = mix(vec2<f32>(p.w), vec2<f32>(1.0), max(rt, vec2<f32>(0.0))) * ring;
    let r_outer = max(r.x, r.y);
    let r_inner = min(r.x, r.y);
    let span = min(abs(a.y - a.x), TAU);
    if (!vec2_f32_is_finite(a) || !vec2_f32_is_finite(r) || !(ring > 0.0)
        || !(r_outer > r_inner) || !(r_outer > 0.0) || !(span > 0.0)) {
        return 0.0;
    }
    // The pixel frame has y down: the polar arc [θ0, θ1] is the sector arc
    // from −θ1, brought into [0, 2π).
    let raw_start = -max(a.x, a.y);
    let arc_start = raw_start - floor(raw_start / TAU) * TAU;
    let sector = Sector(
        vec4<f32>(0.0, 0.0, r_outer, r_inner / r_outer),
        vec4<f32>(arc_start, span, 0.0, 0.0),
        vec4<f32>(0.0),
        Material(vec4<f32>(0.0), vec4<f32>(0.0)),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    );
    let offset = polar_clip_offset(transform, chart_t * 2.0 - vec2<f32>(1.0, 1.0));
    let q = vec2<f32>(offset.x, -offset.y) / r_outer;
    let distance_px = footprint(q, sector) * r_outer;
    if (!f32_is_finite(distance_px)) {
        return 0.0;
    }
    return width_px + distance_px + 0.5;
}

fn selected_contour_coverage(t: vec2<f32>) -> f32 {
    let level_index = data_selection.indices.y;
    let width_px = max(field.line_width_px, 0.0) + max(data_selection.metrics.y, 0.0);
    if (level_index >= field.level_count
        || !f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    let lookup = field_lookup(t, LATTICE_SAMPLES);
    if (!lookup.hit) {
        return 0.0;
    }
    let s = contour_sample(lookup.t);
    let metric = contour_metric_at(s, lookup);
    let distance_px = contour_level_distance_at(t, s, metric, levels[level_index], width_px + 1.0);
    if (distance_px < 0.0) {
        return 0.0;
    }
    return max(width_px, 0.0) * 0.5 - distance_px + 0.5;
}

/// Cell outlines and whole-level contour highlights from the same grid,
/// lattice and implicit-contour functions as the normal field draw.
@fragment
fn fs_data_selection(in: FieldOut) -> @location(0) vec4<f32> {
    return data_selection_color(in);
}

/// `fs_data_selection` for a labelled contour: the selected level's highlight
/// leaves the label gaps `fs_contour_labelled` leaves, with the same test.
@fragment
fn fs_data_selection_labelled(in: FieldOut) -> @location(0) vec4<f32> {
    let color = data_selection_color(in);
    if (color.a > 0.0 && data_selection.indices.x == DATA_PICK_KIND_CONTOUR_LEVEL
        && contour_label_gap_contains(in.axis_t)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return color;
}

fn data_selection_color(in: FieldOut) -> vec4<f32> {
    let kind = data_selection.indices.x;
    if (kind == DATA_PICK_KIND_MATRIX_CELL) {
        return selection_color(selected_cell_coverage(in.axis_t));
    }
    if (kind == DATA_PICK_KIND_CONTOUR_LEVEL) {
        return selection_color(selected_contour_coverage(in.axis_t));
    }
    return vec4<f32>(0.0);
}

// Bounded streamed field replay. No resident pool/grid binding is reachable.

struct StreamFieldAxis {
    t: f32, first: f32, a: f32, lo: u32,
    hi: u32, phase: u32, steps: u32, ascending: u32,
    index: u32, frac: f32, hit: u32, pad: u32,
};
struct StreamFieldPixel {
    x: StreamFieldAxis, y: StreamFieldAxis,
    z: array<vec2<f32>, 4>, valid_mask: u32, sample_marker: u32,
};
struct StreamFieldTicket {
    start: u32, len: u32, n: u32, count: u32,
    axis: u32, column: u32, pair_base: u32, pad1: u32,
};
struct StreamFieldTile {
    origin: vec2<u32>, extent: vec2<u32>,
    samples: u32, chart_width: f32, chart_height: f32, pad0: u32,
    viewport: vec4<f32>,
};
@group(3) @binding(0) var<storage, read_write> stream_field_pixels: array<StreamFieldPixel>;
@group(3) @binding(1) var<storage, read> stream_field_chunk: array<vec2<f32>>;
@group(3) @binding(2) var<uniform> stream_field_ticket: StreamFieldTicket;
@group(3) @binding(3) var<uniform> stream_field_tile: StreamFieldTile;

fn stream_field_key(local: vec2<u32>, sample_index: u32) -> u32 {
    return (sample_index * stream_field_tile.extent.y + local.y) * stream_field_tile.extent.x + local.x;
}
fn stream_field_fragment_key(pos: vec4<f32>, sample_index: u32) -> u32 {
    return stream_field_key(vec2<u32>(pos.xy) - stream_field_tile.origin, sample_index);
}
fn stream_field_fresh_axis(t: f32, count: u32) -> StreamFieldAxis {
    return StreamFieldAxis(t, 0.0, 0.0, 0u, count, 0u, 0u, 0u, 0u, 0.0, 0u, 0u);
}
@compute @workgroup_size(1)
fn cs_stream_field_pick_init() {
    var s: StreamFieldPixel;
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    let t = data_pick_query.cursor_ndc_t.zw;
    s.x = stream_field_fresh_axis(t.x, quad_count(select(field.cols, field.rows, cy), LATTICE_QUADS));
    s.y = stream_field_fresh_axis(t.y, quad_count(select(field.rows, field.cols, cy), LATTICE_QUADS));
    stream_field_pixels[0] = s;
}
@compute @workgroup_size(1)
fn cs_stream_field_pick_final() {
    let s = stream_field_pixels[0];
    var hit = field_pick_invalid();
    if (s.x.hit != 0u && s.y.hit != 0u) {
        hit = DataPickCandidate(1u, data_pick_query.data.y, DATA_PICK_KIND_MATRIX_CELL,
            s.x.index, s.y.index, 0u, 0.0, 0u);
    }
    data_pick_output = hit;
}
@compute @workgroup_size(8, 8, 1)
fn cs_stream_field_init(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= stream_field_tile.extent) || id.z >= stream_field_tile.samples) { return; }
    let viewport = stream_field_tile.viewport;
    let pixel = vec2<f32>(id.xy + stream_field_tile.origin) + vec2<f32>(0.5);
    let screen = (pixel - viewport.xy) / viewport.zw;
    let t = field_pixel_center_t(vec2<f32>(screen.x, 1.0 - screen.y),
        vec2<f32>(stream_field_tile.chart_width, stream_field_tile.chart_height));
    var s: StreamFieldPixel;
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    s.x = stream_field_fresh_axis(t.x, quad_count(select(field.cols, field.rows, cy), LATTICE_QUADS));
    s.y = stream_field_fresh_axis(t.y, quad_count(select(field.rows, field.cols, cy), LATTICE_QUADS));
    s.sample_marker = id.z + 1u;
    stream_field_pixels[stream_field_key(id.xy, id.z)] = s;
}
fn stream_field_has(k: u32) -> bool {
    return k >= stream_field_ticket.start && k - stream_field_ticket.start < stream_field_ticket.len;
}
fn stream_field_pair(k: u32) -> vec2<f32> {
    return stream_field_chunk[stream_field_ticket.pair_base + k - stream_field_ticket.start];
}
struct StreamFieldBoundary { available: bool, value: f32 };
fn stream_field_boundary(k: u32) -> StreamFieldBoundary {
    var out = StreamFieldBoundary(false, 0.0);
    var pair = vec2<f32>(0.0);
    let n = stream_field_ticket.n;
    if (lattice_is_samples(LATTICE_QUADS)) {
        if (field_flag(FIELD_CENTERS)) {
            if (!stream_field_has(k)) { return out; }
            pair = stream_field_pair(k);
        } else {
            let next = min(k + 1u, n - 1u);
            if (!stream_field_has(k) || !stream_field_has(next)) { return out; }
            pair = midpoint_grid_pair(stream_field_pair(k), stream_field_pair(next));
        }
    } else if (!field_flag(FIELD_CENTERS)) {
        if (!stream_field_has(k)) { return out; }
        pair = stream_field_pair(k);
    } else if (k == 0u) {
        let next = min(1u, n - 1u);
        if (!stream_field_has(0u) || !stream_field_has(next)) { return out; }
        let c0 = stream_field_pair(0u);
        let c1 = stream_field_pair(next);
        pair = add_grid_pairs(c0, scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5));
    } else if (k >= n) {
        let last = n - 1u;
        let prev = max(n, 2u) - 2u;
        if (!stream_field_has(last) || !stream_field_has(prev)) { return out; }
        pair = add_grid_pairs(stream_field_pair(last),
            scale_grid_pair(subtract_grid_pairs(stream_field_pair(last), stream_field_pair(prev)), 0.5));
    } else {
        if (!stream_field_has(k - 1u) || !stream_field_has(k)) { return out; }
        pair = midpoint_grid_pair(stream_field_pair(k - 1u), stream_field_pair(k));
    }
    return StreamFieldBoundary(true, axis_t(pair, stream_field_ticket.axis));
}
fn stream_field_advance_axis(input: StreamFieldAxis) -> StreamFieldAxis {
    var s = input;
    if (s.phase >= 5u) { return s; }
    if (s.phase == 0u && (stream_field_ticket.count == 0u || stream_field_ticket.n == 0u
        || !f32_is_finite(s.t))) { s.phase = 6u; return s; }
    if (s.phase == 2u && (s.hi - s.lo <= 1u || s.steps == 32u)) { s.phase = 3u; }
    var k = 0u;
    switch s.phase {
        case 0u: { k = 0u; }
        case 1u: { k = stream_field_ticket.count; }
        case 2u: { k = s.lo + (s.hi - s.lo) / 2u; }
        case 3u: { k = s.lo; }
        case 4u: { k = s.lo + 1u; }
        default: { return s; }
    }
    let boundary = stream_field_boundary(k);
    if (!boundary.available) { return s; }
    let v = boundary.value;
    if (!f32_is_finite(v)) { s.phase = 6u; return s; }
    if (s.phase == 0u) {
        s.first = v;
        s.phase = 1u;
    } else if (s.phase == 1u) {
        if (s.t < min(s.first, v) || s.t > max(s.first, v)) { s.phase = 6u; }
        else { s.ascending = select(0u, 1u, v >= s.first); s.phase = 2u; }
    } else if (s.phase == 2u) {
        let before = select(v > s.t, v <= s.t, s.ascending != 0u);
        if (before) { s.lo = k; } else { s.hi = k; }
        s.steps = s.steps + 1u;
    } else if (s.phase == 3u) {
        s.a = v;
        s.phase = 4u;
    } else if (s.phase == 4u) {
        let span = grid_rounded_subtract(v, s.a);
        if (!f32_is_finite(span)) { s.phase = 6u; return s; }
        var frac = 0.0;
        if (span != 0.0) {
            frac = grid_rounded_subtract(s.t, s.a) / span;
            if (!f32_is_finite(frac)) { s.phase = 6u; return s; }
        }
        s.index = s.lo;
        s.frac = clamp(frac, 0.0, 1.0);
        s.hit = 1u;
        s.phase = 5u;
    }
    return s;
}
@compute @workgroup_size(8, 8, 1)
fn cs_stream_field_axis(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= stream_field_tile.extent) || id.z >= stream_field_tile.samples) { return; }
    let p = stream_field_key(id.xy, id.z);
    var s = stream_field_pixels[p];
    if (stream_field_ticket.axis == 0u) { s.x = stream_field_advance_axis(s.x); }
    else { s.y = stream_field_advance_axis(s.y); }
    stream_field_pixels[p] = s;
}
@compute @workgroup_size(8, 8, 1)
fn cs_stream_field_z(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= stream_field_tile.extent) || id.z >= stream_field_tile.samples) { return; }
    let p = stream_field_key(id.xy, id.z);
    var s = stream_field_pixels[p];
    if (s.x.hit == 0u || s.y.hit == 0u) { return; }
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    let c = select(s.x.index, s.y.index, cy);
    let r = select(s.y.index, s.x.index, cy);
    let need = select(1u, 4u, field_flag(FIELD_INTERPOLATED));
    for (var slot = 0u; slot < need; slot = slot + 1u) {
        let dc = slot & 1u;
        let dr = slot >> 1u;
        let row = r + dr;
        if (c + dc != stream_field_ticket.column || !stream_field_has(row)) { continue; }
        s.z[slot] = stream_field_pair(row);
        s.valid_mask = s.valid_mask | (1u << slot);
    }
    stream_field_pixels[p] = s;
}

@fragment fn fs_stream_field_final(in: FieldOut) -> @location(0) vec4<f32> {
    // MSAA scissor fringes can invoke this shader outside the tile. Check
    // before unsigned subtraction: wrapping x = origin.x - 1 into u32::MAX
    // can otherwise alias the last pixel of the preceding row in storage.
    let pixel = vec2<u32>(in.pos.xy);
    if (any(pixel < stream_field_tile.origin)
        || any(pixel - stream_field_tile.origin >= stream_field_tile.extent)) { discard; }
    let s = stream_field_pixels[stream_field_fragment_key(in.pos, 0u)];
    if (s.x.phase != 5u || s.y.phase != 5u || s.x.hit == 0u || s.y.hit == 0u
        || s.sample_marker != 1u) { return vec4<f32>(0.0); }
    let cy = field_flag(FIELD_COLUMNS_ARE_Y);
    let fc = select(s.x.frac, s.y.frac, cy);
    let fr = select(s.y.frac, s.x.frac, cy);
    return field_fill_color(
        GridValue(s.z[0], (s.valid_mask & 1u) != 0u),
        GridValue(s.z[1], (s.valid_mask & 2u) != 0u),
        GridValue(s.z[2], (s.valid_mask & 4u) != 0u),
        GridValue(s.z[3], (s.valid_mask & 8u) != 0u), fc, fr, style.color_premul);
}
