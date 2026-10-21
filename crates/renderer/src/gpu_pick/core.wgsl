
// Exact GPU point/segment picking over ColumnPool split-f32 values.
//
// This module deliberately uses pick_* names and its own layouts. It is not
// part of the render-shader common block: the picker consumes the same public
// data representation but owns a compute-only ABI.
//
// Small queries scan directly. Larger queries use two streaming gates instead
// of a traversal index. The X pass reads one axis and writes one scatter/line
// bit pair for every 32 logical indices. The Y pass narrows only those bits.
// The exact pass reads the surviving original indices and performs the
// production distance calculation.

const PICK_GATE_WORD_POINTS: u32 = 32u;
const PICK_WORKGROUP_SIZE: u32 = 64u;
const PICK_F32_MAX: f32 = 0x1.fffffep+127;
const PICK_F32_EPSILON: f32 = 1.1920929e-7;

const PICK_FLAG_SCATTER: u32 = 1u;
const PICK_FLAG_LINE: u32 = 2u;
const PICK_FLAG_STYLE_MAP: u32 = 4u;
const PICK_FLAG_STYLE_INDEX: u32 = 8u;
const PICK_FLAG_SIZE_MAP: u32 = 16u;
const PICK_FLAG_SIZE_AREA: u32 = 32u;
// A point vector series: its "markers" are the arrows the vector shader
// draws from each anchor.
const PICK_FLAG_VECTOR: u32 = 64u;
// `PickQueryParams.vector.w` bit 8: `VectorScale::Pixels` (else `Data`).
const PICK_VECTOR_SCALE_PIXELS: u32 = 256u;

// ───── exact-pass variants ─────
// The point picker is assembled from fragments in Rust (`gpu_pick.rs`,
// `PickShader`): this core, the gate entries for the gate module, and for
// each exact module the fragments of one primitive class (markers, vector
// arrows, lines, or markers and lines) with the exact entry. Within an exact
// module, `PICK_VARIANT` says which optional paths a pipeline carries; the
// Rust side creates one pipeline per variant a series needs, so the driver
// folds the others away at pipeline creation. The default carries every
// path, which is what a streamed pick uses.
const PICK_VARIANT_STYLED_MARKER: u32 = 2u; // sketch, planet and star markers (§5.7)
const PICK_VARIANT_DASH: u32 = 16u;         // the dash phase walk
const PICK_VARIANT_STYLED_LINE: u32 = 32u;  // sketch wobble and Milkyway ribbon
const PICK_VARIANT_POLAR: u32 = 64u;        // polar projection and clip
override PICK_VARIANT: u32 = 127u;

fn pick_variant(bits: u32) -> bool {
    return (PICK_VARIANT & bits) != 0u;
}

const PICK_STYLE_MASK_RADIUS: u32 = 2u;
const PICK_STYLE_MASK_SHAPE: u32 = 4u;

struct PickQueryTransform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    style_params: array<vec4<f32>, 3>,
    polar_angle: vec4<f32>,
    polar_extent: vec4<f32>,
};

struct PickQueryParams {
    transform: PickQueryTransform,
    // (cursor_x, cursor_y, chart_x, chart_y), all canvas pixels.
    cursor_chart: vec4<f32>,
    // (chart_width, chart_height, maximum pick distance, maximum per-axis
    // reach of a drawn marker from its centre in canvas pixels, the quad
    // margin included). Exact radii/widths are resolved after both gates.
    chart_limits: vec4<f32>,
    // (base scatter radius, line half width, display scale, direct-scan flag).
    scatter_line: vec4<f32>,
    // (point count, x f32-lane base, y f32-lane base, style-index lane base).
    data: vec4<u32>,
    // (flags, style count, override count, style-index logical length).
    style: vec4<u32>,
    // (gate-word count, series order, base shape id, dispatch X width).
    series: vec4<u32>,
    // (global point offset, owned scatter count, owned segment count,
    //  optional original-row map word base; zero disables the map).
    stream: vec4<u32>,
    // (step interpolation mode: 0 linear, 1 step-pre, 2 step-post,
    //  3 step-mid; 1 when `pick_arc` holds this series' cumulative arc
    //  prefix; the rest reserved). The step mode mirrors the line shader's
    //  Style.primitive_flags bits 2-3 so a pick lands on the drawn path.
    line: vec4<u32>,
    // `point_size_map`: (size f32-lane base, size logical length, 0, 0).
    size_map: vec4<u32>,
    // (domain_min hi, domain_min lo, domain_max hi, domain_max lo).
    size_domain: vec4<f32>,
    // (min_px, max_px, 0, 0), logical pixels; the display scale is applied
    // with the other radii.
    size_px: vec4<f32>,
    // Line geometry in canvas pixels, the display scale applied: (stroke
    // half-width `max(width, 1) / 2`, milkyway half extent, per-axis gate
    // reach of the drawn stroke past its centreline, 0).
    line_px: vec4<f32>,
    // The line's dash pattern in canvas pixels: up to eight sequential
    // [on, off, ...] lengths, `styled.w` of them used (the line `Style.dash`).
    line_dash: array<vec4<f32>, 2>,
    // (marker draw mode, line draw mode, series salt of the styled hashes,
    //  dash count).
    styled: vec4<u32>,
    // A point vector series (`PICK_FLAG_VECTOR`): (u f32-lane base, v f32-lane
    // base, arrow count — the shortest column the vector shader reads —,
    // pivot | `PICK_VECTOR_SCALE_PIXELS`).
    vector: vec4<u32>,
    // (scale — the `Data` factor, or `px_per_unit` times the display scale —,
    //  shaft width, head length, head width), canvas pixels.
    vector_px: vec4<f32>,
    // (minimum length, 0, 0, 0), canvas pixels.
    vector_min: vec4<f32>,
};

const PICK_STEP_LINEAR: u32 = 0u;
const PICK_STEP_PRE: u32 = 1u;
const PICK_STEP_POST: u32 = 2u;
const PICK_STEP_MID: u32 = 3u;

struct PickScatterStyleSlot {
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct PickScatterStyleOverride {
    point_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

// Thirty-two bytes, byte-for-byte with Rust's PickCandidateGpu. The public
// contract returns only identity/index/distance, so no reconstructed data value
// is carried through workgroup storage or readback.
struct PickCandidate {
    valid: u32,
    series_order: u32,
    point_index: u32,
    primitive_kind: u32, // scatter=0, line=1 (CPU traversal order)
    primitive_index: u32,
    distance_sq: f32,
    distance_px: f32,
    _pad: u32,
};

@group(0) @binding(0) var<storage, read> pick_pool_words: array<u32>;
// x=scatter bits, y=line-segment bits. One word pair covers 32 indices.
@group(0) @binding(1) var<storage, read_write> pick_gate_masks: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read_write> pick_workgroup_candidates: array<PickCandidate>;

// Reduction-only resources share group 0 but use bindings not reachable from
// the gate/exact entry points.
@group(0) @binding(3) var<storage, read> pick_reduce_inputs: array<PickCandidate>;
@group(0) @binding(4) var<storage, read_write> pick_reduce_output: PickCandidate;

@group(1) @binding(0) var<uniform> pick_query_params: PickQueryParams;
@group(1) @binding(1) var<storage, read> pick_style_slots: array<PickScatterStyleSlot>;
@group(1) @binding(2) var<storage, read> pick_style_overrides: array<PickScatterStyleOverride>;
@group(1) @binding(3) var<storage, read_write> pick_series_output: PickCandidate;

// The exact pass only: the line's cumulative arc length per point, the
// buffer the line shader's dash phase and sketch wobble were drawn with
// (`line_arc.wgsl`), or a one-value placeholder when `line.y` is zero.
@group(2) @binding(0) var<storage, read> pick_arc: array<f32>;

var<workgroup> pick_shared_candidates: array<PickCandidate, PICK_WORKGROUP_SIZE>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn figgy_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        pick_shared_candidates[local_index] = PickCandidate();
        workgroupBarrier();
    }
}

fn pick_is_finite(v: f32) -> bool {
    return v == v && abs(v) <= PICK_F32_MAX;
}

fn pick_read_pair(base: u32, point_index: u32) -> vec2<f32> {
    let word = base + point_index * 2u;
    return vec2<f32>(bitcast<f32>(pick_pool_words[word]), bitcast<f32>(pick_pool_words[word + 1u]));
}

fn pick_source_row(local_index: u32) -> u32 {
    if pick_query_params.stream.w != 0u {
        return pick_pool_words[pick_query_params.stream.w + local_index];
    }
    return local_index + pick_query_params.stream.x;
}

fn pick_pair_is_valid(v: vec2<f32>) -> bool {
    return pick_is_finite(v.x) && pick_is_finite(v.y) && pick_is_finite(v.x + v.y);
}

fn pick_coordinate_is_valid(v: vec2<f32>, axis: u32) -> bool {
    return pick_pair_is_valid(v)
        && (pick_query_params.transform.scale_log[axis] == 0.0 || v.x + v.y > 0.0);
}

fn pick_log10(v: f32) -> f32 {
    return log(max(v, 1e-30)) / log(10.0);
}

fn pick_axis_pair_to_t(v: vec2<f32>, axis: u32) -> f32 {
    let raw = v.x + v.y;
    let min_hi = pick_query_params.transform.data_min[axis];
    let max_hi = pick_query_params.transform.data_max[axis];
    let min_lo = pick_query_params.transform.data_min_lo[axis];
    let max_lo = pick_query_params.transform.data_max_lo[axis];
    let linear_num = fma(1.0, v.x - min_hi, v.y - min_lo);
    let range = fma(1.0, max_hi - min_hi, max_lo - min_lo);
    let log_num = (pick_log10(raw) - min_hi) - min_lo;
    if pick_query_params.transform.scale_log[axis] > 0.5 {
        return fma(log_num / range, pick_query_params.transform.data_to_panel_scale[axis],
            pick_query_params.transform.data_to_panel_offset[axis]);
    }
    let data_t = mix(linear_num / range, log_num / range, pick_query_params.transform.scale_log[axis]);
    return fma(data_t, pick_query_params.transform.data_to_panel_scale[axis],
        pick_query_params.transform.data_to_panel_offset[axis]);
}

fn pick_project_axis_pair(v: vec2<f32>, axis: u32) -> f32 {
    let t = pick_axis_pair_to_t(v, axis);
    if axis == 0u {
        return pick_query_params.cursor_chart.z + t * pick_query_params.chart_limits.x;
    }
    return pick_query_params.cursor_chart.w + (1.0 - t) * pick_query_params.chart_limits.y;
}

fn pick_polar() -> bool {
    return pick_variant(PICK_VARIANT_POLAR) && pick_query_params.transform.polar_angle.x > 0.5;
}

// Twin of `polar_ndc` in the data shaders, in canvas pixels.
fn pick_project_pair_polar(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    let t = pick_query_params.transform;
    let ty = pick_axis_pair_to_t(y, 1u);
    let angle = t.polar_angle.y + t.polar_angle.z * (x.x + x.y);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    let r = mix(t.polar_angle.w, 1.0, max(rt, 0.0));
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    let ndc = center + vec2<f32>(cos(angle), sin(angle)) * r * t.polar_extent.w * t.pixel_to_ndc;
    return vec2<f32>(
        pick_query_params.cursor_chart.z + (ndc.x + 1.0) * 0.5 * pick_query_params.chart_limits.x,
        pick_query_params.cursor_chart.w + (1.0 - ndc.y) * 0.5 * pick_query_params.chart_limits.y,
    );
}

fn pick_project_pair(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    if pick_polar() {
        return pick_project_pair_polar(x, y);
    }
    return vec2<f32>(pick_project_axis_pair(x, 0u), pick_project_axis_pair(y, 1u));
}

// Twin of `polar_sample_valid`: a radius below the axis start is not drawn,
// so it is not picked. The sector, the outer ring and the hole cut the drawn
// primitive per fragment instead; `pick_polar_clip_distance` applies that
// clip at the cursor.
fn pick_polar_valid(x: vec2<f32>, y: vec2<f32>) -> bool {
    if !pick_polar() {
        return true;
    }
    let t = pick_query_params.transform;
    let ty = pick_axis_pair_to_t(y, 1u);
    let rt = (ty - t.data_to_panel_offset.y) / t.data_to_panel_scale.y;
    return rt >= 0.0;
}

// The polar fragment clip of the draw shaders (SHADER_COMMON.md, coordinate
// sample validity). `polar_fragment_visible` is copied byte-for-byte; the
// three inputs it takes are computed from this shader's transform exactly
// as `polar_clip_offset` / `polar_clip_limits` / `polar_clip_sector` compute
// them from the draw transform.
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

// The cursor's offset from the polar centre in pixels, y up: what
// `polar_clip_offset` interpolates to at the fragment under the cursor.
fn pick_polar_cursor_offset() -> vec2<f32> {
    let t = pick_query_params.transform;
    let cursor = pick_query_params.cursor_chart;
    let ndc = vec2<f32>(
        (cursor.x - cursor.z) / pick_query_params.chart_limits.x * 2.0 - 1.0,
        1.0 - (cursor.y - cursor.w) / pick_query_params.chart_limits.y * 2.0,
    );
    let center = (t.data_to_panel_offset + t.data_to_panel_scale * 0.5) * 2.0 - 1.0;
    return (ndc - center) / t.pixel_to_ndc;
}

fn pick_polar_clip_limits() -> vec4<f32> {
    let p = pick_query_params.transform.polar_angle;
    let e = pick_query_params.transform.polar_extent;
    return vec4<f32>(p.x, e.w, e.w * p.w, select(0.0, 1.0, e.y > e.x));
}

fn pick_polar_clip_sector() -> vec4<f32> {
    let p = pick_query_params.transform.polar_angle;
    let e = pick_query_params.transform.polar_extent;
    return vec4<f32>(p.y, p.z, e.x, e.y);
}

// Distance from the ray at screen angle `angle` (from the polar centre) to
// `px`.
fn pick_polar_ray_distance(px: vec2<f32>, angle: f32) -> f32 {
    let dir = vec2<f32>(cos(angle), sin(angle));
    let along = dot(px, dir);
    if along <= 0.0 {
        return length(px);
    }
    return abs(px.x * dir.y - px.y * dir.x);
}

// Zero where the polar fragment clip keeps the fragment under the cursor,
// else a positive lower bound of the distance to the kept region: past the
// outer ring, inside the hole, or to the nearer sector edge. Every primitive
// a polar chart draws is cut by this clip, so a hit distance is never below
// it. Zero for a rectangular chart.
fn pick_polar_clip_distance() -> f32 {
    if !pick_polar() {
        return 0.0;
    }
    let px = pick_polar_cursor_offset();
    let limits = pick_polar_clip_limits();
    let sector = pick_polar_clip_sector();
    if polar_fragment_visible(px, limits, sector) {
        return 0.0;
    }
    let r = length(px);
    var d = max(r - (limits.y + 0.5), (limits.z - 0.5) - r);
    if limits.w >= 0.5 && !polar_fragment_visible(px, vec4<f32>(limits.x, PICK_F32_MAX, 0.0, limits.w), sector) {
        let start = sector.x + sector.y * sector.z;
        let end = sector.x + sector.y * sector.w;
        d = max(d, min(pick_polar_ray_distance(px, start), pick_polar_ray_distance(px, end)));
    }
    return max(d, PICK_F32_EPSILON);
}

fn pick_axis_cursor(axis: u32) -> f32 {
    return pick_query_params.cursor_chart[axis];
}

fn pick_conservative_gate_threshold(threshold: f32) -> f32 {
    // The exact pass remains authoritative. A tiny outward pad prevents an
    // axis gate from rejecting a boundary hit because sqrt/add/sub rounded in
    // a different direction.
    return threshold + max(abs(threshold), 1.0) * PICK_F32_EPSILON * 8.0;
}

// Polar projections are not separable per axis: the X gate passes every
// valid sample and the Y gate tests the full projection
// (`pick_polar_scatter_may_hit`); line segments go straight to the exact
// pass, whose arc pieces are the drawn path.
fn pick_scatter_projected_may_hit(projected: f32, axis: u32) -> bool {
    if pick_polar() {
        return true;
    }
    if !pick_is_finite(projected) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.chart_limits.w,
    );
    return abs(projected - pick_axis_cursor(axis)) <= threshold;
}

fn pick_polar_scatter_may_hit(point_index: u32, y: vec2<f32>) -> bool {
    let x = pick_read_pair(pick_query_params.data.y, point_index);
    if !pick_pair_is_valid(x) || !pick_polar_valid(x, y) {
        return false;
    }
    let projected = pick_project_pair_polar(x, y);
    if !pick_is_finite(projected.x) || !pick_is_finite(projected.y) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.chart_limits.w,
    );
    let delta = abs(projected - pick_query_params.cursor_chart.xy);
    return delta.x <= threshold && delta.y <= threshold;
}

fn pick_line_projected_may_hit(a_px: f32, b_px: f32, axis: u32) -> bool {
    if pick_polar() {
        return true;
    }
    if !pick_is_finite(a_px) || !pick_is_finite(b_px) {
        return false;
    }
    let threshold = pick_conservative_gate_threshold(
        pick_query_params.chart_limits.z + pick_query_params.line_px.z,
    );
    let cursor = pick_axis_cursor(axis);
    return cursor >= min(a_px, b_px) - threshold
        && cursor <= max(a_px, b_px) + threshold;
}

fn pick_linear_workgroup_id(workgroup_id: vec3<u32>) -> u32 {
    return workgroup_id.x + workgroup_id.y * pick_query_params.series.w;
}

// What a marker series' pipeline draws under the chart's draw style
// (`PickQueryParams.styled.x`).
const PICK_MARKER_PRECISE: u32 = 0u;
// `vs_sketch` / `fs_sketch`: the shape with a hashed contour wobble.
const PICK_MARKER_SKETCH: u32 = 1u;
// Milkyway `vs_planet` / `fs_planet`: a disc with a rim glow and a ring.
const PICK_MARKER_PLANET: u32 = 2u;
// Constellation `vs_constellation_star` / `fs_constellation_star`.
const PICK_MARKER_STAR: u32 = 3u;

// The global point index the styled entries hash (`styled_point_index`).
fn pick_styled_index(point_index: u32) -> u32 {
    return point_index + pick_query_params.stream.x;
}

// What a line series' pipeline draws under the chart's draw style
// (`PickQueryParams.styled.y`). Constellation lines are the precise quads.
const PICK_LINE_PRECISE: u32 = 0u;
// `vs_sketch`: a subdivided strip displaced by `sketch_line_offset`.
const PICK_LINE_SKETCH: u32 = 1u;
// Milkyway: the ribbon strip and the star field scattered along it.
const PICK_LINE_RIBBON: u32 = 2u;

// Canvas pixels (y down) into the draw shaders' pixel frame (y up).
fn pick_flip(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x, -v.y);
}

fn pick_invalid_candidate(series_order: u32) -> PickCandidate {
    return PickCandidate(
        0u,
        series_order,
        0u,
        0u,
        0u,
        PICK_F32_MAX,
        PICK_F32_MAX,
        0u,
    );
}

fn pick_candidate_is_better(candidate: PickCandidate, incumbent: PickCandidate) -> bool {
    if candidate.valid == 0u {
        return false;
    }
    if incumbent.valid == 0u {
        return true;
    }
    if candidate.distance_sq != incumbent.distance_sq {
        return candidate.distance_sq < incumbent.distance_sq;
    }
    // An exact tie goes to whatever is painted on top. Series paint in
    // order, so a later series covers an earlier one; within a series the
    // markers paint over the line; and instances paint in index order, so a
    // higher marker or segment index covers a lower one.
    if candidate.series_order != incumbent.series_order {
        return candidate.series_order > incumbent.series_order;
    }
    if candidate.primitive_kind != incumbent.primitive_kind {
        return candidate.primitive_kind < incumbent.primitive_kind;
    }
    return candidate.primitive_index > incumbent.primitive_index;
}
