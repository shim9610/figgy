// Columnar errorbar shader.
//
// Six columns (x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi) bound as
// per-instance slots. Each instance produces 12 vertices: Y stem (2) + Y caps
// (4) + X stem (2) + X caps (4).
//
// To draw only one direction, fill the unused err columns with zeros (or
// share a single zero-filled column).
//
// `scale_log.{x,y}` = 1.0 applies log10 to both the input coords and the
// computed err endpoints.

struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    point_size_ndc: vec2<f32>, // cap half-widths (rx, ry)
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
    @builtin(vertex_index) vi: u32,
    @location(0) x: f32,
    @location(1) y: f32,
    @location(2) err_y_lo: f32,
    @location(3) err_y_hi: f32,
    @location(4) err_x_lo: f32,
    @location(5) err_x_hi: f32,
};

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn data_to_ndc(v: vec2<f32>) -> vec2<f32> {
    let xv = maybe_log(v.x, transform.scale_log.x);
    let yv = maybe_log(v.y, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv, yv) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    let y_lo = in.y - in.err_y_lo;
    let y_hi = in.y + in.err_y_hi;
    let x_lo = in.x - in.err_x_lo;
    let x_hi = in.x + in.err_x_hi;

    var data_pos: vec2<f32>;
    var ndc_offset = vec2<f32>(0.0, 0.0);
    let cap_x = transform.point_size_ndc.x;
    let cap_y = transform.point_size_ndc.y;

    if (in.vi == 0u) {
        data_pos = vec2<f32>(in.x, y_lo);
    } else if (in.vi == 1u) {
        data_pos = vec2<f32>(in.x, y_hi);
    } else if (in.vi == 2u) {
        data_pos = vec2<f32>(in.x, y_lo);
        ndc_offset.x = -cap_x;
    } else if (in.vi == 3u) {
        data_pos = vec2<f32>(in.x, y_lo);
        ndc_offset.x =  cap_x;
    } else if (in.vi == 4u) {
        data_pos = vec2<f32>(in.x, y_hi);
        ndc_offset.x = -cap_x;
    } else if (in.vi == 5u) {
        data_pos = vec2<f32>(in.x, y_hi);
        ndc_offset.x =  cap_x;
    } else if (in.vi == 6u) {
        data_pos = vec2<f32>(x_lo, in.y);
    } else if (in.vi == 7u) {
        data_pos = vec2<f32>(x_hi, in.y);
    } else if (in.vi == 8u) {
        data_pos = vec2<f32>(x_lo, in.y);
        ndc_offset.y =  cap_y;
    } else if (in.vi == 9u) {
        data_pos = vec2<f32>(x_lo, in.y);
        ndc_offset.y = -cap_y;
    } else if (in.vi == 10u) {
        data_pos = vec2<f32>(x_hi, in.y);
        ndc_offset.y =  cap_y;
    } else {
        data_pos = vec2<f32>(x_hi, in.y);
        ndc_offset.y = -cap_y;
    }

    let ndc = data_to_ndc(data_pos) + ndc_offset;
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}
