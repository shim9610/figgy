// Projection layer: Cartesian 2D.
//
// One fragment of the assembled vector shader (`vector.rs`). It carries the
// common `Transform` / `Style` blocks, the hi/lo axis projection and the sample
// validity rule that every Cartesian data shader shares, and adds the arrow
// projection the entry calls: a `VectorSample` (hi/lo origin, vector,
// colour value) becomes an `ArrowPx` (tail and tip in chart-area pixels).
//
// Pixel space here is the chart area's, centred, y up, one unit per logical
// pixel: `px = ndc / transform.pixel_to_ndc`. The glyph layer works in this
// space and the entry converts back with `px_to_clip`.
//
// A later 3D projection fragment replaces this file alone: it has to provide
// the same `project_arrow` with the same `VectorSample` / `ArrowPx` types, and
// may ignore the hi/lo split once it has its own camera.

// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
    // Polar coordinates (Config.polar); all zero in a rectangular chart.
    // polar_angle:  (flag, zero angle in radians, radians per angle unit
    //               signed by the direction, inner radius fraction)
    // polar_extent: (sector start, sector end in angle units, end <= start
    //               for the full circle; arc pieces per segment, 0 for
    //               chords; outer radius in pixels)
    polar_angle: vec4<f32>,
    polar_extent: vec4<f32>,
};  // 144 B (vec4 array at offset 64, stride 16; polar vec4s at 112 and 128)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    // Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    // for X. Line entries read bits 2-3 as the step interpolation mode
    // (0 linear, 1 step-pre, 2 step-post, 3 step-mid) and the area entry
    // also reads bit 4 (fill to a series instead of a baseline); every
    // other primitive ignores this field.
    primitive_flags: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    if is_log > 0.5 {
        return lv;
    }
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    if is_log > 0.5 {
        return fma(log_num / range, panel_scale, panel_offset);
    }
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return fma(data_t, panel_scale, panel_offset);
}

// Polar (Config.polar): the angle column in radians around the centre of the
// data area, the radius axis fraction between the inner ring and the outer
// radius. A radius below the axis start sits on the inner ring (data samples
// there are masked by `sample_valid`; a fill baseline is not). Twin of
// `polar_projection::ndc` on the CPU.
fn polar_ndc(ty: f32, xv: vec2<f32>) -> vec2<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    let angle = p.y + p.z * (xv.x + xv.y);
    let rt = (ty - transform.data_to_panel_offset.y) / transform.data_to_panel_scale.y;
    let r = mix(p.w, 1.0, max(rt, 0.0));
    let center = (transform.data_to_panel_offset + transform.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return center + vec2<f32>(cos(angle), sin(angle)) * r * e.w * transform.pixel_to_ndc;
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    if (transform.polar_angle.x > 0.5) {
        return polar_ndc(ty, xv);
    }
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}

// Mask source samples explicitly: NaN/Inf clipping is not portable in WGSL.
// Logarithmic coordinate domains exclude nonpositive samples, while bars and
// errorbar extents retain their separate baseline/clipping rules.
fn sample_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// Polar: a radius below the axis start has no place on the plot (it would
// fold onto the inner ring), so the sample is not drawn and a line breaks
// there. Angles outside the sector and radii past the outer ring keep their
// projection and are cut by the fragment clip below.
fn polar_sample_valid(xv: vec2<f32>, yv: vec2<f32>) -> bool {
    if (transform.polar_angle.x < 0.5) {
        return true;
    }
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    let rt = (ty - transform.data_to_panel_offset.y) / transform.data_to_panel_scale.y;
    return rt >= 0.0;
}

fn sample_valid(xv: vec2<f32>, yv: vec2<f32>) -> bool {
    let x = xv.x + xv.y;
    let y = yv.x + yv.y;
    return sample_finite(xv.x) && sample_finite(xv.y)
        && sample_finite(yv.x) && sample_finite(yv.y)
        && sample_finite(x) && sample_finite(y)
        && (transform.scale_log.x == 0.0 || x > 0.0)
        && (transform.scale_log.y == 0.0 || y > 0.0)
        && polar_sample_valid(xv, yv);
}

// Polar clip. The transform is vertex-visible only, so the vertex stage hands
// the fragment stage the fragment's offset from the centre in pixels
// (interpolated from the corner's NDC) and the bounds as flat varyings; the
// fragment discards outside the outer ring, inside the hole and outside the
// sector, cutting a primitive at the edge the way the data-area scissor cuts
// a rectangular chart. A rectangular chart passes a zero flag.
fn polar_clip_offset(ndc: vec2<f32>) -> vec2<f32> {
    let center = (transform.data_to_panel_offset + transform.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return (ndc - center) / transform.pixel_to_ndc;
}

// (flag, outer radius px, inner radius px, sector flag)
fn polar_clip_limits() -> vec4<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    return vec4<f32>(p.x, e.w, e.w * p.w, select(0.0, 1.0, e.y > e.x));
}

// (zero angle rad, radians per angle unit signed by the direction, sector
// start, sector end in angle units)
fn polar_clip_sector() -> vec4<f32> {
    let p = transform.polar_angle;
    let e = transform.polar_extent;
    return vec4<f32>(p.y, p.z, e.x, e.y);
}

fn polar_fragment_visible(px: vec2<f32>, limits: vec4<f32>, sector: vec4<f32>) -> bool {
    if (limits.x < 0.5) {
        return true;
    }
    let r = length(px);
    if (r > limits.y + 0.5 || r < limits.z - 0.5) {
        return false;
    }
    if (limits.w < 0.5) {
        return true;
    }
    // The data angle of the fragment, brought into the turn that starts at
    // the sector start.
    let turn = 6.283185307179586 / abs(sector.y);
    var a = (atan2(px.y, px.x) - sector.x) / sector.y;
    a = a - floor((a - sector.z) / turn) * turn;
    return a <= sector.w + 1e-6 * turn;
}
// ───── END common block ─────

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

/// `pair + s` as a `(hi, lo)` pair: the error-free sum keeps the low part, so a
/// large-offset coordinate still resolves a small data-space step.
fn pair_add(pair: vec2<f32>, s: f32) -> vec2<f32> {
    let sum = fma(1.0, pair.x, s);
    let b_virtual = fma(1.0, sum, -pair.x);
    let a_virtual = fma(1.0, sum, -b_virtual);
    let b_roundoff = fma(1.0, s, -b_virtual);
    let a_roundoff = fma(1.0, pair.x, -a_virtual);
    return vec2<f32>(sum, fma(1.0, a_roundoff, b_roundoff) + pair.y);
}

/// Pixels per data unit along one axis whose range is `range` (the hi/lo
/// sum), at `pair`: the derivative of `axis_pair_to_t` mapped through NDC.
/// Exact on a linear axis; the local direction on a logarithmic one, the way
/// the contour labels orient themselves.
fn axis_px_per_unit_at(range: f32, panel_scale: f32, is_log: f32, pixel_to_ndc: f32, pair: vec2<f32>) -> f32 {
    let linear = 2.0 * panel_scale / (range * pixel_to_ndc);
    // d(log10 v)/dv = 1 / (v ln 10); `sample_valid` has already excluded v <= 0.
    let value = pair.x + pair.y;
    let log_slope = linear / (value * log(10.0));
    if is_log > 0.5 {
        return log_slope;
    }
    return mix(linear, log_slope, is_log);
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
