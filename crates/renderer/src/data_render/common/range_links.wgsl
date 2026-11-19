// Range link input layer of the bar module (SHADER_COMMON.md §5.18, #357):
// the dependency links of a `DataRenderType::RangeLinks` series, spliced
// after `range_bars.wgsl`, whose bars (`range_bar`) they join. A link series
// binds the group 3 of the range bar series it names (its params, lane
// table and slot layout), so `range_bar(row)` is exactly the bar that
// series draws; six lanes past the bars' own hold the link columns and the
// link's look.
//
// One instance is one link, routed on every draw from its two bars in the
// web Gantt convention: out of the predecessor's edge facing the successor's
// row, a small inset from the predecessor's start, straight across to the
// successor's centre line, one rounded corner, then along that line into the
// successor's start, ending in an open chevron just before it. When the
// successor starts before that exit, the exit moves back to leave room for
// the head, so a route never turns back through a bar. Links leaving one bar
// share their trunk. The stroke width, head size and inset come from the
// lane table in render pixels, so the draw, the highlight and the pick
// (which binds no `Style`) read one geometry. The route is appearance: it
// reads the values the bars are drawn from and computes none the user reads
// as data. Nothing per link is stored.

/// Lane table entries past `range_bars.wgsl`'s: the f32 lane bases of the
/// predecessor and successor row columns, the link count, and the stroke
/// width, head length and exit inset in render pixels (f32 bits).
const RANGE_LANE_LINK_FROM: u32 = 8u;
const RANGE_LANE_LINK_TO: u32 = 9u;
const RANGE_LANE_LINK_COUNT: u32 = 10u;
const RANGE_LANE_LINK_WIDTH: u32 = 11u;
const RANGE_LANE_LINK_HEAD: u32 = 12u;
const RANGE_LANE_LINK_INSET: u32 = 13u;

/// The corner radius of a route in logical pixels (`range_link_scale`
/// turns it into render pixels).
const RANGE_LINK_CORNER_PX: f32 = 4.0;
/// The chevron's half opening, as a fraction of its length.
const RANGE_LINK_CHEVRON_SPREAD: f32 = 0.7;
/// Vertices per link: one quad over the whole route. Must match `bar.rs`'s
/// `RANGE_LINK_VERTICES`.
const RANGE_LINK_VERTICES: u32 = 6u;

fn range_link_width_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_WIDTH)), 0.0);
}

fn range_link_head_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_HEAD)), 0.0);
}

fn range_link_inset_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_INSET)), 0.0);
}

/// One routed link in chart-area render pixels, y down: the exit on the
/// predecessor's edge, the corner on the successor's centre line, the tip
/// just before the successor's start, and the unit directions along the
/// last leg (`dir`) and across the first (`down`).
struct RangeLinkRoute {
    valid: bool,
    exit: vec2<f32>,
    corner: vec2<f32>,
    tip: vec2<f32>,
    dir: vec2<f32>,
    down: vec2<f32>,
    /// The corner's radius, fitted to both legs.
    radius: f32,
};

fn range_link_count() -> u32 {
    return range_lane(RANGE_LANE_LINK_COUNT);
}

/// Row `i`'s bar row in column `k`, with `y == 0` for a NaN, negative or
/// out-of-range row.
fn range_link_row(k: u32, i: u32) -> vec2<u32> {
    let v = box_value(k, i);
    if (!v.ok) {
        return vec2<u32>(0u, 0u);
    }
    let r = round(box_sum(v.pair));
    if (!(r >= 0.0) || r >= f32(category_params.count)) {
        return vec2<u32>(0u, 0u);
    }
    return vec2<u32>(u32(r), 1u);
}

/// `(along, across)` to chart-area pixels: `along` runs on the value axis.
fn range_link_px(along: f32, across: f32) -> vec2<f32> {
    if (category_params.horizontal != 0u) {
        return vec2<f32>(along, across);
    }
    return vec2<f32>(across, along);
}

/// A bar's extent across its slot, `(low, high)`.
fn range_link_band(bar: RangeBar) -> vec2<f32> {
    if (category_params.horizontal != 0u) {
        return vec2<f32>(min(bar.rect.y, bar.rect.w), max(bar.rect.y, bar.rect.w));
    }
    return vec2<f32>(min(bar.rect.x, bar.rect.z), max(bar.rect.x, bar.rect.z));
}

/// Link `i`'s route from the two bars the series draws, or invalid when
/// either row or bar is missing.
fn range_link_route(i: u32) -> RangeLinkRoute {
    var out: RangeLinkRoute;
    out.valid = false;
    if (i >= range_link_count()) {
        return out;
    }
    let src = range_link_row(RANGE_LANE_LINK_FROM, i);
    let dst = range_link_row(RANGE_LANE_LINK_TO, i);
    if (src.y == 0u || dst.y == 0u) {
        return out;
    }
    let geometry = category_geometry_style();
    let a = range_bar(src.x, geometry);
    let b = range_bar(dst.x, geometry);
    if (!a.valid || !b.valid) {
        return out;
    }
    // Work in `u`, the value axis signed so that the successor runs toward
    // +u (start to end), and `c`, across the slots.
    let s = select(-1.0, 1.0, b.end_px >= b.start_px);
    let a_band = range_link_band(a);
    let b_band = range_link_band(b);
    let a_mid = 0.5 * (a_band.x + a_band.y);
    let b_mid = 0.5 * (b_band.x + b_band.y);
    let head = range_link_head_px();
    let gap = 1.5;
    let tip_u = b.start_px * s - gap;
    let a_first = min(a.start_px * s, a.end_px * s);
    // The exit: a small inset from the predecessor's first end, moved back
    // when the successor starts too soon for the last leg and its head.
    var exit_u = min(a_first + range_link_inset_px(), tip_u - head - 4.0);
    var down = 1.0;
    var exit_c = a_band.y;
    if (b_mid < a_mid) {
        down = -1.0;
        exit_c = a_band.x;
    }
    if (abs(b_mid - a_mid) < 0.5) {
        // Both bars share a centre line (tasks on one row): a straight
        // link out of the predecessor's last end, so no stroke runs inside
        // a bar whose unfinished part is translucent.
        exit_c = b_mid;
        let a_last = max(a.start_px * s, a.end_px * s);
        exit_u = min(a_last, tip_u - head - 4.0);
    }
    out.exit = range_link_px(exit_u * s, exit_c);
    out.corner = range_link_px(exit_u * s, b_mid);
    out.tip = range_link_px(tip_u * s, b_mid);
    out.dir = range_link_px(s, 0.0);
    out.down = range_link_px(0.0, down);
    let first = abs(b_mid - exit_c);
    let last = tip_u - exit_u;
    out.radius = max(min(RANGE_LINK_CORNER_PX * range_link_scale(), min(first, last)), 0.0);
    out.valid = true;
    return out;
}

/// Render pixels per logical pixel, read from the stroke the CPU scaled:
/// the corner radius follows the export scale with the stroke.
fn range_link_scale() -> f32 {
    return max(category_params.scale, 1e-3);
}

/// The distance from `p` to the segment `a`–`b`.
fn range_link_segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    return length(p - (a + ab * t));
}

/// The chevron's two arm ends behind the tip.
fn range_link_arms(r: RangeLinkRoute) -> array<vec2<f32>, 2> {
    let h = range_link_head_px();
    let n = vec2<f32>(-r.dir.y, r.dir.x);
    let base = r.tip - r.dir * h;
    let spread = n * (h * RANGE_LINK_CHEVRON_SPREAD);
    return array<vec2<f32>, 2>(base + spread, base - spread);
}

/// The distance from `p` to the route's centre line: the first leg, the
/// rounded corner, the last leg and the chevron's arms.
fn range_link_distance(r: RangeLinkRoute, p: vec2<f32>) -> f32 {
    let start = r.corner - r.down * r.radius;
    let end = r.corner + r.dir * r.radius;
    var d = min(
        range_link_segment_distance(p, r.exit, start),
        range_link_segment_distance(p, end, r.tip),
    );
    if (r.radius > 0.0) {
        // The quarter arc from `start` to `end` about its centre.
        let centre = start + r.dir * r.radius;
        let q = p - centre;
        if (dot(q, -r.dir) >= 0.0 && dot(q, r.down) >= 0.0) {
            d = min(d, abs(length(q) - r.radius));
        } else {
            d = min(d, min(length(p - start), length(p - end)));
        }
    }
    if (range_link_head_px() > 0.0) {
        let arms = range_link_arms(r);
        d = min(d, range_link_segment_distance(p, r.tip, arms[0]));
        d = min(d, range_link_segment_distance(p, r.tip, arms[1]));
    }
    return d;
}

struct RangeLinkVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    /// The link's index and the stroke's half width.
    @location(1) @interpolate(flat) link: vec2<f32>,
    @location(2) @interpolate(flat) color: vec4<f32>,
};

fn range_link_empty() -> RangeLinkVsOut {
    var out: RangeLinkVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.link = vec2<f32>(0.0);
    out.color = vec4<f32>(0.0);
    return out;
}

/// Corner `vi` (of six) of the box `lo`..`hi`.
fn range_link_corner(vi: u32, lo: vec2<f32>, hi: vec2<f32>) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    return mix(lo, hi, corners[vi % 6u]);
}

/// Vertex `vi` of link `i`: one quad over its whole route, stroked `half`
/// pixels either side in `color`.
fn range_link_vertex(vi: u32, i: u32, half: f32, color: vec4<f32>) -> RangeLinkVsOut {
    let r = range_link_route(i);
    if (!r.valid) {
        return range_link_empty();
    }
    var out = range_link_empty();
    out.color = color;
    out.link = vec2<f32>(f32(i), half);
    let arms = range_link_arms(r);
    let pad = vec2<f32>(half + 1.0);
    let lo = min(min(min(r.exit, r.tip), min(arms[0], arms[1])), r.corner) - pad;
    let hi = max(max(max(r.exit, r.tip), max(arms[0], arms[1])), r.corner) + pad;
    let p = range_link_corner(vi, lo, hi);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

/// A link fragment's coverage: within half the stroke of its centre line,
/// antialiased over one pixel. The route is rebuilt per fragment from the
/// two bars, so nothing per link is interpolated but its index.
fn range_link_coverage(in: RangeLinkVsOut) -> f32 {
    let r = range_link_route(u32(in.link.x + 0.5));
    if (!r.valid) {
        return 0.0;
    }
    let d = range_link_distance(r, in.local);
    return clamp(in.link.y - d + 0.5, 0.0, 1.0);
}

@vertex
fn vs_range_links(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> RangeLinkVsOut {
    return range_link_vertex(vi, i, range_link_width_px() * 0.5, style.color_premul);
}

@fragment
fn fs_range_links(in: RangeLinkVsOut) -> @location(0) vec4<f32> {
    let cover = range_link_coverage(in);
    if (cover <= 0.0) {
        discard;
    }
    return in.color * cover;
}

// ───── Selection ─────
//
// The picked link's highlight: the same route in the selection's colour
// (`style.color_premul` of the selection style), its stroke widened by the
// selection's width (`style.line_width_px`) on either side of the link's
// own.

@vertex
fn vs_range_link_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> RangeLinkVsOut {
    let half = range_link_width_px() * 0.5 + max(style.line_width_px, 0.0);
    return range_link_vertex(vi, i, half, style.color_premul);
}

@fragment
fn fs_range_link_selection(in: RangeLinkVsOut) -> @location(0) vec4<f32> {
    let cover = range_link_coverage(in);
    if (cover <= 0.0) {
        discard;
    }
    return in.color * cover;
}

// ───── Exact data pick (compute entry) ─────
//
// The link under the cursor: the nearest route (legs, corner or chevron) within
// half its stroke plus the pick distance, from the same route the draw
// builds. A link answers as a histogram bin whose index is its row.

fn range_link_pick(i: u32) -> DataPickCandidate {
    let r = range_link_route(i);
    if (!r.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    var best = range_link_distance(r, cursor);
    best = max(best - range_link_width_px() * 0.5, 0.0);
    if (!data_pick_finite(best)
        || !data_pick_finite(data_pick_query.limits.x)
        || best > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        i,
        0u,
        0u,
        best,
        i,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_range_links(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var i = local_index;
    let count = range_link_count();
    while (i < count) {
        let candidate = range_link_pick(i);
        if (data_pick_better(candidate, best)) {
            best = candidate;
        }
        i = i + DATA_PICK_WORKGROUP_SIZE;
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
