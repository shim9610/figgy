// ───────────── parallel coordinates (#141) ─────────────
// `vs_parallel` / `fs_parallel` draw the rows of a parallel coordinates
// series (`DataRenderType::Parallel` on a chart with `Config.parallel`):
// instance `inst` is segment `i = inst % (axes - 1)` of row
// `r = inst / (axes - 1)`, the stroke between axis `i` and axis `i + 1`.
// Both ends are read from the column pool through the series' lane table
// (group 3, the lane-table pattern of SHADER_COMMON.md §5.13: no vertex
// buffers, so the eight-slot limit of the line layouts does not apply and
// the axis count is unbounded, #107 R3). An end is placed at the axis's
// category position along x and at its value's fraction of the axis's own
// range along y: `(v − min) / (max − min)`, linear or in log10, flipped
// when inverted, the normalized band `left_y` spans 0..1. Normalizing a
// value only to place it is appearance (CLAUDE.md rule 1, as the ternary
// placement); it is computed here on every draw and never stored. A row
// with a non-finite value breaks at that axis: the segments touching it
// collapse. The quad itself is `line_stroke_vertex`: the same square-cap
// stroke, antialiasing and clipping as every precise line.
//
// With a colour column (`ParallelParams.color_lane`, `PARALLEL_COLOR`), a
// row takes the colour of its value through the colourbar ramp, the point
// value map of the line module's group 3 (`line_value_map`,
// `line_value_stops`, bindings 0 and 1; the parallel resources follow at
// 2..5). Without, the series colour (`style.color_premul`).
// `vs_parallel_selected` redraws the picked row's segments in the
// selection's style (its colour and width, group 1), over the same
// geometry. `pick_parallel` answers the row whose nearest segment is within
// the query's distance as a point candidate (kind 0, `index0` the row,
// `index1` the nearer axis), reducing as the other data picks do.

struct ParallelParams {
    // Rows drawn: the shortest axis column (and colour column).
    rows: u32,
    // Axes: the lane table's entries. At least two.
    axes: u32,
    // The colour column's f32 lane base when `PARALLEL_COLOR` is set.
    color_lane: u32,
    flags: u32,
};

// A row's colour comes from its colour column through the ramp.
const PARALLEL_COLOR: u32 = 1u;

// One axis's range: `lo` is `min` (or `log10(min)` on a logarithmic axis)
// as a `(hi, lo)` pair, `span_inv` is `1 / (max − min)` in the same space,
// `mode` carries `PARALLEL_AXIS_INVERTED` and `PARALLEL_AXIS_LOG`.
// CPU twin: `data_render/parallel.rs::ParallelAxisGpu`, 16 B.
struct ParallelAxis {
    lo: vec2<f32>,
    span_inv: f32,
    mode: u32,
};

const PARALLEL_AXIS_INVERTED: u32 = 1u;
const PARALLEL_AXIS_LOG: u32 = 2u;

@group(3) @binding(2) var<storage, read> parallel_pool: array<f32>;
@group(3) @binding(3) var<uniform> parallel_params: ParallelParams;
// The f32 lane base of each axis's column, in axis order.
@group(3) @binding(4) var<storage, read> parallel_lanes: array<u32>;
@group(3) @binding(5) var<storage, read> parallel_axes: array<ParallelAxis>;

struct ParallelVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) dist_px: f32,
    @location(1) along_px: f32,
    @location(2) side_px: f32,
    @location(3) segment_len_px: f32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
    // The row's premultiplied colour, one per row.
    @location(7) @interpolate(flat) color_premul: vec4<f32>,
};

// The `(hi, lo)` value of row `row` of the column at lane base `base`.
fn parallel_value(base: u32, row: u32) -> vec2<f32> {
    let lane = base + row * 2u;
    return vec2<f32>(parallel_pool[lane], parallel_pool[lane + 1u]);
}

// Where `v` sits along axis `axis`: 0 at its min, 1 at its max, not
// clamped (a value past the range places past the axis's end, as a point
// past a Cartesian axis does). NaN for a non-finite value or a non-positive
// one on a logarithmic axis, which `parallel_end_valid` rejects.
fn parallel_fraction(axis: ParallelAxis, v: vec2<f32>) -> f32 {
    if (!sample_finite(v.x) || !sample_finite(v.y)) {
        return bitcast<f32>(0x7fc00000u);
    }
    var f: f32;
    if ((axis.mode & PARALLEL_AXIS_LOG) != 0u) {
        let value = v.x + v.y;
        if (!(value > 0.0)) {
            return bitcast<f32>(0x7fc00000u);
        }
        f = (log(value) / log(10.0) - (axis.lo.x + axis.lo.y)) * axis.span_inv;
    } else {
        // The high parts' difference first, then the low parts', so two
        // close f64 values keep their distance.
        f = ((v.x - axis.lo.x) + (v.y - axis.lo.y)) * axis.span_inv;
    }
    return select(f, 1.0 - f, (axis.mode & PARALLEL_AXIS_INVERTED) != 0u);
}

fn parallel_end_valid(f: f32) -> bool {
    return sample_finite(f);
}

// Segment `inst`'s row and first axis.
fn parallel_segment(inst: u32) -> vec2<u32> {
    let segments = max(parallel_params.axes, 2u) - 1u;
    return vec2<u32>(inst / segments, inst % segments);
}

// The stroke quad of segment `inst`, `vid` its strip vertex: the line
// stroke between the row's two ends, or the masked vertex when either end
// has no value.
fn parallel_stroke(inst: u32, vid: u32) -> VsOut {
    let seg = parallel_segment(inst);
    let row = seg.x;
    let i = seg.y;
    let fa = parallel_fraction(parallel_axes[i], parallel_value(parallel_lanes[i], row));
    let fb = parallel_fraction(parallel_axes[i + 1u], parallel_value(parallel_lanes[i + 1u], row));
    if (!parallel_end_valid(fa) || !parallel_end_valid(fb)) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    // The ends in the chart's data space: the axis's category position
    // along x, the fraction along the normalized band.
    var in: VsIn;
    in.x_a = vec2<f32>(f32(i), 0.0);
    in.y_a = vec2<f32>(fa, 0.0);
    in.x_b = vec2<f32>(f32(i + 1u), 0.0);
    in.y_b = vec2<f32>(fb, 0.0);
    // A solid stroke: no arc prefix, the dash phase is zero.
    var arc: VsArc;
    arc.arc_a = vec2<u32>(0u, 0u);
    arc.arc_b = vec2<u32>(0u, 0u);
    let slot = strip_slot(vid, 4u);
    let piece = step_piece(in.x_a, in.y_a, in.x_b, in.y_b, LINE_STEP_LINEAR, slot.piece);
    let a_ndc = data_to_ndc(transform, in.x_a, in.y_a);
    let b_ndc = data_to_ndc(transform, in.x_b, in.y_b);
    return line_stroke_vertex(in, arc, LINE_STEP_LINEAR, slot, piece, a_ndc, b_ndc);
}

fn parallel_out(stroke: VsOut, color: vec4<f32>) -> ParallelVsOut {
    var out: ParallelVsOut;
    out.pos = stroke.pos;
    out.dist_px = stroke.dist_px;
    out.along_px = stroke.along_px;
    out.side_px = stroke.side_px;
    out.segment_len_px = stroke.segment_len_px;
    out.polar_px = stroke.polar_px;
    out.polar_limits = stroke.polar_limits;
    out.polar_sector = stroke.polar_sector;
    out.color_premul = color;
    return out;
}

// The row's colour: its colour column's value through the ramp when the
// series maps one, else the series colour.
fn parallel_row_color(row: u32) -> vec4<f32> {
    if ((parallel_params.flags & PARALLEL_COLOR) == 0u) {
        return style.color_premul;
    }
    let value = parallel_value(parallel_params.color_lane, row);
    if (!ramp_finite(value.x) || !ramp_finite(value.y)) {
        return line_value_map.nan_color_premul;
    }
    return line_value_color(value);
}

@vertex
fn vs_parallel(@builtin(vertex_index) vid: u32, @builtin(instance_index) inst: u32) -> ParallelVsOut {
    let stroke = parallel_stroke(inst, vid);
    return parallel_out(stroke, parallel_row_color(parallel_segment(inst).x));
}

// The picked row's segments in the selection's colour and width (group 1
// is the selection's style): the caller draws the row's instances only.
@vertex
fn vs_parallel_selected(@builtin(vertex_index) vid: u32, @builtin(instance_index) inst: u32) -> ParallelVsOut {
    let stroke = parallel_stroke(inst, vid);
    return parallel_out(stroke, style.color_premul);
}

@fragment
fn fs_parallel(in: ParallelVsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    var stroke: VsOut;
    stroke.pos = in.pos;
    stroke.dist_px = in.dist_px;
    stroke.along_px = in.along_px;
    stroke.side_px = in.side_px;
    stroke.segment_len_px = in.segment_len_px;
    stroke.polar_px = in.polar_px;
    stroke.polar_limits = in.polar_limits;
    stroke.polar_sector = in.polar_sector;
    return line_fragment_color(stroke, in.color_premul);
}

// ───── Typed data pick (compute entry) ─────
// The query of `gpu_data_pick.rs` (group 1, bindings 1..3) and one
// candidate per workgroup, reduced as the bar module's picks are. The
// candidate of a segment is the pixel distance from the cursor to the
// stroke's centre line in the draw's pixel frame; a segment with a missing
// end is no candidate.

const PARALLEL_PICK_WORKGROUP_SIZE: u32 = 64u;

@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(2) var<storage, read> data_pick_pool: array<f32>;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

var<workgroup> parallel_pick_shared: array<DataPickCandidate, PARALLEL_PICK_WORKGROUP_SIZE>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn parallel_pick_zero(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        parallel_pick_shared[local_index] = DataPickCandidate();
        workgroupBarrier();
    }
}

fn parallel_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
}

// Nearer wins; the earlier row on a tie, so the answer does not depend on
// the reduction order.
fn parallel_pick_better(candidate: DataPickCandidate, incumbent: DataPickCandidate) -> bool {
    if (candidate.valid == 0u) {
        return false;
    }
    if (incumbent.valid == 0u) {
        return true;
    }
    if (candidate.distance_px != incumbent.distance_px) {
        return candidate.distance_px < incumbent.distance_px;
    }
    return candidate.index0 < incumbent.index0;
}

// The pixel distance from `p` to the segment `a`–`b`.
fn parallel_segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = b - a;
    let d2 = dot(d, d);
    let t = select(0.0, clamp(dot(p - a, d) / d2, 0.0, 1.0), d2 > 1e-12);
    return length(p - (a + d * t));
}

fn parallel_pick_candidate(inst: u32) -> DataPickCandidate {
    let seg = parallel_segment(inst);
    let row = seg.x;
    let i = seg.y;
    let fa = parallel_fraction(parallel_axes[i], parallel_value(parallel_lanes[i], row));
    let fb = parallel_fraction(parallel_axes[i + 1u], parallel_value(parallel_lanes[i + 1u], row));
    if (!parallel_end_valid(fa) || !parallel_end_valid(fb)) {
        return parallel_pick_invalid();
    }
    let a = data_to_px(transform, vec2<f32>(f32(i), 0.0), vec2<f32>(fa, 0.0));
    let b = data_to_px(transform, vec2<f32>(f32(i + 1u), 0.0), vec2<f32>(fb, 0.0));
    let cursor = data_pick_query.cursor_ndc_t.xy / transform.pixel_to_ndc;
    let distance = parallel_segment_distance(cursor, a, b);
    if (!(distance <= data_pick_query.limits.x)) {
        return parallel_pick_invalid();
    }
    let nearer = select(i, i + 1u, length(cursor - b) < length(cursor - a));
    return DataPickCandidate(1u, data_pick_query.data.y, 0u, row, nearer, 0u, distance, 0u);
}

@compute @workgroup_size(PARALLEL_PICK_WORKGROUP_SIZE)
fn pick_parallel(@builtin(local_invocation_index) local_index: u32) {
    parallel_pick_zero(local_index);
    var best = parallel_pick_invalid();
    let segments = max(parallel_params.axes, 2u) - 1u;
    let count = parallel_params.rows * segments;
    var index = local_index;
    while (index < count) {
        let candidate = parallel_pick_candidate(index);
        if (parallel_pick_better(candidate, best)) {
            best = candidate;
        }
        index = index + PARALLEL_PICK_WORKGROUP_SIZE;
    }
    parallel_pick_shared[local_index] = best;
    workgroupBarrier();
    for (var stride = PARALLEL_PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if (local_index < stride) {
            let candidate = parallel_pick_shared[local_index + stride];
            if (parallel_pick_better(candidate, parallel_pick_shared[local_index])) {
                parallel_pick_shared[local_index] = candidate;
            }
        }
        workgroupBarrier();
    }
    if (local_index == 0u) {
        data_pick_output = parallel_pick_shared[0];
    }
}
