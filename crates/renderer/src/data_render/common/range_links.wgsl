// Range link input layer of the bar module (SHADER_COMMON.md §5.18, #357):
// the dependency links of a `DataRenderType::RangeLinks` series, spliced
// after `range_bars.wgsl`, whose bars (`range_bar`) they join. A link series
// binds the group 3 of the range bar series it names (its params, lane
// table and slot layout), so `range_bar(row)` is exactly the bar that
// series draws; three lanes past the bars' own hold the link columns.
//
// One instance is one link: the elbow from the bar of row `from[i]` to the
// bar of row `to[i]`, routed on every draw from those two bars (out of the
// predecessor's end, across to the successor's row, into its start, with an
// arrowhead), in the series' colour (`Style`) and the stroke width, head
// and lead-out its lane table carries in render pixels, so the draw, the
// highlight and the pick (which binds no `Style`) read one geometry. A backward link (the
// successor starts before the predecessor's end plus two lead-outs) takes an
// S route through the midline between the rows. The route is appearance: it
// reads the values the bars are drawn from and computes none the user reads
// as data. Nothing per link is stored.

/// Lane table entries past `range_bars.wgsl`'s: the f32 lane bases of the
/// predecessor and successor row columns, the link count, and the stroke
/// width, head and lead-out in render pixels (f32 bits).
const RANGE_LANE_LINK_FROM: u32 = 8u;
const RANGE_LANE_LINK_TO: u32 = 9u;
const RANGE_LANE_LINK_COUNT: u32 = 10u;
const RANGE_LANE_LINK_WIDTH: u32 = 11u;
const RANGE_LANE_LINK_HEAD: u32 = 12u;
const RANGE_LANE_LINK_LEAD: u32 = 13u;

fn range_link_width_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_WIDTH)), 0.0);
}

fn range_link_head_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_HEAD)), 0.0);
}

fn range_link_lead_px() -> f32 {
    return max(bitcast<f32>(range_lane(RANGE_LANE_LINK_LEAD)), 0.0);
}

/// Segments of one route, a quad each, then the arrowhead's quad.
const RANGE_LINK_SEGMENTS: u32 = 5u;
/// Vertices per link. Must match `bar.rs`'s `RANGE_LINK_VERTICES`.
const RANGE_LINK_VERTICES: u32 = 36u;

/// One routed link in chart-area render pixels, y down: up to five
/// axis-aligned segments (`points[0..=n]`) and the arrowhead's tip.
struct RangeLinkRoute {
    valid: bool,
    /// Points used: 4 (three segments) or 6 (five segments).
    n: u32,
    p0: vec2<f32>,
    p1: vec2<f32>,
    p2: vec2<f32>,
    p3: vec2<f32>,
    p4: vec2<f32>,
    p5: vec2<f32>,
    /// The direction the head points in, a unit vector.
    dir: vec2<f32>,
};

fn range_link_count() -> u32 {
    return range_lane(RANGE_LANE_LINK_COUNT);
}

/// Row `i`'s bar row in column `k`, or `false` for a NaN, negative or
/// fractional-garbage row.
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

/// The middle of a bar across its slot.
fn range_link_across(bar: RangeBar) -> f32 {
    if (category_params.horizontal != 0u) {
        return 0.5 * (bar.rect.y + bar.rect.w);
    }
    return 0.5 * (bar.rect.x + bar.rect.z);
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
    let lead = range_link_lead_px();
    // The predecessor is left the way it runs (start to end), the successor
    // entered the way it runs.
    let s_a = select(-1.0, 1.0, a.end_px >= a.start_px);
    let s_b = select(-1.0, 1.0, b.end_px >= b.start_px);
    let a_c = range_link_across(a);
    let b_c = range_link_across(b);
    let out_along = a.end_px + s_a * lead;
    let in_along = b.start_px - s_b * lead;
    out.p0 = range_link_px(a.end_px, a_c);
    out.p1 = range_link_px(out_along, a_c);
    if ((in_along - out_along) * s_b >= 0.0) {
        // Forward: down at the lead-out, into the successor's start.
        out.n = 4u;
        out.p2 = range_link_px(out_along, b_c);
        out.p3 = range_link_px(b.start_px, b_c);
        out.p4 = out.p3;
        out.p5 = out.p3;
    } else {
        // Backward: the S route through the midline between the rows.
        var mid = 0.5 * (a_c + b_c);
        if (abs(a_c - b_c) < 1.0) {
            mid = a_c + abs(a.rect.w - a.rect.y + a.rect.z - a.rect.x);
        }
        out.n = 6u;
        out.p2 = range_link_px(out_along, mid);
        out.p3 = range_link_px(in_along, mid);
        out.p4 = range_link_px(in_along, b_c);
        out.p5 = range_link_px(b.start_px, b_c);
    }
    out.dir = range_link_px(s_b, 0.0);
    out.valid = true;
    return out;
}

fn range_link_point(r: RangeLinkRoute, k: u32) -> vec2<f32> {
    switch (k) {
        case 0u: { return r.p0; }
        case 1u: { return r.p1; }
        case 2u: { return r.p2; }
        case 3u: { return r.p3; }
        case 4u: { return r.p4; }
        default: { return r.p5; }
    }
}

/// The tip of the route: its last point.
fn range_link_tip(r: RangeLinkRoute) -> vec2<f32> {
    return range_link_point(r, r.n - 1u);
}

/// Segment `k` of the route, its last one stopping at the head's base so
/// the stroke never pokes past the tip; `ok == false` past the route.
struct RangeLinkSegment {
    ok: bool,
    a: vec2<f32>,
    b: vec2<f32>,
};

fn range_link_segment(r: RangeLinkRoute, k: u32) -> RangeLinkSegment {
    if (!r.valid || k + 1u >= r.n) {
        return RangeLinkSegment(false, vec2<f32>(0.0), vec2<f32>(0.0));
    }
    let a = range_link_point(r, k);
    var b = range_link_point(r, k + 1u);
    if (k + 2u == r.n) {
        let back = min(range_link_head_px(), length(b - a));
        b = b - r.dir * back;
    }
    return RangeLinkSegment(true, a, b);
}

/// The distance from `p` to the segment `a`–`b`.
fn range_link_segment_distance(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    return length(p - (a + ab * t));
}

/// The head's triangle: the tip and its two base corners.
fn range_link_head(r: RangeLinkRoute) -> array<vec2<f32>, 3> {
    let tip = range_link_tip(r);
    let h = range_link_head_px();
    let n = vec2<f32>(-r.dir.y, r.dir.x);
    let base = tip - r.dir * h;
    return array<vec2<f32>, 3>(tip, base + n * (h * 0.5), base - n * (h * 0.5));
}

/// The signed distance from `p` to the triangle `t`, negative inside.
fn range_link_triangle_distance(p: vec2<f32>, t: array<vec2<f32>, 3>) -> f32 {
    var d = -1e9;
    var inside = true;
    var best = 1e9;
    for (var k = 0u; k < 3u; k = k + 1u) {
        let a = t[k];
        let b = t[(k + 1u) % 3u];
        best = min(best, range_link_segment_distance(p, a, b));
        let e = b - a;
        let c = (p.x - a.x) * e.y - (p.y - a.y) * e.x;
        let o = t[(k + 2u) % 3u];
        let co = (o.x - a.x) * e.y - (o.y - a.y) * e.x;
        if (c * co < 0.0) {
            inside = false;
        }
    }
    return select(best, -best, inside);
}

struct RangeLinkVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    /// x = 0 for a segment, 1 for the head; y = the stroke's half width.
    @location(1) @interpolate(flat) kind: vec2<f32>,
    /// A segment's ends, or the head's tip and first base corner.
    @location(2) @interpolate(flat) ab: vec4<f32>,
    /// The head's second base corner.
    @location(3) @interpolate(flat) c: vec2<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
};

fn range_link_empty() -> RangeLinkVsOut {
    var out: RangeLinkVsOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.kind = vec2<f32>(0.0);
    out.ab = vec4<f32>(0.0);
    out.c = vec2<f32>(0.0);
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

/// Vertex `vi` of link `i`, stroked `half` pixels either side in `color`.
fn range_link_vertex(vi: u32, i: u32, half: f32, color: vec4<f32>) -> RangeLinkVsOut {
    let r = range_link_route(i);
    if (!r.valid) {
        return range_link_empty();
    }
    var out = range_link_empty();
    out.color = color;
    let part = vi / 6u;
    let pad = half + 1.0;
    if (part < RANGE_LINK_SEGMENTS) {
        let s = range_link_segment(r, part);
        if (!s.ok) {
            return range_link_empty();
        }
        out.kind = vec2<f32>(0.0, half);
        out.ab = vec4<f32>(s.a, s.b);
        let p = range_link_corner(vi, min(s.a, s.b) - vec2<f32>(pad), max(s.a, s.b) + vec2<f32>(pad));
        out.local = p;
        out.pos = box_px_to_clip(p);
        return out;
    }
    if (range_link_head_px() <= 0.0) {
        return range_link_empty();
    }
    let t = range_link_head(r);
    out.kind = vec2<f32>(1.0, half);
    out.ab = vec4<f32>(t[0], t[1]);
    out.c = t[2];
    let lo = min(min(t[0], t[1]), t[2]) - vec2<f32>(pad);
    let hi = max(max(t[0], t[1]), t[2]) + vec2<f32>(pad);
    let p = range_link_corner(vi, lo, hi);
    out.local = p;
    out.pos = box_px_to_clip(p);
    return out;
}

/// A link fragment's coverage: within half the width of its segment, or
/// inside its head (antialiased over one pixel).
fn range_link_coverage(in: RangeLinkVsOut) -> f32 {
    if (in.kind.x < 0.5) {
        let d = range_link_segment_distance(in.local, in.ab.xy, in.ab.zw);
        return clamp(in.kind.y - d + 0.5, 0.0, 1.0);
    }
    let t = array<vec2<f32>, 3>(in.ab.xy, in.ab.zw, in.c);
    return clamp(0.5 - range_link_triangle_distance(in.local, t), 0.0, 1.0);
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
// The link under the cursor: the nearest segment or head of any link within
// half its stroke plus the pick distance, from the same route the draw
// builds. A link answers as a histogram bin whose index is its row.

fn range_link_pick(i: u32) -> DataPickCandidate {
    let r = range_link_route(i);
    if (!r.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    var best = 1e9;
    for (var k = 0u; k < RANGE_LINK_SEGMENTS; k = k + 1u) {
        let s = range_link_segment(r, k);
        if (s.ok) {
            best = min(best, range_link_segment_distance(cursor, s.a, s.b));
        }
    }
    if (range_link_head_px() > 0.0) {
        best = min(best, max(range_link_triangle_distance(cursor, range_link_head(r)), 0.0));
    }
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
