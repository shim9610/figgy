// Value labels of the bar module (SHADER_COMMON.md §5.17): the drawn string
// of each category of a category bar, box plot or radar series, one label
// quad per category, placed from the pool on every draw by the series' own
// input fragment and sampled from the series' label atlas through the
// shared label quad (`label_quad.wgsl`). Spliced into the bar module after
// `category_bars.wgsl` and `box_plot.wgsl`, whose geometry it reads:
//
// - a bar's label sits at its value end (`category_bar`'s `end_px`), outside
//   or inside the bar as its placement says;
// - a box's label sits past the top of the box: the farthest of its high
//   whisker, its mean, its notch and its outliers (`box_stats`,
//   `box_outlier_range`);
// - a radar vertex's label sits outside the vertex along its spoke: the
//   vertex is the pair `(category, value)` of the series' columns through
//   the polar projection, read through the category bar group 3
//   (`positions` the category column, `values[0][0]` the value column).
//
// The CPU bakes the strings (SSOT configuration, `labels.text`) into the
// atlas with the one text stack and writes each label's content width,
// placement and drag offset into `ValueLabelParams`; nothing per label is
// computed or stored on the CPU from the data. The same placement feeds the
// draw, the selection box (`vs_value_label_selection`) and the pick
// (`pick_value_labels`), so a label is picked and outlined where it is
// drawn. Labels never move each other.

/// CPU twin: `data_render/value_label.rs::ValueLabelParamsGpu` (1120 B).
struct ValueLabelParams {
    atlas: LabelParams,
    /// x = `VALUE_LABEL_KIND_*`, y = labels (one per category), zw = 0.
    shape: vec4<u32>,
    /// x = the gap between the anchor and the label, y = the text
    /// baseline's height above the content's bottom edge, z = the font size,
    /// w = render pixels per logical pixel; all in render pixels but `w`.
    metrics: vec4<f32>,
    /// The selection box colour, premultiplied.
    selection_color: vec4<f32>,
    /// x = the selection box stroke width, y = its padding around the
    /// label's frame, both in render pixels; zw = 0.
    selection: vec4<f32>,
    /// One per label: x = the content width in render pixels (0 draws
    /// nothing: an empty string, or a label its placement hides), y = the
    /// placement (`VALUE_LABEL_OUTSIDE` / `_INSIDE` / `_AUTO`), zw = the drag
    /// offset in render pixels, x right and y down.
    cells: array<vec4<f32>, 64>,
};

const VALUE_LABEL_KIND_BARS: u32 = 0u;
const VALUE_LABEL_KIND_BOXES: u32 = 1u;
const VALUE_LABEL_KIND_RADAR: u32 = 2u;
const VALUE_LABEL_OUTSIDE: u32 = 0u;
const VALUE_LABEL_INSIDE: u32 = 1u;
const VALUE_LABEL_AUTO: u32 = 2u;
/// Labels one series can hold: one per category. CPU twin:
/// `value_label.rs::VALUE_LABEL_CAPACITY`.
const VALUE_LABEL_CAPACITY: u32 = 64u;
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
/// The baseline below a radar label's anchor, in font sizes.
const VALUE_LABEL_RADAR_BASELINE: f32 = 0.35;

@group(1) @binding(4) var<uniform> value_label: ValueLabelParams;
@group(1) @binding(5) var value_label_atlas: texture_2d<f32>;
@group(1) @binding(6) var value_label_sampler: sampler;

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
    let origin = floor(centre + value_label.cells[row].zw - size * 0.5 + vec2<f32>(0.5));
    if (!bar_finite(origin.x) || !bar_finite(origin.y)) {
        return value_label_hidden();
    }
    return ValueLabelBox(true, vec4<f32>(origin, origin + size));
}

/// The centre's y of a content box whose baseline is at `baseline`.
fn value_label_centre_y(baseline: f32, size: vec2<f32>) -> f32 {
    return baseline + value_label.metrics.y - size.y * 0.5;
}

/// A bar's label: outside its value end, or inside the bar centred on it.
/// `VALUE_LABEL_AUTO` is the former chart's rule: outside grouped bars,
/// inside a stack's segments that fit it, else none. An outside label of a
/// stack segment a later segment covers draws nothing.
fn value_label_bar(row: u32, size: vec2<f32>, placement: u32) -> ValueLabelBox {
    let bar = category_bar(row, resolve_bar_style(category_geometry_style(), row));
    if (!bar.valid) {
        return value_label_hidden();
    }
    let horizontal = category_params.horizontal != 0u;
    let grouped = category_params.mode == CATEGORY_BAR_MODE_GROUPED;
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
    if (horizontal) {
        // Past the end, away from the bar's start.
        let right = end >= middle.x;
        let x = select(end - gap - size.x * 0.5, end + gap + size.x * 0.5, right);
        centre = vec2<f32>(x, value_label_centre_y(middle.y + font * VALUE_LABEL_CENTRE_BASELINE, size));
    } else if (end <= middle.y) {
        // Above an end that rises: the baseline `gap` above it.
        centre = vec2<f32>(middle.x, value_label_centre_y(end - gap, size));
    } else {
        // Below an end that falls: the text's top about `gap` below it.
        centre = vec2<f32>(middle.x, value_label_centre_y(end + gap + font, size));
    }
    return value_label_box_at(row, centre, size);
}

/// A box's label: `gap` past the farthest of its high whisker, its mean,
/// its notch and its outliers along the value axis, centred across the box.
fn value_label_box_plot(row: u32, size: vec2<f32>) -> ValueLabelBox {
    let s = box_stats(row);
    if (!s.valid) {
        return value_label_hidden();
    }
    let place = box_layout(s, box_body_style(category_geometry_style(), row));
    if (!place.valid) {
        return value_label_hidden();
    }
    let horizontal = category_params.horizontal != 0u;
    // The top of the box: the smallest y down, or the largest x.
    var top = box_along(s.position, s.high);
    if (s.has_mean) {
        top = value_label_farther(top, box_along(s.position, s.mean), horizontal);
    }
    if (s.has_ci) {
        top = value_label_farther(top, box_along(s.position, s.ci_high), horizontal);
    }
    let range = box_outlier_range(row);
    let values = box_lane(BOX_LANE_OUTLIER_VALUES);
    for (var k = 0u; k < range.y; k = k + 1u) {
        let v = category_pair(values, range.x + k);
        if (category_finite(v)) {
            top = value_label_farther(top, box_along(s.position, v), horizontal);
        }
    }
    if (!bar_finite(top)) {
        return value_label_hidden();
    }
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    var centre: vec2<f32>;
    if (horizontal) {
        centre = vec2<f32>(top + gap + size.x * 0.5, value_label_centre_y(place.middle + font * VALUE_LABEL_CENTRE_BASELINE, size));
    } else {
        centre = vec2<f32>(place.middle, value_label_centre_y(top - gap, size));
    }
    return value_label_box_at(row, centre, size);
}

/// Of two positions along the value axis, the one farther towards larger
/// values on screen: the smaller y (y down), or the larger x.
fn value_label_farther(a: f32, b: f32, horizontal: bool) -> f32 {
    if (!bar_finite(b)) {
        return a;
    }
    return select(min(a, b), max(a, b), horizontal);
}

/// A radar vertex's label: outside the vertex along its spoke, `gap` from
/// it, as the former radar chart placed it.
fn value_label_radar(row: u32, size: vec2<f32>) -> ValueLabelBox {
    if (row >= category_params.count) {
        return value_label_hidden();
    }
    let x = category_pair(category_params.positions, row);
    let y = category_value(0u, row);
    if (!category_finite(x) || !category_finite(y)) {
        return value_label_hidden();
    }
    let vertex = category_px(data_to_ndc(transform, x, y));
    // The spoke from the inner ring to the outer one, through the vertex.
    let inner = category_px(data_to_ndc(transform, x, vec2<f32>(transform.data_min.y, transform.data_min_lo.y)));
    let outer = category_px(data_to_ndc(transform, x, vec2<f32>(transform.data_max.y, transform.data_max_lo.y)));
    let spoke = outer - inner;
    if (!(dot(spoke, spoke) > 0.0) || !bar_finite(vertex.x) || !bar_finite(vertex.y)) {
        return value_label_hidden();
    }
    let d = normalize(spoke);
    let gap = value_label.metrics.x;
    let font = value_label.metrics.z;
    let centre = vec2<f32>(
        vertex.x + d.x * (gap + size.x * 0.5),
        value_label_centre_y(vertex.y + d.y * gap + font * VALUE_LABEL_RADAR_BASELINE, size),
    );
    return value_label_box_at(row, centre, size);
}

/// Label `row` of the series as the draw, the selection box and the pick
/// see it: its content box, or hidden for an empty string, a label its
/// placement hides, or a bar, box or vertex that is not drawn.
fn value_label_place(row: u32) -> ValueLabelBox {
    if (row >= min(value_label.shape.y, VALUE_LABEL_CAPACITY)) {
        return value_label_hidden();
    }
    let cell = value_label.cells[row];
    let size = vec2<f32>(cell.x, value_label.atlas.cell_stride_h - 2.0 * value_label.atlas.gutter);
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
    while (row < min(value_label.shape.y, VALUE_LABEL_CAPACITY)) {
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
