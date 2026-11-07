// ── Triangle mesh input (#210, SHADER_COMMON.md §5.25) ─────────────────────
//
// A triangle mesh reads the field's group 2 (`field_grid.wgsl`) under
// `FIELD_MESH`: `x_base` / `y_base` and `x_len` / `y_len` are the vertex
// coordinate columns, `rows` the triangle count, and the grid table holds the
// value column (entry `MESH_VALUES`) and the triangle index column (entry
// `MESH_INDICES`, three `u32` lanes per triangle, §5.14). The host gives the
// triangles, coordinates and values; this fragment only places them. The
// flat entries (`mesh_entry.wgsl`), the floor contours (`mesh_floor.wgsl`)
// and the 3D surface (`surface_mesh/surface_input_mesh.wgsl`) share it.

const MESH_VALUES: u32 = 0u;
const MESH_INDICES: u32 = 1u;

/// The vertices a triangle may name: the shortest of the coordinate columns
/// and, for vertex values, the value column. A later upload can shorten a
/// column, and no re-validation runs then, so every draw checks each index
/// against this (which also keeps a read inside its own column).
fn mesh_vertex_count() -> u32 {
    var n = min(field.x_len, field.y_len);
    if (!field_flag(FIELD_FACE_VALUES)) {
        n = min(n, grid[MESH_VALUES].len);
    }
    return n;
}

/// The triangles drawn: whole triangles of the index column, and for face
/// values only those that have a value.
fn mesh_triangle_count() -> u32 {
    var n = min(field.rows, grid[MESH_INDICES].len / 3u);
    if (field_flag(FIELD_FACE_VALUES)) {
        n = min(n, grid[MESH_VALUES].len);
    }
    return n;
}

/// Vertex index `corner` (0, 1 or 2) of triangle `tri`, from its `u32` lane.
fn mesh_index(tri: u32, corner: u32) -> u32 {
    return bitcast<u32>(field_pool[grid[MESH_INDICES].base + (tri * 3u + corner) * 2u]);
}

fn pair_finite(v: vec2<f32>) -> bool {
    return f32_is_finite(v.x) && f32_is_finite(v.y) && f32_is_finite(v.x + v.y);
}

/// The value the fill shades at barycentric weights `bary` of triangle
/// `tri`, read through its indices: the triangle's own value under
/// `FIELD_FACE_VALUES`, else the blend of its corners' pairs. Not placeable
/// when a value is missing or not finite, as a fragment of the flat draw is
/// there (`mesh_fragment_value`).
fn mesh_values_at(tri: u32, bary: vec3<f32>) -> FieldFillValue {
    var none: FieldFillValue;
    none.placeable = false;
    none.z = vec2<f32>(0.0);
    none.t = -1.0;
    if (field_flag(FIELD_FACE_VALUES)) {
        let v = grid_value(MESH_VALUES, tri);
        if (!v.valid || !pair_finite(v.pair)) {
            return none;
        }
        return field_placed_value(v.pair);
    }
    let v0 = grid_value(MESH_VALUES, mesh_index(tri, 0u));
    let v1 = grid_value(MESH_VALUES, mesh_index(tri, 1u));
    let v2 = grid_value(MESH_VALUES, mesh_index(tri, 2u));
    if (!(v0.valid && v1.valid && v2.valid)
        || !pair_finite(v0.pair) || !pair_finite(v1.pair) || !pair_finite(v2.pair)) {
        return none;
    }
    let b = max(bary, vec3<f32>(0.0));
    let w = b / max(b.x + b.y + b.z, 1e-30);
    return field_placed_value(v0.pair * w.x + v1.pair * w.y + v2.pair * w.z);
}

/// One triangle as the draw places it: its three vertices in NDC and their
/// value pairs (vertex values) or its own value in `value0` (face values).
/// `valid` is false when the triangle is past the drawn count, names a
/// vertex past the vertex count, or has a vertex the axes cannot place; it
/// is then not drawn, as a non-finite sample breaks a line.
struct MeshTriangle {
    valid: bool,
    ndc0: vec2<f32>,
    ndc1: vec2<f32>,
    ndc2: vec2<f32>,
    value0: vec2<f32>,
    value1: vec2<f32>,
    value2: vec2<f32>,
};

fn mesh_triangle(tri: u32) -> MeshTriangle {
    var out: MeshTriangle;
    out.valid = false;
    out.ndc0 = vec2<f32>(0.0);
    out.ndc1 = vec2<f32>(0.0);
    out.ndc2 = vec2<f32>(0.0);
    out.value0 = vec2<f32>(0.0);
    out.value1 = vec2<f32>(0.0);
    out.value2 = vec2<f32>(0.0);
    if (tri >= mesh_triangle_count()) {
        return out;
    }
    let vertices = mesh_vertex_count();
    let i0 = mesh_index(tri, 0u);
    let i1 = mesh_index(tri, 1u);
    let i2 = mesh_index(tri, 2u);
    if (i0 >= vertices || i1 >= vertices || i2 >= vertices) {
        return out;
    }
    let x0 = pool_pair(field.x_base, i0);
    let y0 = pool_pair(field.y_base, i0);
    let x1 = pool_pair(field.x_base, i1);
    let y1 = pool_pair(field.y_base, i1);
    let x2 = pool_pair(field.x_base, i2);
    let y2 = pool_pair(field.y_base, i2);
    if (!sample_valid(transform, x0, y0) || !sample_valid(transform, x1, y1)
        || !sample_valid(transform, x2, y2)) {
        return out;
    }
    out.ndc0 = data_to_ndc(transform, x0, y0);
    out.ndc1 = data_to_ndc(transform, x1, y1);
    out.ndc2 = data_to_ndc(transform, x2, y2);
    if (field_flag(FIELD_FACE_VALUES)) {
        out.value0 = grid_value(MESH_VALUES, tri).pair;
    } else {
        out.value0 = grid_value(MESH_VALUES, i0).pair;
        out.value1 = grid_value(MESH_VALUES, i1).pair;
        out.value2 = grid_value(MESH_VALUES, i2).pair;
    }
    out.valid = true;
    return out;
}

/// The length of the gradient of a triangle's linear interpolant, in value
/// units per output pixel, from its three vertices in pixels and their value
/// pairs: analytic, as the heatmap's contour metric is, never `dpdx`. It
/// only sets the contour stroke's coverage and is never shown as data (rule
/// 1). Zero is a plateau; a negative answer is a degenerate or unplaceable
/// triangle.
fn mesh_gradient_from_px(
    p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>,
    value0: vec2<f32>, value1: vec2<f32>, value2: vec2<f32>,
) -> f32 {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    // Differences of pairs keep the low halves' precision.
    let d1 = (value1.x - value0.x) + (value1.y - value0.y);
    let d2 = (value2.x - value0.x) + (value2.y - value0.y);
    let det = e1.x * e2.y - e1.y * e2.x;
    if (!f32_is_finite(det) || det == 0.0) {
        return -1.0;
    }
    let g = vec2<f32>(d1 * e2.y - d2 * e1.y, d2 * e1.x - d1 * e2.x) / det;
    let len = length(g);
    if (!f32_is_finite(len)) {
        return -1.0;
    }
    return len;
}

/// `mesh_gradient_from_px` of a triangle the flat draw placed: its NDC
/// vertices in the target's pixels.
fn mesh_gradient_px(m: MeshTriangle) -> f32 {
    return mesh_gradient_from_px(
        m.ndc0 / transform.pixel_to_ndc,
        m.ndc1 / transform.pixel_to_ndc,
        m.ndc2 / transform.pixel_to_ndc,
        m.value0, m.value1, m.value2,
    );
}
