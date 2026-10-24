// Box plot input layer of the bar module (SHADER_COMMON.md §5.15): the boxes
// of a `DataRenderType::BoxPlot` series, spliced into the module after
// `category_bars.wgsl`, whose group 3 (the pool and `CategoryBarParams`),
// slot layout and flat-body shading it shares, `bar_columnar.wgsl`, whose
// override table (`resolve_bar_style`) it reads, `errorbar_segment.wgsl`,
// whose stroke quads draw the whiskers, the caps and the median, and
// `marker.wgsl`, whose shapes draw the mean and the outliers.
//
// One instance is the box of one category: row `row` of the series' record
// (the box plot schema's fields, read by their pool lanes) and group `row`
// of its ragged outlier column. No vertex buffers; the glyph is built here
// on every draw, pick and highlight from the record, the category position,
// the group layout and the direction. Nothing per box is stored anywhere
// else. Statistics the record does not order are clamped where they are
// drawn: the box spans min(q1, q3)..max(q1, q3), the median is held inside
// it, and a whisker never reaches into it.
//
// `CategoryBarParams` holds the box plot's lanes (SHADER_COMMON.md §5.15):
// `values` carries the f32 lane bases of the nine record fields in
// `BOX_PLOT_RECORD` order (`BOX_LANE_ABSENT` for an absent optional
// field), then the outlier values and offsets lanes, the outlier group
// count and the longest group; `mode` is grouped. The series' part styles
// are the bar style map's style slots, one per `BOX_PART_*`, and a per-box
// override is one override row per part at `row * BOX_PLOT_PARTS + part`.

const BOX_PLOT_PARTS: u32 = 3u;
// The box body: material, outline, corner radius, notch.
const BOX_PART_BODY: u32 = 0u;
// The median (fill colour, border width) and the whiskers and caps (border
// colour, gap = stroke width, width ratio = cap length ratio).
const BOX_PART_STROKES: u32 = 1u;
// The mean (fill colour) and the outliers (border colour); the material
// lanes are (mean size, outlier size, outlier shape, outlier filled).
const BOX_PART_MARKERS: u32 = 2u;

const BOX_FIELD_MEDIAN: u32 = 0u;
const BOX_FIELD_Q1: u32 = 1u;
const BOX_FIELD_Q3: u32 = 2u;
const BOX_FIELD_WHISKER_LOW: u32 = 3u;
const BOX_FIELD_WHISKER_HIGH: u32 = 4u;
const BOX_FIELD_MEAN: u32 = 5u;
const BOX_FIELD_CI_LOW: u32 = 7u;
const BOX_FIELD_CI_HIGH: u32 = 8u;
const BOX_LANE_OUTLIER_VALUES: u32 = 9u;
const BOX_LANE_OUTLIER_OFFSETS: u32 = 10u;
const BOX_LANE_OUTLIER_GROUPS: u32 = 11u;
const BOX_LANE_MAX_GROUP: u32 = 12u;
const BOX_LANE_ABSENT: u32 = 4294967295u;

// Quads of one box, six vertices each, in the legacy paint order: the low
// whisker and its cap, the high whisker and its cap, the body, the median,
// the mean, then one quad per outlier of the longest group.
const BOX_PLOT_QUAD_VERTICES: u32 = 6u;
const BOX_QUAD_LOW_STEM: u32 = 0u;
const BOX_QUAD_LOW_CAP: u32 = 1u;
const BOX_QUAD_HIGH_STEM: u32 = 2u;
const BOX_QUAD_HIGH_CAP: u32 = 3u;
const BOX_QUAD_BODY: u32 = 4u;
const BOX_QUAD_MEDIAN: u32 = 5u;
const BOX_QUAD_MEAN: u32 = 6u;
const BOX_QUAD_OUTLIERS: u32 = 7u;
/// Vertices of a box before its outliers. Must match `bar.rs`'s
/// `BOX_PLOT_BASE_VERTICES`.
const BOX_PLOT_BASE_VERTICES: u32 = 42u;

// What a fragment of the box draws.
const BOX_DRAW_STROKE: u32 = 0u;
const BOX_DRAW_BODY: u32 = 1u;
const BOX_DRAW_MARKER: u32 = 2u;

// `marker.wgsl` base shapes of the mean and the outliers.
const BOX_MARKER_CIRCLE: u32 = 0u;
const BOX_MARKER_SQUARE: u32 = 1u;
const BOX_MARKER_DIAMOND: u32 = 3u;

fn box_lane(k: u32) -> u32 {
    return category_params.values[k / 4u][k % 4u];
}

/// Field `k` of row `row`, or `ok == false` when the field is absent or the
/// value is not finite.
struct BoxValue {
    pair: vec2<f32>,
    ok: bool,
};

fn box_value(k: u32, row: u32) -> BoxValue {
    let base = box_lane(k);
    if (base == BOX_LANE_ABSENT) {
        return BoxValue(vec2<f32>(0.0), false);
    }
    let v = category_pair(base, row);
    return BoxValue(v, category_finite(v));
}

fn box_sum(v: vec2<f32>) -> f32 {
    return v.x + v.y;
}

fn box_pair_min(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return select(b, a, box_sum(a) <= box_sum(b));
}

fn box_pair_max(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return select(b, a, box_sum(a) >= box_sum(b));
}

/// The statistics of one box as drawn: a box needs its five summary fields;
/// the mean and the confidence interval are optional.
struct BoxStats {
    valid: bool,
    q_low: vec2<f32>,
    q_high: vec2<f32>,
    median: vec2<f32>,
    low: vec2<f32>,
    high: vec2<f32>,
    mean: vec2<f32>,
    has_mean: bool,
    ci_low: vec2<f32>,
    ci_high: vec2<f32>,
    has_ci: bool,
    position: vec2<f32>,
};

fn box_stats(row: u32) -> BoxStats {
    var s: BoxStats;
    s.valid = false;
    if (row >= category_params.count) {
        return s;
    }
    let median = box_value(BOX_FIELD_MEDIAN, row);
    let q1 = box_value(BOX_FIELD_Q1, row);
    let q3 = box_value(BOX_FIELD_Q3, row);
    let low = box_value(BOX_FIELD_WHISKER_LOW, row);
    let high = box_value(BOX_FIELD_WHISKER_HIGH, row);
    s.position = category_pair(category_params.positions, row);
    if (!(median.ok && q1.ok && q3.ok && low.ok && high.ok) || !category_finite(s.position)) {
        return s;
    }
    s.valid = true;
    s.q_low = box_pair_min(q1.pair, q3.pair);
    s.q_high = box_pair_max(q1.pair, q3.pair);
    s.median = box_pair_min(box_pair_max(median.pair, s.q_low), s.q_high);
    s.low = box_pair_min(low.pair, s.q_low);
    s.high = box_pair_max(high.pair, s.q_high);
    let mean = box_value(BOX_FIELD_MEAN, row);
    s.mean = mean.pair;
    s.has_mean = mean.ok;
    let ci_low = box_value(BOX_FIELD_CI_LOW, row);
    let ci_high = box_value(BOX_FIELD_CI_HIGH, row);
    s.ci_low = box_pair_min(ci_low.pair, ci_high.pair);
    s.ci_high = box_pair_max(ci_low.pair, ci_high.pair);
    s.has_ci = ci_low.ok && ci_high.ok;
    return s;
}

/// Value `v` on the value axis, in chart-area render pixels (x when the
/// categories run along y, else y down).
fn box_along(position: vec2<f32>, v: vec2<f32>) -> f32 {
    if (category_params.horizontal != 0u) {
        return category_px(data_to_ndc(transform, v, position)).x;
    }
    return category_px(data_to_ndc(transform, position, v)).y;
}

/// Where the box sits across its category slot, in chart-area render pixels
/// along the category axis: its two edges, its middle, and the offset of the
/// middle from the category centre.
struct BoxLayout {
    valid: bool,
    cross0: f32,
    cross1: f32,
    middle: f32,
    offset: f32,
    thickness: f32,
};

/// The box of this series in its slot: the group covers the slot's centred
/// `group_width`, its boxes side by side `gap_px` apart in series order, as
/// grouped category bars stand.
fn box_layout(s: BoxStats, resolved: ResolvedBarStyle) -> BoxLayout {
    var out: BoxLayout;
    out.valid = false;
    let horizontal = category_params.horizontal != 0u;
    let low = vec2<f32>(s.position.x - 0.5, s.position.y);
    let high = vec2<f32>(s.position.x + 0.5, s.position.y);
    var slot_a: f32;
    var slot_b: f32;
    if (horizontal) {
        slot_a = category_px(data_to_ndc(transform, s.q_low, low)).y;
        slot_b = category_px(data_to_ndc(transform, s.q_low, high)).y;
    } else {
        slot_a = category_px(data_to_ndc(transform, low, s.q_low)).x;
        slot_b = category_px(data_to_ndc(transform, high, s.q_low)).x;
    }
    let members = f32(max(category_params.members, 1u));
    let slot = abs(slot_b - slot_a);
    let span = slot * clamp(resolved.width_ratio, 0.0, 1.0);
    let gap = max(resolved.gap_px, 0.0);
    let thickness = (span - gap * (members - 1.0)) / members;
    if (!(thickness > 0.0) || !bar_finite(slot_a) || !bar_finite(slot_b)) {
        return out;
    }
    let centre = (slot_a + slot_b) * 0.5;
    out.cross0 = centre - span * 0.5 + f32(category_params.member) * (thickness + gap);
    out.cross1 = out.cross0 + thickness;
    out.middle = out.cross0 + thickness * 0.5;
    out.offset = out.middle - centre;
    out.thickness = thickness;
    out.valid = true;
    return out;
}

/// A rect `(x0, y0, x1, y1)` from spans along the category axis and the
/// value axis.
fn box_rect(cross0: f32, cross1: f32, along0: f32, along1: f32) -> vec4<f32> {
    let a = min(along0, along1);
    let b = max(along0, along1);
    if (category_params.horizontal != 0u) {
        return vec4<f32>(a, cross0, b, cross1);
    }
    return vec4<f32>(cross0, a, cross1, b);
}

/// A point from a coordinate along the category axis and one along the
/// value axis.
fn box_point(cross: f32, along: f32) -> vec2<f32> {
    return select(vec2<f32>(cross, along), vec2<f32>(along, cross), category_params.horizontal != 0u);
}

// ───── Part styles ─────

fn box_empty_style() -> ResolvedBarStyle {
    return ResolvedBarStyle(vec4<f32>(0.0), vec4<f32>(0.0), 0.0, 0.0, 0.0, vec4<f32>(0.0), vec4<f32>(0.0));
}

/// `base` with the series' style slot of `part`, then the box's override row
/// of that part.
fn box_part_style(base: ResolvedBarStyle, part: u32, row: u32) -> ResolvedBarStyle {
    var out = base;
    if (part < bar_style_meta.style_count) {
        out = apply_bar_style_slot(out, bar_style_slots[part]);
    }
    return resolve_bar_style(out, row * BOX_PLOT_PARTS + part);
}

/// The body's style: fill, outline and material from the `Style` lanes (the
/// pick binds no `Style` and passes `category_geometry_style`), the group
/// layout from the params, the corner radius, the emphasis and the notch
/// depth (`finish.z`, a share of the box thickness) from the body slot.
fn box_body_style(base: ResolvedBarStyle, row: u32) -> ResolvedBarStyle {
    return box_part_style(base, BOX_PART_BODY, row);
}

fn box_strokes_style(row: u32) -> ResolvedBarStyle {
    return box_part_style(box_empty_style(), BOX_PART_STROKES, row);
}

fn box_markers_style(row: u32) -> ResolvedBarStyle {
    return box_part_style(box_empty_style(), BOX_PART_MARKERS, row);
}

// ───── Body ─────

/// The body in chart-area render pixels: its rect (the box from q1 to q3),
/// its corner radii and, for a notched box, its notch in the box's own
/// (across, along) frame: `(low CI, median, high CI, depth)`, depth 0 for a
/// plain box. `quad` is what the body may paint: the rect, grown to the
/// confidence interval when a notch reaches past the box.
struct BoxBody {
    valid: bool,
    rect: vec4<f32>,
    radii: vec4<f32>,
    notch: vec4<f32>,
    quad: vec4<f32>,
};

fn box_body(s: BoxStats, place: BoxLayout, resolved: ResolvedBarStyle) -> BoxBody {
    var out: BoxBody;
    out.valid = false;
    let a = box_along(s.position, s.q_low);
    let b = box_along(s.position, s.q_high);
    if (!bar_finite(a) || !bar_finite(b) || a == b) {
        return out;
    }
    out.rect = box_rect(place.cross0, place.cross1, a, b);
    let extent = abs(b - a);
    let r = min(min(max(resolved.finish.x, 0.0), place.thickness * 0.5), extent * 0.5);
    out.radii = vec4<f32>(r);
    out.notch = vec4<f32>(0.0);
    out.quad = out.rect;
    let depth = clamp(resolved.finish.z, 0.0, 0.5) * place.thickness;
    if (depth > 0.0 && s.has_ci) {
        let c0 = box_along(s.position, s.ci_low);
        let c1 = box_along(s.position, s.ci_high);
        let m = box_along(s.position, s.median);
        if (bar_finite(c0) && bar_finite(c1) && bar_finite(m)) {
            out.notch = vec4<f32>(min(c0, c1), m, max(c0, c1), depth);
            out.quad = box_rect(
                place.cross0,
                place.cross1,
                min(min(a, b), out.notch.x),
                max(max(a, b), out.notch.z),
            );
        }
    }
    out.valid = true;
    return out;
}

/// The signed distance from `p` to the body in pixels: the notched outline
/// (as the former box plot shader's polygon), else the rounded rect.
fn box_body_distance(p: vec2<f32>, body: BoxBody) -> f32 {
    if (body.notch.w > 0.0) {
        let horizontal = category_params.horizontal != 0u;
        let point = select(p, p.yx, horizontal);
        let r4 = select(body.rect, body.rect.yxwz, horizontal);
        let l = r4.x;
        let a = r4.y;
        let r = r4.z;
        let z = r4.w;
        let low = body.notch.x;
        let mid = body.notch.y;
        let high = body.notch.z;
        let depth = body.notch.w;
        let vertices = array<vec2<f32>, 10>(
            vec2<f32>(l, a), vec2<f32>(r, a), vec2<f32>(r, low), vec2<f32>(r - depth, mid),
            vec2<f32>(r, high), vec2<f32>(r, z), vec2<f32>(l, z), vec2<f32>(l, high),
            vec2<f32>(l + depth, mid), vec2<f32>(l, low),
        );
        var inside = false;
        var distance = 1e30;
        for (var i = 0u; i < 10u; i = i + 1u) {
            let va = vertices[i];
            let vb = vertices[(i + 1u) % 10u];
            let edge = vb - va;
            let q = point - va;
            let t = clamp(dot(q, edge) / max(dot(edge, edge), 1e-20), 0.0, 1.0);
            distance = min(distance, length(q - edge * t));
            if ((va.y > point.y) != (vb.y > point.y)) {
                if (point.x < va.x + (point.y - va.y) * edge.x / edge.y) {
                    inside = !inside;
                }
            }
        }
        return select(distance, -distance, inside);
    }
    let shape = Bar(
        body.rect,
        body.radii,
        vec4<f32>(0.0),
        Material(vec4<f32>(0.0), vec4<f32>(0.0)),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    );
    return sdf(p, shape);
}

/// Four subpixel samples of the body: `(coverage, outline coverage)`.
fn box_body_coverage(p: vec2<f32>, body: BoxBody, band: f32) -> vec2<f32> {
    var coverage = 0.0;
    var outline = 0.0;
    for (var y = 0u; y < 2u; y = y + 1u) {
        for (var x = 0u; x < 2u; x = x + 1u) {
            let d = box_body_distance(p + (vec2<f32>(f32(x), f32(y)) + 0.5) * 0.5 - 0.5, body);
            coverage = coverage + select(0.0, 0.25, d <= 0.0);
            outline = outline + select(0.0, 0.25, d <= 0.0 && d >= -band && band > 0.0);
        }
    }
    return vec2<f32>(coverage, outline);
}

// ───── Strokes ─────

/// Stroke quad `quad` (a `BOX_QUAD_*` stem, cap or the median) as the error
/// bar's segment: data-space ends at the category position, pixel offsets
/// across to the box's middle, expanded by half its stroke width.
struct BoxStroke {
    enabled: bool,
    segment: ErrorBarSegment,
};

fn box_stroke(quad: u32, s: BoxStats, place: BoxLayout, body: BoxBody, strokes: ResolvedBarStyle) -> BoxStroke {
    var out: BoxStroke;
    out.enabled = false;
    let horizontal = category_params.horizontal != 0u;
    // Pixel directions, y up as the segment's offsets are: across the
    // category axis (the box's thickness) and along the value axis.
    let across = select(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, -1.0), horizontal);
    let along = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), horizontal);
    let offset = across * place.offset;
    let whisker_half = max(strokes.gap_px, 0.0) * 0.5;
    let median_half = max(strokes.border_width_px, 0.0) * 0.5;
    var a = s.low;
    var b = s.q_low;
    var cap_half = 0.0;
    var half = whisker_half;
    var cap = false;
    if (quad == BOX_QUAD_LOW_STEM) {
        out.enabled = whisker_half > 0.0 && box_sum(s.low) < box_sum(s.q_low);
    } else if (quad == BOX_QUAD_HIGH_STEM) {
        a = s.q_high;
        b = s.high;
        out.enabled = whisker_half > 0.0 && box_sum(s.high) > box_sum(s.q_high);
    } else if (quad == BOX_QUAD_LOW_CAP || quad == BOX_QUAD_HIGH_CAP) {
        a = select(s.high, s.low, quad == BOX_QUAD_LOW_CAP);
        b = a;
        cap = true;
        cap_half = place.thickness * clamp(strokes.width_ratio, 0.0, 1.0) * 0.5;
        out.enabled = whisker_half > 0.0 && cap_half > 0.0;
    } else {
        // The median spans the box, inside the notch when it has one.
        a = s.median;
        b = a;
        cap = true;
        half = median_half;
        cap_half = max(place.thickness * 0.5 - select(0.0, body.notch.w, body.valid), 0.0);
        out.enabled = median_half > 0.0;
    }
    var point_a = DataPoint(s.position, a);
    var point_b = DataPoint(s.position, b);
    if (horizontal) {
        point_a = DataPoint(a, s.position);
        point_b = DataPoint(b, s.position);
    }
    out.segment = ErrorBarSegment(
        point_a,
        point_b,
        offset - select(vec2<f32>(0.0), across * cap_half, cap),
        offset + select(vec2<f32>(0.0), across * cap_half, cap),
        select(across, along, cap),
        half,
    );
    return out;
}

/// The stroke's quad in chart-area render pixels.
fn box_stroke_rect(stroke: BoxStroke) -> vec4<f32> {
    let g = stroke.segment;
    let p0 = category_px(errorbar_corner_ndc(transform, g, true, -g.half_stroke));
    let p1 = category_px(errorbar_corner_ndc(transform, g, false, g.half_stroke));
    return vec4<f32>(min(p0, p1), max(p0, p1));
}

/// The distance from `p` to a rect, 0 inside.
fn box_rect_distance(p: vec2<f32>, rect: vec4<f32>) -> f32 {
    let half = (rect.zw - rect.xy) * 0.5;
    let q = abs(p - (rect.xy + rect.zw) * 0.5) - half;
    return length(max(q, vec2<f32>(0.0)));
}

// ───── Markers ─────

/// A marker of the box: its centre in chart-area render pixels, its radius,
/// its `marker.wgsl` base shape and whether it is filled.
struct BoxMarker {
    valid: bool,
    centre: vec2<f32>,
    radius: f32,
    shape: u32,
    filled: bool,
};

fn box_marker_at(s: BoxStats, place: BoxLayout, v: vec2<f32>, radius: f32, shape: u32, filled: bool) -> BoxMarker {
    var out: BoxMarker;
    let along = box_along(s.position, v);
    out.valid = radius > 0.0 && bar_finite(along);
    out.centre = box_point(place.middle, along);
    out.radius = radius;
    out.shape = shape;
    out.filled = filled;
    return out;
}

/// The mean: a filled diamond whose half diagonal is half the mean size.
fn box_mean(s: BoxStats, place: BoxLayout, markers: ResolvedBarStyle) -> BoxMarker {
    var out = box_marker_at(s, place, s.mean, max(markers.material.x, 0.0) * 0.5, BOX_MARKER_DIAMOND, true);
    out.valid = out.valid && s.has_mean;
    return out;
}

/// The outliers of group `row` of the ragged column: `(first value, count)`.
fn box_outlier_range(row: u32) -> vec2<u32> {
    let values = box_lane(BOX_LANE_OUTLIER_VALUES);
    let offsets = box_lane(BOX_LANE_OUTLIER_OFFSETS);
    if (values == BOX_LANE_ABSENT || offsets == BOX_LANE_ABSENT || row >= box_lane(BOX_LANE_OUTLIER_GROUPS)) {
        return vec2<u32>(0u);
    }
    let start = category_pair(offsets, row);
    let end = category_pair(offsets, row + 1u);
    let first = u32(start.x + start.y);
    let last = u32(end.x + end.y);
    return vec2<u32>(first, select(0u, last - first, last > first));
}

/// Outlier `k` of the box: a circle or square of half the outlier size.
fn box_outlier(s: BoxStats, place: BoxLayout, markers: ResolvedBarStyle, row: u32, k: u32) -> BoxMarker {
    var out: BoxMarker;
    out.valid = false;
    let range = box_outlier_range(row);
    if (k >= range.y) {
        return out;
    }
    let v = category_pair(box_lane(BOX_LANE_OUTLIER_VALUES), range.x + k);
    if (!category_finite(v)) {
        return out;
    }
    let shape = select(BOX_MARKER_CIRCLE, BOX_MARKER_SQUARE, markers.material.z > 0.5);
    return box_marker_at(s, place, v, max(markers.material.y, 0.0) * 0.5, shape, markers.material.w > 0.5);
}

/// The marker's hit distance: 0 where it paints at full coverage.
fn box_marker_distance(p: vec2<f32>, m: BoxMarker) -> f32 {
    return max(marker_edge(shape_distance(m.shape, p - m.centre, m.radius, m.filled), m.filled), 0.0);
}

// ───── Draw ─────

struct BoxVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    /// The body's rect, or a marker's (centre x, centre y, radius, shape).
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) outline_color: vec4<f32>,
    @location(5) @interpolate(flat) material: vec4<f32>,
    /// x = outline width, y = emphasis, z = grain seed, w = marker filled.
    @location(6) @interpolate(flat) metrics: vec4<f32>,
    @location(7) @interpolate(flat) notch: vec4<f32>,
    /// `BOX_DRAW_*`.
    @location(8) @interpolate(flat) draws: u32,
};

fn box_vs_empty() -> BoxVsOut {
    var out: BoxVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.rect = vec4<f32>(0.0);
    out.radii = vec4<f32>(0.0);
    out.fill = vec4<f32>(0.0);
    out.outline_color = vec4<f32>(0.0);
    out.material = vec4<f32>(0.0);
    out.metrics = vec4<f32>(0.0);
    out.notch = vec4<f32>(0.0);
    out.draws = BOX_DRAW_STROKE;
    return out;
}

/// Corner `vi` of a pixel rect quad, one pixel past `rect` for the edge's
/// analytic coverage.
fn box_quad_corner(vi: u32, rect: vec4<f32>, margin: f32) -> vec2<f32> {
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[vi % 6u];
    let lo = rect.xy - vec2<f32>(margin);
    let hi = rect.zw + vec2<f32>(margin);
    return vec2<f32>(select(lo.x, hi.x, (corner & 1u) != 0u), select(lo.y, hi.y, corner >= 2u));
}

fn box_px_to_clip(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x * transform.pixel_to_ndc.x - 1.0, 1.0 - p.y * transform.pixel_to_ndc.y, 0.0, 1.0);
}

/// A body vertex: the box's quad with its fill, outline, material and notch.
fn box_body_vertex(vi: u32, row: u32, s: BoxStats, place: BoxLayout, body: BoxBody, resolved: ResolvedBarStyle) -> BoxVsOut {
    var out = box_vs_empty();
    out.draws = BOX_DRAW_BODY;
    out.rect = body.rect;
    out.radii = body.radii;
    out.notch = body.notch;
    out.fill = resolved.fill_color_premul;
    out.outline_color = resolved.border_color_premul;
    out.material = resolved.material;
    let seed = (category_params.seed ^ (row * 2654435761u)) % 4096u;
    out.metrics = vec4<f32>(resolved.border_width_px, resolved.finish.y, f32(seed), 0.0);
    if (!body.valid) {
        return out;
    }
    let p = box_quad_corner(vi, body.quad, 1.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

fn box_marker_vertex(vi: u32, m: BoxMarker, color: vec4<f32>) -> BoxVsOut {
    var out = box_vs_empty();
    out.draws = BOX_DRAW_MARKER;
    out.rect = vec4<f32>(m.centre, m.radius, f32(m.shape));
    out.fill = color;
    out.metrics = vec4<f32>(0.0, 0.0, 0.0, select(0.0, 1.0, m.filled));
    if (!m.valid) {
        return out;
    }
    let reach = m.radius + QUAD_MARGIN_PX;
    let p = box_quad_corner(vi, vec4<f32>(m.centre - vec2<f32>(reach), m.centre + vec2<f32>(reach)), 0.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

@vertex
fn vs_box_plot(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> BoxVsOut {
    let quad = vi / BOX_PLOT_QUAD_VERTICES;
    let s = box_stats(row);
    if (!s.valid) {
        return box_vs_empty();
    }
    let resolved = box_body_style(category_base_style(), row);
    let place = box_layout(s, resolved);
    if (!place.valid) {
        return box_vs_empty();
    }
    let body = box_body(s, place, resolved);
    if (quad == BOX_QUAD_BODY) {
        return box_body_vertex(vi, row, s, place, body, resolved);
    }
    if (quad == BOX_QUAD_MEAN || quad >= BOX_QUAD_OUTLIERS) {
        let markers = box_markers_style(row);
        if (quad == BOX_QUAD_MEAN) {
            return box_marker_vertex(vi, box_mean(s, place, markers), markers.fill_color_premul);
        }
        return box_marker_vertex(
            vi,
            box_outlier(s, place, markers, row, quad - BOX_QUAD_OUTLIERS),
            markers.border_color_premul,
        );
    }
    let strokes = box_strokes_style(row);
    let stroke = box_stroke(quad, s, place, body, strokes);
    var out = box_vs_empty();
    out.fill = select(strokes.border_color_premul, strokes.fill_color_premul, quad == BOX_QUAD_MEDIAN);
    if (!stroke.enabled) {
        return out;
    }
    let corner = errorbar_corner(vi);
    let g = stroke.segment;
    let ndc = errorbar_corner_ndc(transform, g, corner.at_a, corner.side * g.half_stroke);
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.local = category_px(ndc);
    return out;
}

/// The body as the fragment's flat inputs carry it.
fn box_fragment_body(in: BoxVsOut) -> BoxBody {
    return BoxBody(true, in.rect, in.radii, in.notch, in.rect);
}

fn box_body_color(in: BoxVsOut) -> vec4<f32> {
    let body = box_fragment_body(in);
    let p = in.local;
    let cover = box_body_coverage(p, body, in.metrics.x);
    if (cover.x <= 0.0) {
        return vec4<f32>(0.0);
    }
    let fill_alpha = in.fill.a;
    // The main draw never brightens: the picked box's emphasis is the
    // selection pass's.
    let shaded = Bar(
        in.rect,
        in.radii,
        vec4<f32>(in.fill.rgb / max(fill_alpha, 1e-6), fill_alpha),
        Material(
            vec4<f32>(in.material.x, CATEGORY_BAR_ROUGHNESS, in.material.y, in.material.z),
            vec4<f32>(in.material.w, in.metrics.z, 0.0, 0.0),
        ),
        vec4<f32>(select(0.0, 1.0, in.metrics.x > 0.0), in.metrics.x, f32(category_params.horizontal), 0.0),
        in.outline_color,
        vec4<f32>(0.0, category_params.scale, 0.0, 0.0),
    );
    // Each covered sample is the premultiplied lit fill; a sample in the
    // outline band has the outline over it, as the former box plot shader
    // composed them.
    let fill = vec4<f32>(bar_surface(p, shaded) * fill_alpha, fill_alpha);
    let ring = in.outline_color + fill * (1.0 - in.outline_color.a);
    return fill * (cover.x - cover.y) + ring * cover.y;
}

@fragment
fn fs_box_plot(in: BoxVsOut) -> @location(0) vec4<f32> {
    // The marker's anti-aliasing width, taken before the branches: a
    // derivative needs uniform control flow.
    let filled = in.metrics.w > 0.5;
    let d = shape_distance(u32(in.rect.w), in.local - in.rect.xy, in.rect.z, filled);
    let aa = fwidth(d);
    if (in.draws == BOX_DRAW_BODY) {
        let c = box_body_color(in);
        if (c.a <= 0.0) {
            discard;
        }
        return c;
    }
    if (in.draws == BOX_DRAW_MARKER) {
        let alpha = 1.0 - smoothstep(-aa, aa, marker_edge(d, filled));
        if (alpha <= 0.0) {
            discard;
        }
        return in.fill * alpha;
    }
    // A stroke is a flat quad, as an error bar is.
    return in.fill;
}

// ───── Selection ─────
//
// The picked box's highlight: an inside ring around the body in the
// selection's colour (`style.color_premul`) and width (`style.line_width_px`),
// over the box's emphasis brightness, as a picked category bar's.

@vertex
fn vs_box_plot_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> BoxVsOut {
    let s = box_stats(row);
    if (!s.valid) {
        return box_vs_empty();
    }
    let resolved = box_body_style(category_base_style(), row);
    let place = box_layout(s, resolved);
    if (!place.valid) {
        return box_vs_empty();
    }
    var out = box_body_vertex(vi, row, s, place, box_body(s, place, resolved), resolved);
    out.fill = style.color_premul;
    out.metrics.x = max(style.line_width_px, 0.0);
    return out;
}

@fragment
fn fs_box_plot_selection(in: BoxVsOut) -> @location(0) vec4<f32> {
    let cover = box_body_coverage(in.local, box_fragment_body(in), in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
    }
    let ring = in.fill * cover.y;
    let brighten = vec3<f32>(max(in.metrics.y, 0.0) * max(cover.x - cover.y, 0.0));
    return vec4<f32>(ring.rgb + brighten, ring.a);
}

// ───── Exact data pick (compute entry) ─────
//
// The box under the cursor: the nearest part of any box within the pick
// distance — the body, a whisker, a cap or the median (the error bar's
// segment quads), the mean or an outlier (the marker shapes) — from the same
// functions the draw builds them with. Every part answers its box, as a
// histogram bin whose index is the row; an outlier is a candidate of its
// box, not an item of its own.

fn box_pick_row(row: u32) -> DataPickCandidate {
    let s = box_stats(row);
    if (!s.valid) {
        return data_pick_invalid();
    }
    let resolved = box_body_style(category_geometry_style(), row);
    let place = box_layout(s, resolved);
    if (!place.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    var best = 1e30;
    let body = box_body(s, place, resolved);
    if (body.valid) {
        best = min(best, max(box_body_distance(cursor, body), 0.0));
    }
    let strokes = box_strokes_style(row);
    for (var quad = BOX_QUAD_LOW_STEM; quad <= BOX_QUAD_MEDIAN; quad = quad + 1u) {
        if (quad == BOX_QUAD_BODY) {
            continue;
        }
        let stroke = box_stroke(quad, s, place, body, strokes);
        if (stroke.enabled) {
            best = min(best, box_rect_distance(cursor, box_stroke_rect(stroke)));
        }
    }
    let markers = box_markers_style(row);
    let mean = box_mean(s, place, markers);
    if (mean.valid) {
        best = min(best, box_marker_distance(cursor, mean));
    }
    let outliers = box_outlier_range(row).y;
    for (var k = 0u; k < outliers; k = k + 1u) {
        let m = box_outlier(s, place, markers, row, k);
        if (m.valid) {
            best = min(best, box_marker_distance(cursor, m));
        }
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
fn pick_box_plot(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var row = local_index;
    while (row < category_params.count) {
        let candidate = box_pick_row(row);
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
