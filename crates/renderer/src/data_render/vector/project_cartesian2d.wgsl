// ── Projection ──────────────────────────────────────────────────────────────

/// One arrow sample as the input layer produces it: the anchor as the pool's
/// `(hi, lo)` pairs, the vector, and the colour-map value. 2D inputs set z = 0.
/// `value_valid` is false where the colour column has no value for the
/// sample, which then takes `nan_color`.
struct VectorSample {
    origin_hi: vec3<f32>,
    origin_lo: vec3<f32>,
    vector: vec3<f32>,
    value: vec2<f32>,
    value_valid: bool,
    valid: bool,
};

/// An arrow in chart-area pixels, before the pivot and head are applied.
struct ArrowPx {
    tail: vec2<f32>,
    tip: vec2<f32>,
    valid: bool,
};

/// `VectorScale::Data { factor }`: the tip is `origin + factor * vector` in
/// data units.
const VECTOR_SCALE_DATA: u32 = 0u;
/// `VectorScale::Pixels { px_per_unit }`: the screen length is
/// `px_per_unit * |vector|` along the projected data-space direction.
const VECTOR_SCALE_PIXELS: u32 = 1u;

fn ndc_to_px(ndc: vec2<f32>) -> vec2<f32> {
    return ndc / transform.pixel_to_ndc;
}

fn px_to_clip(px: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(px * transform.pixel_to_ndc, 0.0, 1.0);
}

/// `axis_px_per_unit_at` for one axis of the transform: 0 = x, 1 = y.
fn axis_px_per_unit(axis: u32, pair: vec2<f32>) -> f32 {
    if (axis == 0u) {
        let range = fma(1.0, transform.data_max.x - transform.data_min.x, transform.data_max_lo.x - transform.data_min_lo.x);
        return axis_px_per_unit_at(range, transform.data_to_panel_scale.x, transform.scale_log.x, transform.pixel_to_ndc.x, pair);
    }
    let range = fma(1.0, transform.data_max.y - transform.data_min.y, transform.data_max_lo.y - transform.data_min_lo.y);
    return axis_px_per_unit_at(range, transform.data_to_panel_scale.y, transform.scale_log.y, transform.pixel_to_ndc.y, pair);
}

/// Project one sample. `mode` is a `VECTOR_SCALE_*` constant and `scale` the
/// mode's factor. An anchor that is not placeable (non-finite, or non-positive
/// on a logarithmic axis) hides the arrow; in `Data` mode so does such a tip.
fn project_arrow(sample: VectorSample, mode: u32, scale: f32) -> ArrowPx {
    var out: ArrowPx;
    out.tail = vec2<f32>(0.0, 0.0);
    out.tip = vec2<f32>(0.0, 0.0);
    out.valid = false;
    let ox = vec2<f32>(sample.origin_hi.x, sample.origin_lo.x);
    let oy = vec2<f32>(sample.origin_hi.y, sample.origin_lo.y);
    if (!sample.valid || !sample_valid(ox, oy)) {
        return out;
    }
    let u = sample.vector.x;
    let v = sample.vector.y;
    if (!sample_finite(u) || !sample_finite(v)) {
        return out;
    }
    let tail = ndc_to_px(data_to_ndc(ox, oy));
    var tip: vec2<f32>;
    if (mode == VECTOR_SCALE_PIXELS) {
        let direction = vec2<f32>(axis_px_per_unit(0u, ox) * u, axis_px_per_unit(1u, oy) * v);
        let direction_len = length(direction);
        let len_px = scale * length(vec2<f32>(u, v));
        if (!sample_finite(direction_len) || !sample_finite(len_px)) {
            return out;
        }
        if (direction_len > 0.0 && len_px != 0.0) {
            tip = tail + direction * (len_px / direction_len);
        } else {
            tip = tail;
        }
    } else {
        let tx = pair_add(ox, scale * u);
        let ty = pair_add(oy, scale * v);
        if (!sample_valid(tx, ty)) {
            return out;
        }
        tip = ndc_to_px(data_to_ndc(tx, ty));
    }
    if (!sample_finite(tail.x) || !sample_finite(tail.y) || !sample_finite(tip.x) || !sample_finite(tip.y)) {
        return out;
    }
    out.tail = tail;
    out.tip = tip;
    out.valid = true;
    return out;
}
