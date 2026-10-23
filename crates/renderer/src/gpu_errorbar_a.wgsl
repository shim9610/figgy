// Drawable-series fit-bound reduction over one ColumnPool buffer.
//
// The initial pass interprets `input_words` as the pool's packed `(hi, lo)`
// f32 lanes.  Later passes bind a SeriesState scratch buffer to the same raw
// u32 interface.  Keeping both entry points on three bindings lets them use
// separate explicit pipeline layouts without inactive bind groups.

const WORKGROUP_SIZE: u32 = 64u;
const BOUND_WORDS: u32 = 2u;
const AXIS_STATE_WORDS: u32 = 6u;
const SERIES_STATE_WORDS: u32 = 12u;

const MODE_LINE: u32 = 0u;
const MODE_POINTS: u32 = 1u;
const MODE_POINTS_X: u32 = 2u;
const MODE_POINTS_Y: u32 = 3u;
const MODE_POINTS_XY: u32 = 4u;
const MODE_LEGACY_AXIS: u32 = 5u;
const MODE_FIELD_EDGES_CELLS: u32 = 6u;
const MODE_FIELD_EDGES_SAMPLES: u32 = 7u;
const MODE_FIELD_CENTERS_CELLS: u32 = 8u;
const MODE_FIELD_CENTERS_SAMPLES: u32 = 9u;
// Finite paired anchors plus the tips `anchor + factor * (u, v)` of a
// Data-scaled point vector series: u / v ride in the x-upper / y-upper slots.
const MODE_VECTOR_TIPS: u32 = 10u;

const F32_SIGN_MASK: u32 = 0x80000000u;
const F32_ABS_MASK: u32 = 0x7fffffffu;
const F32_EXP_MASK: u32 = 0x7f800000u;
const F32_MAX_BITS: u32 = 0x7f7fffffu;

struct AxisState {
    // These are the only values the CPU-side axis SSoT consumes.
    minimum: vec2<f32>,
    maximum: vec2<f32>,
    minimum_positive: vec2<f32>,
};

struct SeriesState {
    x: AxisState,
    y: AxisState,
};

struct FieldBound {
    pair: vec2<f32>,
    valid: bool,
};

// Four vec4s keep the Rust/WGSL uniform ABI explicit and 16-byte aligned.
struct Params {
    // x, y, x-lower, x-upper offsets, in vec2<f32> pool elements.
    offsets_0: vec4<u32>,
    // y-lower, y-upper, mode, reserved.
    offsets_1: vec4<u32>,
    // x, y, x-lower, x-upper lengths.
    lengths_0: vec4<u32>,
    // y-lower, y-upper, input length, dispatch groups.
    lengths_1: vec4<u32>,
};

@group(0) @binding(0) var<storage, read> input_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> output_words: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

var<workgroup> shared_states: array<SeriesState, 64>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        shared_states[local_index] = SeriesState();
        workgroupBarrier();
    }
}

fn mode() -> u32 {
    return params.offsets_1.z;
}

fn x_offset() -> u32 { return params.offsets_0.x; }
fn y_offset() -> u32 { return params.offsets_0.y; }
fn x_lower_offset() -> u32 { return params.offsets_0.z; }
fn x_upper_offset() -> u32 { return params.offsets_0.w; }
fn y_lower_offset() -> u32 { return params.offsets_1.x; }
fn y_upper_offset() -> u32 { return params.offsets_1.y; }

fn x_len() -> u32 { return params.lengths_0.x; }
fn y_len() -> u32 { return params.lengths_0.y; }
fn x_lower_len() -> u32 { return params.lengths_0.z; }
fn x_upper_len() -> u32 { return params.lengths_0.w; }
fn y_lower_len() -> u32 { return params.lengths_1.x; }
fn y_upper_len() -> u32 { return params.lengths_1.y; }
fn input_len() -> u32 { return params.lengths_1.z; }
fn tip_factor() -> f32 { return bitcast<f32>(params.offsets_1.w); }
fn dispatch_groups() -> u32 { return params.lengths_1.w; }

fn maximum_finite_pair() -> vec2<f32> {
    let maximum = bitcast<f32>(F32_MAX_BITS);
    return vec2<f32>(maximum, maximum);
}

fn empty_minimum() -> vec2<f32> {
    return maximum_finite_pair();
}

fn empty_maximum() -> vec2<f32> {
    return -maximum_finite_pair();
}

fn empty_axis_state() -> AxisState {
    return AxisState(empty_minimum(), empty_maximum(), empty_minimum());
}

fn empty_series_state() -> SeriesState {
    return SeriesState(empty_axis_state(), empty_axis_state());
}

fn lane_is_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & F32_EXP_MASK) != F32_EXP_MASK;
}

fn pair_is_finite(v: vec2<f32>) -> bool {
    return lane_is_finite(v.x) && lane_is_finite(v.y);
}

fn load_pair(offset: u32, index: u32) -> vec2<f32> {
    let word = (offset + index) * 2u;
    return vec2<f32>(
        bitcast<f32>(input_words[word]),
        bitcast<f32>(input_words[word + 1u]),
    );
}

fn mode_is_field() -> bool {
    return mode() >= MODE_FIELD_EDGES_CELLS
        && mode() <= MODE_FIELD_CENTERS_SAMPLES;
}

fn field_uses_centers() -> bool {
    return mode() == MODE_FIELD_CENTERS_CELLS
        || mode() == MODE_FIELD_CENTERS_SAMPLES;
}

fn field_uses_samples() -> bool {
    return mode() == MODE_FIELD_EDGES_SAMPLES
        || mode() == MODE_FIELD_CENTERS_SAMPLES;
}
