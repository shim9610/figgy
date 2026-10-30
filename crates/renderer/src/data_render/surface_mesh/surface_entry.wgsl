// ── Surface entries ─────────────────────────────────────────────────────────
//
// `vs_main` / `fs_main` draw the surface into the panel composite with the
// depth test; `vs_selected` / `fs_selected` draw the picked sample's patch
// in the frame's live highlight slot, over anything in front of it (no
// depth); `fs_pick` writes the ID of what a pixel shows for the 1×1 ID pick.

struct SurfaceOut {
    @builtin(position) pos: vec4<f32>,
    /// Box point, perspective-correct: the box clip and the material.
    @location(0) t: vec3<f32>,
    /// Position inside the quad, 0..1 along x and y.
    @location(1) frac: vec2<f32>,
    @location(2) @interpolate(flat) quad: vec2<u32>,
    @location(3) normal: vec3<f32>,
};

fn surface_hidden() -> SurfaceOut {
    var out: SurfaceOut;
    out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.t = vec3<f32>(0.0);
    out.frac = vec2<f32>(0.0);
    out.quad = vec2<u32>(0u);
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    return out;
}

fn surface_out(v: SurfaceVertex, pos: vec4<f32>) -> SurfaceOut {
    var out: SurfaceOut;
    out.pos = pos;
    out.t = v.t;
    out.frac = vec2<f32>(v.corner);
    out.quad = v.quad;
    out.normal = v.normal;
    return out;
}

@vertex
fn vs_main(@builtin(instance_index) quad: u32, @builtin(vertex_index) vertex: u32) -> SurfaceOut {
    let v = surface_vertex(quad, vertex);
    if (!v.valid) {
        return surface_hidden();
    }
    return surface_out(v, project_box(v.t));
}

/// The lit, wireframed colour of a surface fragment, premultiplied.
fn surface_color(in: SurfaceOut, emphasis: f32) -> vec4<f32> {
    let fill = surface_fill_color(in.quad, clamp(in.frac, vec2<f32>(0.0), vec2<f32>(1.0)));
    var rgb = select(vec3<f32>(0.0), fill.rgb / max(fill.a, 1e-6), fill.a > 0.0);
    var alpha = fill.a;
    if (surface_flag(SURFACE_LIT)) {
        var n = normalize(in.normal);
        // The side facing the eye is lit: a surface seen from below shows
        // its underside's shading.
        if (dot(n, view3d.eye.xyz) < 0.0) {
            n = -n;
        }
        let p = box_space_point(in.t);
        let tangent = normalize(vec3<f32>(1.0, 0.0, 0.0) - n * n.x + vec3<f32>(0.0, 1e-6, 0.0));
        rgb = material_surface(surface.material, rgb, p, n, view3d.eye.xyz,
            view3d.light.xyz, tangent, vec2<f32>(p.y, p.x), view3d.flags.z);
    }
    rgb = min(rgb + vec3<f32>(emphasis), vec3<f32>(1.0));
    var color = vec4<f32>(rgb * alpha, alpha);
    if (surface_flag(SURFACE_WIREFRAME)) {
        // Distance in pixels to the nearest edge of the quad: constant
        // width on screen at every scale.
        let fw = max(fwidth(in.frac), vec2<f32>(1e-6));
        let d = min(
            min(in.frac.x, 1.0 - in.frac.x) / fw.x,
            min(in.frac.y, 1.0 - in.frac.y) / fw.y,
        );
        let coverage = clamp(surface.wire_width_px * 0.5 + 0.5 - d, 0.0, 1.0);
        let wire = surface.wire_color * coverage;
        color = wire + color * (1.0 - wire.a);
    }
    return color;
}

@fragment
fn fs_main(in: SurfaceOut) -> @location(0) vec4<f32> {
    if (!box_visible(in.t)) {
        discard;
    }
    return surface_color(in, 0.0);
}

/// The ID pick: the series, the kind and the grid sample nearest the
/// fragment inside its quad, the sample whose patch the pixel shows.
@fragment
fn fs_pick(in: SurfaceOut) -> @location(0) vec4<u32> {
    if (!box_visible(in.t)) {
        discard;
    }
    let near = in.quad + vec2<u32>(round(clamp(in.frac, vec2<f32>(0.0), vec2<f32>(1.0))));
    let cr = surface_cell(near);
    return vec4<u32>(surface.pick_id, 0u, cr.x, cr.y);
}

/// The picked sample's patch: the halves of the up to four quads around it
/// nearest the sample, instance `k` the quad at offset `(k % 2, k / 2)` left
/// and below of it.
@vertex
fn vs_selected(@builtin(instance_index) k: u32, @builtin(vertex_index) vertex: u32) -> SurfaceOut {
    let picked = vec2<u32>(surface.picked_c, surface.picked_r);
    if (picked.x == 0xffffffffu) {
        return surface_hidden();
    }
    // The picked address in lattice (i, j) order.
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let ij = vec2<u32>(select(picked.x, picked.y, columns_are_y), select(picked.y, picked.x, columns_are_y));
    let offset = vec2<u32>(k % 2u, k / 2u);
    if (ij.x + offset.x == 0u || ij.y + offset.y == 0u) {
        return surface_hidden();
    }
    let quad = ij + offset - vec2<u32>(1u);
    let q = surface_quads();
    if (quad.x >= q.x || quad.y >= q.y) {
        return surface_hidden();
    }
    let v = surface_vertex(quad.x + quad.y * q.x, vertex);
    if (!v.valid) {
        return surface_hidden();
    }
    return surface_out(v, project_box_live(v.t));
}

@fragment
fn fs_selected(in: SurfaceOut) -> @location(0) vec4<f32> {
    let picked = vec2<u32>(surface.picked_c, surface.picked_r);
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let ij = vec2<u32>(select(picked.x, picked.y, columns_are_y), select(picked.y, picked.x, columns_are_y));
    // Distance from the picked corner in quad fractions: the patch is the
    // half of each quad nearest the sample.
    let corner = vec2<f32>(ij) - vec2<f32>(in.quad);
    let d = abs(in.frac - corner);
    if (!box_visible(in.t) || d.x > 0.5 || d.y > 0.5) {
        discard;
    }
    // An outline 1.5 px wide at the patch's border, the brightened surface
    // inside it.
    let fw = max(fwidth(in.frac), vec2<f32>(1e-6));
    let edge = min((0.5 - d.x) / fw.x, (0.5 - d.y) / fw.y);
    if (edge < 1.5) {
        return style.color_premul;
    }
    return surface_color(in, surface.material.finish.w);
}
