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

// Byte-for-byte twins of the field grid-pair arithmetic.
fn grid_rounded_add(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a + b));
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a - b));
}

fn normalize_grid_pair(v: vec2<f32>) -> vec2<f32> {
    let sum = grid_rounded_add(v.x, v.y);
    let b_virtual = grid_rounded_subtract(sum, v.x);
    let a_virtual = grid_rounded_subtract(sum, b_virtual);
    let b_roundoff = grid_rounded_subtract(v.y, b_virtual);
    let a_roundoff = grid_rounded_subtract(v.x, a_virtual);
    return vec2<f32>(sum, grid_rounded_add(a_roundoff, b_roundoff));
}

fn add_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high = normalize_grid_pair(vec2<f32>(a.x, b.x));
    let low = grid_rounded_add(grid_rounded_add(a.y, b.y), high.y);
    return normalize_grid_pair(vec2<f32>(high.x, low));
}

fn subtract_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return add_grid_pairs(a, -b);
}

fn scale_grid_pair(v: vec2<f32>, factor: f32) -> vec2<f32> {
    return normalize_grid_pair(vec2<f32>(v.x * factor, v.y * factor));
}

fn midpoint_grid_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    // Scale first so two same-sign maximum f32 coordinates can still have a
    // finite midpoint.
    return add_grid_pairs(scale_grid_pair(a, 0.5), scale_grid_pair(b, 0.5));
}

fn field_bound(pair: vec2<f32>) -> FieldBound {
    var out: FieldBound;
    out.pair = pair;
    out.valid = pair_is_finite(pair);
    return out;
}

fn invalid_field_bound() -> FieldBound {
    var out: FieldBound;
    out.pair = vec2<f32>(0.0, 0.0);
    out.valid = false;
    return out;
}

/// One outer bound of the exact lattice used by the field fragment entry.
/// `cells` is already resolved against matrix-column truncation by the renderer;
/// the coordinate arithmetic itself stays here beside the source pairs.
fn field_axis_bound(offset: u32, len: u32, cells: u32, high: bool) -> FieldBound {
    if (cells == 0u) {
        return invalid_field_bound();
    }
    if (field_uses_centers()) {
        if (cells > len) {
            return invalid_field_bound();
        }
        if (field_uses_samples()) {
            return field_bound(load_pair(offset, select(0u, cells - 1u, high)));
        }
        if (!high) {
            let c0 = load_pair(offset, 0u);
            let c1 = load_pair(offset, min(1u, cells - 1u));
            return field_bound(add_grid_pairs(
                c0,
                scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5),
            ));
        }
        let last = load_pair(offset, cells - 1u);
        let prev = load_pair(offset, max(cells, 2u) - 2u);
        return field_bound(add_grid_pairs(
            last,
            scale_grid_pair(subtract_grid_pairs(last, prev), 0.5),
        ));
    }
    if (cells >= len) {
        return invalid_field_bound();
    }
    if (!field_uses_samples()) {
        return field_bound(load_pair(offset, select(0u, cells, high)));
    }
    let i = select(0u, cells - 1u, high);
    return field_bound(midpoint_grid_pair(
        load_pair(offset, i),
        load_pair(offset, i + 1u),
    ));
}

fn pair_value(v: vec2<f32>) -> f32 {
    return v.x + v.y;
}

// Error-free addition for finite, non-overflowing f32 inputs. The returned
// pair carries the rounded sum in x and its residual in y.
fn rounded_add(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a + b));
}

fn rounded_subtract(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a - b));
}

fn two_sum(a: f32, b: f32) -> vec2<f32> {
    // The bitcasts make each f32 rounding boundary observable and prevent a
    // backend from reassociating the compensation terms back into `a + b`.
    let sum = rounded_add(a, b);
    let b_virtual = rounded_subtract(sum, a);
    let a_virtual = rounded_subtract(sum, b_virtual);
    let b_roundoff = rounded_subtract(b, b_virtual);
    let a_roundoff = rounded_subtract(a, a_virtual);
    return vec2<f32>(sum, rounded_add(a_roundoff, b_roundoff));
}

// Keep a fit bound in the same compact `(hi, lo)` form the axis SSoT already
// consumes. The overflow fallback deliberately keeps the two dominant finite
// lanes instead of manufacturing infinity; the CPU can still reconstruct a
// finite f64 bound such as `f32::MAX + f32::MAX`.
fn add_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high_sum = rounded_add(a.x, b.x);
    if (!lane_is_finite(high_sum)) {
        if (abs(a.x) >= abs(b.x)) {
            return vec2<f32>(a.x, b.x);
        }
        return vec2<f32>(b.x, a.x);
    }
    // When one high lane is below the other's f32 ULP, the rounded high sum
    // is unchanged. Put that entire smaller pair into the residual directly;
    // relying on a backend to preserve a symbolic TwoSum cancellation here is
    // both slower and less portable.
    if (bitcast<u32>(high_sum) == bitcast<u32>(a.x)
        && bitcast<u32>(a.x) != bitcast<u32>(b.x)) {
        return vec2<f32>(a.x, rounded_add(a.y, rounded_add(b.x, b.y)));
    }
    if (bitcast<u32>(high_sum) == bitcast<u32>(b.x)
        && bitcast<u32>(a.x) != bitcast<u32>(b.x)) {
        return vec2<f32>(b.x, rounded_add(b.y, rounded_add(a.x, a.y)));
    }
    let high = two_sum(a.x, b.x);
    let low = rounded_add(rounded_add(a.y, b.y), high.y);
    let normalized = two_sum(high.x, low);
    if (pair_is_finite(normalized)) {
        return normalized;
    }
    return vec2<f32>(high.x, low);
}

fn subtract_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return add_pairs(a, -b);
}

fn pair_is_positive(pair: vec2<f32>) -> bool {
    let high = bitcast<u32>(pair.x);
    if ((high & F32_ABS_MASK) != 0u) {
        return (high & F32_SIGN_MASK) == 0u;
    }
    let low = bitcast<u32>(pair.y);
    return (low & F32_ABS_MASK) != 0u && (low & F32_SIGN_MASK) == 0u;
}

fn float_order_key(value: f32) -> u32 {
    let bits = bitcast<u32>(value);
    if ((bits & F32_ABS_MASK) == 0u) {
        // Treat positive and negative zero as one value.
        return F32_SIGN_MASK;
    }
    if ((bits & F32_SIGN_MASK) != 0u) {
        return ~bits;
    }
    return bits | F32_SIGN_MASK;
}

// Uploaded pairs and pairs produced by `add_pairs` are normalized in the
// ordinary finite case, so lexicographic hi/lo ordering preserves timestamp
// residuals without emulating arbitrary-precision integer arithmetic.
fn pair_is_less(a: vec2<f32>, b: vec2<f32>) -> bool {
    let a_high = float_order_key(a.x);
    let b_high = float_order_key(b.x);
    if (a_high != b_high) {
        return a_high < b_high;
    }
    return float_order_key(a.y) < float_order_key(b.y);
}

fn choose_minimum(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    if (!pair_is_finite(a)) {
        return b;
    }
    if (!pair_is_finite(b)) {
        return a;
    }
    if (pair_is_less(b, a)) {
        return b;
    }
    return a;
}

fn choose_maximum(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    if (!pair_is_finite(a)) {
        return b;
    }
    if (!pair_is_finite(b)) {
        return a;
    }
    if (pair_is_less(a, b)) {
        return b;
    }
    return a;
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

fn state_for_bound(bound: vec2<f32>) -> AxisState {
    var minimum_positive = empty_minimum();
    if (pair_is_positive(bound)) {
        minimum_positive = bound;
    }
    return AxisState(bound, bound, minimum_positive);
}

fn state_for_value(value: vec2<f32>) -> AxisState {
    return state_for_bound(value);
}

fn state_for_errors(
    value: vec2<f32>,
    lower_error: vec2<f32>,
    upper_error: vec2<f32>,
) -> AxisState {
    let lower = subtract_pairs(value, lower_error);
    let upper = add_pairs(value, upper_error);
    return merge_axis_states(
        state_for_bound(lower),
        state_for_bound(upper),
    );
}

fn state_for_active_errors(
    value: vec2<f32>,
    lower_error: vec2<f32>,
    upper_error: vec2<f32>,
) -> AxisState {
    var state = empty_axis_state();
    if (pair_is_finite(lower_error)) {
        state = merge_axis_states(
            state,
            state_for_bound(subtract_pairs(value, lower_error)),
        );
    }
    if (pair_is_finite(upper_error)) {
        state = merge_axis_states(
            state,
            state_for_bound(add_pairs(value, upper_error)),
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
        state.x = state_for_value(load_pair(x_offset(), index));
        state.y = state_for_value(load_pair(y_offset(), index));
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
                ),
            );
        }
    }
    return state;
}

fn state_for_field() -> SeriesState {
    let x_lo = field_axis_bound(x_offset(), x_len(), params.lengths_1.x, false);
    let x_hi = field_axis_bound(x_offset(), x_len(), params.lengths_1.x, true);
    let y_lo = field_axis_bound(y_offset(), y_len(), params.lengths_1.y, false);
    let y_hi = field_axis_bound(y_offset(), y_len(), params.lengths_1.y, true);
    if (!x_lo.valid || !x_hi.valid || !y_lo.valid || !y_hi.valid) {
        return empty_series_state();
    }
    return SeriesState(
        merge_axis_states(state_for_bound(x_lo.pair), state_for_bound(x_hi.pair)),
        merge_axis_states(state_for_bound(y_lo.pair), state_for_bound(y_hi.pair)),
    );
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
        state_for_errors(value, lower, upper),
        empty_axis_state(),
    );
}

fn load_bound(base: u32) -> vec2<f32> {
    return vec2<f32>(
        bitcast<f32>(input_words[base]),
        bitcast<f32>(input_words[base + 1u]),
    );
}

fn load_axis_state(base: u32) -> AxisState {
    return AxisState(
        load_bound(base),
        load_bound(base + BOUND_WORDS),
        load_bound(base + BOUND_WORDS * 2u),
    );
}

fn load_series_state(index: u32) -> SeriesState {
    let base = index * SERIES_STATE_WORDS;
    return SeriesState(
        load_axis_state(base),
        load_axis_state(base + AXIS_STATE_WORDS),
    );
}

fn store_bound(base: u32, bound: vec2<f32>) {
    output_words[base] = bitcast<u32>(bound.x);
    output_words[base + 1u] = bitcast<u32>(bound.y);
}

fn store_axis_state(base: u32, state: AxisState) {
    store_bound(base, state.minimum);
    store_bound(base + BOUND_WORDS, state.maximum);
    store_bound(base + BOUND_WORDS * 2u, state.minimum_positive);
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
        if (mode_is_field()) {
            next = state_for_field();
        } else if (mode() == MODE_LEGACY_AXIS) {
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
