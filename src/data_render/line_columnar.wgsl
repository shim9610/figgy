// Columnar line shader — variable-thickness via quad extrusion.
//
// One line segment = one instance, 4 vertices per instance (TriangleStrip).
// The X and Y columns are bound twice: the second binding starts at a 4-byte
// (one f32) offset, so instance `i` sees points [i] and [i+1] at once.
//
// Corner mapping per segment:
//   vid 0 : at A, −normal
//   vid 1 : at A, +normal
//   vid 2 : at B, −normal
//   vid 3 : at B, +normal
//
// The normal is computed in pixel space and converted back to NDC, so width
// stays constant across panels with different X/Y pixel ratios.
// Zero-length segments collapse the quad and become invisible.

struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    point_size_ndc: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    _pad: vec2<f32>,
};

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
};

@group(0) @binding(0) var<uniform> transform: Transform;
@group(1) @binding(0) var<uniform> style: Style;

struct VsIn {
    @location(0) x_a: f32,
    @location(1) y_a: f32,
    @location(2) x_b: f32,
    @location(3) y_b: f32,
};

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn data_to_ndc(xv: f32, yv: f32) -> vec2<f32> {
    let xv2 = maybe_log(xv, transform.scale_log.x);
    let yv2 = maybe_log(yv, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv2, yv2) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}

@vertex
fn vs_main(in: VsIn, @builtin(vertex_index) vid: u32) -> @builtin(position) vec4<f32> {
    let a_ndc = data_to_ndc(in.x_a, in.y_a);
    let b_ndc = data_to_ndc(in.x_b, in.y_b);

    // Convert to pixel space so the direction vector can be normalized.
    let a_px = a_ndc / transform.pixel_to_ndc;
    let b_px = b_ndc / transform.pixel_to_ndc;

    let delta = b_px - a_px;
    let len = length(delta);
    // Zero-length guard — normal=0 makes corner=center, collapsing the quad.
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    let half_w = max(style.line_width_px, 1.0) * 0.5;

    let at_b = vid >= 2u;
    let on_pos = (vid & 1u) == 1u;
    let center_px = select(a_px, b_px, at_b);
    let side = select(-1.0, 1.0, on_pos);

    let corner_px = center_px + normal_px * half_w * side;
    let corner_ndc = corner_px * transform.pixel_to_ndc;
    return vec4<f32>(corner_ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}
