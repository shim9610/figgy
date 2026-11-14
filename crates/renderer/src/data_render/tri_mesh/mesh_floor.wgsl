// ── Triangle mesh floor contours (#210) ─────────────────────────────────────
//
// On a 3D chart (`Config.view_3d`) a mesh's contour lines draw on the box
// floor, as a heatmap's do (`field_columnar.wgsl::vs_floor`): `vs_tri_floor`
// places each triangle's vertices at `t.z = 0` through the camera
// (`project_box`), carrying the values and the gradient the flat draw
// carries (`mesh_triangle`, `mesh_gradient_px`). The fragment carries the
// stroke's half-width to the floor through the screen gradient of its value
// (`mesh_floor_screen_scale`), so a floor stroke is `line_width_px` wide on
// screen whichever way it runs, as a heatmap's floor contours are (#259),
// and the draw and the 1×1 ID pick (whose camera maps the cursor pixel onto
// its target) agree. `fs_tri_floor` is the flat contour fragment
// (`mesh_contour_paint`) with the floor clipped to the box, and
// `fs_tri_floor_pick` writes what the pick reads: the series with the top
// bit marking a floor contour, the level painted and the triangle.

/// Vertex `i`'s floor point: its axis fractions (`view3d_axis_t`), the box
/// point `(x, y, 0)`.
fn mesh_floor_point(i: u32) -> vec2<f32> {
    return vec2<f32>(
        view3d_axis_t(pool_pair(field.x_base, i), 0u),
        view3d_axis_t(pool_pair(field.y_base, i), 1u),
    );
}

@vertex
fn vs_tri_floor(@builtin(vertex_index) vid: u32, @builtin(instance_index) tri: u32) -> MeshOut {
    var out: MeshOut;
    // Outside the clip volume: a triangle that does not draw is clipped.
    out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.bary = vec3<f32>(0.0);
    out.values01 = vec4<f32>(0.0);
    out.values2 = vec4<f32>(0.0);
    out.info = vec2<u32>(tri, 0u);
    out.floor_t = vec2<f32>(0.0);
    // The triangle as the flat draw places it: its values and, below, its
    // gradient in floor-plane pixels.
    let m = mesh_triangle(tri);
    if (!m.valid) {
        return out;
    }
    let i0 = mesh_index(tri, 0u);
    let i1 = mesh_index(tri, 1u);
    let i2 = mesh_index(tri, 2u);
    let f0 = mesh_floor_point(i0);
    let f1 = mesh_floor_point(i1);
    let f2 = mesh_floor_point(i2);
    if (!vec2_f32_is_finite(f0) || !vec2_f32_is_finite(f1) || !vec2_f32_is_finite(f2)) {
        return out;
    }
    let c0 = project_box(vec3<f32>(f0, 0.0));
    let c1 = project_box(vec3<f32>(f1, 0.0));
    let c2 = project_box(vec3<f32>(f2, 0.0));
    if (c0.w <= 0.0 || c1.w <= 0.0 || c2.w <= 0.0) {
        return out;
    }
    let corner = vid % 3u;
    var clip = c0;
    var foot = f0;
    if (corner == 1u) {
        clip = c1;
        foot = f1;
    } else if (corner == 2u) {
        clip = c2;
        foot = f2;
    }
    out.pos = clip;
    out.floor_t = foot;
    out.bary = vec3<f32>(f32(corner == 0u), f32(corner == 1u), f32(corner == 2u));
    out.values01 = vec4<f32>(m.value0, m.value1);
    var finite = pair_finite(m.value0);
    if (!field_flag(FIELD_FACE_VALUES)) {
        finite = finite && pair_finite(m.value1) && pair_finite(m.value2);
        out.values2 = vec4<f32>(m.value2, mesh_gradient_px(m), 0.0);
    }
    out.info.y = select(0u, MESH_VALUES_FINITE, finite);
    return out;
}

/// Whether a floor fragment lies on the box floor: a triangle reaching past
/// the axis ranges is clipped to the box, as the surface is.
fn mesh_floor_visible(floor_t: vec2<f32>) -> bool {
    return box_visible(vec3<f32>(floor_t, 0.0));
}

/// The screen gradient of the fragment's value over its floor-plane
/// gradient (`in.values2.z`): how many floor pixels across the isoline make
/// one screen pixel here. Taken before any discard, in uniform control flow.
/// One where either is degenerate.
fn mesh_floor_screen_scale(in: MeshOut) -> f32 {
    let z = mesh_fragment_value(in).z;
    let f = z.x + z.y;
    let screen = length(vec2<f32>(dpdx(f), dpdy(f)));
    let floor = in.values2.z;
    let scale = screen / floor;
    return select(1.0, scale, f32_is_finite(scale) && scale > 0.0 && floor > 0.0);
}

@fragment
fn fs_tri_floor(in: MeshOut) -> @location(0) vec4<f32> {
    mesh_floor_width_scale = mesh_floor_screen_scale(in);
    if (!mesh_floor_visible(in.floor_t)) {
        discard;
    }
    return mesh_contour_paint(in).color;
}

/// The floor contours' ID pick: the series (`view3d.flags.w`, its pick slot
/// plus one, with the top bit marking a floor contour), the topmost level
/// painted at the pixel and the triangle it lies in.
@fragment
fn fs_tri_floor_pick(in: MeshOut) -> @location(0) vec4<u32> {
    mesh_floor_width_scale = mesh_floor_screen_scale(in);
    let paint = mesh_contour_paint(in);
    if (!mesh_floor_visible(in.floor_t) || paint.level == 0xffffffffu) {
        discard;
    }
    return vec4<u32>(u32(view3d.flags.w) | 0x80000000u, paint.level, in.info.x, 0u);
}
