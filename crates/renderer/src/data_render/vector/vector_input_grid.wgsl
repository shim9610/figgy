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
