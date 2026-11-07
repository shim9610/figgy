// ── Surface entries ─────────────────────────────────────────────────────────
//
// `vs_main` / `fs_main` draw the surface into the panel composite with the
// depth test; `vs_selected` / `fs_selected` draw the picked patch in the
// frame's live highlight slot, over anything in front of it (no depth);
// `fs_pick` writes the ID of what a pixel shows for the 1×1 ID pick.
//
// The geometry comes from the input fragment spliced before this one: the
// heatmap grid (`surface_input_grid.wgsl`) or the triangle mesh
// (`surface_input_mesh.wgsl`, #210). Each input defines `SurfaceVertex` and
// `surface_vertex` (one instance's vertex), `surface_fill_color` (the
// premultiplied fill at a position inside the primitive) and the hooks
// `surface_edge_px` (the wireframe's distance to the nearest edge),
// `surface_pick_ids` (what the ID pick writes), `surface_selected_vertex`
// (the picked patch's instances) and `surface_patch` (the patch's fragment
// test and outline distance). The entries themselves know no lattice.

struct SurfaceOut {
    @builtin(position) pos: vec4<f32>,
    /// Box point, perspective-correct: the box clip and the material.
    @location(0) t: vec3<f32>,
    /// Position inside the primitive: `(x, y, 0)` in 0..1 of a grid quad,
    /// or the barycentric weights of a mesh triangle.
    @location(1) frac: vec3<f32>,
    /// The primitive: a grid quad's `(i, j)`, or a mesh triangle's
    /// `(index, 0)`.
    @location(2) @interpolate(flat) quad: vec2<u32>,
    @location(3) normal: vec3<f32>,
};

fn surface_hidden() -> SurfaceOut {
    var out: SurfaceOut;
    out.pos = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.t = vec3<f32>(0.0);
    out.frac = vec3<f32>(0.0);
    out.quad = vec2<u32>(0u);
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    return out;
}

fn surface_out(v: SurfaceVertex, pos: vec4<f32>) -> SurfaceOut {
    var out: SurfaceOut;
    out.pos = pos;
    out.t = v.t;
    out.frac = v.frac;
    out.quad = v.quad;
    out.normal = v.normal;
    return out;
}

@vertex
fn vs_main(@builtin(instance_index) instance: u32, @builtin(vertex_index) vertex: u32) -> SurfaceOut {
    let v = surface_vertex(instance, vertex);
    if (!v.valid) {
        return surface_hidden();
    }
    return surface_out(v, project_box(v.t));
}

/// The lit, wireframed colour of a surface fragment, premultiplied.
fn surface_color(in: SurfaceOut, emphasis: f32) -> vec4<f32> {
    let fill = surface_fill_color(in.quad, in.frac);
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
        // Distance in pixels to the nearest edge of the primitive: constant
        // width on screen at every scale.
        let d = surface_edge_px(in);
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

/// The ID pick: what the input names for the pixel's fragment (the series,
/// the kind and the sample or triangle the pixel shows).
@fragment
fn fs_pick(in: SurfaceOut) -> @location(0) vec4<u32> {
    if (!box_visible(in.t)) {
        discard;
    }
    return surface_pick_ids(in);
}

/// The picked patch's instances: the input's `surface_selected_vertex`
/// places instance `k`'s vertex, or hides it.
@vertex
fn vs_selected(@builtin(instance_index) k: u32, @builtin(vertex_index) vertex: u32) -> SurfaceOut {
    let v = surface_selected_vertex(k, vertex);
    if (!v.valid) {
        return surface_hidden();
    }
    return surface_out(v, project_box_live(v.t));
}

@fragment
fn fs_selected(in: SurfaceOut) -> @location(0) vec4<f32> {
    // The patch test first, in uniform control flow: it and the surface
    // colour's wireframe call `fwidth`, which WGSL allows only there, so
    // neither follows a return that depends on the fragment.
    let part = surface_patch(in);
    if (!box_visible(in.t) || !part.inside) {
        discard;
    }
    // An outline 1.5 px wide at the patch's border, the brightened surface
    // inside it.
    let inside = surface_color(in, surface.material.finish.w);
    return select(inside, style.color_premul, part.edge_px < 1.5);
}
