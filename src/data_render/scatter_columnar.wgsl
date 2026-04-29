// Columnar SDF scatter shader.
//
// slot 0: unit quad (per-vertex, ±1 NDC).
// slot 1: X column (per-instance, f32).
// slot 2: Y column (per-instance, f32).

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
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: f32,
    @location(2) y: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
};

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let xv = maybe_log(in.x, transform.scale_log.x);
    let yv = maybe_log(in.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    let center_ndc = t * 2.0 - 1.0;
    let world = center_ndc + in.quad_pos * transform.point_size_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.local_pos = in.quad_pos;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.local_pos);
    let aa = fwidth(d);
    let alpha = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, d);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}
