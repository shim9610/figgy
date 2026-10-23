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
