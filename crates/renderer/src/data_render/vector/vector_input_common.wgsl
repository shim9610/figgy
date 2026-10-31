// Input layer, shared half: the vector series' group-2 bindings and the colour
// ramp.
//
// No vertex buffers: the column pool is bound whole as storage, the way the
// field shader binds it, and the per-form input fragment
// (`vector_input_points.wgsl` / `vector_input_grid.wgsl` /
// `vector_input_path.wgsl`) turns an instance
// index into one `VectorSample` by reading it. Nothing per arrow is stored
// anywhere else.

/// One constituent grid column: where it starts in the pool (as an f32 lane
/// index) and how many logical values it holds. Same layout as the field's
/// table (`data_render/field.rs::GridColumnGpu`).
struct GridColumn {
    base: u32,
    len: u32,
};

/// Everything the vector shader needs beyond `Transform` / `Style`.
///
/// Points form: `*_base` are f32 lane indices into the pool and `*_len` logical
/// lengths; `count` is the shortest of them. Grid form: `u_base` / `v_base` /
/// `c_base` index the `vector_grid` table (one entry per constituent column),
/// `cols` / `rows` are the effective extent in constituent-column order, and
/// `count = cols * rows`. `z_min` / `z_max` are the colourbar's bounds as
/// `(hi, lo)` pairs, already log-transformed under `VECTOR_FLAG_LOG_Z`.
///
/// CPU twin: `vector.rs::VectorParamsGpu` (`#[repr(C)]`, 128 B).
struct VectorParams {
    x_base: u32,
    y_base: u32,
    x_len: u32,
    y_len: u32,
    u_base: u32,
    u_len: u32,
    v_base: u32,
    v_len: u32,
    c_base: u32,
    c_len: u32,
    cols: u32,
    rows: u32,
    flags: u32,
    count: u32,
    stop_count: u32,
    pivot: u32,
    /// `factor` or `px_per_unit`, by `VECTOR_FLAG_SCALE_PIXELS`.
    scale: f32,
    width_px: f32,
    head_length_px: f32,
    head_width_px: f32,
    min_length_px: f32,
    /// Anti-aliasing width in pixels (one logical pixel times the render scale).
    aa_px: f32,
    /// Path form: the arc between two arrowheads, render pixels.
    spacing_px: f32,
    pad1: f32,
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    nan_color_premul: vec4<f32>,
};

const VECTOR_FLAG_GRID: u32 = 1u;
const VECTOR_FLAG_COLUMNS_ARE_Y: u32 = 2u;
const VECTOR_FLAG_CENTERS: u32 = 4u;
const VECTOR_FLAG_SCALE_PIXELS: u32 = 8u;
const VECTOR_FLAG_LOG_Z: u32 = 16u;
const VECTOR_FLAG_COLOR_MAP: u32 = 32u;

@group(2) @binding(0) var<storage, read> vector_pool: array<f32>;
@group(2) @binding(1) var<storage, read> vector_grid: array<GridColumn>;
@group(2) @binding(2) var<storage, read> vector_stops: array<vec4<f32>>;
@group(2) @binding(3) var<uniform> vector_params: VectorParams;

fn vector_flag(bit: u32) -> bool {
    return (vector_params.flags & bit) != 0u;
}

fn vector_pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(vector_pool[j], vector_pool[j + 1u]);
}

fn vector_pair_finite(v: vec2<f32>) -> bool {
    return sample_finite(v.x) && sample_finite(v.y) && sample_finite(v.x + v.y);
}

/// The scale mode constant for `project_arrow`.
fn vector_scale_mode() -> u32 {
    return select(VECTOR_SCALE_DATA, VECTOR_SCALE_PIXELS, vector_flag(VECTOR_FLAG_SCALE_PIXELS));
}

/// The colour-map value as a ramp position, or a negative number when it
/// cannot be placed.
fn vector_ramp_t(zv: vec2<f32>) -> f32 {
    return ramp_t(zv, vector_params.z_min, vector_params.z_max, vector_flag(VECTOR_FLAG_LOG_Z));
}

/// The colourbar's ramp at `t`, through the shared `ramp_segment` /
/// `ramp_blend`; only the stop lookup is this fragment's.
fn vector_ramp(t: f32) -> vec4<f32> {
    let n = vector_params.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return vector_stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(vector_stops[index], vector_stops[index + 1u], segment.y);
}

/// Premultiplied arrow colour: the series colour, or the colour-map value
/// through the colourbar's ramp (`nan_color` when it cannot be placed).
fn vector_color(sample: VectorSample) -> vec4<f32> {
    if (!vector_flag(VECTOR_FLAG_COLOR_MAP)) {
        return style.color_premul;
    }
    let t = vector_ramp_t(sample.value);
    if (!sample.value_valid || t < 0.0) {
        return vector_params.nan_color_premul;
    }
    let c = vector_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}
