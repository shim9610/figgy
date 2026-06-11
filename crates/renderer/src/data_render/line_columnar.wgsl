// Columnar line shader — variable-thickness via quad extrusion.
//
// One line segment = one instance, 4 vertices per instance (TriangleStrip).
// The X and Y columns are bound twice: the second binding starts at a 4-byte
// (one f32) offset, so instance `i` sees points [i] and [i+1] at once.
//
// Corner mapping per segment:
//   vid 0 : at A, −normal
//   vid 1 : at A, +normal
//   vid 2 : at B, −normal
//   vid 3 : at B, +normal
//
// The normal is computed in pixel space and converted back to NDC, so width
// stays constant across panels with different X/Y pixel ratios.
// Zero-length segments collapse the quad and become invisible.
//
// Dash patterns (Style.dash / Style.dash_len) are evaluated per fragment
// against the polyline's CUMULATIVE ARC LENGTH: slots 4/5 carry a CPU-built
// prefix (pixels from the first point to point i), bound twice with a
// one-f32 shift like x/y, so the phase is exact and continuous across every
// joint regardless of curvature or sampling density. (A screen-position
// projection was tried first: its phase jumps by |position|·Δdirection at
// each joint, which shatters the pattern on curves.) Solid lines bind the X
// column as inert filler — the fragment stage never reads the varying when
// dash_len == 0.

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
    @location(0) x_a: f32,
    @location(1) y_a: f32,
    @location(2) x_b: f32,
    @location(3) y_b: f32,
};

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn data_to_ndc(xv: f32, yv: f32) -> vec2<f32> {
    let xv2 = maybe_log(xv, transform.scale_log.x);
    let yv2 = maybe_log(yv, transform.scale_log.y);
    let range = transform.data_max - transform.data_min;
    let t = (vec2<f32>(xv2, yv2) - transform.data_min) / range;
    return t * 2.0 - 1.0;
}
// ───── END common block ─────

// Line-only extra instance inputs (outside the common block): cumulative
// arc length at the segment's two endpoints, from the CPU-built prefix.
struct VsArc {
    @location(4) arc_a: f32,
    @location(5) arc_b: f32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Arc length (px) from the polyline start: arc_a at the two A-side
    // corners, arc_b at the two B-side corners — interpolation yields the
    // per-fragment arc position the dash pattern walks.
    @location(0) dist_px: f32,
};

@vertex
fn vs_main(in: VsIn, arc: VsArc, @builtin(vertex_index) vid: u32) -> VsOut {
    let a_ndc = data_to_ndc(in.x_a, in.y_a);
    let b_ndc = data_to_ndc(in.x_b, in.y_b);

    // Convert to pixel space so the direction vector can be normalized.
    let a_px = a_ndc / transform.pixel_to_ndc;
    let b_px = b_ndc / transform.pixel_to_ndc;

    let delta = b_px - a_px;
    let len = length(delta);
    // Zero-length guard — normal=0 makes corner=center, collapsing the quad.
    let dir = select(vec2<f32>(0.0, 0.0), delta / max(len, 1e-6), len > 1e-6);
    let normal_px = vec2<f32>(-dir.y, dir.x);

    let half_w = max(style.line_width_px, 1.0) * 0.5;

    let at_b = vid >= 2u;
    let on_pos = (vid & 1u) == 1u;
    let center_px = select(a_px, b_px, at_b);
    let side = select(-1.0, 1.0, on_pos);

    // Square caps: extend the quad by half_w along the segment direction on
    // both ends. Adjacent quads then overlap at the joint, which keeps the
    // stroke CONTINUOUS when segments are sub-pixel (dense / noisy data
    // flips the direction at every point; butt-ended quads degenerate into
    // disconnected slivers there — the "stippled line" artifact).
    let cap = select(-half_w, half_w, at_b);
    let corner_px = center_px + dir * cap + normal_px * half_w * side;
    let corner_ndc = corner_px * transform.pixel_to_ndc;

    var out: VsOut;
    out.pos = vec4<f32>(corner_ndc, 0.0, 1.0);
    // The cap extension keeps advancing the arc coordinate, so dash cuts
    // stay put in arc space (and the first cap can go slightly negative).
    out.dist_px = select(arc.arc_a - half_w, arc.arc_b + half_w, at_b);
    return out;
}

// Pattern scalar i (0..7): style.dash packs 8 sequential [on, off, ...]
// pixel lengths as two vec4s — dash[0].xyzw first, then dash[1].xyzw.
fn dash_scalar(i: u32) -> f32 {
    return style.dash[i / 4u][i % 4u];
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if style.dash_len == 0u {
        return style.color_premul;
    }
    // Clamp to the 8-scalar capacity so a bad CPU-side count cannot index
    // past the array.
    let n = min(style.dash_len, 8u);
    var period = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        period = period + max(dash_scalar(i), 0.0);
    }
    // Degenerate pattern (all spans zero/negative) — treat as solid.
    if period <= 0.0 {
        return style.color_premul;
    }
    // The square-cap extension can push dist_px slightly negative at the
    // polyline start — wrap into [0, period) with the sign-safe form.
    let phase = ((in.dist_px % period) + period) % period;
    var acc = 0.0;
    for (var i = 0u; i < n; i = i + 1u) {
        acc = acc + max(dash_scalar(i), 0.0);
        if phase < acc {
            // Even spans are "on", odd spans are "off". Hard cut — the
            // line edge has no AA today either.
            if (i & 1u) == 1u {
                discard;
            }
            return style.color_premul;
        }
    }
    // phase == period can occur from float rounding; that point is the
    // start of the next repetition, which always opens with an "on" span.
    return style.color_premul;
}
