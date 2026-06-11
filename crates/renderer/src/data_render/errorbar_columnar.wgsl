// Columnar errorbar shader.
//
// Six columns (x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi) bound as
// per-instance slots. Each instance produces 36 vertices on a TriangleList:
// six axis-aligned quads (6 vertices each) — Y stem, cap @ y_lo, cap @ y_hi,
// X stem, cap @ x_lo, cap @ x_hi. Stem endpoints live in data space; caps
// span ±style.cap_half_px (pixels) perpendicular to their stem. Each quad is
// expanded perpendicular to its own axis by half its stroke width in pixels
// (stems: style.line_width_px, caps: style.cap_width_px); pixel offsets are
// converted to NDC via transform.pixel_to_ndc.
//
// To draw only one direction, fill the unused err columns with zeros (or
// share a single zero-filled column): a direction with err_lo + err_hi <= 0
// collapses to zero-area quads, so neither its stem nor its caps rasterize.
//
// `scale_log.{x,y}` = 1.0 applies log10 to both the input coords and the
// computed err endpoints.

// ───── BEGIN common block (SHADER_COMMON.md) ─────
// WGSL has no import. The Transform/Style/binding/maybe_log/data_to_ndc
// definitions below are duplicated across scatter/line/errorbar shaders.
// To modify any of them, FIRST edit src/data_render/SHADER_COMMON.md,
// then mirror the change into every sibling shader. Do not edit only one
// file — silent drift here causes very hard-to-debug rendering bugs.
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> transform: Transform;

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    _pad: vec2<f32>,
    dash: array<vec4<f32>, 2>,
};

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
// ───── END common block ─────

@vertex
fn vs_main(in: VsIn) -> @builtin(position) vec4<f32> {
    // segment: 0 Y-stem, 1 cap@y_lo, 2 cap@y_hi, 3 X-stem, 4 cap@x_lo,
    // 5 cap@x_hi.
    let seg = in.vi / 6u;

    // Zero-filled err columns disable a whole direction, caps included:
    // collapse its segments to the anchor so every triangle has zero area.
    let has_y = (in.err_y_lo + in.err_y_hi) > 0.0;
    let has_x = (in.err_x_lo + in.err_x_hi) > 0.0;
    let dir_enabled = select(has_x, has_y, seg < 3u);
    if (!dir_enabled) {
        let anchor = data_to_ndc(vec2<f32>(in.x, in.y));
        return vec4<f32>(anchor, 0.0, 1.0);
    }

    let y_lo = in.y - in.err_y_lo;
    let y_hi = in.y + in.err_y_hi;
    let x_lo = in.x - in.err_x_lo;
    let x_hi = in.x + in.err_x_hi;

    // Each segment is a quad between endpoints A and B: a data-space
    // position plus a pixel-space offset (caps run ±cap_half_px from a
    // single data point). `perp` is the pixel-space expansion axis; all
    // segments are axis-aligned, so no normalization is needed.
    var a_data: vec2<f32>;
    var b_data: vec2<f32>;
    var a_px = vec2<f32>(0.0, 0.0);
    var b_px = vec2<f32>(0.0, 0.0);
    var perp: vec2<f32>;
    var half_stroke: f32;

    if (seg == 0u) {
        // Y stem: vertical in data space, expand along X.
        a_data = vec2<f32>(in.x, y_lo);
        b_data = vec2<f32>(in.x, y_hi);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 1u) {
        // Cap @ y_lo: horizontal, expand along Y.
        a_data = vec2<f32>(in.x, y_lo);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 2u) {
        // Cap @ y_hi: horizontal, expand along Y.
        a_data = vec2<f32>(in.x, y_hi);
        b_data = a_data;
        a_px = vec2<f32>(-style.cap_half_px, 0.0);
        b_px = vec2<f32>( style.cap_half_px, 0.0);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.cap_width_px * 0.5;
    } else if (seg == 3u) {
        // X stem: horizontal in data space, expand along Y.
        a_data = vec2<f32>(x_lo, in.y);
        b_data = vec2<f32>(x_hi, in.y);
        perp = vec2<f32>(0.0, 1.0);
        half_stroke = style.line_width_px * 0.5;
    } else if (seg == 4u) {
        // Cap @ x_lo: vertical, expand along X.
        a_data = vec2<f32>(x_lo, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    } else {
        // Cap @ x_hi: vertical, expand along X.
        a_data = vec2<f32>(x_hi, in.y);
        b_data = a_data;
        a_px = vec2<f32>(0.0, -style.cap_half_px);
        b_px = vec2<f32>(0.0,  style.cap_half_px);
        perp = vec2<f32>(1.0, 0.0);
        half_stroke = style.cap_width_px * 0.5;
    }

    // Triangle-list corner map [0,1,2, 2,1,3] over corners
    // {0: A-, 1: A+, 2: B-, 3: B+} (-/+ = perpendicular half-stroke side).
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let at_a = corner < 2u;
    let side = select(1.0, -1.0, (corner & 1u) == 0u);

    let end_data = select(b_data, a_data, at_a);
    let end_px = select(b_px, a_px, at_a);
    let offset_px = end_px + perp * (side * half_stroke);
    let ndc = data_to_ndc(end_data) + offset_px * transform.pixel_to_ndc;
    return vec4<f32>(ndc, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return style.color_premul;
}
