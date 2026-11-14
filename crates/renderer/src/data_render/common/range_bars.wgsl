// Range bar input layer of the bar module (SHADER_COMMON.md §5.18): the bars
// of a `DataRenderType::RangeBars` series, spliced into the module after
// `box_plot.wgsl`, whose slot layout (`box_slot`), value reads
// (`box_value`), rects and flat quads it shares, `category_bars.wgsl`,
// whose group 3, rounded-rect coverage and flat-body shading it reads, and
// `bar_columnar.wgsl`, whose style slots (`bar_style_slots`) hold its part
// colours.
//
// One instance is the bar of one row: from the start to the end value at
// the row's position, a whisker from its low to its high value under it,
// and a connector at its end to the next row's bar. No vertex buffers;
// every part is built here on every draw, pick, highlight and label from
// the series' columns, read by their pool lanes from its lane table
// (`category_lanes`, §5.18). The renderer compares the two given ends to
// choose a direction colour and maps a state index to a style; it computes
// no value the user reads as data. Nothing per bar is stored anywhere else.

// Lane table entries: the f32 lane bases of the end, start, low, high,
// colour (the total flag or the state index) and progress columns
// (`BOX_LANE_ABSENT` for an absent column), then the colour mode and the
// state style count.
const RANGE_LANE_END: u32 = 0u;
const RANGE_LANE_START: u32 = 1u;
const RANGE_LANE_LOW: u32 = 2u;
const RANGE_LANE_HIGH: u32 = 3u;
const RANGE_LANE_COLOR: u32 = 4u;
const RANGE_LANE_PROGRESS: u32 = 5u;
const RANGE_LANE_COLOR_MODE: u32 = 6u;
const RANGE_LANE_STATES: u32 = 7u;

// `RANGE_LANE_COLOR_MODE` values: `RangeColor::{Single, Direction, State}`.
const RANGE_COLOR_SINGLE: u32 = 0u;
const RANGE_COLOR_DIRECTION: u32 = 1u;
const RANGE_COLOR_STATE: u32 = 2u;

// The series' style slots: the whisker (fill, gap = width), the connector
// (fill, gap = width), the up, down and total fills, the progress (width
// ratio = the remaining opacity), then one fill per state.
const RANGE_SLOT_WHISKER: u32 = 0u;
const RANGE_SLOT_CONNECTOR: u32 = 1u;
const RANGE_SLOT_UP: u32 = 2u;
const RANGE_SLOT_DOWN: u32 = 3u;
const RANGE_SLOT_TOTAL: u32 = 4u;
const RANGE_SLOT_PROGRESS: u32 = 5u;
const RANGE_SLOT_STATES: u32 = 6u;

// Quads of one bar, six vertices each, in paint order.
const RANGE_QUAD_WHISKER: u32 = 0u;
const RANGE_QUAD_BODY: u32 = 1u;
const RANGE_QUAD_CONNECTOR: u32 = 2u;
/// Vertices per bar. Must match `bar.rs`'s `RANGE_BAR_VERTICES`.
const RANGE_BAR_VERTICES: u32 = 18u;

// What a fragment of the bar draws.
const RANGE_DRAW_FLAT: u32 = 0u;
const RANGE_DRAW_BODY: u32 = 1u;

fn range_lane(k: u32) -> u32 {
    return category_lanes[k];
}

/// Whether the pair `a` is at least `b`: the high words first, the low ones
/// on a tie, so the comparison keeps the pairs' precision.
fn range_pair_ge(a: vec2<f32>, b: vec2<f32>) -> bool {
    return a.x > b.x || (a.x == b.x && a.y >= b.y);
}

/// One bar as the draw, the pick, the highlight and the label see it, in
/// chart-area render pixels with y down.
struct RangeBar {
    valid: bool,
    position: vec2<f32>,
    start: vec2<f32>,
    end: vec2<f32>,
    /// Its place across the slot.
    place: BoxLayout,
    /// The start and the end along the value axis.
    start_px: f32,
    end_px: f32,
    /// `(x0, y0, x1, y1)`.
    rect: vec4<f32>,
    /// Corner radii, all four the same: a floating bar is rounded at both
    /// ends.
    radii: vec4<f32>,
};

/// The bar of row `row`: from its start to its end at its position, placed
/// in its slot by the group layout, rounded by the resolved corner radius.
/// A NaN position, start or end, equal ends, or a slot too narrow for the
/// group draws no bar.
fn range_bar(row: u32, resolved: ResolvedBarStyle) -> RangeBar {
    var out: RangeBar;
    out.valid = false;
    if (row >= category_params.count) {
        return out;
    }
    let end = box_value(RANGE_LANE_END, row);
    let start = box_value(RANGE_LANE_START, row);
    out.position = category_pair(category_params.positions, row);
    if (!end.ok || !start.ok || !category_finite(out.position)) {
        return out;
    }
    if (end.pair.x == start.pair.x && end.pair.y == start.pair.y) {
        return out;
    }
    out.start = start.pair;
    out.end = end.pair;
    let place = box_slot(out.position, out.end, resolved);
    if (!place.valid) {
        return out;
    }
    let a = box_along(out.position, out.start);
    let b = box_along(out.position, out.end);
    if (!bar_finite(a) || !bar_finite(b) || a == b) {
        return out;
    }
    out.place = place;
    out.start_px = a;
    out.end_px = b;
    out.rect = box_rect(place.cross0, place.cross1, a, b);
    let r = min(min(max(resolved.finish.x, 0.0), place.thickness * 0.5), abs(b - a) * 0.5);
    out.radii = vec4<f32>(r);
    out.valid = true;
    return out;
}

/// The bar as the category bar shape, for its rounded-rect distance and
/// coverage.
fn range_shape(bar: RangeBar) -> CategoryBar {
    return CategoryBar(bar.valid, bar.rect, bar.radii, 0u, bar.end_px, true, vec4<f32>(0.0), vec4<f32>(0.0));
}

/// Style slot `k`, or `None` (an all-zero mask) when the series has none.
fn range_slot(k: u32) -> BarStyleSlot {
    if (k < bar_style_meta.style_count) {
        return bar_style_slots[k];
    }
    return BarStyleSlot(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

/// The fill of slot `k`, or `fallback` when the slot sets none.
fn range_slot_fill(k: u32, fallback: vec4<f32>) -> vec4<f32> {
    let slot = range_slot(k);
    return select(fallback, slot.fill_color_premul, (u32(slot.params.w) & BAR_STYLE_MASK_FILL) != 0u);
}

/// The width of stroke slot `k` (the whisker or the connector) in render
/// pixels, 0 when the series draws none.
fn range_stroke_width(k: u32) -> f32 {
    let slot = range_slot(k);
    return select(0.0, max(slot.params.y, 0.0), (u32(slot.params.w) & BAR_STYLE_MASK_GAP) != 0u);
}

/// The fill of row `row`'s bar under the series' colour mode: the series
/// colour (`base`), the direction of its given ends, the total colour where
/// its flag reads 1, or its state's style.
fn range_fill(row: u32, bar: RangeBar, base: vec4<f32>) -> vec4<f32> {
    let mode = range_lane(RANGE_LANE_COLOR_MODE);
    let flag = box_value(RANGE_LANE_COLOR, row);
    if (mode == RANGE_COLOR_DIRECTION) {
        if (flag.ok && box_sum(flag.pair) == 1.0) {
            return range_slot_fill(RANGE_SLOT_TOTAL, base);
        }
        let up = range_pair_ge(bar.end, bar.start);
        return range_slot_fill(select(RANGE_SLOT_DOWN, RANGE_SLOT_UP, up), base);
    }
    if (mode == RANGE_COLOR_STATE && flag.ok) {
        let state = round(box_sum(flag.pair));
        if (state >= 0.0 && state < f32(range_lane(RANGE_LANE_STATES))) {
            return range_slot_fill(RANGE_SLOT_STATES + u32(state), base);
        }
    }
    return base;
}

/// Where row `row`'s progress splits its bar along the value axis, in
/// render pixels: `start + p · (end − start)` for the row's progress `p`
/// clamped to `0..=1`, the difference taken on the pairs so a time axis
/// keeps its precision. `ok` is false without a progress column or for a
/// NaN progress, and the bar is drawn whole.
struct RangeSplit {
    ok: bool,
    along: f32,
};

fn range_progress(row: u32, bar: RangeBar) -> RangeSplit {
    let p = box_value(RANGE_LANE_PROGRESS, row);
    if (!p.ok) {
        return RangeSplit(false, 0.0);
    }
    let share = clamp(box_sum(p.pair), 0.0, 1.0);
    let span = (bar.end.x - bar.start.x) + (bar.end.y - bar.start.y);
    let at = box_along(bar.position, vec2<f32>(bar.start.x, bar.start.y + span * share));
    return RangeSplit(bar_finite(at), at);
}

/// Row `row`'s whisker: a stroke rect from its low to its high value at the
/// middle of its bar, or `ok == false` without whiskers, a NaN low or high,
/// or a zero width.
struct RangeRect {
    ok: bool,
    rect: vec4<f32>,
};

fn range_whisker(row: u32, bar: RangeBar) -> RangeRect {
    let low = box_value(RANGE_LANE_LOW, row);
    let high = box_value(RANGE_LANE_HIGH, row);
    let half = range_stroke_width(RANGE_SLOT_WHISKER) * 0.5;
    if (!bar.valid || !low.ok || !high.ok || !(half > 0.0)) {
        return RangeRect(false, vec4<f32>(0.0));
    }
    let a = box_along(bar.position, low.pair);
    let b = box_along(bar.position, high.pair);
    if (!bar_finite(a) || !bar_finite(b)) {
        return RangeRect(false, vec4<f32>(0.0));
    }
    return RangeRect(true, box_rect(bar.place.middle - half, bar.place.middle + half, a, b));
}

/// Row `row`'s connector: a stroke rect at its bar's end value, from the
/// bar's edge to the edge of the next row's bar facing it, or `ok == false`
/// for the last row, a missing neighbour, overlapping bars or a zero width.
fn range_connector(row: u32, bar: RangeBar, resolved: ResolvedBarStyle) -> RangeRect {
    let half = range_stroke_width(RANGE_SLOT_CONNECTOR) * 0.5;
    if (!bar.valid || !(half > 0.0)) {
        return RangeRect(false, vec4<f32>(0.0));
    }
    let next = range_bar(row + 1u, resolved);
    if (!next.valid) {
        return RangeRect(false, vec4<f32>(0.0));
    }
    let ahead = next.place.middle > bar.place.middle;
    let c0 = select(next.place.cross1, bar.place.cross1, ahead);
    let c1 = select(bar.place.cross0, next.place.cross0, ahead);
    if (!(c1 > c0)) {
        return RangeRect(false, vec4<f32>(0.0));
    }
    return RangeRect(true, box_rect(c0, c1, bar.end_px - half, bar.end_px + half));
}

// ───── Draw ─────

struct RangeVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) outline_color: vec4<f32>,
    @location(5) @interpolate(flat) material: vec4<f32>,
    /// x = outline width, y = emphasis, z = grain seed, w = 0.
    @location(6) @interpolate(flat) metrics: vec4<f32>,
    /// x = the progress split along the value axis, y = 1 past the split
    /// towards larger render pixels, -1 towards smaller ones, 0 for no
    /// progress, z = the remaining opacity.
    @location(7) @interpolate(flat) progress: vec4<f32>,
    /// `RANGE_DRAW_*`.
    @location(8) @interpolate(flat) draws: u32,
};

fn range_vs_empty() -> RangeVsOut {
    var out: RangeVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.rect = vec4<f32>(0.0);
    out.radii = vec4<f32>(0.0);
    out.fill = vec4<f32>(0.0);
    out.outline_color = vec4<f32>(0.0);
    out.material = vec4<f32>(0.0);
    out.metrics = vec4<f32>(0.0);
    out.progress = vec4<f32>(0.0);
    out.draws = RANGE_DRAW_FLAT;
    return out;
}

/// A flat stroke quad over `rect` in `color`, as a box plot's whisker is.
fn range_flat_vertex(vi: u32, rect: RangeRect, color: vec4<f32>) -> RangeVsOut {
    var out = range_vs_empty();
    out.fill = color;
    if (!rect.ok) {
        return out;
    }
    let p = box_quad_corner(vi, rect.rect, 0.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

/// A body vertex: the bar's quad, one pixel past its rect, with its fill,
/// outline, material and progress.
fn range_body_vertex(vi: u32, row: u32, bar: RangeBar, resolved: ResolvedBarStyle) -> RangeVsOut {
    var out = range_vs_empty();
    out.draws = RANGE_DRAW_BODY;
    out.rect = bar.rect;
    out.radii = bar.radii;
    out.fill = range_fill(row, bar, resolved.fill_color_premul);
    out.outline_color = resolved.border_color_premul;
    out.material = resolved.material;
    let seed = (category_params.seed ^ (row * 2654435761u)) % 4096u;
    out.metrics = vec4<f32>(resolved.border_width_px, resolved.finish.y, f32(seed), 0.0);
    let split = range_progress(row, bar);
    let remaining = range_slot(RANGE_SLOT_PROGRESS);
    let has_opacity = (u32(remaining.params.w) & BAR_STYLE_MASK_WIDTH_RATIO) != 0u;
    if (split.ok && has_opacity) {
        out.progress = vec4<f32>(split.along, select(-1.0, 1.0, bar.end_px > bar.start_px), clamp(remaining.params.z, 0.0, 1.0), 0.0);
    }
    if (!bar.valid) {
        return out;
    }
    let p = box_quad_corner(vi, bar.rect, 1.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

@vertex
fn vs_range_bars(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> RangeVsOut {
    let quad = vi / BOX_PLOT_QUAD_VERTICES;
    let resolved = resolve_bar_style(category_base_style(), row);
    let bar = range_bar(row, resolved);
    if (!bar.valid) {
        return range_vs_empty();
    }
    if (quad == RANGE_QUAD_BODY) {
        return range_body_vertex(vi, row, bar, resolved);
    }
    if (quad == RANGE_QUAD_WHISKER) {
        return range_flat_vertex(vi, range_whisker(row, bar), range_slot_fill(RANGE_SLOT_WHISKER, resolved.fill_color_premul));
    }
    return range_flat_vertex(vi, range_connector(row, bar, resolved), range_slot_fill(RANGE_SLOT_CONNECTOR, resolved.fill_color_premul));
}

/// The body's colour at `p`: the flat-body shading of a category bar, the
/// outline over it, and the part past the progress split at the remaining
/// opacity.
fn range_body_color(in: RangeVsOut) -> vec4<f32> {
    let bar = CategoryBar(true, in.rect, in.radii, 0u, 0.0, true, vec4<f32>(0.0), vec4<f32>(0.0));
    let p = in.local;
    let cover = category_coverage(p, bar, in.metrics.x);
    if (cover.x <= 0.0) {
        return vec4<f32>(0.0);
    }
    let fill_alpha = max(in.fill.a, 1e-6);
    // The main draw never brightens: the picked bar's emphasis is the
    // selection pass's.
    let shaded = Bar(
        in.rect,
        in.radii,
        vec4<f32>(in.fill.rgb / fill_alpha, in.fill.a),
        Material(
            vec4<f32>(in.material.x, CATEGORY_BAR_ROUGHNESS, in.material.y, in.material.z),
            vec4<f32>(in.material.w, in.metrics.z, 0.0, 0.0),
        ),
        vec4<f32>(select(0.0, 1.0, in.metrics.x > 0.0), in.metrics.x, f32(category_params.horizontal), 0.0),
        in.outline_color,
        vec4<f32>(0.0, category_params.scale, 0.0, 0.0),
    );
    let outline_alpha = in.outline_color.a;
    let outline_rgb = in.outline_color.rgb / max(outline_alpha, 1e-6);
    let c = mix(bar_surface(p, shaded), outline_rgb, outline_alpha * cover.y / max(cover.x, 0.001));
    var color = vec4<f32>(c * cover.x, cover.x);
    if (in.progress.y != 0.0) {
        // Past the split, over one pixel, the bar fades to the remaining
        // opacity.
        let along = select(p.y, p.x, category_params.horizontal != 0u);
        let past = clamp((along - in.progress.x) * in.progress.y + 0.5, 0.0, 1.0);
        color = color * mix(1.0, in.progress.z, past);
    }
    return color;
}

@fragment
fn fs_range_bars(in: RangeVsOut) -> @location(0) vec4<f32> {
    if (in.draws == RANGE_DRAW_BODY) {
        let c = range_body_color(in);
        if (c.a <= 0.0) {
            discard;
        }
        return c;
    }
    // A whisker or a connector is a flat quad, as an error bar is.
    return in.fill;
}

// ───── Selection ─────
//
// The picked bar's highlight: an inside ring around the body in the
// selection's colour (`style.color_premul`) and width
// (`style.line_width_px`), over the bar's emphasis brightness, as a picked
// category bar's. The whisker and the connector add nothing: the pass
// draws one quad (`CATEGORY_BAR_VERTICES`), the body's.

@vertex
fn vs_range_bar_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> RangeVsOut {
    let resolved = resolve_bar_style(category_base_style(), row);
    let bar = range_bar(row, resolved);
    if (!bar.valid) {
        return range_vs_empty();
    }
    var out = range_body_vertex(vi, row, bar, resolved);
    out.fill = style.color_premul;
    out.metrics.x = max(style.line_width_px, 0.0);
    return out;
}

@fragment
fn fs_range_bar_selection(in: RangeVsOut) -> @location(0) vec4<f32> {
    let cover = category_coverage(in.local, CategoryBar(true, in.rect, in.radii, 0u, 0.0, true, vec4<f32>(0.0), vec4<f32>(0.0)), in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
    }
    let ring = in.fill * cover.y;
    let brighten = vec3<f32>(max(in.metrics.y, 0.0) * max(cover.x - cover.y, 0.0));
    return vec4<f32>(ring.rgb + brighten, ring.a);
}

// ───── Exact data pick (compute entry) ─────
//
// The bar under the cursor: the nearest part of any bar within the pick
// distance — the body (its rounded rect), the whisker or the connector (their
// stroke rects) — from the same functions the draw builds them with. Every
// part answers its bar, as a histogram bin whose index is the row; a
// connector is a candidate of the bar it leaves.

fn range_pick_row(row: u32) -> DataPickCandidate {
    let resolved = resolve_bar_style(category_geometry_style(), row);
    let bar = range_bar(row, resolved);
    if (!bar.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    var best = max(category_bar_distance(cursor, range_shape(bar)), 0.0);
    let whisker = range_whisker(row, bar);
    if (whisker.ok) {
        best = min(best, box_rect_distance(cursor, whisker.rect));
    }
    let connector = range_connector(row, bar, resolved);
    if (connector.ok) {
        best = min(best, box_rect_distance(cursor, connector.rect));
    }
    if (!data_pick_finite(best)
        || !data_pick_finite(data_pick_query.limits.x)
        || best > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        row,
        0u,
        0u,
        best,
        row,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_range_bar(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var row = local_index;
    while (row < category_params.count) {
        let candidate = range_pick_row(row);
        if (data_pick_better(candidate, best)) {
            best = candidate;
        }
        row = row + DATA_PICK_WORKGROUP_SIZE;
    }
    data_pick_shared[local_index] = best;
    workgroupBarrier();
    for (var stride = DATA_PICK_WORKGROUP_SIZE / 2u; stride > 0u; stride = stride / 2u) {
        if (local_index < stride) {
            let candidate = data_pick_shared[local_index + stride];
            if (data_pick_better(candidate, data_pick_shared[local_index])) {
                data_pick_shared[local_index] = candidate;
            }
        }
        workgroupBarrier();
    }
    if (local_index == 0u) {
        data_pick_output = data_pick_shared[0];
    }
}
