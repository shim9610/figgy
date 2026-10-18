// Input layer, shared half: the vector series' group-2 bindings and the colour
// ramp.
//
// No vertex buffers: the column pool is bound whole as storage, the way the
// field shader binds it, and the per-form input fragment
// (`vector_input_points.wgsl` / `vector_input_grid.wgsl`) turns an instance
// index into one `VectorSample` by reading it. Nothing per arrow is stored
// anywhere else.

/// One constituent grid column: where it starts in the pool (as an f32 lane
/// index) and how many logical values it holds. Same layout as the field's
/// table (`mod.rs::GridColumnGpu`).
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
    pad0: f32,
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

// ───── shared colour ramp (SHADER_COMMON.md §5.4; `shader_consistency` compares
// these function bodies with the field and scatter shaders) ─────

/// A finite f32 by its exponent bits. Every ramp input is checked
/// explicitly: NaN and infinity propagation through arithmetic is not
/// portable in WGSL.
fn ramp_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// `zv` (a hi/lo pair) as a ramp position in `[0, 1]`, or a negative number
/// when it cannot be placed: a non-finite half or sum, a non-positive value
/// on a logarithmic bar, or a degenerate range. `z_min` / `z_max` are the
/// colourbar bounds as hi/lo pairs, already log10 when `log_z`.
///
/// CPU twin: `ColorBarOptions::normalized_z`. "Unplaceable" and "smallest"
/// are different facts, so the miss is reported instead of clamping to an
/// end.
fn ramp_t(zv: vec2<f32>, z_min: vec2<f32>, z_max: vec2<f32>, log_z: bool) -> f32 {
    // Both halves are checked before they are added.
    if (!ramp_finite(zv.x) || !ramp_finite(zv.y)) {
        return -1.0;
    }
    let raw = zv.x + zv.y;
    if (!ramp_finite(raw)) {
        return -1.0;
    }
    if (log_z && raw <= 0.0) {
        return -1.0;
    }
    let is_log = select(0.0, 1.0, log_z);
    // Same shape as `axis_pair_to_t`: the linear form keeps the hi/lo split so
    // a large absolute z still resolves small deltas.
    let linear_num = (zv.x - z_min.x) + (zv.y - z_min.y);
    let log_num = (maybe_log(raw, 1.0) - z_min.x) - z_min.y;
    let span = (z_max.x - z_min.x) + (z_max.y - z_min.y);
    if (!ramp_finite(linear_num) || !ramp_finite(log_num)
        || !ramp_finite(span) || span <= 0.0) {
        return -1.0;
    }
    let numerator = mix(linear_num, log_num, is_log);
    let normalized = numerator / span;
    if (!ramp_finite(numerator) || !ramp_finite(normalized)) {
        return -1.0;
    }
    return clamp(normalized, 0.0, 1.0);
}

/// Where `t` falls among `n >= 2` control points: `x` is the lower stop's
/// index as a float, `y` the blend towards the next. Twin of the index and
/// fraction `model::colormap::sample` computes, including the `n - 2` index
/// clamp that lets the last segment own `t = 1`.
fn ramp_segment(t: f32, n: u32) -> vec2<f32> {
    let tc = select(clamp(t, 0.0, 1.0), 0.0, !ramp_finite(t));
    let scaled = tc * f32(n - 1u);
    let index = min(u32(floor(scaled)), n - 2u);
    return vec2<f32>(f32(index), scaled - f32(index));
}

/// The control points `a` (the segment's lower stop) and `b` blended by
/// `frac`, with the exact-endpoint short circuits `colormap::sample` has.
/// `mix` is not used: `a + (b - a) * t` is what the CPU computes, and the two
/// paths have to agree at the ends.
fn ramp_blend(a: vec4<f32>, b: vec4<f32>, frac: f32) -> vec4<f32> {
    if (frac <= 0.0) {
        return a;
    }
    if (frac >= 1.0) {
        return b;
    }
    return a + (b - a) * frac;
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
    if (t < 0.0) {
        return vector_params.nan_color_premul;
    }
    let c = vector_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}
