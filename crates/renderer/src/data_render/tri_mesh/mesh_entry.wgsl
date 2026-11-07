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
    /// On a 3D chart's floor (`mesh_floor.wgsl`): the fragment's axis
    /// fractions, which clip it to the box floor. Zero in the flat entries.
    @location(4) floor_t: vec2<f32>,
};

@vertex
fn vs_tri(@builtin(vertex_index) vid: u32, @builtin(instance_index) tri: u32) -> MeshOut {
    var out: MeshOut;
    // Outside the clip volume: a triangle that does not draw is clipped.
    out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.bary = vec3<f32>(0.0);
    out.values01 = vec4<f32>(0.0);
    out.values2 = vec4<f32>(0.0);
    out.info = vec2<u32>(tri, 0u);
    out.floor_t = vec2<f32>(0.0);
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

// ───── Typed data pick (compute entry) ──────────────────────────────────────
//
// `pick_tri_mesh` answers the triangle under the cursor and, over it, a
// contour stroke (#210 D5). One 64-lane workgroup walks every triangle, as the
// parallel pick walks its segments, and reduces in shared memory. Each test is
// made in the draw's pixel frame at the centre of the cursor's pixel, the
// point the fill and contour fragments shade:
// - inside a triangle the distance is 0; outside, a triangle counts only
//   within one output pixel of its edge (the multisampled fringe);
// - the fill answers only where it paints (`field_fill_paints`);
// - on overlap the higher triangle wins, as it is drawn later and on top;
// - a contour stroke within the pick tolerance wins over the fill, as it is
//   drawn over it. It is the stroke of the triangle the cursor lies in, or
//   within the fringe of (a stroke's edge pixel shades its covered samples
//   from the triangle they fall in), taken from that triangle's own linear
//   interpolant, as the contour fragment there does. The level loop runs once, after the reduce, so
// no loop is nested in another (SwiftShader, #236).

const MESH_PICK_WORKGROUP_SIZE: u32 = 64u;
const DATA_PICK_FLAG_FILL: u32 = 1u;
const DATA_PICK_FLAG_CONTOUR: u32 = 2u;
const DATA_PICK_KIND_CONTOUR_LEVEL: u32 = 3u;
const DATA_PICK_KIND_MESH_TRIANGLE: u32 = 5u;
/// How far outside its edges a triangle still picks, in output pixels: the
/// multisampled fringe its edge pixels paint.
const MESH_PICK_FRINGE_PX: f32 = 1.0;

@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

var<workgroup> mesh_pick_shared: array<DataPickCandidate, MESH_PICK_WORKGROUP_SIZE>;
/// Per lane, the drawn triangle nearest the cursor within the fringe, for
/// the contour test: `index0` is the triangle, `distance_px` its distance.
var<workgroup> mesh_pick_near: array<DataPickCandidate, MESH_PICK_WORKGROUP_SIZE>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        mesh_pick_shared[local_index] = DataPickCandidate();
        mesh_pick_near[local_index] = DataPickCandidate();
        workgroupBarrier();
    }
}

fn mesh_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
}

/// The draw's target size in pixels; axis `t` times it is the pixel frame.
fn mesh_pick_size() -> vec2<f32> {
    return round(vec2<f32>(2.0) / transform.pixel_to_ndc);
}

fn mesh_pick_px(ndc: vec2<f32>) -> vec2<f32> {
    return (ndc + vec2<f32>(1.0)) * 0.5 * mesh_pick_size();
}

/// The centre of the cursor's pixel, in the pixel frame.
fn mesh_pick_cursor() -> vec2<f32> {
    let p = data_pick_query.cursor_ndc_t.zw * mesh_pick_size();
    return round(p - vec2<f32>(0.5)) + vec2<f32>(0.5);
}

/// Where `p` lies against triangle `a`, `b`, `c`: the distance to it (0
/// inside, negative for a degenerate triangle, which draws nothing) and the
/// barycentric weights of the nearest point of it.
struct MeshPickLocation {
    distance: f32,
    bary: vec3<f32>,
};

fn mesh_pick_edge(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    // (distance, parameter along a → b) of the nearest point of the edge.
    let d = b - a;
    let d2 = dot(d, d);
    let s = select(0.0, clamp(dot(p - a, d) / d2, 0.0, 1.0), d2 > 0.0);
    return vec2<f32>(length(p - (a + d * s)), s);
}

fn mesh_pick_locate(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> MeshPickLocation {
    var out: MeshPickLocation;
    out.distance = -1.0;
    out.bary = vec3<f32>(0.0);
    let e1 = b - a;
    let e2 = c - a;
    let det = e1.x * e2.y - e1.y * e2.x;
    if (!f32_is_finite(det) || det == 0.0) {
        return out;
    }
    let d = p - a;
    let w1 = (d.x * e2.y - d.y * e2.x) / det;
    let w2 = (e1.x * d.y - e1.y * d.x) / det;
    let w0 = 1.0 - w1 - w2;
    if (!f32_is_finite(w0) || !f32_is_finite(w1) || !f32_is_finite(w2)) {
        return out;
    }
    if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) {
        out.distance = 0.0;
        out.bary = vec3<f32>(w0, w1, w2);
        return out;
    }
    let ab = mesh_pick_edge(p, a, b);
    let bc = mesh_pick_edge(p, b, c);
    let ca = mesh_pick_edge(p, c, a);
    out.distance = ab.x;
    out.bary = vec3<f32>(1.0 - ab.y, ab.y, 0.0);
    if (bc.x < out.distance) {
        out.distance = bc.x;
        out.bary = vec3<f32>(0.0, 1.0 - bc.y, bc.y);
    }
    if (ca.x < out.distance) {
        out.distance = ca.x;
        out.bary = vec3<f32>(ca.y, 0.0, 1.0 - ca.y);
    }
    return out;
}

/// The value the fill shades at barycentric `bary` of `m`, as
/// `mesh_fragment_value` does for a fragment there.
fn mesh_pick_value(m: MeshTriangle, bary: vec3<f32>) -> FieldFillValue {
    var finite = pair_finite(m.value0);
    if (!field_flag(FIELD_FACE_VALUES)) {
        finite = finite && pair_finite(m.value1) && pair_finite(m.value2);
    }
    if (!finite) {
        var out: FieldFillValue;
        out.placeable = false;
        out.z = vec2<f32>(0.0);
        out.t = -1.0;
        return out;
    }
    if (field_flag(FIELD_FACE_VALUES)) {
        return field_placed_value(m.value0);
    }
    let b = max(bary, vec3<f32>(0.0));
    let w = b / max(b.x + b.y + b.z, 1e-30);
    return field_placed_value(m.value0 * w.x + m.value1 * w.y + m.value2 * w.z);
}

/// Triangle `tri` as a fill candidate at `cursor`: valid where it is drawn,
/// within the fringe and painted. `index1` is its vertex nearest the cursor.
fn mesh_pick_candidate(tri: u32, cursor: vec2<f32>) -> DataPickCandidate {
    let m = mesh_triangle(tri);
    if (!m.valid) {
        return mesh_pick_invalid();
    }
    let a = mesh_pick_px(m.ndc0);
    let b = mesh_pick_px(m.ndc1);
    let c = mesh_pick_px(m.ndc2);
    let at = mesh_pick_locate(cursor, a, b, c);
    if (at.distance < 0.0 || at.distance > MESH_PICK_FRINGE_PX
        || !field_fill_paints(mesh_pick_value(m, at.bary), data_pick_query.baseline)) {
        return mesh_pick_invalid();
    }
    var corner = 0u;
    var nearest = length(cursor - a);
    if (length(cursor - b) < nearest) {
        corner = 1u;
        nearest = length(cursor - b);
    }
    if (length(cursor - c) < nearest) {
        corner = 2u;
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_MESH_TRIANGLE,
        tri,
        mesh_index(tri, corner),
        0u,
        at.distance,
        tri,
    );
}

/// Nearer wins; on a tie the higher triangle, which is drawn on top.
fn mesh_pick_better(candidate: DataPickCandidate, incumbent: DataPickCandidate) -> bool {
    if (candidate.valid == 0u) {
        return false;
    }
    if (incumbent.valid == 0u) {
        return true;
    }
    if (candidate.distance_px != incumbent.distance_px) {
        return candidate.distance_px < incumbent.distance_px;
    }
    return candidate.index0 > incumbent.index0;
}

/// `tri` as the contour test's triangle: valid where it is drawn and the
/// cursor lies within its fringe, at that distance.
fn mesh_pick_near_candidate(tri: u32, cursor: vec2<f32>) -> DataPickCandidate {
    let m = mesh_triangle(tri);
    if (!m.valid) {
        return mesh_pick_invalid();
    }
    let at = mesh_pick_locate(cursor, mesh_pick_px(m.ndc0), mesh_pick_px(m.ndc1),
        mesh_pick_px(m.ndc2));
    if (at.distance < 0.0 || at.distance > MESH_PICK_FRINGE_PX) {
        return mesh_pick_invalid();
    }
    return DataPickCandidate(1u, data_pick_query.data.y, 0u, tri, 0u, 0u, at.distance, tri);
}

/// The contour stroke nearest `cursor` on triangle `tri`, as
/// `mesh_contour_paint` draws it there: the distance to level `L` is
/// `|f − L| / |∇f|`, less half the stroke, with `f` taken at the nearest
/// point of the triangle; a cursor in the fringe adds its distance to that
/// point, so only a cursor inside picks a stroke at distance zero. Nearer
/// wins; on a tie the later level, which is drawn on top.
fn mesh_pick_contour(tri: u32, cursor: vec2<f32>) -> DataPickCandidate {
    var best = mesh_pick_invalid();
    if (field_flag(FIELD_FACE_VALUES) || !f32_is_finite(data_pick_query.limits.x)
        || !f32_is_finite(data_pick_query.limits.z)) {
        return best;
    }
    let m = mesh_triangle(tri);
    if (!m.valid) {
        return best;
    }
    let g = mesh_gradient_px(m);
    if (!f32_is_finite(g) || g <= 0.0) {
        return best;
    }
    let at = mesh_pick_locate(cursor, mesh_pick_px(m.ndc0), mesh_pick_px(m.ndc1),
        mesh_pick_px(m.ndc2));
    let v = mesh_pick_value(m, at.bary);
    if (at.distance < 0.0 || at.distance > MESH_PICK_FRINGE_PX || !v.placeable) {
        return best;
    }
    let half_width = max(data_pick_query.limits.z, 0.0) * 0.5;
    let max_distance = max(data_pick_query.limits.x, 0.0);
    for (var level = 0u; level < field.level_count; level = level + 1u) {
        let lv = levels[level];
        if (!f32_is_finite(lv)) {
            continue;
        }
        let hit = max(abs((v.z.x - lv) + v.z.y) / g - half_width, 0.0) + at.distance;
        if (f32_is_finite(hit) && hit <= max_distance
            && (best.valid == 0u || hit < best.distance_px
                || (hit == best.distance_px && level > best.index0))) {
            best = DataPickCandidate(
                1u,
                data_pick_query.data.y,
                DATA_PICK_KIND_CONTOUR_LEVEL,
                level,
                tri,
                0u,
                hit,
                level,
            );
        }
    }
    return best;
}

@compute @workgroup_size(MESH_PICK_WORKGROUP_SIZE)
fn pick_tri_mesh(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    let cursor = mesh_pick_cursor();
    let fill = (data_pick_query.data.x & DATA_PICK_FLAG_FILL) != 0u;
    let contour = (data_pick_query.data.x & DATA_PICK_FLAG_CONTOUR) != 0u;
    var best = mesh_pick_invalid();
    var near = mesh_pick_invalid();
    let count = mesh_triangle_count();
    var tri = local_index;
    while (tri < count) {
        if (fill) {
            let candidate = mesh_pick_candidate(tri, cursor);
            if (mesh_pick_better(candidate, best)) {
                best = candidate;
            }
        }
        if (contour) {
            let candidate = mesh_pick_near_candidate(tri, cursor);
            if (mesh_pick_better(candidate, near)) {
                near = candidate;
            }
        }
        tri = tri + MESH_PICK_WORKGROUP_SIZE;
    }
    mesh_pick_shared[local_index] = best;
    mesh_pick_near[local_index] = near;
    workgroupBarrier();
    for (var stride = MESH_PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if (local_index < stride) {
            let candidate = mesh_pick_shared[local_index + stride];
            if (mesh_pick_better(candidate, mesh_pick_shared[local_index])) {
                mesh_pick_shared[local_index] = candidate;
            }
            let other = mesh_pick_near[local_index + stride];
            if (mesh_pick_better(other, mesh_pick_near[local_index])) {
                mesh_pick_near[local_index] = other;
            }
        }
        workgroupBarrier();
    }
    if (local_index == 0u) {
        var answer = mesh_pick_shared[0];
        let near_triangle = mesh_pick_near[0];
        if (near_triangle.valid != 0u) {
            let stroke = mesh_pick_contour(near_triangle.index0, cursor);
            if (stroke.valid != 0u) {
                answer = stroke;
            }
        }
        data_pick_output = answer;
    }
}

// ───── Typed selection overlay (render entry) ───────────────────────────────
//
// `fs_tri_selection` runs over `vs_tri`, through the selection layer's
// instances (#210): the picked triangle alone, outlined inside its edges at
// the selection's outline width; or every triangle, painting the picked
// contour level's stroke widened by the selection's extra width, from the
// same linear interpolant the contour draw uses.

const DATA_SELECTION_KIND_MESH_TRIANGLE: u32 = 5u;

@group(1) @binding(4) var<uniform> data_selection: DataSelection;

fn mesh_selection_color(coverage: f32) -> vec4<f32> {
    if (!f32_is_finite(coverage) || coverage <= 0.0
        || !vec4_f32_is_finite(data_selection.color_premul)) {
        return vec4<f32>(0.0);
    }
    return data_selection.color_premul * clamp(coverage, 0.0, 1.0);
}

/// The picked triangle's outline coverage: its width less the fragment's
/// distance to the nearest edge, in pixels, from each barycentric weight
/// over its screen-space rate (`bary_px`).
fn mesh_selected_triangle_coverage(in: MeshOut, bary_px: vec3<f32>) -> f32 {
    let width_px = max(data_selection.metrics.x, 0.0);
    if (!f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    let rate = max(bary_px, vec3<f32>(1e-30));
    let edge = max(in.bary, vec3<f32>(0.0)) / rate;
    let distance_px = min(edge.x, min(edge.y, edge.z));
    return width_px - distance_px + 0.5;
}

/// The picked level's widened stroke, as `mesh_contour_paint` draws one level.
fn mesh_selected_contour_coverage(in: MeshOut) -> f32 {
    let level = data_selection.indices.y;
    let width_px = max(field.line_width_px, 0.0) + max(data_selection.metrics.y, 0.0);
    if (level >= field.level_count || !f32_is_finite(width_px) || width_px <= 0.0
        || (in.info.y & MESH_VALUES_FINITE) == 0u || field_flag(FIELD_FACE_VALUES)) {
        return 0.0;
    }
    let g = in.values2.z;
    let lv = levels[level];
    if (!f32_is_finite(g) || g <= 0.0 || !f32_is_finite(lv)) {
        return 0.0;
    }
    let z = mesh_fragment_value(in).z;
    let distance_px = abs((z.x - lv) + z.y) / g;
    return width_px * 0.5 - distance_px + 0.5;
}

@fragment
fn fs_tri_selection(in: MeshOut) -> @location(0) vec4<f32> {
    // Derivatives first, in uniform control flow.
    let bary_px = fwidth(in.bary);
    let kind = data_selection.indices.x;
    if (kind == DATA_SELECTION_KIND_MESH_TRIANGLE) {
        return mesh_selection_color(mesh_selected_triangle_coverage(in, bary_px));
    }
    if (kind == DATA_PICK_KIND_CONTOUR_LEVEL) {
        return mesh_selection_color(mesh_selected_contour_coverage(in));
    }
    return vec4<f32>(0.0);
}
