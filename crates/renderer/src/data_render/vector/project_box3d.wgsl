// Projection layer: the 3D box (#211).
//
// The 3D form of `vector/project_cartesian2d.wgsl`: the same `VectorSample` /
// `ArrowPx` types and the same `project_arrow` / `arrow_clip` interface,
// so the glyph (`common/arrow.wgsl`), the input and the entry are spliced
// unchanged (rule 9). An arrow's anchor is the box point of its three
// columns (`view3d_point_t`, `common/project_3d.wgsl`), placed through the
// panel's camera; the glyph stays camera-facing and constant in pixels,
// like a 3D marker and stroke.
//
// Pixel space is the chart area's, centred, y up, as the flat projection's:
// the camera's clip space shares `transform.pixel_to_ndc`. Depth is the NDC
// depth of the arrow's tail and tip at the front of a tube of half its head
// width (`project_box_front`), as a 3D stroke sits at the front of a tube of
// its half width. NDC depth is affine along a projected line, so a corner's
// depth extrapolated along the tail → tip axis is exact.

/// One arrow sample as the input layer produces it (see the flat layer).
struct VectorSample {
    origin_hi: vec3<f32>,
    origin_lo: vec3<f32>,
    vector: vec3<f32>,
    value: vec2<f32>,
    value_valid: bool,
    valid: bool,
};

/// An arrow in chart-area pixels with the NDC depth at its tail and tip.
struct ArrowPx {
    tail: vec2<f32>,
    tip: vec2<f32>,
    depth: vec2<f32>,
    valid: bool,
};

const VECTOR_SCALE_DATA: u32 = 0u;
const VECTOR_SCALE_PIXELS: u32 = 1u;

/// A box point in pixels and its NDC depth at the front of a tube of
/// `radius_px` there.
struct BoxPx {
    px: vec2<f32>,
    depth: f32,
    valid: bool,
};

fn box_px(t: vec3<f32>, radius_px: f32) -> BoxPx {
    var out: BoxPx;
    let clip = project_box(t);
    let front = project_box_front(t, radius_px);
    out.px = clip.xy / clip.w / transform.pixel_to_ndc;
    out.depth = front.z / front.w;
    out.valid = clip.w > 0.0 && front.w > 0.0
        && sample_finite(out.px.x) && sample_finite(out.px.y) && sample_finite(out.depth);
    return out;
}

/// The box-`t` derivative of one data axis at `pair`: the chart's x or y
/// axis (0, 1) or the height axis (2), exact on a linear axis and local on
/// a logarithmic one, as `axis_px_per_unit_at` is in 2D.
fn box_t_per_unit(axis: u32, pair: vec2<f32>) -> f32 {
    var range: f32;
    var is_log: f32;
    if (axis == 0u) {
        range = fma(1.0, transform.data_max.x - transform.data_min.x, transform.data_max_lo.x - transform.data_min_lo.x);
        is_log = transform.scale_log.x;
    } else if (axis == 1u) {
        range = fma(1.0, transform.data_max.y - transform.data_min.y, transform.data_max_lo.y - transform.data_min_lo.y);
        is_log = transform.scale_log.y;
    } else {
        range = fma(1.0, view3d.z_max.x - view3d.z_min.x, view3d.z_max.y - view3d.z_min.y);
        is_log = view3d.flags.x;
    }
    let linear = 1.0 / range;
    if (is_log > 0.5) {
        // d(log10 v)/dv = 1 / (v ln 10); the anchor is valid, so v > 0.
        return linear / ((pair.x + pair.y) * log(10.0));
    }
    return linear;
}

/// Project one sample through the camera. The anchor is hidden unless it is
/// a valid box point (`view3d_point_valid`), the 3D form of the data-area
/// clip. `Data`: the tip is `origin + scale · (u, v, w)` per axis, hidden
/// when it leaves the box, as a 3D line drops a segment whose end does.
/// `Pixels`: the direction is the vector carried into box space by each
/// axis's derivative, and the length in the screen plane at the anchor is
/// `scale · |(u, v, w)|` pixels; an arrow out of that plane is foreshortened.
fn project_arrow(sample: VectorSample, mode: u32, scale: f32) -> ArrowPx {
    var out: ArrowPx;
    out.tail = vec2<f32>(0.0, 0.0);
    out.tip = vec2<f32>(0.0, 0.0);
    out.depth = vec2<f32>(0.0, 0.0);
    out.valid = false;
    let ox = vec2<f32>(sample.origin_hi.x, sample.origin_lo.x);
    let oy = vec2<f32>(sample.origin_hi.y, sample.origin_lo.y);
    let oz = vec2<f32>(sample.origin_hi.z, sample.origin_lo.z);
    if (!sample.valid || !view3d_point_valid(transform, ox, oy, oz)) {
        return out;
    }
    let u = sample.vector.x;
    let v = sample.vector.y;
    let w = sample.vector.z;
    if (!sample_finite(u) || !sample_finite(v) || !sample_finite(w)) {
        return out;
    }
    let radius = max(vector_params.head_width_px, vector_params.width_px) * 0.5;
    let anchor_t = view3d_point_t(ox, oy, oz);
    var tip_t: vec3<f32>;
    if (mode == VECTOR_SCALE_PIXELS) {
        let len_px = scale * length(vec3<f32>(u, v, w));
        // The vector's direction in box space (each `t` scaled to its
        // half extent): the derivative of every axis at the anchor.
        let d_t = vec3<f32>(box_t_per_unit(0u, ox) * u, box_t_per_unit(1u, oy) * v, box_t_per_unit(2u, oz) * w);
        let d_box = d_t * 2.0 * view3d.half_extents.xyz;
        let d_len = length(d_box);
        if (!sample_finite(len_px) || !sample_finite(d_len)) {
            return out;
        }
        if (d_len > 0.0 && len_px != 0.0) {
            // Pixels per box unit in the screen plane at the anchor: a step
            // perpendicular to the eye direction.
            let eye = view3d.eye.xyz;
            var side = cross(eye, vec3<f32>(0.0, 0.0, 1.0));
            if (dot(side, side) < 1e-6) {
                side = cross(eye, vec3<f32>(0.0, 1.0, 0.0));
            }
            side = normalize(side);
            let step = 1e-2;
            let a = box_px(anchor_t, 0.0);
            let b = box_px(anchor_t + side * step / (2.0 * view3d.half_extents.xyz), 0.0);
            let k = length(b.px - a.px) / step;
            if (!a.valid || !b.valid || !sample_finite(k) || k <= 0.0) {
                return out;
            }
            let box_len = len_px / k;
            tip_t = anchor_t + (d_box / d_len) * box_len / (2.0 * view3d.half_extents.xyz);
        } else {
            tip_t = anchor_t;
        }
    } else {
        let tx = pair_add(ox, scale * u);
        let ty = pair_add(oy, scale * v);
        let tz = pair_add(oz, scale * w);
        if (!view3d_point_valid(transform, tx, ty, tz)) {
            return out;
        }
        tip_t = view3d_point_t(tx, ty, tz);
    }
    let tail = box_px(anchor_t, radius);
    let tip = box_px(tip_t, radius);
    if (!tail.valid || !tip.valid) {
        return out;
    }
    out.tail = tail.px;
    out.tip = tip.px;
    out.depth = vec2<f32>(tail.depth, tip.depth);
    out.valid = true;
    return out;
}

/// The clip position of a quad corner of `arrow`: its pixel place, and the
/// depth along the tail → tip screen axis, extrapolated past the ends for
/// the head, the margin and the pivot shift (NDC depth is affine along the
/// projected line), clamped to the depth range.
fn arrow_clip(corner: vec2<f32>, arrow: ArrowPx) -> vec4<f32> {
    let axis = arrow.tip - arrow.tail;
    let len2 = dot(axis, axis);
    var depth = arrow.depth.x;
    if (len2 > 0.0) {
        let s = dot(corner - arrow.tail, axis) / len2;
        depth = mix(arrow.depth.x, arrow.depth.y, s);
    }
    return vec4<f32>(corner * transform.pixel_to_ndc, clamp(depth, 0.0, 1.0), 1.0);
}
