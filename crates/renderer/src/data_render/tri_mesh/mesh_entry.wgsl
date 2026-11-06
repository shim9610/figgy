// ── Triangle mesh entries (#210) ──────────────────────────────────────────
//
// One instance per triangle and three vertices, `draw(0..3, 0..T)`: no vertex
// buffers. Each vertex reads its triangle's three indices and places all
// three vertices, so the three invocations agree on whether the triangle
// draws and on its flat varyings. A triangle that does not draw puts every
// vertex outside the clip volume.
//
// The fill is opaque per sample without coverage antialiasing: under MSAA
// every sample of a shared edge is covered by exactly one triangle, so
// adjacent triangles do not seam (the reason the heatmap is one quad,
// `field_columnar.wgsl`, does not arise).

/// Bit 0 of `MeshOut.info.y`: every value the fill blends is finite.
const MESH_VALUES_FINITE: u32 = 1u;

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    /// The fragment's barycentric weights. Centroid sampling keeps an edge
    /// pixel's weights inside the triangle, so it never extrapolates.
    @location(0) @interpolate(perspective, centroid) bary: vec3<f32>,
    /// Corner 0's and corner 1's value pairs; a face value in `.xy`.
    @location(1) @interpolate(flat) values01: vec4<f32>,
    /// Corner 2's value pair, and the gradient length in value units per
    /// pixel (`mesh_gradient_px`) in `.z`.
    @location(2) @interpolate(flat) values2: vec4<f32>,
    /// (triangle index, `MESH_VALUES_FINITE` when set).
    @location(3) @interpolate(flat) info: vec2<u32>,
};

fn pair_finite(v: vec2<f32>) -> bool {
    return f32_is_finite(v.x) && f32_is_finite(v.y) && f32_is_finite(v.x + v.y);
}

@vertex
fn vs_tri(@builtin(vertex_index) vid: u32, @builtin(instance_index) tri: u32) -> MeshOut {
    var out: MeshOut;
    // Outside the clip volume: a triangle that does not draw is clipped.
    out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.bary = vec3<f32>(0.0);
    out.values01 = vec4<f32>(0.0);
    out.values2 = vec4<f32>(0.0);
    out.info = vec2<u32>(tri, 0u);
    let m = mesh_triangle(tri);
    if (!m.valid) {
        return out;
    }
    let corner = vid % 3u;
    var ndc = m.ndc0;
    if (corner == 1u) {
        ndc = m.ndc1;
    } else if (corner == 2u) {
        ndc = m.ndc2;
    }
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
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

/// The value the fill shades at a fragment: the triangle's own value, or the
/// barycentric blend of its corners' pairs (linear in screen space, so in
/// axis `t` on a logarithmic axis, as the heatmap's interpolation is).
fn mesh_fragment_value(in: MeshOut) -> FieldFillValue {
    if ((in.info.y & MESH_VALUES_FINITE) == 0u) {
        var out: FieldFillValue;
        out.placeable = false;
        out.z = vec2<f32>(0.0);
        out.t = -1.0;
        return out;
    }
    if (field_flag(FIELD_FACE_VALUES)) {
        return field_placed_value(in.values01.xy);
    }
    let b = max(in.bary, vec3<f32>(0.0));
    let w = b / max(b.x + b.y + b.z, 1e-30);
    let z = in.values01.xy * w.x + in.values01.zw * w.y + in.values2.xy * w.z;
    return field_placed_value(z);
}

@fragment
fn fs_tri(in: MeshOut) -> @location(0) vec4<f32> {
    return field_value_color(mesh_fragment_value(in), style.color_premul);
}

/// The contour strokes over a triangle: the level set of its linear
/// interpolant, the distance to level `L` being `|f − L| / |∇f|` in pixels
/// (exact for a linear function, with no curvature). A plateau (zero
/// gradient) draws nothing; face values draw no contours. The level search
/// and the compositing are the heatmap's (`field_paint.wgsl`).
fn mesh_contour_paint(in: MeshOut) -> ContourPaint {
    var out = contour_paint_none();
    if ((in.info.y & MESH_VALUES_FINITE) == 0u || field_flag(FIELD_FACE_VALUES)) {
        return out;
    }
    let g = in.values2.z;
    if (!f32_is_finite(g) || g <= 0.0 || !f32_is_finite(field.line_width_px)) {
        return out;
    }
    let v = mesh_fragment_value(in);
    let z = v.z;
    let f = z.x + z.y;
    let half_w = max(field.line_width_px, 0.0) * 0.5;
    let margin = (half_w + 1.0) * g;
    let search_lo = f - margin;
    let search_hi = f + margin;
    if (!f32_is_finite(f) || !f32_is_finite(margin) || !f32_is_finite(search_lo)
        || !f32_is_finite(search_hi)) {
        return out;
    }
    for (var block = 0u; block < contour_level_block_count(); block = block + 1u) {
        let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
        var candidates = contour_block_candidates(block, search_lo, search_hi);
        while (candidates != 0u) {
            let original = firstTrailingBit(candidates);
            candidates = candidates & ~(1u << original);
            let i = start + original;
            let lv = levels[i];
            if (!f32_is_finite(lv)) {
                continue;
            }
            let distance_to_level = abs((z.x - lv) + z.y) / g;
            out = contour_level_over(out, i, half_w - distance_to_level + 0.5);
        }
    }
    return out;
}

@fragment
fn fs_tri_contour(in: MeshOut) -> @location(0) vec4<f32> {
    return mesh_contour_paint(in).color;
}
