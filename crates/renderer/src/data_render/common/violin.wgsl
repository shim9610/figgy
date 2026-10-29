// Violin and raw point input layer of the bar module (SHADER_COMMON.md
// §5.19): a `DataRenderType::BoxPlot` series' violin bodies and raw points,
// spliced into the module after `box_plot.wgsl`, whose slot layout
// (`box_slot`), value reads, quads and markers it shares, and
// `category_bars.wgsl`, whose group 3 it binds; the part colours are the
// series' bar style slots (`bar_columnar.wgsl`) and a box override's rows
// restyle its category's violin (D7).
//
// A violin half is the host's density curve: a ragged column with one group
// of value-axis positions per category, ascending, and a plain density
// column index-aligned with its values. One instance is one curve segment
// (point `s` to `s + 1` of a half), so a long curve costs only its own
// segments: the instance finds its category by a binary search over the
// group offsets and draws a quad around the trapezoid the two half widths
// span. A pixel belongs to the segment whose value range holds it, half-open,
// so the segments tile without seams; the outline is the distance to the
// segment's edge and its neighbours'. The renderer scales the density by
// the half's `density_scale` to the half slot and estimates nothing.
//
// A raw point is one instance of two plain columns (its category position
// and its value): a marker at its member's middle, jittered across it by
// `category_jitter_cross`, the function the point pick places it with.

// The violin draw's lane table: the number of halves, the segments of the
// first half (instances past them are the second half's), then
// `VIOLIN_HALF_LANES` per half.
const VIOLIN_LANE_HALVES: u32 = 0u;
const VIOLIN_LANE_FIRST_SEGMENTS: u32 = 1u;
const VIOLIN_HALF_BASE: u32 = 2u;
const VIOLIN_HALF_LANES: u32 = 7u;
// Per half: its side, the curve's values and offsets lanes, its group
// count, its value count, the density lane and the density scale (`f32`
// bits).
const VIOLIN_HALF_SIDE: u32 = 0u;
const VIOLIN_HALF_VALUES: u32 = 1u;
const VIOLIN_HALF_OFFSETS: u32 = 2u;
const VIOLIN_HALF_GROUPS: u32 = 3u;
const VIOLIN_HALF_COUNT: u32 = 4u;
const VIOLIN_HALF_DENSITY: u32 = 5u;
const VIOLIN_HALF_SCALE: u32 = 6u;

// `VIOLIN_HALF_SIDE`: a symmetric body, or the side below or above the
// category position.
const VIOLIN_SIDE_BOTH: u32 = 0u;
const VIOLIN_SIDE_LOW: u32 = 1u;
const VIOLIN_SIDE_HIGH: u32 = 2u;

/// Vertices per violin segment and per raw point: one quad.
const VIOLIN_VERTICES: u32 = 6u;

// The raw point draw's lane table: the values lane.
const BOX_POINTS_LANE_VALUES: u32 = 0u;

fn violin_lane(h: u32, k: u32) -> u32 {
    return category_lanes[VIOLIN_HALF_BASE + h * VIOLIN_HALF_LANES + k];
}

/// The group of half `h` that value `s` of its curve belongs to: the last
/// group whose first offset is at most `s`, by a binary search over the
/// offsets (`log2` of the groups steps). `groups` when there is none.
fn violin_group(h: u32, s: u32) -> u32 {
    let offsets = violin_lane(h, VIOLIN_HALF_OFFSETS);
    let groups = violin_lane(h, VIOLIN_HALF_GROUPS);
    if (groups == 0u || box_ragged_offset(offsets, 0u) > s) {
        return groups;
    }
    var lo = 0u;
    var hi = groups;
    loop {
        if (hi - lo <= 1u) {
            break;
        }
        let mid = lo + (hi - lo) / 2u;
        if (box_ragged_offset(offsets, mid) <= s) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if (box_ragged_offset(offsets, lo + 1u) <= s) {
        return groups;
    }
    return lo;
}

/// One point of a curve, placed: its position along the value axis and its
/// half width across the member, in chart-area render pixels.
struct ViolinPoint {
    ok: bool,
    along: f32,
    width: f32,
};

fn violin_point(h: u32, s: u32, position: vec2<f32>, thickness: f32) -> ViolinPoint {
    let v = category_pair(violin_lane(h, VIOLIN_HALF_VALUES), s);
    let d = category_pair(violin_lane(h, VIOLIN_HALF_DENSITY), s);
    let scale = bitcast<f32>(violin_lane(h, VIOLIN_HALF_SCALE));
    if (!category_finite(v) || !category_finite(d) || !(scale > 0.0)) {
        return ViolinPoint(false, 0.0, 0.0);
    }
    let along = box_along(position, v);
    let width = max(box_sum(d) / scale, 0.0) * thickness * 0.5;
    return ViolinPoint(bar_finite(along) && bar_finite(width), along, width);
}

/// The half and the segment of instance `inst`.
fn violin_instance(inst: u32) -> vec2<u32> {
    let first = category_lanes[VIOLIN_LANE_FIRST_SEGMENTS];
    if (inst < first) {
        return vec2<u32>(0u, inst);
    }
    return vec2<u32>(1u, inst - first);
}

/// The side sign of half `h` across the category axis in render pixels: +1
/// or -1 for a one-sided half (low is towards smaller category values:
/// left of a vertical violin, below a horizontal one), 0 for a symmetric
/// body.
fn violin_side_sign(h: u32) -> f32 {
    let side = violin_lane(h, VIOLIN_HALF_SIDE);
    if (side == VIOLIN_SIDE_BOTH) {
        return 0.0;
    }
    let low = select(-1.0, 1.0, category_params.horizontal != 0u);
    return select(-low, low, side == VIOLIN_SIDE_LOW);
}

/// Segment `s` of half `h` as the draw and the highlight see it.
struct ViolinSegment {
    valid: bool,
    group: u32,
    place: BoxLayout,
    a: ViolinPoint,
    b: ViolinPoint,
    /// The neighbours' far points (`ok` false at a group's end).
    prev: ViolinPoint,
    next: ViolinPoint,
};

fn violin_segment(h: u32, s: u32, resolved: ResolvedBarStyle) -> ViolinSegment {
    var out: ViolinSegment;
    out.valid = false;
    let g = violin_group(h, s);
    let groups = violin_lane(h, VIOLIN_HALF_GROUPS);
    if (g >= groups || g >= category_params.count) {
        return out;
    }
    let offsets = violin_lane(h, VIOLIN_HALF_OFFSETS);
    let first = box_ragged_offset(offsets, g);
    let end = box_ragged_offset(offsets, g + 1u);
    if (s + 1u >= end) {
        return out;
    }
    let position = category_pair(category_params.positions, g);
    if (!category_finite(position)) {
        return out;
    }
    let at = category_pair(violin_lane(h, VIOLIN_HALF_VALUES), s);
    let place = box_slot(position, at, resolved);
    if (!place.valid) {
        return out;
    }
    out.group = g;
    out.place = place;
    out.a = violin_point(h, s, position, place.thickness);
    out.b = violin_point(h, s + 1u, position, place.thickness);
    out.prev = ViolinPoint(false, 0.0, 0.0);
    out.next = ViolinPoint(false, 0.0, 0.0);
    if (s > first) {
        out.prev = violin_point(h, s - 1u, position, place.thickness);
    }
    if (s + 2u < end) {
        out.next = violin_point(h, s + 2u, position, place.thickness);
    }
    out.valid = out.a.ok && out.b.ok;
    return out;
}

/// The half's style: the series' base, the half's slot (its fill, outline
/// colour and width) and the category's box override rows over them.
fn violin_style(h: u32, row: u32) -> ResolvedBarStyle {
    return violin_style_over(category_base_style(), h, row);
}

/// `violin_style` over `base`: the pick's, which binds no `Style`, starts
/// from the geometry alone (`category_geometry_style`).
fn violin_style_over(base_in: ResolvedBarStyle, h: u32, row: u32) -> ResolvedBarStyle {
    var base = base_in;
    if (h < bar_style_meta.style_count) {
        base = apply_bar_style_slot(base, bar_style_slots[h]);
    }
    return resolve_bar_style(base, row);
}

// ───── Draw ─────

struct ViolinVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    /// The segment: `(along a, width a, along b, width b)`.
    @location(1) @interpolate(flat) segment: vec4<f32>,
    /// The neighbours: `(along prev, width prev, along next, width next)`.
    @location(2) @interpolate(flat) neighbours: vec4<f32>,
    /// x = the member's middle across the category axis, y = the side sign,
    /// z = 1 with a previous point, w = 1 with a next one.
    @location(3) @interpolate(flat) frame: vec4<f32>,
    @location(4) @interpolate(flat) fill: vec4<f32>,
    @location(5) @interpolate(flat) outline_color: vec4<f32>,
    /// x = outline width, y = emphasis.
    @location(6) @interpolate(flat) metrics: vec4<f32>,
};

fn violin_vs_empty() -> ViolinVsOut {
    var out: ViolinVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.segment = vec4<f32>(0.0);
    out.neighbours = vec4<f32>(0.0);
    out.frame = vec4<f32>(0.0);
    out.fill = vec4<f32>(0.0);
    out.outline_color = vec4<f32>(0.0);
    out.metrics = vec4<f32>(0.0);
    return out;
}

/// A segment's quad: the trapezoid, its neighbours' widths and the outline
/// past them, one pixel further for the edge's coverage; at a group's end
/// the outline's reach past the last point too.
fn violin_vertex(vi: u32, h: u32, seg: ViolinSegment, resolved: ResolvedBarStyle, outline: f32) -> ViolinVsOut {
    var out = violin_vs_empty();
    out.fill = resolved.fill_color_premul;
    out.outline_color = resolved.border_color_premul;
    out.metrics = vec4<f32>(outline, resolved.finish.y, 0.0, 0.0);
    let sign = violin_side_sign(h);
    out.segment = vec4<f32>(seg.a.along, seg.a.width, seg.b.along, seg.b.width);
    out.neighbours = vec4<f32>(seg.prev.along, seg.prev.width, seg.next.along, seg.next.width);
    out.frame = vec4<f32>(seg.place.middle, sign, select(0.0, 1.0, seg.prev.ok), select(0.0, 1.0, seg.next.ok));
    var reach = max(seg.a.width, seg.b.width);
    if (seg.prev.ok) {
        reach = max(reach, seg.prev.width);
    }
    if (seg.next.ok) {
        reach = max(reach, seg.next.width);
    }
    let margin = outline * 0.5 + 1.0;
    reach = reach + margin;
    var c0 = seg.place.middle - reach;
    var c1 = seg.place.middle + reach;
    if (sign > 0.0) {
        c0 = seg.place.middle - margin;
    } else if (sign < 0.0) {
        c1 = seg.place.middle + margin;
    }
    // One pixel past the segment's own value range, so the pixel at each
    // end is covered whole by the segment that owns its centre (the
    // others discard it): under multisampling every sample of the pixel
    // then belongs to its owner, with no seam. A group's ends draw the
    // outline past their last point.
    let a0 = min(seg.a.along, seg.b.along) - margin;
    let a1 = max(seg.a.along, seg.b.along) + margin;
    let p = box_quad_corner(vi, box_rect(c0, c1, a0, a1), 0.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

@vertex
fn vs_violin(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> ViolinVsOut {
    let at = violin_instance(inst);
    if (at.x >= category_lanes[VIOLIN_LANE_HALVES]) {
        return violin_vs_empty();
    }
    // The geometry first, for the group whose overrides style it.
    let seg0 = violin_segment(at.x, at.y, category_geometry_style());
    if (!seg0.valid) {
        return violin_vs_empty();
    }
    let resolved = violin_style(at.x, seg0.group);
    let seg = violin_segment(at.x, at.y, resolved);
    if (!seg.valid) {
        return violin_vs_empty();
    }
    return violin_vertex(vi, at.x, seg, resolved, max(resolved.border_width_px, 0.0));
}

/// The distance from `q` to the segment from `a` to `b`, all `(along,
/// across)` pixels.
fn violin_edge_distance(q: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let len2 = dot(ab, ab);
    var t = 0.0;
    if (len2 > 0.0) {
        t = clamp(dot(q - a, ab) / len2, 0.0, 1.0);
    }
    return length(q - (a + ab * t));
}

/// The coverage of the segment's fill (x) and of the outline (y) at `p`, the
/// outline `width` wide, centred on the edge; both 0 outside the pixels the
/// segment owns.
fn violin_coverage(p: vec2<f32>, in: ViolinVsOut, width: f32) -> vec2<f32> {
    let horizontal = category_params.horizontal != 0u;
    // Ownership is decided at the pixel's centre, snapped, so two segments
    // sharing a boundary agree on it whatever their interpolation error.
    let along = floor(select(p.y, p.x, horizontal)) + 0.5;
    let cross = select(p.x, p.y, horizontal);
    let a = in.segment.xy;
    let b = in.segment.zw;
    let lo = min(a.x, b.x);
    let hi = max(a.x, b.x);
    let has_prev = in.frame.z > 0.5;
    let has_next = in.frame.w > 0.5;
    // Which end is the group's: the end without a neighbour.
    let a_end = !has_prev;
    let b_end = !has_next;
    let lo_end = select(b_end, a_end, a.x <= b.x);
    let hi_end = select(a_end, b_end, a.x <= b.x);
    let inside_range = along >= lo && along < hi;
    let margin = width * 0.5 + 1.0;
    let in_extension = (lo_end && along < lo && along >= lo - margin) || (hi_end && along >= hi && along < hi + margin);
    if (!inside_range && !in_extension) {
        return vec2<f32>(0.0);
    }
    let sign = in.frame.y;
    var u = cross - in.frame.x;
    if (sign != 0.0) {
        u = u * sign;
    } else {
        u = abs(u);
    }
    // The fill: inside the trapezoid, measured across the slanted edge.
    var fill = 0.0;
    if (inside_range) {
        let span = b.x - a.x;
        var t = 0.0;
        if (span != 0.0) {
            t = clamp((along - a.x) / span, 0.0, 1.0);
        }
        let w = mix(a.y, b.y, t);
        let slope = select(0.0, (b.y - a.y) / span, span != 0.0);
        let d = (u - w) / sqrt(1.0 + slope * slope);
        fill = clamp(0.5 - d, 0.0, 1.0);
        if (sign != 0.0) {
            // A half's straight side, at the middle.
            fill = min(fill, clamp(u + 0.5, 0.0, 1.0));
        }
    }
    if (!(width > 0.0)) {
        return vec2<f32>(fill, 0.0);
    }
    // The outline: the nearest of this segment's edge and its neighbours',
    // the caps at a group's ends, and a half's straight side.
    let q = vec2<f32>(along, u);
    var d = violin_edge_distance(q, a, b);
    if (has_prev) {
        d = min(d, violin_edge_distance(q, in.neighbours.xy, a));
    } else {
        d = min(d, violin_edge_distance(q, vec2<f32>(a.x, select(-a.y, 0.0, sign != 0.0)), a));
    }
    if (has_next) {
        d = min(d, violin_edge_distance(q, b, in.neighbours.zw));
    } else {
        d = min(d, violin_edge_distance(q, vec2<f32>(b.x, select(-b.y, 0.0, sign != 0.0)), b));
    }
    if (sign != 0.0 && inside_range) {
        d = min(d, abs(u));
    }
    let ring = clamp(width * 0.5 + 0.5 - d, 0.0, 1.0);
    return vec2<f32>(max(fill, ring), ring);
}

@fragment
fn fs_violin(in: ViolinVsOut) -> @location(0) vec4<f32> {
    let cover = violin_coverage(in.local, in, in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
    }
    let fill = in.fill * max(cover.x - cover.y, 0.0);
    let ring = in.outline_color + in.fill * (1.0 - in.outline_color.a);
    return fill + ring * cover.y;
}

// ───── Violin selection ─────
//
// The picked violin's highlight: the outline in the selection's colour
// (`style.color_premul`) and width (`style.line_width_px`), over the body's
// emphasis brightness. The pass draws every segment of the series and
// keeps those of the selected category (`style.dash_len`), and of the
// selected half (`style.primitive_flags`: 0 every half, `h + 1` half `h`).

@vertex
fn vs_violin_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) inst: u32) -> ViolinVsOut {
    let at = violin_instance(inst);
    if (at.x >= category_lanes[VIOLIN_LANE_HALVES]) {
        return violin_vs_empty();
    }
    let half = style.primitive_flags;
    if (half != 0u && half != at.x + 1u) {
        return violin_vs_empty();
    }
    let resolved = violin_style(at.x, style.dash_len);
    let seg = violin_segment(at.x, at.y, resolved);
    if (!seg.valid || seg.group != style.dash_len) {
        return violin_vs_empty();
    }
    var out = violin_vertex(vi, at.x, seg, resolved, max(style.line_width_px, 0.0));
    out.fill = style.color_premul;
    return out;
}

@fragment
fn fs_violin_selection(in: ViolinVsOut) -> @location(0) vec4<f32> {
    let cover = violin_coverage(in.local, in, in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
    }
    let ring = in.fill * cover.y;
    let brighten = vec3<f32>(max(in.metrics.y, 0.0) * max(cover.x - cover.y, 0.0));
    return vec4<f32>(ring.rgb + brighten, ring.a);
}

// ───── Violin data pick (compute entry) ─────
//
// The violin under the cursor: one test per category and half. The
// cursor's position along the value axis selects the group's segment by a
// binary search over its values (ascending, so their pixels are monotone),
// and the cursor's offset from the middle is compared with that segment's
// interpolated half width, the outline's half width added. Past the
// curve's ends the distance is to the nearer end. A hit answers the
// category as a histogram bin, with the half in `index1` (0 for a
// symmetric body, 1 + the half's index for a split one).

fn violin_pick_half(h: u32, g: u32, cursor: vec2<f32>) -> f32 {
    let resolved = violin_style_over(category_geometry_style(), h, g);
    let offsets = violin_lane(h, VIOLIN_HALF_OFFSETS);
    if (g >= violin_lane(h, VIOLIN_HALF_GROUPS)) {
        return 1e30;
    }
    let first = box_ragged_offset(offsets, g);
    let end = box_ragged_offset(offsets, g + 1u);
    if (end < first + 2u) {
        return 1e30;
    }
    let position = category_pair(category_params.positions, g);
    if (!category_finite(position)) {
        return 1e30;
    }
    let place = box_slot(position, category_pair(violin_lane(h, VIOLIN_HALF_VALUES), first), resolved);
    if (!place.valid) {
        return 1e30;
    }
    let horizontal = category_params.horizontal != 0u;
    let along = select(cursor.y, cursor.x, horizontal);
    let sign = violin_side_sign(h);
    var u = select(cursor.x, cursor.y, horizontal) - place.middle;
    if (sign != 0.0) {
        u = u * sign;
    } else {
        u = abs(u);
    }
    let p_first = violin_point(h, first, position, place.thickness);
    let p_last = violin_point(h, end - 1u, position, place.thickness);
    if (!p_first.ok || !p_last.ok) {
        return 1e30;
    }
    let rising = p_last.along > p_first.along;
    // The last segment start whose along is not past the cursor.
    var lo = first;
    var hi = end - 1u;
    loop {
        if (hi - lo <= 1u) {
            break;
        }
        let mid = lo + (hi - lo) / 2u;
        let m = violin_point(h, mid, position, place.thickness);
        let before = select(m.along >= along, m.along <= along, rising);
        if (m.ok && before) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let a = violin_point(h, lo, position, place.thickness);
    let b = violin_point(h, lo + 1u, position, place.thickness);
    if (!a.ok || !b.ok) {
        return 1e30;
    }
    let outline = max(resolved.border_width_px, 0.0) * 0.5;
    let span = b.along - a.along;
    var t = 0.0;
    if (span != 0.0) {
        t = (along - a.along) / span;
    }
    var past = 0.0;
    if (t < 0.0 || t > 1.0) {
        // Past an end of the curve: the distance along to it.
        let end_along = select(a.along, b.along, t > 1.0);
        past = abs(along - end_along);
        t = clamp(t, 0.0, 1.0);
    }
    let w = mix(a.width, b.width, t);
    var across = max(u - w, 0.0);
    if (sign != 0.0) {
        across = max(across, -u);
    }
    return max(length(vec2<f32>(past, across)) - outline, 0.0);
}

fn violin_pick_row(g: u32) -> DataPickCandidate {
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    let halves = category_lanes[VIOLIN_LANE_HALVES];
    var best = 1e30;
    var best_half = 0u;
    for (var h = 0u; h < halves; h = h + 1u) {
        let d = violin_pick_half(h, g, cursor);
        // A later half paints over an earlier one: it wins a tie.
        if (d <= best) {
            best = d;
            best_half = h;
        }
    }
    if (!data_pick_finite(best)
        || !data_pick_finite(data_pick_query.limits.x)
        || best > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    let symmetric = halves == 1u && violin_lane(0u, VIOLIN_HALF_SIDE) == VIOLIN_SIDE_BOTH;
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        g,
        select(best_half + 1u, 0u, symmetric),
        0u,
        best,
        g,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_violin(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var g = local_index;
    while (g < category_params.count) {
        let candidate = violin_pick_row(g);
        if (data_pick_better(candidate, best)) {
            best = candidate;
        }
        g = g + DATA_PICK_WORKGROUP_SIZE;
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

// ───── Raw points ─────
//
// The draw's params are the box group's layout with `positions` the
// points' category position column and `count` the points; the lane table
// holds the values lane, and style slot 0 the colour (fill), the radius
// (finish.x, render pixels), the jitter (finish.y) and offset (finish.z)
// shares and the shape (finish.w: `marker.wgsl` base shape id, + 100 when
// filled).

struct BoxRawPoint {
    valid: bool,
    centre: vec2<f32>,
    radius: f32,
    shape: u32,
    filled: bool,
};

fn box_points_slot() -> BarStyleSlot {
    if (bar_style_meta.style_count > 0u) {
        return bar_style_slots[0];
    }
    return BarStyleSlot(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

/// Raw point `i`: its marker at its member's middle, jittered.
fn box_raw_point(i: u32) -> BoxRawPoint {
    var out: BoxRawPoint;
    out.valid = false;
    if (i >= category_params.count) {
        return out;
    }
    let position = category_pair(category_params.positions, i);
    let value = category_pair(category_lanes[BOX_POINTS_LANE_VALUES], i);
    if (!category_finite(position) || !category_finite(value)) {
        return out;
    }
    let slot = box_points_slot();
    let place = box_slot(position, value, category_geometry_style());
    let along = box_along(position, value);
    if (!place.valid || !bar_finite(along) || !(slot.finish.x > 0.0)) {
        return out;
    }
    let cross = category_jitter_cross(place.middle, place.thickness, slot.finish.z, slot.finish.y, i, category_params.seed, category_params.horizontal != 0u);
    out.centre = box_point(cross, along);
    out.radius = slot.finish.x;
    let code = u32(round(max(slot.finish.w, 0.0)));
    out.filled = code >= 100u;
    out.shape = code % 100u;
    out.valid = bar_finite(cross);
    return out;
}

struct BoxPointVsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    /// `(centre x, centre y, radius, shape)`.
    @location(1) @interpolate(flat) marker: vec4<f32>,
    @location(2) @interpolate(flat) fill: vec4<f32>,
    /// x = 1 when filled, y = the ring width of a highlight (0 in the draw).
    @location(3) @interpolate(flat) metrics: vec4<f32>,
};

fn box_point_vertex(vi: u32, m: BoxRawPoint, color: vec4<f32>, ring: f32) -> BoxPointVsOut {
    var out: BoxPointVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.marker = vec4<f32>(m.centre, m.radius, f32(m.shape));
    out.fill = color;
    out.metrics = vec4<f32>(select(0.0, 1.0, m.filled), ring, 0.0, 0.0);
    if (!m.valid) {
        return out;
    }
    let reach = m.radius * 1.6 + ring + QUAD_MARGIN_PX;
    let p = box_quad_corner(vi, vec4<f32>(m.centre - vec2<f32>(reach), m.centre + vec2<f32>(reach)), 0.0);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

@vertex
fn vs_box_points(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> BoxPointVsOut {
    return box_point_vertex(vi, box_raw_point(i), box_points_slot().fill_color_premul, 0.0);
}

/// The marker's signed distance at `p`, the scatter shader's rule (the
/// shape's visual radius), so the point pick measures the same marker.
fn box_point_distance(in: BoxPointVsOut) -> f32 {
    let shape = u32(in.marker.w);
    let filled = in.metrics.x > 0.5;
    // Marker frames run y up, render pixels down.
    let local = (in.local - in.marker.xy) * vec2<f32>(1.0, -1.0);
    return shape_distance(shape, local, visual_shape_radius(shape, in.marker.z), filled);
}

@fragment
fn fs_box_points(in: BoxPointVsOut) -> @location(0) vec4<f32> {
    let filled = in.metrics.x > 0.5;
    let d = box_point_distance(in);
    let aa = fwidth(d);
    let alpha = 1.0 - smoothstep(-aa, aa, marker_edge(d, filled));
    if (alpha <= 0.0) {
        discard;
    }
    return in.fill * alpha;
}

// ───── Raw point selection ─────
//
// The picked point's highlight: a ring of the selection's width
// (`style.line_width_px`) and colour around its marker, the instance the
// selected point.

@vertex
fn vs_box_point_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> BoxPointVsOut {
    return box_point_vertex(vi, box_raw_point(i), style.color_premul, max(style.line_width_px, 0.0));
}

@fragment
fn fs_box_point_selection(in: BoxPointVsOut) -> @location(0) vec4<f32> {
    let d = box_point_distance(in);
    let aa = max(fwidth(d), 1e-4);
    // Outside the drawn marker, `ring` pixels wide.
    let outer = 1.0 - smoothstep(-aa, aa, d - in.metrics.y);
    let inner = smoothstep(-aa, aa, d);
    let alpha = outer * inner;
    if (alpha <= 0.0) {
        discard;
    }
    return in.fill * alpha;
}
