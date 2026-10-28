
// Exact GPU point/segment picking over ColumnPool split-f32 values.
//
// The picker owns a compute-only ABI (pick_* names, the pool at group 0
// binding 0, the transform inside `PickQueryParams`) and splices the draw
// shaders' projection, sample validity, ramp and step fragments, passing
// `pick_query_params.transform` where a draw passes its `transform` uniform
// (SHADER_COMMON.md §1). It does not splice `transform.wgsl`, whose uniform
// declaration would take the pool's binding; `Transform` below is the same
// struct.
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
// The point picker is assembled from fragments in Rust (`gpu_pick/shader.rs`,
// `PickShader`): this core, the gate entries for the gate module, and for
// each exact module the fragments of one primitive class (markers, vector
// arrows, lines, or markers and lines) with the exact entry. Within an exact
// module, `PICK_VARIANT` says which optional paths a pipeline carries; the
// Rust side creates one pipeline per variant a series needs, so the driver
// folds the others away at pipeline creation. The default carries every
// path, which is what a streamed pick uses.
const PICK_VARIANT_STYLED_MARKER: u32 = 2u; // sketch, planet and star markers (§5.7)
const PICK_VARIANT_ERRORBAR: u32 = 4u;      // error-bar stems and caps (§5.11)
const PICK_VARIANT_AREA: u32 = 8u;          // a line's area fill (§2.2)
const PICK_VARIANT_DASH: u32 = 16u;         // the dash phase walk
const PICK_VARIANT_STYLED_LINE: u32 = 32u;  // sketch wobble and Milkyway ribbon
const PICK_VARIANT_POLAR: u32 = 64u;        // polar projection and clip
const PICK_VARIANT_PATH_ARROWS: u32 = 128u; // streamline arrowheads (§5.8)
override PICK_VARIANT: u32 = 255u;

fn pick_variant(bits: u32) -> bool {
    return (PICK_VARIANT & bits) != 0u;
}

const PICK_STYLE_MASK_RADIUS: u32 = 2u;
const PICK_STYLE_MASK_SHAPE: u32 = 4u;

struct Transform {
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
    transform: Transform,
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
    //  prefix; 1 when the line is closed, its last point joined back to the
    //  first; reserved). The step mode mirrors the line shader's
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
    //  shaft width, head length, head width), canvas pixels. A streamline
    //  series' arrowheads (`PICK_VARIANT_PATH_ARROWS`): (head length, 0,
    //  head length, head width).
    vector_px: vec4<f32>,
    // (minimum length, arrowhead spacing along the drawn arc — zero without
    //  streamline arrowheads —, 0, 0), canvas pixels.
    vector_min: vec4<f32>,
    // The error bars drawn with the markers (`gpu_pick/errorbar.wgsl`): (bar
    // count — the shortest of the six columns the error-bar shader reads,
    // zero without error bars —, the `ERRORBAR_HAS_*` directions,
    // error-bar style-index f32-lane base, style-index logical length; zero
    // without an index column).
    errorbar: vec4<u32>,
    // The error columns' f32-lane bases: (y lower, y upper, x lower, x upper).
    errorbar_lanes: vec4<u32>,
    // The error-bar style rows, after the marker rows in `pick_style_slots`
    // and `pick_style_overrides`: (first slot row, slot count, first
    // override row, override count).
    errorbar_rows: vec4<u32>,
    // (stem width, cap half length, cap width, 1 when the base colour
    // paints else 0), logical pixels; the display scale is applied with the
    // other sizes.
    errorbar_px: vec4<f32>,
    // The line's area fill (`gpu_pick/area.wgsl`): (second y column's f32-lane
    // base, filled segment count — zero without a fill —, 1 when it fills to
    // that series else to the baseline, 0).
    area: vec4<u32>,
    // (baseline hi, baseline lo, 0, 0): `Style.dash[1].xy` of the fill.
    area_baseline: vec4<f32>,
};

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
    primitive_kind: u32, // `PICK_KIND_*`
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

// The exact pass only: the line's cumulative arc length per point as arc
// pairs (SHADER_COMMON.md §5.6), the buffer the line shader's dash phase and
// sketch wobble were drawn with (`line_arc.wgsl`), or a one-pair placeholder
// when `line.y` is zero.
@group(2) @binding(0) var<storage, read> pick_arc: array<vec2<u32>>;

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

fn pick_project_axis_pair(v: vec2<f32>, axis: u32) -> f32 {
    let tr = pick_query_params.transform;
    let t = axis_pair_to_t(v, tr.data_min[axis], tr.data_max[axis], tr.data_min_lo[axis], tr.data_max_lo[axis],
        tr.scale_log[axis], tr.data_to_panel_scale[axis], tr.data_to_panel_offset[axis]);
    if axis == 0u {
        return pick_query_params.cursor_chart.z + t * pick_query_params.chart_limits.x;
    }
    return pick_query_params.cursor_chart.w + (1.0 - t) * pick_query_params.chart_limits.y;
}

fn pick_polar() -> bool {
    return pick_variant(PICK_VARIANT_POLAR) && pick_query_params.transform.polar_angle.x > 0.5;
}

// NDC into canvas pixels (y down) on this query's chart rectangle.
fn pick_ndc_to_canvas(ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        pick_query_params.cursor_chart.z + (ndc.x + 1.0) * 0.5 * pick_query_params.chart_limits.x,
        pick_query_params.cursor_chart.w + (1.0 - ndc.y) * 0.5 * pick_query_params.chart_limits.y,
    );
}

// The draw's projection in canvas pixels. A polar sample goes through the
// draw's NDC (`data_to_ndc`); a rectangular one maps each axis' panel `t`
// straight to pixels, which keeps the precision `t * 2 - 1` would round off.
// The cursor in NDC on this query's chart rectangle.
fn pick_cursor_ndc() -> vec2<f32> {
    let cursor = pick_query_params.cursor_chart;
    return vec2<f32>(
        (cursor.x - cursor.z) / pick_query_params.chart_limits.x * 2.0 - 1.0,
        1.0 - (cursor.y - cursor.w) / pick_query_params.chart_limits.y * 2.0,
    );
}

// The cursor in the draw's pixel frame (`data_to_px`: render pixels from the
// target's centre, y up). The exact pass measures every primitive in this
// frame from `pick_data_px`, the draw's own projection, so a primitive's
// pixels are the values its vertex stage computed (#114).
fn pick_cursor_px() -> vec2<f32> {
    return pick_cursor_ndc() / pick_query_params.transform.pixel_to_ndc;
}

// A data point in the draw's pixel frame, with the draw's projection.
fn pick_data_px(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    return data_to_px(pick_query_params.transform, x, y);
}

// Canvas pixels for the axis gates, which only prefilter (the exact pass
// uses `pick_data_px`).
fn pick_project_pair(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    if pick_polar() {
        return pick_ndc_to_canvas(data_to_ndc(pick_query_params.transform, x, y));
    }
    return vec2<f32>(pick_project_axis_pair(x, 0u), pick_project_axis_pair(y, 1u));
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
    // The cursor in NDC: where the draw's `polar_clip_offset` is
    // interpolated at the fragment under the cursor.
    let t = pick_query_params.transform;
    let px = polar_clip_offset(t, pick_cursor_ndc());
    let limits = polar_clip_limits(t);
    let sector = polar_clip_sector(t);
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
    if !pick_pair_is_valid(x) || !polar_sample_valid(pick_query_params.transform, x, y) {
        return false;
    }
    let projected = pick_project_pair(x, y);
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

// The point after `point_index` along the drawn line: the next one, or the
// first after the last on a closed line (`line.z`, a radar outline), whose
// closing segment the line shader draws as one more instance.
fn pick_next_point(point_index: u32) -> u32 {
    let next = point_index + 1u;
    if pick_query_params.line.z != 0u && next >= pick_query_params.data.x {
        return 0u;
    }
    return next;
}

// Whether `point_index` starts a drawn segment: every point but the last,
// and the last too on a closed line.
fn pick_starts_segment(point_index: u32) -> bool {
    let count = pick_query_params.data.x;
    return point_index + 1u < count || (pick_query_params.line.z != 0u && count > 1u);
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
// `vs_sketch`: a subdivided strip displaced by `sketch_line_offset_from`.
const PICK_LINE_SKETCH: u32 = 1u;
// Milkyway: the ribbon strip and the star field scattered along it.
const PICK_LINE_RIBBON: u32 = 2u;


// `PickCandidate.primitive_kind`, numbered so that within one series a lower
// kind is painted over a higher one: markers over the line over the error
// bars (`issue_series_data`). Fills are painted before every series' body
// (`issue_series_fill`).
const PICK_KIND_MARKER: u32 = 0u;
const PICK_KIND_LINE: u32 = 1u;
const PICK_KIND_ERRORBAR: u32 = 2u;
const PICK_KIND_FILL: u32 = 3u;

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
    // An exact tie goes to whatever is painted on top. Every fill paints
    // before every series' body, so a fill loses a tie to anything else;
    // series paint in order, so a later series covers an earlier one (a
    // later fill an earlier fill too); within a series the markers paint
    // over the line and the line over the error bars; and instances paint
    // in index order, so a higher marker, bar or segment index covers a
    // lower one.
    let candidate_fill = candidate.primitive_kind == PICK_KIND_FILL;
    let incumbent_fill = incumbent.primitive_kind == PICK_KIND_FILL;
    if candidate_fill != incumbent_fill {
        return incumbent_fill;
    }
    if candidate.series_order != incumbent.series_order {
        return candidate.series_order > incumbent.series_order;
    }
    if candidate.primitive_kind != incumbent.primitive_kind {
        return candidate.primitive_kind < incumbent.primitive_kind;
    }
    return candidate.primitive_index > incumbent.primitive_index;
}
