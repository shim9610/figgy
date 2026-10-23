/// Where sample `k` sits along one coordinate column: the coordinate itself
/// under `Centers`, the centre of cell `k` under `Edges`.
fn vector_anchor_pair(base: u32, len: u32, k: u32) -> vec2<f32> {
    if (vector_flag(VECTOR_FLAG_CENTERS)) {
        return vector_pool_pair(base, min(k, max(len, 1u) - 1u));
    }
    let next = min(k + 1u, max(len, 1u) - 1u);
    return midpoint_grid_pair(vector_pool_pair(base, k), vector_pool_pair(base, next));
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
// `gpu_pick/vector.wgsl`, through the same arrow fragment), so its entry
// closes this fragment: every assembled module's entry points are compiled
// and measured. It walks the samples `vs_main` draws through the same
// projection, placement and glyph distance, so a grid vector picks the arrow
// it draws. WGSL module scope is order-free; the glyph and the entry layer
// follow in the assembly.

const DATA_PICK_KIND_MATRIX_CELL: u32 = 2u;
