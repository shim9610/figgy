// ── 3D projection (group 3, binding 4) ──────────────────────────────────────
//
// A 3D chart (`Config.view_3d`) places a point by its three axis fractions
// `t` (x on `bottom_x`, y on `left_y`, z on `right_y`), each `0..=1` inside
// the box, and projects them through the panel's camera. The matrix is
// `Config::view_matrix` composed on the host with the map from chart pixels
// to the clip space of the target being drawn, so every draw, the ID pick
// and the CPU decorations project through one camera. This is the
// projection layer a later 3D vector glyph reuses (rule 9): it replaces
// `vector/project_cartesian2d.wgsl` alone.
//
// CPU twin: `data_render/surface_mesh.rs::View3dGpu` (`#[repr(C)]`, 208 B).
struct View3d {
    /// Box `t` to the clip space of the panel composite target.
    view_proj: mat4x4<f32>,
    /// Box `t` to the clip space of the frame target (the live highlight).
    view_proj_live: mat4x4<f32>,
    /// `right_y`'s range as `(hi, lo)` pairs, log10 already taken on a
    /// logarithmic axis.
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    /// x: 1 for a logarithmic z axis; y: 1 for a perspective camera;
    /// z: output pixels per box half-diagonal (the material's scale).
    flags: vec4<f32>,
    /// The box's half extents (its half-diagonal is 1).
    half_extents: vec4<f32>,
    /// The unit direction from the box towards the eye, in box space.
    eye: vec4<f32>,
    /// The unit direction towards the light, in box space.
    light: vec4<f32>,
};

@group(3) @binding(4) var<uniform> view3d: View3d;

/// The axis fraction of an x (0) or y (1) value: the chart's axis range,
/// not the panel placement the flat draws add.
fn view3d_axis_t(pair: vec2<f32>, axis: u32) -> f32 {
    if (axis == 0u) {
        return axis_pair_to_t(pair, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, 1.0, 0.0);
    }
    return axis_pair_to_t(pair, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, 1.0, 0.0);
}

/// The axis fraction of a height on `right_y`.
fn view3d_z_t(pair: vec2<f32>) -> f32 {
    return axis_pair_to_t(pair, view3d.z_min.x, view3d.z_max.x, view3d.z_min.y, view3d.z_max.y, view3d.flags.x, 1.0, 0.0);
}

/// Clip position of box point `t` in the composite target, with depth.
fn project_box(t: vec3<f32>) -> vec4<f32> {
    return view3d.view_proj * vec4<f32>(t, 1.0);
}

/// Clip position of box point `t` moved `radius_px` output pixels towards
/// the eye: the depth of the front of a ball of that radius at `t`. A 3D
/// marker is drawn at the front of a ball of its radius and a stroke at
/// the front of a tube of its half width (#132), so each is in front of a
/// surface it sits on by its radius and a marker in front of its own stroke
/// by the difference; the mark's screen place is the centre's.
fn project_box_front(t: vec3<f32>, radius_px: f32) -> vec4<f32> {
    let shift = view3d.eye.xyz * (radius_px / view3d.flags.z) / (2.0 * view3d.half_extents.xyz);
    return project_box(t + shift);
}

/// The clip position `clip` of a point at the depth of `front`.
fn clip_at_depth(clip: vec4<f32>, front: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(clip.xy, front.z / front.w * clip.w, clip.w);
}

/// Clip position of box point `t` in the frame target.
fn project_box_live(t: vec3<f32>) -> vec4<f32> {
    return view3d.view_proj_live * vec4<f32>(t, 1.0);
}

/// Box space: `t` centred and scaled to the box's half extents.
fn box_space_point(t: vec3<f32>) -> vec3<f32> {
    return (t * 2.0 - vec3<f32>(1.0)) * view3d.half_extents.xyz;
}

/// Whether box point `t` is inside the box: fragments outside it (a height
/// past the z axis range) are discarded, the 3D form of the data-area clip.
fn box_visible(t: vec3<f32>) -> bool {
    let e = 1e-4;
    return all(t >= vec3<f32>(-e)) && all(t <= vec3<f32>(1.0 + e));
}

/// The box point of a sample of three columns (x, y and the height z of a
/// scatter or line series' `z_column`, #132).
fn view3d_point_t(xv: vec2<f32>, yv: vec2<f32>, zv: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(view3d_axis_t(xv, 0u), view3d_axis_t(yv, 1u), view3d_z_t(zv));
}

/// Whether a sample of three columns is drawn on a 3D chart: a valid flat
/// sample, a finite height (positive on a logarithmic z axis) and a box
/// point inside the box. A sample outside the box is not drawn and a line
/// breaks there, the 3D form of the data-area clip for a point primitive.
fn view3d_point_valid(t: Transform, xv: vec2<f32>, yv: vec2<f32>, zv: vec2<f32>) -> bool {
    let z = zv.x + zv.y;
    return sample_valid(t, xv, yv)
        && sample_finite(zv.x) && sample_finite(zv.y) && sample_finite(z)
        && (view3d.flags.x == 0.0 || z > 0.0)
        && box_visible(view3d_point_t(xv, yv, zv));
}

/// The ID the 1×1 pick writes for this series: its pick slot plus one,
/// carried in `flags.w` by every 3D pick bind group.
fn view3d_pick_id() -> u32 {
    return u32(view3d.flags.w);
}
