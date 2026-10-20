// Input layer, grid form: `u` / `v` (and the colour value) are grids over the
// series' coordinate columns, declared like a heatmap's matrix.
//
// Instance `i` is cell `(col = i / rows, row = i % rows)` in constituent-column
// order. `col` walks the coordinate the columns run along (x for
// `ColumnsAreX`, y for `ColumnsAreY`) and `row` the other one. With `Centers`
// the arrow sits on the coordinate; with `Edges` it sits at the cell centre,
// the midpoint of two consecutive coordinates in data space, which is where
// the field's `sample_point_pair` puts a cell's value and what
// `selection_details` reports for an edge grid.

fn vector_rounded_add(a: f32, b: f32) -> f32 {
    return fma(1.0, a, b);
}

fn vector_rounded_subtract(a: f32, b: f32) -> f32 {
    return fma(1.0, a, -b);
}

/// Normalize a finite pair with error-free addition (the field's
/// `normalize_grid_pair`).
fn vector_normalize_pair(v: vec2<f32>) -> vec2<f32> {
    let sum = vector_rounded_add(v.x, v.y);
    let b_virtual = vector_rounded_subtract(sum, v.x);
    let a_virtual = vector_rounded_subtract(sum, b_virtual);
    let b_roundoff = vector_rounded_subtract(v.y, b_virtual);
    let a_roundoff = vector_rounded_subtract(v.x, a_virtual);
    return vec2<f32>(sum, vector_rounded_add(a_roundoff, b_roundoff));
}

fn vector_add_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high = vector_normalize_pair(vec2<f32>(a.x, b.x));
    let low = vector_rounded_add(vector_rounded_add(a.y, b.y), high.y);
    return vector_normalize_pair(vec2<f32>(high.x, low));
}

fn vector_scale_pair(v: vec2<f32>, factor: f32) -> vec2<f32> {
    return vector_normalize_pair(vec2<f32>(v.x * factor, v.y * factor));
}

/// The midpoint of two coordinate pairs, as the field computes a cell centre.
fn vector_midpoint_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vector_add_pairs(vector_scale_pair(a, 0.5), vector_scale_pair(b, 0.5));
}

/// Where sample `k` sits along one coordinate column: the coordinate itself
/// under `Centers`, the centre of cell `k` under `Edges`.
fn vector_anchor_pair(base: u32, len: u32, k: u32) -> vec2<f32> {
    if (vector_flag(VECTOR_FLAG_CENTERS)) {
        return vector_pool_pair(base, min(k, max(len, 1u) - 1u));
    }
    let next = min(k + 1u, max(len, 1u) - 1u);
    return vector_midpoint_pair(vector_pool_pair(base, k), vector_pool_pair(base, next));
}

/// Value `row` of constituent column `col` of the table starting at `base`,
/// as a pool pair plus explicit bounds validity (the field shader's
/// `grid_value` rule). The pair stays zero past the column's length.
struct VectorGridValue {
    pair: vec2<f32>,
    valid: bool,
};

fn vector_grid_value(base: u32, col: u32, row: u32) -> VectorGridValue {
    var out: VectorGridValue;
    out.pair = vec2<f32>(0.0, 0.0);
    out.valid = false;
    let column = vector_grid[base + col];
    if (row >= column.len) {
        return out;
    }
    out.pair = vector_pool_pair(column.base, row);
    out.valid = true;
    return out;
}

fn vector_sample(i: u32) -> VectorSample {
    var sample: VectorSample;
    sample.origin_hi = vec3<f32>(0.0, 0.0, 0.0);
    sample.origin_lo = vec3<f32>(0.0, 0.0, 0.0);
    sample.vector = vec3<f32>(0.0, 0.0, 0.0);
    sample.value = vec2<f32>(0.0, 0.0);
    sample.value_valid = false;
    sample.valid = false;
    let rows = vector_params.rows;
    if (rows == 0u || i >= vector_params.count) {
        return sample;
    }
    let col = i / rows;
    let row = i % rows;
    if (col >= vector_params.cols) {
        return sample;
    }
    let u_value = vector_grid_value(vector_params.u_base, col, row);
    let v_value = vector_grid_value(vector_params.v_base, col, row);
    let u = u_value.pair;
    let v = v_value.pair;
    if (!u_value.valid || !v_value.valid || !vector_pair_finite(u) || !vector_pair_finite(v)) {
        return sample;
    }
    var x: vec2<f32>;
    var y: vec2<f32>;
    if (vector_flag(VECTOR_FLAG_COLUMNS_ARE_Y)) {
        y = vector_anchor_pair(vector_params.y_base, vector_params.y_len, col);
        x = vector_anchor_pair(vector_params.x_base, vector_params.x_len, row);
    } else {
        x = vector_anchor_pair(vector_params.x_base, vector_params.x_len, col);
        y = vector_anchor_pair(vector_params.y_base, vector_params.y_len, row);
    }
    sample.origin_hi = vec3<f32>(x.x, y.x, 0.0);
    sample.origin_lo = vec3<f32>(x.y, y.y, 0.0);
    sample.vector = vec3<f32>(u.x + u.y, v.x + v.y, 0.0);
    if (vector_flag(VECTOR_FLAG_COLOR_MAP)) {
        let c = vector_grid_value(vector_params.c_base, col, row);
        sample.value = c.pair;
        sample.value_valid = c.valid;
    }
    sample.valid = true;
    return sample;
}

// ───── Grid arrow pick (compute entry, typed data pick) ─────────────────────
//
// Only the grid form has a typed data pick (a point vector is a point pick,
// `gpu_pick.wgsl`, through the SHADER_COMMON.md §5.8 twins), so its entry
// closes this fragment: every assembled module's entry points are compiled
// and measured. It walks the samples `vs_main` draws through the same
// projection, placement and glyph distance, so a grid vector picks the arrow
// it draws. WGSL module scope is order-free; the glyph and the entry layer
// follow in the assembly.

const DATA_PICK_KIND_MATRIX_CELL: u32 = 2u;

// The typed data pick's query (`gpu_data_pick.rs::DataPickQueryGpu`).
struct DataPickQuery {
    // xy = cursor NDC, zw = cursor axis-t.
    cursor_ndc_t: vec4<f32>,
    // x = max distance px.
    limits: vec4<f32>,
    // x = flags, y = source paint order.
    data: vec4<u32>,
    bases: vec4<u32>,
    baseline: vec4<f32>,
};

// Thirty-two bytes, byte-for-byte with Rust's `DataPickCandidateGpu`.
struct DataPickCandidate {
    valid: u32,
    paint_order: u32,
    kind: u32,
    index0: u32,
    index1: u32,
    index2: u32,
    distance_px: f32,
    primitive_order: u32,
};

@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

/// The nearest drawn arrow of a grid vector series to the cursor, as the
/// matrix cell it is drawn for: every instance `vs_main` would draw, placed
/// as it places it, measured with `arrow_distance` in the draw's pixel frame
/// (`ndc_to_px` of the cursor). An arrow that is not drawn does not pick.
/// Arrows paint in instance order, so an exact tie goes to the later one.
@compute @workgroup_size(1)
fn pick_vector_cell() {
    var best = DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
    let cursor = ndc_to_px(data_pick_query.cursor_ndc_t.xy);
    let max_distance = data_pick_query.limits.x;
    let rows = vector_params.rows;
    if (rows == 0u || !arrow_finite(cursor.x) || !arrow_finite(cursor.y)
        || !arrow_finite(max_distance) || max_distance < 0.0) {
        data_pick_output = best;
        return;
    }
    let style = vector_arrow_style();
    let mode = vector_scale_mode();
    var i = 0u;
    loop {
        if (i >= vector_params.count) {
            break;
        }
        let arrow = project_arrow(vector_sample(i), mode, vector_params.scale);
        if (arrow.valid) {
            let placed = arrow_place(arrow.tail, arrow.tip, style);
            if (placed.valid) {
                let hit = max(arrow_distance(cursor, placed, vector_params.width_px), 0.0);
                if (arrow_finite(hit) && hit <= max_distance
                    && (best.valid == 0u || hit <= best.distance_px)) {
                    let col = i / rows;
                    let row = i % rows;
                    var x_index = col;
                    var y_index = row;
                    if (vector_flag(VECTOR_FLAG_COLUMNS_ARE_Y)) {
                        x_index = row;
                        y_index = col;
                    }
                    best = DataPickCandidate(
                        1u,
                        data_pick_query.data.y,
                        DATA_PICK_KIND_MATRIX_CELL,
                        x_index,
                        y_index,
                        0u,
                        hit,
                        i,
                    );
                }
            }
        }
        i = i + 1u;
    }
    data_pick_output = best;
}
