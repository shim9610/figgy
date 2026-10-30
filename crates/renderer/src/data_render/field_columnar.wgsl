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

fn vec4_f32_is_finite(v: vec4<f32>) -> bool {
    return vec2_f32_is_finite(v.xy) && vec2_f32_is_finite(v.zw);
}

/// The stroke colour of contour level `i`, premultiplied.
///
/// A level past the end of the table takes the last entry rather than nothing:
/// the host writes one entry per level, and a short table is a mismatch to draw
/// through with the last declared colour, not a reason to drop the contour.
fn level_color(i: u32) -> vec4<f32> {
    if (field.level_color_count == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return level_colors[min(i, field.level_color_count - 1u)];
}

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
    let s = field_cell_sample(field_fragment_center_t(in.axis_t));
    if (!s.hit) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
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
};

/// The fill fragment's cell lookup at pixel-centre axis `t`
/// (`field_fragment_center_t`); the cell pick calls it at the cursor's pixel
/// centre, so a cell picks exactly where it paints.
fn field_cell_sample(t: vec2<f32>) -> FieldCellSample {
    var out: FieldCellSample;
    out.hit = false;
    out.fc = 0.0;
    out.fr = 0.0;
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
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
};

fn contour_metric(s: ContourSample) -> ContourMetric {
    var out: ContourMetric;
    out.valid = false;
    out.g_len = 0.0;
    out.kappa = 0.0;
    out.gradient_square = 0.0;
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
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let s = contour_sample(in.axis_t);
    if (!s.hit) {
        return acc;
    }
    let metric = contour_metric(s);
    if (!metric.valid) {
        return acc;
    }
    if (!f32_is_finite(field.line_width_px)) {
        return acc;
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
        return acc;
    }
    for (var block = 0u; block < contour_level_block_count(); block = block + 1u) {
        let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
        let count = contour_lookup_metadata[block].finite_count;
        let first = contour_lower_bound(start, count, search_lo);
        let last = contour_upper_bound(start, count, search_hi);
        var candidates = 0u;
        for (var sorted = first; sorted < last; sorted = sorted + 1u) {
            let record = contour_search_record(start + sorted);
            let original = u32(record.y) - start;
            candidates = candidates | (1u << original);
        }
        // A single mask restores the block's declaration order. That preserves
        // duplicate-level colour and source-over behavior without walking all 32
        // levels when only a few can reach this cell.
        while (candidates != 0u) {
            let original = firstTrailingBit(candidates);
            candidates = candidates & ~(1u << original);
            let i = start + original;
            let lv = levels[i];
            if (!f32_is_finite(lv)) {
                continue;
            }
            let distance_to_level = contour_level_distance_px(s, metric, lv);
            let coverage_unclamped = half_w - distance_to_level + 0.5;
            if (distance_to_level < 0.0 || !f32_is_finite(coverage_unclamped)) {
                continue;
            }
            let cov = clamp(coverage_unclamped, 0.0, 1.0);
            if (!f32_is_finite(cov) || cov <= 0.0) {
                continue;
            }
            let c = level_color(i) * cov;
            let composited = c + acc * (1.0 - c.a);
            if (!vec4_f32_is_finite(c) || !vec4_f32_is_finite(composited)) {
                continue;
            }
            acc = composited;
        }
    }
    return acc;
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
    if ((data_pick_query.data.x & DATA_PICK_FLAG_PAINTED) != 0u) {
        t = field_fragment_center_t(cursor_t);
        let s = field_cell_sample(t);
        if (!s.hit || !field_fill_paints(
            field_fill_value(s.z00, s.z10, s.z01, s.z11, s.fc, s.fr),
            data_pick_query.baseline,
        )) {
            return field_pick_invalid();
        }
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

fn field_pick_contour(t: vec2<f32>) -> DataPickCandidate {
    var best = field_pick_invalid();
    let s = contour_sample(t);
    let metric = contour_metric(s);
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
    var level_index = 0u;
    while (level_index < field.level_count) {
        let center_distance = contour_level_distance_px(s, metric, levels[level_index]);
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

@compute @workgroup_size(1)
fn pick_field_data() {
    let flags = data_pick_query.data.x;
    let t = data_pick_query.cursor_ndc_t.zw;
    // Contours paint over their fill. A contour within the configured pick
    // tolerance therefore wins before the containing heatmap cell. A label
    // gap (`fs_contour_labelled`'s test; group 3 holds the series' label
    // placement, or none) paints no contour, so it picks none.
    if ((flags & DATA_PICK_FLAG_CONTOUR) != 0u) {
        let contour = field_pick_contour(t);
        if (contour.valid != 0u && !contour_label_gap_contains(t)) {
            data_pick_output = contour;
            return;
        }
    }
    if ((flags & DATA_PICK_FLAG_FILL) != 0u) {
        data_pick_output = field_pick_cell(t);
        return;
    }
    data_pick_output = field_pick_invalid();
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

fn selected_cell_coverage(t: vec2<f32>) -> f32 {
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

fn selected_contour_coverage(t: vec2<f32>) -> f32 {
    let level_index = data_selection.indices.y;
    let width_px = max(field.line_width_px, 0.0) + max(data_selection.metrics.y, 0.0);
    if (level_index >= field.level_count
        || !f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    let s = contour_sample(t);
    let metric = contour_metric(s);
    let distance_px = contour_level_distance_px(s, metric, levels[level_index]);
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
