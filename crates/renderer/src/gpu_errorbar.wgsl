// Exact pairwise errorbar extent reduction over ColumnPool (hi, lo) values.
//
// Endpoint comparisons never add the lanes in f32.  Every finite f32 is an
// integer multiple of 2^-149, so a 320-bit two's-complement superaccumulator
// can compare the exact real sums
//
//     value_hi + value_lo - error_lo_hi - error_lo_lo
//     value_hi + value_lo + error_hi_hi + error_hi_lo
//
// without cancellation, subnormal, or f32-overflow loss.  The winning raw
// lanes are returned; Rust reconstructs the scalar in f64.

const WORKGROUP_SIZE: u32 = 128u;
const ACC_WORDS: u32 = 10u;

const ENDPOINT_INVALID: u32 = 0u;
const ENDPOINT_LOWER: u32 = 1u;
const ENDPOINT_UPPER: u32 = 2u;

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

struct ExtentState {
    minimum: Endpoint,
    maximum: Endpoint,
    minimum_positive: Endpoint,
};

struct Params {
    input_len: u32,
    lower_len: u32,
    upper_len: u32,
    dispatch_groups: u32,
};

struct SignedAccumulator {
    // Little-endian two's-complement limbs.  Bit zero represents 2^-149.
    // A finite f32's highest magnitude bit is 276; eight signed lanes need
    // at most bit 279.  Ten limbs leave forty guard/sign-extension bits.
    words: array<u32, 10>,
};

@group(0) @binding(0) var<storage, read> values: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read> lower_errors: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> upper_errors: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> value_output: array<ExtentState>;
@group(0) @binding(4) var<uniform> value_params: Params;

@group(1) @binding(0) var<storage, read> state_input: array<ExtentState>;
@group(1) @binding(1) var<storage, read_write> state_output: array<ExtentState>;
@group(1) @binding(2) var<uniform> state_params: Params;

var<workgroup> shared_states: array<ExtentState, 128>;

fn empty_endpoint() -> Endpoint {
    return Endpoint(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        ENDPOINT_INVALID,
        0xffffffffu,
    );
}

fn empty_state() -> ExtentState {
    let empty = empty_endpoint();
    return ExtentState(empty, empty, empty);
}

fn lane_is_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & F32_EXP_MASK) != F32_EXP_MASK;
}

fn pair_is_finite(v: vec2<f32>) -> bool {
    return lane_is_finite(v.x) && lane_is_finite(v.y);
}

// Return word `word_index` of the non-negative integer
// `mantissa << shift`.  A decoded f32 mantissa has at most 24 bits, so only
// two adjacent words can be non-zero.
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

// Add `coefficient_negative ? -lane : lane` exactly to the integer
// superaccumulator.  Callers only pass finite lanes.
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
        // A normal f32 with biased exponent E is
        // mantissa * 2^(E-150), hence shift E-1 at the 2^-149 base.
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

    // Lower endpoint subtracts its error; upper endpoint adds it.  Negating
    // the whole endpoint flips that coefficient as well.
    let error_is_negative = (endpoint.kind == ENDPOINT_LOWER) != negate_endpoint;
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

// Numeric comparison only: negative iff a < b, zero iff exactly equal.
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

// The CPU oracle walks indices in ascending order and includes the lower
// endpoint before the upper endpoint.  This key makes ties independent of
// workgroup scheduling and reduction-tree shape.
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

fn merge_states(a: ExtentState, b: ExtentState) -> ExtentState {
    return ExtentState(
        choose_minimum(a.minimum, b.minimum),
        choose_maximum(a.maximum, b.maximum),
        choose_minimum(a.minimum_positive, b.minimum_positive),
    );
}

fn state_for_value(index: u32) -> ExtentState {
    let value = values[index];
    if (!pair_is_finite(value)) {
        return empty_state();
    }

    var lower_error = vec2<f32>(0.0, 0.0);
    if (index < value_params.lower_len) {
        let candidate = lower_errors[index];
        if (pair_is_finite(candidate)) {
            lower_error = candidate;
        }
    }

    var upper_error = vec2<f32>(0.0, 0.0);
    if (index < value_params.upper_len) {
        let candidate = upper_errors[index];
        if (pair_is_finite(candidate)) {
            upper_error = candidate;
        }
    }

    let lower = Endpoint(value, lower_error, ENDPOINT_LOWER, index);
    let upper = Endpoint(value, upper_error, ENDPOINT_UPPER, index);
    var minimum_positive = empty_endpoint();
    if (endpoint_sign(lower) > 0) {
        minimum_positive = lower;
    }
    if (endpoint_sign(upper) > 0) {
        minimum_positive = choose_minimum(minimum_positive, upper);
    }

    return ExtentState(
        choose_minimum(lower, upper),
        choose_maximum(lower, upper),
        minimum_positive,
    );
}

fn reduce_shared(local_index: u32) {
    var offset = WORKGROUP_SIZE >> 1u;
    while (offset != 0u) {
        workgroupBarrier();
        if (local_index < offset) {
            shared_states[local_index] = merge_states(
                shared_states[local_index],
                shared_states[local_index + offset],
            );
        }
        offset = offset >> 1u;
    }
    workgroupBarrier();
}

@compute @workgroup_size(128)
fn reduce_values(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
) {
    var local_state = empty_state();
    let stride = value_params.dispatch_groups * WORKGROUP_SIZE;
    var index = global_id.x;
    while (index < value_params.input_len) {
        local_state = merge_states(local_state, state_for_value(index));
        index = index + stride;
    }

    shared_states[local_index] = local_state;
    reduce_shared(local_index);
    if (local_index == 0u) {
        value_output[workgroup_id.x] = shared_states[0];
    }
}

@compute @workgroup_size(128)
fn reduce_states(
    @builtin(global_invocation_id) global_id: vec3<u32>,
    @builtin(local_invocation_index) local_index: u32,
    @builtin(workgroup_id) workgroup_id: vec3<u32>,
) {
    var local_state = empty_state();
    let stride = state_params.dispatch_groups * WORKGROUP_SIZE;
    var index = global_id.x;
    while (index < state_params.input_len) {
        local_state = merge_states(local_state, state_input[index]);
        index = index + stride;
    }

    shared_states[local_index] = local_state;
    reduce_shared(local_index);
    if (local_index == 0u) {
        state_output[workgroup_id.x] = shared_states[0];
    }
}
