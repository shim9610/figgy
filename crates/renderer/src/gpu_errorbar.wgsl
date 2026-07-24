// Exact drawable-series extent reduction over one ColumnPool buffer.
//
// The initial pass interprets `input_words` as the pool's packed `(hi, lo)`
// f32 lanes.  Later passes bind a SeriesState scratch buffer to the same raw
// u32 interface.  Keeping both entry points on three bindings lets them use
// separate explicit pipeline layouts without inactive bind groups.

const WORKGROUP_SIZE: u32 = 64u;
const ACC_WORDS: u32 = 10u;
const ENDPOINT_WORDS: u32 = 6u;
const AXIS_STATE_WORDS: u32 = 18u;
const SERIES_STATE_WORDS: u32 = 36u;

const ENDPOINT_INVALID: u32 = 0u;
const ENDPOINT_VALUE: u32 = 1u;
const ENDPOINT_LOWER: u32 = 2u;
const ENDPOINT_UPPER: u32 = 3u;

const MODE_LINE: u32 = 0u;
const MODE_POINTS: u32 = 1u;
const MODE_POINTS_X: u32 = 2u;
const MODE_POINTS_Y: u32 = 3u;
const MODE_POINTS_XY: u32 = 4u;
const MODE_LEGACY_AXIS: u32 = 5u;

const F32_SIGN_MASK: u32 = 0x80000000u;
const F32_ABS_MASK: u32 = 0x7fffffffu;
const F32_EXP_MASK: u32 = 0x7f800000u;
const F32_FRAC_MASK: u32 = 0x007fffffu;
const F32_HIDDEN_BIT: u32 = 0x00800000u;

struct Endpoint {
    value: vec2<f32>,
    error: vec2<f32>,
    kind: u32,
    source_index: u32,
};

struct AxisState {
    minimum: Endpoint,
    maximum: Endpoint,
    minimum_positive: Endpoint,
};

struct SeriesState {
    x: AxisState,
    y: AxisState,
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

struct SignedAccumulator {
    // Little-endian two's-complement limbs. Bit zero represents 2^-149.
    words: array<u32, 10>,
};

@group(0) @binding(0) var<storage, read> input_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> output_words: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

var<workgroup> shared_states: array<SeriesState, 64>;

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
fn dispatch_groups() -> u32 { return params.lengths_1.w; }

fn empty_endpoint() -> Endpoint {
    return Endpoint(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        ENDPOINT_INVALID,
        0xffffffffu,
    );
}

fn empty_axis_state() -> AxisState {
    let empty = empty_endpoint();
    return AxisState(empty, empty, empty);
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

fn pair_value(v: vec2<f32>) -> f32 {
    return v.x + v.y;
}

fn shifted_magnitude_word(
    mantissa: u32,
    shift: u32,
    word_index: u32,
) -> u32 {
    let base = shift >> 5u;
    let intra = shift & 31u;
    if (word_index == base) {
        return mantissa << intra;
    }
    if (intra != 0u && word_index == base + 1u) {
        return mantissa >> (32u - intra);
    }
    return 0u;
}

fn add_shifted_magnitude(
    accumulator: SignedAccumulator,
    mantissa: u32,
    shift: u32,
    subtract: bool,
) -> SignedAccumulator {
    var out = accumulator;
    if (subtract) {
        var borrow = 0u;
        for (var word = 0u; word < ACC_WORDS; word = word + 1u) {
            let part = shifted_magnitude_word(mantissa, shift, word);
            let before = out.words[word];
            let first = before - part;
            let borrow_part = select(0u, 1u, before < part);
            let second = first - borrow;
            let borrow_carry = select(0u, 1u, first < borrow);
            out.words[word] = second;
            borrow = borrow_part | borrow_carry;
        }
    } else {
        var carry = 0u;
        for (var word = 0u; word < ACC_WORDS; word = word + 1u) {
            let part = shifted_magnitude_word(mantissa, shift, word);
            let before = out.words[word];
            let first = before + part;
            let carry_part = select(0u, 1u, first < before);
            let second = first + carry;
            let carry_carry = select(0u, 1u, second < first);
            out.words[word] = second;
            carry = carry_part | carry_carry;
        }
    }
    return out;
}

fn add_lane(
    accumulator: SignedAccumulator,
    lane: f32,
    coefficient_negative: bool,
) -> SignedAccumulator {
    let bits = bitcast<u32>(lane);
    let magnitude_bits = bits & F32_ABS_MASK;
    if (magnitude_bits == 0u || (magnitude_bits & F32_EXP_MASK) == F32_EXP_MASK) {
        return accumulator;
    }

    let exponent = magnitude_bits >> 23u;
    var mantissa = magnitude_bits & F32_FRAC_MASK;
    var shift = 0u;
    if (exponent != 0u) {
        mantissa = mantissa | F32_HIDDEN_BIT;
        shift = exponent - 1u;
    }

    let lane_negative = (bits & F32_SIGN_MASK) != 0u;
    return add_shifted_magnitude(
        accumulator,
        mantissa,
        shift,
        lane_negative != coefficient_negative,
    );
}

fn add_endpoint(
    accumulator: SignedAccumulator,
    endpoint: Endpoint,
    negate_endpoint: bool,
) -> SignedAccumulator {
    var out = accumulator;
    out = add_lane(out, endpoint.value.x, negate_endpoint);
    out = add_lane(out, endpoint.value.y, negate_endpoint);

    let error_is_negative =
        (endpoint.kind == ENDPOINT_LOWER) != negate_endpoint;
    out = add_lane(out, endpoint.error.x, error_is_negative);
    out = add_lane(out, endpoint.error.y, error_is_negative);
    return out;
}

fn accumulator_sign(accumulator: SignedAccumulator) -> i32 {
    var any = 0u;
    for (var word = 0u; word < ACC_WORDS; word = word + 1u) {
        any = any | accumulator.words[word];
    }
    if (any == 0u) {
        return 0;
    }
    if ((accumulator.words[ACC_WORDS - 1u] & F32_SIGN_MASK) != 0u) {
        return -1;
    }
    return 1;
}

fn zero_accumulator() -> SignedAccumulator {
    return SignedAccumulator(array<u32, 10>(
        0u, 0u, 0u, 0u, 0u,
        0u, 0u, 0u, 0u, 0u,
    ));
}

fn compare_endpoint_values(a: Endpoint, b: Endpoint) -> i32 {
    var accumulator = zero_accumulator();
    accumulator = add_endpoint(accumulator, a, false);
    accumulator = add_endpoint(accumulator, b, true);
    return accumulator_sign(accumulator);
}

fn endpoint_sign(endpoint: Endpoint) -> i32 {
    var accumulator = zero_accumulator();
    accumulator = add_endpoint(accumulator, endpoint, false);
    return accumulator_sign(accumulator);
}

fn endpoint_is_earlier(a: Endpoint, b: Endpoint) -> bool {
    if (a.source_index != b.source_index) {
        return a.source_index < b.source_index;
    }
    return a.kind < b.kind;
}

fn choose_minimum(a: Endpoint, b: Endpoint) -> Endpoint {
    if (a.kind == ENDPOINT_INVALID) {
        return b;
    }
    if (b.kind == ENDPOINT_INVALID) {
        return a;
    }
    let comparison = compare_endpoint_values(a, b);
    if (comparison < 0) {
        return a;
    }
    if (comparison > 0) {
        return b;
    }
    if (endpoint_is_earlier(a, b)) {
        return a;
    }
    return b;
}

fn choose_maximum(a: Endpoint, b: Endpoint) -> Endpoint {
    if (a.kind == ENDPOINT_INVALID) {
        return b;
    }
    if (b.kind == ENDPOINT_INVALID) {
        return a;
    }
    let comparison = compare_endpoint_values(a, b);
    if (comparison > 0) {
        return a;
    }
    if (comparison < 0) {
        return b;
    }
    if (endpoint_is_earlier(a, b)) {
        return a;
    }
    return b;
}

fn merge_axis_states(a: AxisState, b: AxisState) -> AxisState {
    return AxisState(
        choose_minimum(a.minimum, b.minimum),
        choose_maximum(a.maximum, b.maximum),
        choose_minimum(a.minimum_positive, b.minimum_positive),
    );
}

fn merge_series_states(a: SeriesState, b: SeriesState) -> SeriesState {
    return SeriesState(
        merge_axis_states(a.x, b.x),
        merge_axis_states(a.y, b.y),
    );
}

fn state_for_endpoint(endpoint: Endpoint) -> AxisState {
    var minimum_positive = empty_endpoint();
    if (endpoint_sign(endpoint) > 0) {
        minimum_positive = endpoint;
    }
    return AxisState(endpoint, endpoint, minimum_positive);
}

fn state_for_value(value: vec2<f32>, index: u32) -> AxisState {
    return state_for_endpoint(Endpoint(
        value,
        vec2<f32>(0.0, 0.0),
        ENDPOINT_VALUE,
        index,
    ));
}

fn state_for_errors(
    value: vec2<f32>,
    lower_error: vec2<f32>,
    upper_error: vec2<f32>,
    index: u32,
) -> AxisState {
    let lower = Endpoint(value, lower_error, ENDPOINT_LOWER, index);
    let upper = Endpoint(value, upper_error, ENDPOINT_UPPER, index);
    return merge_axis_states(
        state_for_endpoint(lower),
        state_for_endpoint(upper),
    );
}

fn state_for_active_errors(
    value: vec2<f32>,
    lower_error: vec2<f32>,
    upper_error: vec2<f32>,
    index: u32,
) -> AxisState {
    var state = empty_axis_state();
    if (pair_is_finite(lower_error)) {
        state = merge_axis_states(
            state,
            state_for_endpoint(Endpoint(
                value,
                lower_error,
                ENDPOINT_LOWER,
                index,
            )),
        );
    }
    if (pair_is_finite(upper_error)) {
        state = merge_axis_states(
            state,
            state_for_endpoint(Endpoint(
                value,
                upper_error,
                ENDPOINT_UPPER,
                index,
            )),
        );
    }
    return state;
}

fn paired_value_is_finite(index: u32) -> bool {
    if (index >= x_len() || index >= y_len()) {
        return false;
    }
    return pair_is_finite(load_pair(x_offset(), index))
        && pair_is_finite(load_pair(y_offset(), index));
}

fn point_is_in_base_domain(index: u32) -> bool {
    if (!paired_value_is_finite(index)) {
        return false;
    }
    if (mode() != MODE_LINE) {
        return true;
    }
    let has_previous = index > 0u && paired_value_is_finite(index - 1u);
    let has_next =
        index + 1u < min(x_len(), y_len())
        && paired_value_is_finite(index + 1u);
    return has_previous || has_next;
}

fn x_errors_are_active() -> bool {
    return mode() == MODE_POINTS_X || mode() == MODE_POINTS_XY;
}

fn y_errors_are_active() -> bool {
    return mode() == MODE_POINTS_Y || mode() == MODE_POINTS_XY;
}

fn error_domain_len() -> u32 {
    var count = min(x_len(), y_len());
    if (x_errors_are_active()) {
        count = min(count, min(x_lower_len(), x_upper_len()));
    }
    if (y_errors_are_active()) {
        count = min(count, min(y_lower_len(), y_upper_len()));
    }
    return count;
}

fn state_for_series_index(index: u32) -> SeriesState {
    var state = empty_series_state();
    if (point_is_in_base_domain(index)) {
        state.x = state_for_value(load_pair(x_offset(), index), index);
        state.y = state_for_value(load_pair(y_offset(), index), index);
    }

    if (index >= error_domain_len() || !paired_value_is_finite(index)) {
        return state;
    }

    if (x_errors_are_active()) {
        let lower = load_pair(x_lower_offset(), index);
        let upper = load_pair(x_upper_offset(), index);
        // Byte-for-byte arithmetic order from errorbar_columnar.wgsl:
        // (lo.hi + lo.lo) + (hi.hi + hi.lo) > 0.
        let enabled =
            (pair_value(lower) + pair_value(upper)) > 0.0;
        if (enabled) {
            state.x = merge_axis_states(
                state.x,
                state_for_active_errors(
                    load_pair(x_offset(), index),
                    lower,
                    upper,
                    index,
                ),
            );
        }
    }

    if (y_errors_are_active()) {
        let lower = load_pair(y_lower_offset(), index);
        let upper = load_pair(y_upper_offset(), index);
        let enabled =
            (pair_value(lower) + pair_value(upper)) > 0.0;
        if (enabled) {
            state.y = merge_axis_states(
                state.y,
                state_for_active_errors(
                    load_pair(y_offset(), index),
                    lower,
                    upper,
                    index,
                ),
            );
        }
    }
    return state;
}

fn state_for_legacy_index(index: u32) -> SeriesState {
    let value = load_pair(x_offset(), index);
    if (!pair_is_finite(value)) {
        return empty_series_state();
    }

    var lower = vec2<f32>(0.0, 0.0);
    if (index < x_lower_len()) {
        let candidate = load_pair(x_lower_offset(), index);
        if (pair_is_finite(candidate)) {
            lower = candidate;
        }
    }
    var upper = vec2<f32>(0.0, 0.0);
    if (index < x_upper_len()) {
        let candidate = load_pair(x_upper_offset(), index);
        if (pair_is_finite(candidate)) {
            upper = candidate;
        }
    }
    return SeriesState(
        state_for_errors(value, lower, upper, index),
        empty_axis_state(),
    );
}

fn load_endpoint(base: u32) -> Endpoint {
    return Endpoint(
        vec2<f32>(
            bitcast<f32>(input_words[base]),
            bitcast<f32>(input_words[base + 1u]),
        ),
        vec2<f32>(
            bitcast<f32>(input_words[base + 2u]),
            bitcast<f32>(input_words[base + 3u]),
        ),
        input_words[base + 4u],
        input_words[base + 5u],
    );
}

fn load_axis_state(base: u32) -> AxisState {
    return AxisState(
        load_endpoint(base),
        load_endpoint(base + ENDPOINT_WORDS),
        load_endpoint(base + ENDPOINT_WORDS * 2u),
    );
}

fn load_series_state(index: u32) -> SeriesState {
    let base = index * SERIES_STATE_WORDS;
    return SeriesState(
        load_axis_state(base),
        load_axis_state(base + AXIS_STATE_WORDS),
    );
}

fn store_endpoint(base: u32, endpoint: Endpoint) {
    output_words[base] = bitcast<u32>(endpoint.value.x);
    output_words[base + 1u] = bitcast<u32>(endpoint.value.y);
    output_words[base + 2u] = bitcast<u32>(endpoint.error.x);
    output_words[base + 3u] = bitcast<u32>(endpoint.error.y);
    output_words[base + 4u] = endpoint.kind;
    output_words[base + 5u] = endpoint.source_index;
}

fn store_axis_state(base: u32, state: AxisState) {
    store_endpoint(base, state.minimum);
    store_endpoint(base + ENDPOINT_WORDS, state.maximum);
    store_endpoint(base + ENDPOINT_WORDS * 2u, state.minimum_positive);
}

fn store_series_state(index: u32, state: SeriesState) {
    let base = index * SERIES_STATE_WORDS;
    store_axis_state(base, state.x);
    store_axis_state(base + AXIS_STATE_WORDS, state.y);
}

fn reduce_shared(local_index: u32) {
    var offset = WORKGROUP_SIZE >> 1u;
    while (offset != 0u) {
        workgroupBarrier();
        if (local_index < offset) {
            shared_states[local_index] = merge_series_states(
                shared_states[local_index],
                shared_states[local_index + offset],
            );
        }
        offset = offset >> 1u;
    }
    workgroupBarrier();
}

@compute @workgroup_size(64)
fn reduce_values(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
) {
    var local_state = empty_series_state();
    let stride = dispatch_groups() * WORKGROUP_SIZE;
    var index = global_id.x;
    while (index < input_len()) {
        var next = empty_series_state();
        if (mode() == MODE_LEGACY_AXIS) {
            next = state_for_legacy_index(index);
        } else {
            next = state_for_series_index(index);
        }
        local_state = merge_series_states(local_state, next);
        index = index + stride;
    }

    shared_states[local_index] = local_state;
    reduce_shared(local_index);
    if (local_index == 0u) {
        store_series_state(workgroup_id.x, shared_states[0]);
    }
}

@compute @workgroup_size(64)
fn reduce_states(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
) {
    var local_state = empty_series_state();
    let stride = dispatch_groups() * WORKGROUP_SIZE;
    var index = global_id.x;
    while (index < input_len()) {
        local_state = merge_series_states(local_state, load_series_state(index));
        index = index + stride;
    }

    shared_states[local_index] = local_state;
    reduce_shared(local_index);
    if (local_index == 0u) {
        store_series_state(workgroup_id.x, shared_states[0]);
    }
}
