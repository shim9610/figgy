// ── Surface input: a triangle mesh as a surface (group 2 and group 3) ───────
//
// A triangle mesh on a 3D chart (`Config.view_3d`, #210) draws as a surface
// over its triangles: one instance per triangle, three vertices, no vertex
// buffers. Each vertex reads its triangle's three indices
// (`tri_mesh/mesh_input.wgsl`, SHADER_COMMON.md §5.25) and places all three
// vertices by their x, y and height columns through the camera, so the three
// invocations agree on whether the triangle draws and on its flat normal.
// The grid table holds the value column (entry `MESH_VALUES`), the triangle
// index column (`MESH_INDICES`) and the height column (`MESH_HEIGHTS`, the
// series' `z_column`). Nothing is stored per vertex: positions, colours and
// the per-triangle normal are computed here on every draw (rule 1, rule 2).
//
// The surface's look is `SurfaceParams` (`surface_params.wgsl`); the entries
// are `surface_entry.wgsl`'s, over the hooks defined at the end of this
// fragment.

/// The grid table entry of the mesh's height column.
const MESH_HEIGHTS: u32 = 2u;

/// The vertices a triangle may name on a surface: the flat count
/// (`mesh_vertex_count`) and the height column.
fn mesh_surface_vertex_count() -> u32 {
    return min(mesh_vertex_count(), grid[MESH_HEIGHTS].len);
}

/// Box point of vertex `i` and whether it can be placed: its x, y and
/// height finite (a height positive on a logarithmic z axis).
struct MeshSurfacePoint {
    t: vec3<f32>,
    valid: bool,
};

fn mesh_surface_point(i: u32) -> MeshSurfacePoint {
    var out: MeshSurfacePoint;
    let x = view3d_axis_t(pool_pair(field.x_base, i), 0u);
    let y = view3d_axis_t(pool_pair(field.y_base, i), 1u);
    let h = grid_value(MESH_HEIGHTS, i);
    let z = view3d_z_t(h.pair);
    out.t = vec3<f32>(x, y, z);
    out.valid = h.valid && vec2_f32_is_finite(h.pair)
        && f32_is_finite(x) && f32_is_finite(y) && f32_is_finite(z);
    return out;
}

/// The box-space normal of a triangle from its three box points: a shading
/// value computed on every draw, never stored. The +z normal for a
/// degenerate triangle.
fn mesh_surface_normal(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    let nrm = cross(
        box_space_point(b) - box_space_point(a),
        box_space_point(c) - box_space_point(a),
    );
    let len = length(nrm);
    if (len <= 0.0 || !f32_is_finite(len)) {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    return nrm / len;
}

/// One vertex of the mesh surface: its triangle in `quad.x`, its corner's
/// barycentric weight in `frac`, its box point and the triangle's normal.
/// `valid` is false when the triangle is past the drawn count, names a
/// vertex past the vertex count, or has a vertex that cannot be placed: the
/// triangle is dropped, as a NaN breaks a line.
struct SurfaceVertex {
    quad: vec2<u32>,
    frac: vec3<f32>,
    t: vec3<f32>,
    normal: vec3<f32>,
    valid: bool,
};

fn surface_vertex(tri: u32, vertex: u32) -> SurfaceVertex {
    var out: SurfaceVertex;
    out.valid = false;
    out.quad = vec2<u32>(tri, 0u);
    let corner = vertex % 3u;
    out.frac = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    if (tri >= mesh_triangle_count()) {
        return out;
    }
    let vertices = mesh_surface_vertex_count();
    let i0 = mesh_index(tri, 0u);
    let i1 = mesh_index(tri, 1u);
    let i2 = mesh_index(tri, 2u);
    if (i0 >= vertices || i1 >= vertices || i2 >= vertices) {
        return out;
    }
    let p0 = mesh_surface_point(i0);
    let p1 = mesh_surface_point(i1);
    let p2 = mesh_surface_point(i2);
    if (!p0.valid || !p1.valid || !p2.valid) {
        return out;
    }
    var here = p0.t;
    if (corner == 1u) {
        here = p1.t;
    } else if (corner == 2u) {
        here = p2.t;
    }
    out.t = here;
    if (surface_flag(SURFACE_LIT)) {
        out.normal = mesh_surface_normal(p0.t, p1.t, p2.t);
    }
    out.valid = true;
    return out;
}

/// The premultiplied fill colour at barycentric weights `frac` of triangle
/// `quad.x`: the mesh's own value blend (`mesh_values_at`) through the
/// heatmap's `field_value_color`, as the flat mesh paints it.
fn surface_fill_color(quad: vec2<u32>, frac: vec3<f32>) -> vec4<f32> {
    return field_value_color(mesh_values_at(quad.x, frac), style.color_premul);
}

// ── The entries' hooks for a mesh ───────────────────────────────────────────

/// Distance in pixels from the fragment to the nearest edge of its triangle:
/// each barycentric weight over its screen-space rate.
fn surface_edge_px(in: SurfaceOut) -> f32 {
    let rate = max(fwidth(in.frac), vec3<f32>(1e-6));
    let edge = max(in.frac, vec3<f32>(0.0)) / rate;
    return min(edge.x, min(edge.y, edge.z));
}

/// The ID pick: the series, kind 2, the triangle and its vertex nearest the
/// fragment (the largest barycentric weight).
fn surface_pick_ids(in: SurfaceOut) -> vec4<u32> {
    var corner = 0u;
    if (in.frac.y > in.frac.x && in.frac.y >= in.frac.z) {
        corner = 1u;
    } else if (in.frac.z > in.frac.x && in.frac.z > in.frac.y) {
        corner = 2u;
    }
    return vec4<u32>(surface.pick_id, 2u, in.quad.x, mesh_index(in.quad.x, corner));
}

/// The picked triangle's patch: the triangle itself, one instance.
fn surface_selected_vertex(k: u32, vertex: u32) -> SurfaceVertex {
    var none: SurfaceVertex;
    none.valid = false;
    if (k != 0u || surface.picked_c == SURFACE_NO_PICK) {
        return none;
    }
    return surface_vertex(surface.picked_c, vertex);
}

/// Every fragment of the picked triangle is in its patch; the outline's
/// distance is the distance to the triangle's edges.
fn surface_patch(in: SurfaceOut) -> SurfacePatch {
    var out: SurfacePatch;
    out.inside = true;
    out.edge_px = surface_edge_px(in);
    return out;
}
