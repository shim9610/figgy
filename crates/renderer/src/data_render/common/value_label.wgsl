// Value labels of the bar module (SHADER_COMMON.md §5.17): the drawn string
// of each category of a category bar, box plot or radar series, one label
// quad per category, placed from the pool on every draw by the series' own
// input fragment and sampled from the series' label atlas through the
// shared label quad (`label_quad.wgsl`). Spliced into the bar module after
// `category_bars.wgsl` and `box_plot.wgsl`, whose geometry it reads:
//
// - a bar's label sits at its value end (`category_bar`'s `end_px`), outside
//   or inside the bar as its placement says;
// - a box's label sits beside its median line, past the side of its slot
//   away from its group's centre (`box_stats`, `box_slot`); a violin
//   without a box labels past the top of its curves;
// - a radar vertex's label sits next to its vertex (#272): outside it along
//   its spoke, a gap past the marker, and only where it would touch another
//   labelled radar series' label at the same spoke (or the ring labels)
//   inside the vertex or beside it (`value_label_radar`). The vertex is the
//   pair `(category, value)` of the series' columns through the polar
//   projection, read through the category bar group 3 (`positions` the
//   category column, `category_lanes[k]` the value column of the label
//   group's member `k`, this series' at its rank).
//
// The CPU bakes the strings (SSOT configuration, `labels.text`) into the
// atlas with the one text stack and writes each label's content width,
// placement and drag offset into the series' label cells
// (`value_label_cells`, a storage array sized by its labels; a radar
// series' holds a block per member of its label group); nothing per
// label is computed or stored on the CPU from the data. The same placement feeds the
// draw, the selection box (`vs_value_label_selection`) and the pick
// (`pick_value_labels`), so a label is picked and outlined where it is
// drawn. Only radar labels move each other, by the values at their spoke,
// read from the pool on every draw: per-label appearance (rule 1), never
// stored.

/// CPU twin: `data_render/value_label.rs::ValueLabelParamsGpu` (128 B).
struct ValueLabelParams {
    atlas: LabelParams,
    /// x = `VALUE_LABEL_KIND_*`, y = labels (one per category, any
    /// number: the cells of a block of `value_label_cells`); for a radar
    /// series z = its rank among the members of its label group (the panel's
    /// radar series that draw value labels) and w = their count
    /// (`RadarLabelGroup`), each a block of cells; 0 and 1 otherwise.
    shape: vec4<u32>,
    /// x = the gap between the anchor and the label (a radar label's past
    /// its vertex's marker), y = the text baseline's height above the
    /// content's bottom edge, z = the font size, w = render pixels per
    /// logical pixel; all in render pixels but `w`.
    metrics: vec4<f32>,
    /// The selection box colour, premultiplied.
    selection_color: vec4<f32>,
    /// x = the selection box stroke width, y = its padding around the
    /// label's frame, both in render pixels; zw = 0.
    selection: vec4<f32>,
    /// A radar series': x = the visual gap two labels keep (#272), in
    /// render pixels, y = 0; zw = the radius axis' ray, a unit vector y
    /// down.
    radar: vec4<f32>,
    /// A radar series': the box of the drawn ring labels in the ray's frame
    /// from the polar centre, as fractions of the outer radius: (along min,
    /// along max, across min, across max), across toward the ring labels'
    /// side; empty (x > y) without ring labels.
    ring: vec4<f32>,
};

const VALUE_LABEL_KIND_BARS: u32 = 0u;
const VALUE_LABEL_KIND_BOXES: u32 = 1u;
const VALUE_LABEL_KIND_RADAR: u32 = 2u;
const VALUE_LABEL_KIND_RANGE: u32 = 3u;
const VALUE_LABEL_OUTSIDE: u32 = 0u;
const VALUE_LABEL_INSIDE: u32 = 1u;
const VALUE_LABEL_AUTO: u32 = 2u;
/// Vertices per label: one quad.
const VALUE_LABEL_VERTICES: u32 = 6u;
/// The typed pick kind of a value label (`gpu_data_pick.rs`).
const DATA_PICK_KIND_VALUE_LABEL: u32 = 4u;
/// Logical pixels a label inside a bar keeps clear of its sides under
/// `VALUE_LABEL_AUTO`, and the font sizes of bar height it needs.
const VALUE_LABEL_FIT_PAD: f32 = 8.0;
const VALUE_LABEL_FIT_HEIGHT: f32 = 1.4;
/// The text baseline below the middle of what a label is centred on, in
/// font sizes: a bar's middle, a horizontal bar or box's middle.
const VALUE_LABEL_CENTRE_BASELINE: f32 = 0.32;
/// A placement direction component below this is no direction: a radar
/// spoke straight up checks no left or right edge.
const VALUE_LABEL_FLIP_EPSILON: f32 = 0.001;

@group(1) @binding(4) var<uniform> value_label: ValueLabelParams;
@group(1) @binding(5) var value_label_atlas: texture_2d<f32>;
@group(1) @binding(6) var value_label_sampler: sampler;
/// One cell per label, `shape.y` of them in each of `shape.w` blocks, sized
/// by the series' labels and its radar label group (#107 R11): x = the
/// content width in render pixels (0 draws nothing: an empty string, or a
/// label its placement hides), y = the placement (`VALUE_LABEL_OUTSIDE` /
/// `_INSIDE` / `_AUTO`; for a radar label, how far its vertex's marker
/// reaches in render pixels), zw = the drag offset in render pixels, x
/// right and y down. Block `shape.z` is this series'.
@group(1) @binding(7) var<storage, read> value_label_cells: array<vec4<f32>>;

/// Label `row`'s cell of the label group's member `member` (block
/// `member`); this series' labels are member `shape.z`'s.
fn value_label_cell(member: u32, row: u32) -> vec4<f32> {
    return value_label_cells[member * value_label.shape.y + row];
}

/// The content height of a label: the atlas cell's, which every label of
/// a radar label group shares (the members' labels take the radius axis'
/// tick label face).
fn value_label_height() -> f32 {
    return value_label.atlas.cell_stride_h - 2.0 * value_label.atlas.gutter;
}

/// Where one label goes: its content box in chart-area render pixels, y
/// down, `(x0, y0, x1, y1)`, its left and top edge on the pixel grid.
struct ValueLabelBox {
    valid: bool,
    rect: vec4<f32>,
};

fn value_label_hidden() -> ValueLabelBox {
    return ValueLabelBox(false, vec4<f32>(0.0));
}

/// The content box of `size` centred on `centre` and moved by the label's
/// drag offset, its origin snapped to the pixel grid so the atlas is
/// sampled texel for texel.
fn value_label_box_at(row: u32, centre: vec2<f32>, size: vec2<f32>) -> ValueLabelBox {
    let origin = floor(centre + value_label_cell(value_label.shape.z, row).zw - size * 0.5 + vec2<f32>(0.5));
    if (!bar_finite(origin.x) || !bar_finite(origin.y)) {
        return value_label_hidden();
    }
    return ValueLabelBox(true, vec4<f32>(origin, origin + size));
}

/// The centre's y of a content box whose baseline is at `baseline`.
fn value_label_centre_y(baseline: f32, size: vec2<f32>) -> f32 {
    return baseline + value_label.metrics.y - size.y * 0.5;
}

/// The data area in chart-area render pixels, y down, `(x0, y0, x1, y1)`:
/// the transform's data-to-panel affine over the chart area its
/// `pixel_to_ndc` spans.
fn value_label_data_area() -> vec4<f32> {
    let chart = vec2<f32>(2.0) / transform.pixel_to_ndc;
    let lo = transform.data_to_panel_offset;
    let hi = transform.data_to_panel_offset + transform.data_to_panel_scale;
    return vec4<f32>(lo.x * chart.x, chart.y - hi.y * chart.y, hi.x * chart.x, chart.y - lo.y * chart.y);
}

/// Whether a content box of `size` centred on `centre` crosses the data
/// area's edge in the direction `dir` it was placed in from its anchor (y
/// down; a zero component checks no edge on that axis). The flip rule of
/// every kind reads this before the drag offset: a label placed outside its
/// anchor that would cross the edge goes to the other side of the anchor,
/// as the bars' `Auto` rule puts a label inside its bar.
fn value_label_crosses(centre: vec2<f32>, size: vec2<f32>, dir: vec2<f32>) -> bool {
    let area = value_label_data_area();
    let lo = centre - size * 0.5;
    let hi = centre + size * 0.5;
    return (dir.x > VALUE_LABEL_FLIP_EPSILON && hi.x > area.z)
        || (dir.x < -VALUE_LABEL_FLIP_EPSILON && lo.x < area.x)
        || (dir.y > VALUE_LABEL_FLIP_EPSILON && hi.y > area.w)
        || (dir.y < -VALUE_LABEL_FLIP_EPSILON && lo.y < area.y);
}

/// A bar's label: outside its value end, or inside the bar centred on it.
/// `VALUE_LABEL_AUTO` is the former chart's rule: outside grouped bars,
/// inside a stack's segments that fit it, else none. An outside label of a
/// stack segment a later segment covers draws nothing. An outside label
/// that would cross the data area's edge goes inside the bar's end.
fn value_label_bar(row: u32, size: vec2<f32>, placement: u32) -> ValueLabelBox {
    let bar = category_bar(row, resolve_bar_style(category_geometry_style(), row));
    return value_label_bar_at(row, bar, category_params.mode == CATEGORY_BAR_MODE_GROUPED, size, placement);
}

/// A range bar's label: the bar rule of `value_label_bar` at the bar's end,
/// a range bar counting as grouped (outside under `VALUE_LABEL_AUTO`).
fn value_label_range_bar(row: u32, size: vec2<f32>, placement: u32) -> ValueLabelBox {
    let r = range_bar(row, category_geometry_style());
    let bar = CategoryBar(r.valid, r.rect, r.radii, 0u, r.end_px, true, vec4<f32>(0.0), vec4<f32>(0.0));
    return value_label_bar_at(row, bar, true, size, placement);
}

/// The label of row `row` at `bar` (drawn, `grouped` or a stack's segment)
/// as `placement` says.
fn value_label_bar_at(row: u32, bar: CategoryBar, grouped: bool, size: vec2<f32>, placement: u32) -> ValueLabelBox {
    if (!bar.valid) {
        return value_label_hidden();
    }
    if (bar.wedge.z > 0.0) {
        return value_label_wedge(row, bar, grouped, size, placement);
    }
    let horizontal = category_params.horizontal != 0u;
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    let middle = (bar.rect.xy + bar.rect.zw) * 0.5;
    var inside = placement == VALUE_LABEL_INSIDE;
    if (placement == VALUE_LABEL_AUTO) {
        inside = !grouped;
        let pad = VALUE_LABEL_FIT_PAD * value_label.metrics.w;
        if (inside && (size.x + pad > bar.rect.z - bar.rect.x
            || font * VALUE_LABEL_FIT_HEIGHT > bar.rect.w - bar.rect.y)) {
            return value_label_hidden();
        }
    }
    if (inside) {
        let centre = vec2<f32>(middle.x, value_label_centre_y(middle.y + font * VALUE_LABEL_CENTRE_BASELINE, size));
        return value_label_box_at(row, centre, size);
    }
    if (!bar.exposed) {
        return value_label_hidden();
    }
    let end = bar.end_px;
    var centre: vec2<f32>;
    var dir: vec2<f32>;
    if (horizontal) {
        // Past the end, away from the bar's start.
        let right = end >= middle.x;
        dir = vec2<f32>(select(-1.0, 1.0, right), 0.0);
        centre = vec2<f32>(
            value_label_beside(end, gap, size.x, dir.x),
            value_label_centre_y(middle.y + font * VALUE_LABEL_CENTRE_BASELINE, size),
        );
        if (value_label_crosses(centre, size, dir)) {
            // Inside the bar's end instead.
            centre.x = value_label_beside(end, gap, size.x, -dir.x);
        }
    } else {
        // Above an end that rises, below one that falls.
        dir = vec2<f32>(0.0, select(1.0, -1.0, end <= middle.y));
        centre = vec2<f32>(middle.x, value_label_across(end, gap, font, size, dir.y));
        if (value_label_crosses(centre, size, dir)) {
            // Inside the bar's end instead.
            centre.y = value_label_across(end, gap, font, size, -dir.y);
        }
    }
    return value_label_box_at(row, centre, size);
}

/// A rose wedge's label (#133): inside, centred on the wedge's middle
/// radius along its middle direction; outside, past the exposed end (the
/// outer arc of a bar growing outward, the inner arc of one growing
/// inward) along that direction, by the gap and the box's reach along it.
/// `VALUE_LABEL_AUTO` is the bar rule: outside grouped wedges, inside a
/// stack's segments whose radial depth and arc fit the label. An outside
/// label that would cross the data area's edge goes to the other side of
/// the end.
fn value_label_wedge(row: u32, bar: CategoryBar, grouped: bool, size: vec2<f32>, placement: u32) -> ValueLabelBox {
    let centre = bar.wedge.xy;
    let r_outer = bar.wedge.z;
    let r_inner = r_outer * bar.wedge.w;
    let mid = bar.arc.x + bar.arc.y * 0.5;
    let dir = vec2<f32>(cos(mid), sin(mid));
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    let reach = abs(dir.x) * size.x * 0.5 + abs(dir.y) * size.y * 0.5;
    var inside = placement == VALUE_LABEL_INSIDE;
    if (placement == VALUE_LABEL_AUTO) {
        inside = !grouped;
        let pad = VALUE_LABEL_FIT_PAD * value_label.metrics.w;
        let depth = r_outer - r_inner;
        let width = (r_outer + r_inner) * 0.5 * bar.arc.y;
        if (inside && (size.x + pad > max(width, depth) || font * VALUE_LABEL_FIT_HEIGHT > min(width, depth))) {
            return value_label_hidden();
        }
    }
    if (inside) {
        return value_label_box_at(row, centre + dir * (r_outer + r_inner) * 0.5, size);
    }
    if (!bar.exposed) {
        return value_label_hidden();
    }
    let outward = bar.end_px >= r_outer - 0.001;
    let anchor = centre + dir * select(r_inner, r_outer, outward);
    let away = select(-dir, dir, outward);
    var box_centre = anchor + away * (gap + reach);
    if (value_label_crosses(box_centre, size, away)) {
        box_centre = anchor - away * (gap + reach);
    }
    return value_label_box_at(row, box_centre, size);
}

/// The centre's x of a label `gap` beside the anchor `x`, to its right
/// (`side` > 0) or its left.
fn value_label_beside(x: f32, gap: f32, width: f32, side: f32) -> f32 {
    return select(x - gap - width * 0.5, x + gap + width * 0.5, side > 0.0);
}

/// The centre's y of a label beyond the anchor `y` along the value axis:
/// above it (`side` < 0) with its baseline `gap` above, or below it with its
/// text's top about `gap` below, as the former charts placed them.
fn value_label_across(y: f32, gap: f32, font: f32, size: vec2<f32>, side: f32) -> f32 {
    return value_label_centre_y(select(y + gap + font, y - gap, side < 0.0), size);
}

/// A box's label (#272): beside its median line, centred on the median
/// along the value axis and `gap` past the side of the box's slot (its full
/// width, before a violin narrows it), so it never reads as the value of an
/// outlier or a whisker. The side is the one away from its group's centre:
/// a box centred in its slot puts it right of a vertical box and above a
/// horizontal one. When that side would cross the data area's edge it goes
/// to the other side. A violin whose box is hidden labels `gap` past the
/// farther end of its curves, centred across its slot.
fn value_label_box_plot(row: u32, size: vec2<f32>) -> ValueLabelBox {
    let s = box_stats(row);
    let style = box_body_style(category_geometry_style(), row);
    let horizontal = category_params.horizontal != 0u;
    let boxed = box_layout(s, style);
    if (s.valid && boxed.valid) {
        let slot = box_slot(s.position, s.q_low, style);
        let median = box_along(s.position, s.median);
        if (!slot.valid || !bar_finite(median)) {
            return value_label_hidden();
        }
        var side = select(1.0, -1.0, horizontal);
        if (abs(slot.offset) > 0.5) {
            side = sign(slot.offset);
        }
        var centre = value_label_beside_box(slot, median, side, size, horizontal);
        let dir = select(vec2<f32>(side, 0.0), vec2<f32>(0.0, side), horizontal);
        if (value_label_crosses(centre, size, dir)) {
            centre = value_label_beside_box(slot, median, -side, size, horizontal);
        }
        return value_label_box_at(row, centre, size);
    }
    // No box (a violin without one, or hidden): the category's slot, past
    // the farther end of the curves. No top until a violin curve gives one
    // (an explicit flag, not a NaN constant: WGSL refuses a constant that
    // evaluates to NaN).
    let position = category_pair(category_params.positions, row);
    var place = boxed;
    place.valid = false;
    var top = 0.0;
    var has_top = false;
    // Each violin curve's ends: its values ascend, so its first and last
    // values bound it.
    for (var h = 0u; h < 2u; h = h + 1u) {
        let values = box_lane(BOX_LANE_VIOLIN_CURVES + 2u * h);
        let offsets = box_lane(BOX_LANE_VIOLIN_CURVES + 2u * h + 1u);
        if (values == BOX_LANE_ABSENT || offsets == BOX_LANE_ABSENT || !category_finite(position)) {
            continue;
        }
        let first = box_ragged_offset(offsets, row);
        let last = box_ragged_offset(offsets, row + 1u);
        if (last <= first) {
            continue;
        }
        for (var k = 0u; k < 2u; k = k + 1u) {
            let v = category_pair(values, select(first, last - 1u, k == 1u));
            if (!category_finite(v)) {
                continue;
            }
            let along = box_along(position, v);
            top = select(value_label_farther(top, along, horizontal), along, !has_top || !bar_finite(top));
            has_top = true;
            if (!place.valid) {
                place = box_slot(position, v, style);
            }
        }
    }
    if (!place.valid || !has_top || !bar_finite(top)) {
        return value_label_hidden();
    }
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    var centre: vec2<f32>;
    if (horizontal) {
        centre = vec2<f32>(
            value_label_beside(top, gap, size.x, 1.0),
            value_label_centre_y(place.middle + font * VALUE_LABEL_CENTRE_BASELINE, size),
        );
        if (value_label_crosses(centre, size, vec2<f32>(1.0, 0.0))) {
            // Before the top of the curves instead.
            centre.x = value_label_beside(top, gap, size.x, -1.0);
        }
    } else {
        centre = vec2<f32>(place.middle, value_label_across(top, gap, font, size, -1.0));
        if (value_label_crosses(centre, size, vec2<f32>(0.0, -1.0))) {
            // Below the top of the curves instead.
            centre.y = value_label_across(top, gap, font, size, 1.0);
        }
    }
    return value_label_box_at(row, centre, size);
}

/// The centre of a box's label beside its median at pixel `median` along
/// the value axis, `gap` past the side `side` of the box's `slot` (`-1` the
/// left of a vertical box or the top of a horizontal one, `1` the other):
/// beside a vertical box its text is centred on the median line, as a
/// bar's inside label is on the bar's middle; over or under a horizontal
/// box it is centred on the median and placed as a bar's outside label.
fn value_label_beside_box(slot: BoxLayout, median: f32, side: f32, size: vec2<f32>, horizontal: bool) -> vec2<f32> {
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    let edge = select(slot.cross0, slot.cross1, side > 0.0);
    if (horizontal) {
        return vec2<f32>(median, value_label_across(edge, gap, font, size, side));
    }
    return vec2<f32>(
        value_label_beside(edge, gap, size.x, side),
        value_label_centre_y(median + font * VALUE_LABEL_CENTRE_BASELINE, size),
    );
}

/// Of two positions along the value axis, the one farther towards larger
/// values on screen: the smaller y (y down), or the larger x.
fn value_label_farther(a: f32, b: f32, horizontal: bool) -> f32 {
    if (!bar_finite(b)) {
        return a;
    }
    return select(min(a, b), max(a, b), horizontal);
}

/// A radar vertex's label (#272): next to its vertex, its box's near edge
/// `gap` past the vertex's marker, outside, inside or beside it.
///
/// The labels of the series' label group (the panel's radar series that
/// draw value labels, `shape.w` of them) at this label's spoke, the same
/// row of their columns, are placed together in one scan, by their
/// vertices from the outermost (the larger value) inward, equal positions
/// by rank (`value_label_radar_before`). Each takes the first of these
/// positions that keeps the visual gap (`radar.x`) from the two labels
/// placed before it and from the ring labels (`value_label_radar_blocked`):
///
/// 1. its home, outside the vertex along the spoke, `gap` past the marker,
///    or inward along the spoke when outward would cross the outer ring or
///    the data area's edge (the existing flip);
/// 2. the other side of the vertex along the spoke (inside it; none for a
///    flipped label): the smaller value's label of two that would touch;
/// 3. beside the vertex at its radius, counter-clockwise of the spoke, then
///    clockwise.
///
/// A label whose home is free splits with the next label instead when
/// their homes touch and that label cannot clear it on the other side of
/// its vertex: their vertices lie within the visual gap of each other
/// (equal values), the next one is flipped, or its other side still
/// touches. The two then go beside their vertices, one on each side of the
/// spoke: this one on the first free side, the next one on the other.
/// With no free position a label keeps its home: crowding past what four
/// positions next to a vertex hold, which the host resolves.
///
/// Every label of the spoke runs the same scan over the same values, so the
/// labels agree on where each one goes without anything stored; the scan
/// is `shape.w` steps of a search over the `shape.w` members (no cap,
/// rule 10). The drag offset applies after the placement.
fn value_label_radar(row: u32, size: vec2<f32>) -> ValueLabelBox {
    if (row >= category_params.count) {
        return value_label_hidden();
    }
    let x = category_pair(category_params.positions, row);
    if (!category_finite(x)) {
        return value_label_hidden();
    }
    // The spoke from the inner ring to the outer one at the category.
    let inner = category_px(data_to_ndc(transform, x, vec2<f32>(transform.data_min.y, transform.data_min_lo.y)));
    let outer = category_px(data_to_ndc(transform, x, vec2<f32>(transform.data_max.y, transform.data_max_lo.y)));
    let along = outer - inner;
    if (!(dot(along, along) > 0.0) || !bar_finite(inner.x) || !bar_finite(inner.y)) {
        return value_label_hidden();
    }
    let u = normalize(along);
    // y down: `(u.y, -u.x)` is counter-clockwise of `u` on screen.
    let spoke = RadarSpoke(inner, u, vec2<f32>(u.y, -u.x));
    let own = value_label.shape.z;
    var prev1 = value_label_no_rect();
    var prev2 = value_label_no_rect();
    var split = 0.0;
    var cur = value_label_radar_next(value_label_radar_none(), row, x, spoke);
    for (var step = 0u; step < value_label.shape.w; step = step + 1u) {
        if (!cur.valid) {
            break;
        }
        let following = value_label_radar_next(cur, row, x, spoke);
        let placed = value_label_radar_choose(cur, following, split, prev1, prev2, spoke);
        if (cur.member == own) {
            return value_label_box_at(row, placed.xy, size);
        }
        prev2 = prev1;
        prev1 = value_label_rect(placed.xy, cur.size);
        split = placed.z;
        cur = following;
    }
    return value_label_hidden();
}

/// A radar spoke in chart-area render pixels, y down: the inner ring's
/// point, the unit direction outward and the unit direction
/// counter-clockwise of it on screen.
struct RadarSpoke {
    inner: vec2<f32>,
    u: vec2<f32>,
    n: vec2<f32>,
};

/// One member's label at a spoke as the radar placement weighs it: the
/// member, how far out along the spoke its vertex lies (`t`), the vertex,
/// the label's content size and how far it keeps from the vertex (`clear`:
/// the marker's reach and the gap).
struct RadarSpokeLabel {
    valid: bool,
    member: u32,
    t: f32,
    vertex: vec2<f32>,
    size: vec2<f32>,
    clear: f32,
};

fn value_label_radar_none() -> RadarSpokeLabel {
    return RadarSpokeLabel(false, 0u, 0.0, vec2<f32>(0.0), vec2<f32>(0.0), 0.0);
}

/// Member `k`'s label at row `row` of category `x` on `spoke`: none for an
/// empty or hidden label or a vertex that is not drawn.
fn value_label_radar_member(k: u32, row: u32, x: vec2<f32>, spoke: RadarSpoke) -> RadarSpokeLabel {
    let cell = value_label_cell(k, row);
    let size = vec2<f32>(cell.x, value_label_height());
    if (!(size.x > 0.0) || !(size.y > 0.0)) {
        return value_label_radar_none();
    }
    let y = category_value(k, row);
    if (!category_finite(y)) {
        return value_label_radar_none();
    }
    let vertex = category_px(data_to_ndc(transform, x, y));
    let t = dot(vertex - spoke.inner, spoke.u);
    if (!bar_finite(vertex.x) || !bar_finite(vertex.y) || !bar_finite(t)) {
        return value_label_radar_none();
    }
    return RadarSpokeLabel(true, k, t, vertex, size, max(cell.y, 0.0) + value_label.metrics.x);
}

/// Whether `a` comes before `b` in the scan: farther out along the spoke,
/// or as far and of a lower rank.
fn value_label_radar_before(a: RadarSpokeLabel, b: RadarSpokeLabel) -> bool {
    return a.t > b.t || (a.t == b.t && a.member < b.member);
}

/// The first label after `after` in the scan (the first of all when
/// `after` is none): a search over every member.
fn value_label_radar_next(after: RadarSpokeLabel, row: u32, x: vec2<f32>, spoke: RadarSpoke) -> RadarSpokeLabel {
    var best = value_label_radar_none();
    for (var k = 0u; k < value_label.shape.w; k = k + 1u) {
        let l = value_label_radar_member(k, row, x, spoke);
        if (!l.valid || (after.valid && !value_label_radar_before(after, l))) {
            continue;
        }
        if (!best.valid || value_label_radar_before(l, best)) {
            best = l;
        }
    }
    return best;
}

/// The centre of `l`'s label placed along the unit direction `d` from its
/// vertex: the box's near edge `clear` past the vertex.
fn value_label_radar_along(l: RadarSpokeLabel, d: vec2<f32>) -> vec2<f32> {
    return l.vertex + d * (l.clear + value_label_half_across(d, l.size));
}

/// Whether `l`'s label outside its vertex would cross the data area's edge
/// or the outer ring (where the category labels sit): its home is then
/// inward along the spoke.
fn value_label_radar_flips(l: RadarSpokeLabel, spoke: RadarSpoke) -> bool {
    let centre = value_label_radar_along(l, spoke.u);
    return value_label_crosses(centre, l.size, spoke.u) || value_label_past_rim(centre, l.size);
}

/// The centre of `l`'s label beside its vertex on side `side` of the spoke
/// (`1` counter-clockwise, `-1` clockwise), at the vertex's radius, moved
/// inward along the spoke until it lies inside the outer ring.
fn value_label_radar_side(l: RadarSpokeLabel, side: f32, spoke: RadarSpoke) -> vec2<f32> {
    let n = spoke.n * side;
    let centre = l.vertex + n * (l.clear + value_label_half_across(n, l.size));
    return value_label_inside_rim(centre, l.size, spoke.u);
}

/// The box of `size` centred on `centre`, `(x0, y0, x1, y1)`.
fn value_label_rect(centre: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(centre - size * 0.5, centre + size * 0.5);
}

/// A box that touches nothing: no label placed yet.
fn value_label_no_rect() -> vec4<f32> {
    return vec4<f32>(1e30, 1e30, -1e30, -1e30);
}

/// Whether boxes `a` and `b` come closer than the visual gap (`radar.x`)
/// both across and along the screen: they touch to the eye (#272, the
/// model's `boxes_touch`).
fn value_label_touch(a: vec4<f32>, b: vec4<f32>) -> bool {
    let g = value_label.radar.x;
    return a.x < b.z + g && b.x < a.z + g && a.y < b.w + g && b.y < a.w + g;
}

/// Whether a label of `size` centred on `centre` touches either label
/// placed before it or the ring labels.
fn value_label_radar_blocked(centre: vec2<f32>, size: vec2<f32>, prev1: vec4<f32>, prev2: vec4<f32>) -> bool {
    let r = value_label_rect(centre, size);
    return value_label_touch(r, prev1) || value_label_touch(r, prev2) || value_label_meets_ring_labels(centre, size);
}

/// Whether the label `cur` at `home` splits with the next label in the
/// scan, `following`: their homes touch and `following` cannot clear
/// `home` on the other side of its vertex (equal values within the visual
/// gap, a flipped `following`, or
/// its other side still touching).
fn value_label_radar_splits(cur: RadarSpokeLabel, home: vec2<f32>, following: RadarSpokeLabel, spoke: RadarSpoke) -> bool {
    let at = value_label_rect(home, cur.size);
    let flipped = value_label_radar_flips(following, spoke);
    let next_home = value_label_radar_along(following, select(spoke.u, -spoke.u, flipped));
    if (!value_label_touch(at, value_label_rect(next_home, following.size))) {
        return false;
    }
    if (abs(cur.t - following.t) <= value_label.radar.x || flipped) {
        return true;
    }
    let other = value_label_radar_along(following, -spoke.u);
    return value_label_touch(at, value_label_rect(other, following.size));
}

/// Where `cur` goes in the scan, given the next label `following`, the side
/// `split` the label before took when it split with `cur` (0 for none) and
/// the two labels placed before (`value_label_radar`): xy = the label's
/// centre, z = the side `cur` took when it splits with `following` (0 for
/// none).
fn value_label_radar_choose(
    cur: RadarSpokeLabel,
    following: RadarSpokeLabel,
    split: f32,
    prev1: vec4<f32>,
    prev2: vec4<f32>,
    spoke: RadarSpoke,
) -> vec3<f32> {
    if (split != 0.0) {
        // The other side of the spoke from the label it splits with.
        let side = value_label_radar_side(cur, -split, spoke);
        if (!value_label_radar_blocked(side, cur.size, prev1, prev2)) {
            return vec3<f32>(side, 0.0);
        }
    }
    let flipped = value_label_radar_flips(cur, spoke);
    let home = value_label_radar_along(cur, select(spoke.u, -spoke.u, flipped));
    if (!value_label_radar_blocked(home, cur.size, prev1, prev2)) {
        if (following.valid && value_label_radar_splits(cur, home, following, spoke)) {
            for (var k = 0u; k < 2u; k = k + 1u) {
                let s = select(1.0, -1.0, k == 1u);
                let side = value_label_radar_side(cur, s, spoke);
                if (!value_label_radar_blocked(side, cur.size, prev1, prev2)) {
                    return vec3<f32>(side, s);
                }
            }
        }
        return vec3<f32>(home, 0.0);
    }
    if (!flipped) {
        let other = value_label_radar_along(cur, -spoke.u);
        if (!value_label_radar_blocked(other, cur.size, prev1, prev2)) {
            return vec3<f32>(other, 0.0);
        }
    }
    for (var k = 0u; k < 2u; k = k + 1u) {
        let side = value_label_radar_side(cur, select(1.0, -1.0, k == 1u), spoke);
        if (!value_label_radar_blocked(side, cur.size, prev1, prev2)) {
            return vec3<f32>(side, 0.0);
        }
    }
    return vec3<f32>(home, 0.0);
}

/// The polar centre in chart-area render pixels, y down, as `polar_ndc`
/// places it.
fn value_label_polar_centre() -> vec2<f32> {
    return category_px((transform.data_to_panel_offset + transform.data_to_panel_scale * 0.5) * 2.0 - 1.0);
}

/// Half the extent of a box of `size` across the unit direction `n`.
fn value_label_half_across(n: vec2<f32>, size: vec2<f32>) -> f32 {
    return abs(n.x) * size.x * 0.5 + abs(n.y) * size.y * 0.5;
}

/// Whether a box of `size` centred on `centre` reaches past the outer ring.
fn value_label_past_rim(centre: vec2<f32>, size: vec2<f32>) -> bool {
    let rim = transform.polar_extent.w;
    let c = value_label_polar_centre();
    let lo = centre - size * 0.5 - c;
    let hi = centre + size * 0.5 - c;
    let far = max(abs(lo), abs(hi));
    return dot(far, far) > rim * rim;
}

/// `centre` moved inward along the unit spoke `u` until a box of `size`
/// centred on it lies inside the outer ring: each corner's distance along
/// the spoke at most what the ring leaves at its distance across.
fn value_label_inside_rim(centre: vec2<f32>, size: vec2<f32>, u: vec2<f32>) -> vec2<f32> {
    let rim = transform.polar_extent.w;
    let c = value_label_polar_centre();
    var travel = 0.0;
    for (var k = 0u; k < 4u; k = k + 1u) {
        let corner = vec2<f32>(select(-0.5, 0.5, (k & 1u) == 1u), select(-0.5, 0.5, k >= 2u));
        let p = centre + corner * size - c;
        let along = dot(p, u);
        let across = max(dot(p, p) - along * along, 0.0);
        travel = max(travel, along - sqrt(max(rim * rim - across, 0.0)));
    }
    return centre - u * travel;
}

/// Whether a box of `size` centred on `centre` comes within the visual gap
/// of the drawn ring labels' box (`ValueLabelParams.ring`, in the ray's
/// frame): the two boxes overlap across both of the ray's axes and both
/// screen axes.
fn value_label_meets_ring_labels(centre: vec2<f32>, size: vec2<f32>) -> bool {
    let band = value_label.ring;
    if (!(band.x <= band.y) || !(band.z <= band.w)) {
        return false;
    }
    let rim = transform.polar_extent.w;
    let pad = value_label.radar.x;
    let ray = value_label.radar.zw;
    let across = vec2<f32>(ray.y, -ray.x);
    let c = value_label_polar_centre();
    let a0 = band.x * rim - pad;
    let a1 = band.y * rim + pad;
    let b0 = band.z * rim - pad;
    let b1 = band.w * rim + pad;
    // The label against the band's axes.
    let rel = centre - c;
    let half_along = value_label_half_across(ray, size);
    let half_across = value_label_half_across(across, size);
    let along_at = dot(rel, ray);
    let across_at = dot(rel, across);
    if (along_at + half_along <= a0 || along_at - half_along >= a1
        || across_at + half_across <= b0 || across_at - half_across >= b1) {
        return false;
    }
    // The band against the screen axes.
    var lo = vec2<f32>(1e30);
    var hi = vec2<f32>(-1e30);
    for (var k = 0u; k < 4u; k = k + 1u) {
        let p = c + ray * select(a0, a1, (k & 1u) == 1u) + across * select(b0, b1, k >= 2u);
        lo = min(lo, p);
        hi = max(hi, p);
    }
    let box_lo = centre - size * 0.5;
    let box_hi = centre + size * 0.5;
    return all(box_lo < hi) && all(box_hi > lo);
}

/// Label `row` of the series as the draw, the selection box and the pick
/// see it: its content box, or hidden for an empty string, a label its
/// placement hides, or a bar, box or vertex that is not drawn.
fn value_label_place(row: u32) -> ValueLabelBox {
    if (row >= value_label.shape.y) {
        return value_label_hidden();
    }
    let cell = value_label_cell(value_label.shape.z, row);
    let size = vec2<f32>(cell.x, value_label_height());
    if (!(size.x > 0.0) || !(size.y > 0.0)) {
        return value_label_hidden();
    }
    let kind = value_label.shape.x;
    if (kind == VALUE_LABEL_KIND_BARS) {
        return value_label_bar(row, size, u32(cell.y));
    }
    if (kind == VALUE_LABEL_KIND_BOXES) {
        return value_label_box_plot(row, size);
    }
    if (kind == VALUE_LABEL_KIND_RADAR) {
        return value_label_radar(row, size);
    }
    if (kind == VALUE_LABEL_KIND_RANGE) {
        return value_label_range_bar(row, size, u32(cell.y));
    }
    return value_label_hidden();
}

/// Chart-area render pixels (y down) to NDC.
fn value_label_ndc(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(p.x * transform.pixel_to_ndc.x - 1.0, 1.0 - p.y * transform.pixel_to_ndc.y);
}

// ───── Draw ─────

/// One corner of label `row`'s quad, upright, through the shared label
/// quad; a hidden label collapses.
@vertex
fn vs_value_label(@builtin(vertex_index) vid: u32, @builtin(instance_index) row: u32) -> LabelOut {
    let place = value_label_place(row);
    if (!place.valid) {
        return label_quad_hidden();
    }
    let size = place.rect.zw - place.rect.xy;
    let frame = ContourLabelFrame(
        value_label_ndc((place.rect.xy + place.rect.zw) * 0.5),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
    );
    return label_quad(transform, value_label.atlas, vid, frame, row, size.x, true);
}

@fragment
fn fs_value_label(in: LabelOut) -> @location(0) vec4<f32> {
    // The atlas is tiny-skia output, premultiplied, as the contour labels'.
    return textureSampleLevel(value_label_atlas, value_label_sampler, in.uv, 0.0);
}

// ───── Selection box ─────
//
// The selected label's box, the selection policy of the decorations (the
// model's `select.rs`: `SELECTION_PADDING` around the frame, stroked
// `SELECTION_STROKE_WIDTH` wide in `SELECTION_COLOR`, centred on the box
// edge), around the same content box the label is drawn in.

struct ValueLabelSelectionOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    /// The stroked box, `(x0, y0, x1, y1)`.
    @location(1) @interpolate(flat) rect: vec4<f32>,
};

/// The selection box of label `row`: its content box grown by the
/// selection padding.
fn value_label_selection_rect(place: ValueLabelBox) -> vec4<f32> {
    let pad = value_label.selection.y;
    return place.rect + vec4<f32>(-pad, -pad, pad, pad);
}

@vertex
fn vs_value_label_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> ValueLabelSelectionOut {
    var out: ValueLabelSelectionOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.rect = vec4<f32>(0.0);
    let place = value_label_place(row);
    if (!place.valid) {
        return out;
    }
    let rect = value_label_selection_rect(place);
    out.rect = rect;
    let reach = value_label.selection.x * 0.5 + 1.0;
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[vi % 6u];
    let lo = rect.xy - vec2<f32>(reach);
    let hi = rect.zw + vec2<f32>(reach);
    let p = vec2<f32>(select(lo.x, hi.x, (corner & 1u) != 0u), select(lo.y, hi.y, corner >= 2u));
    out.local = p;
    out.pos = vec4<f32>(value_label_ndc(p), 0.0, 1.0);
    return out;
}

/// The signed distance from `p` to the edge of `rect`, negative inside.
fn value_label_rect_distance(p: vec2<f32>, rect: vec4<f32>) -> f32 {
    let half = (rect.zw - rect.xy) * 0.5;
    let q = abs(p - (rect.xy + rect.zw) * 0.5) - half;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

@fragment
fn fs_value_label_selection(in: ValueLabelSelectionOut) -> @location(0) vec4<f32> {
    let d = abs(value_label_rect_distance(in.local, in.rect));
    let coverage = clamp(value_label.selection.x * 0.5 - d + 0.5, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    return value_label.selection_color * coverage;
}

// ───── Typed data pick (compute entry) ─────
//
// The label under the cursor: the nearest label within the pick distance,
// by the distance to its selection box (0 inside it), answered as a
// `DATA_PICK_KIND_VALUE_LABEL` whose index is the category. The renderer
// gives a series' labels a paint order after every series of the panel, so
// a label wins every tie against the data it is drawn over.

fn value_label_pick(row: u32) -> DataPickCandidate {
    let place = value_label_place(row);
    if (!place.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    let distance_px = max(value_label_rect_distance(cursor, value_label_selection_rect(place)), 0.0);
    if (!data_pick_finite(distance_px)
        || !data_pick_finite(data_pick_query.limits.x)
        || distance_px > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_VALUE_LABEL,
        row,
        0u,
        0u,
        distance_px,
        row,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_value_labels(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var row = local_index;
    while (row < value_label.shape.y) {
        let candidate = value_label_pick(row);
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
