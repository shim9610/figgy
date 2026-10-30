// Category bar input layer of the bar module (SHADER_COMMON.md §5.13): the
// bars of a `DataRenderType::CategoryBars` series, spliced into the module
// with `bar_columnar.wgsl`, whose override table (`resolve_bar_style`) and
// data pick helpers it reads.
//
// One instance is the bar of one category (a row of the series' columns).
// No vertex buffers: the column pool is bound whole as storage, the way the
// vector shader binds it, and the bar's rect is built here on every draw from
// the category position, the group layout, the direction, the value and, for
// a stack, the values of the earlier series of the group at the same row,
// read by their pool lanes from the series' lane table (`category_lanes`,
// sized by the group) in a loop over the members. Nothing per bar is stored
// anywhere else. The draw, the pick and the selection all build the rect
// through `category_bar`.

/// The group a series' bars belong to and how they are laid out.
///
/// CPU twin: `data_render/bar.rs::CategoryBarParamsGpu`
/// (`#[repr(C)]`, 64 B).
struct CategoryBarParams {
    /// f32 lane base of this series' category position column.
    positions: u32,
    /// Bars of this series: one per category.
    count: u32,
    /// This series' place in its group.
    member: u32,
    /// Series in the group: the entries of `category_lanes`, any number.
    members: u32,
    /// `CATEGORY_BAR_MODE_*`.
    mode: u32,
    /// 1 when categories run along y.
    horizontal: u32,
    /// Render pixels between grouped bars.
    gap_px: f32,
    /// Fraction of a category slot the bars of the slot cover.
    group_width: f32,
    /// The series' corner radius in render pixels.
    corner_radius_px: f32,
    /// The series' emphasis brightness of a picked bar.
    emphasis: f32,
    /// Render pixels per logical pixel: the material's object unit.
    scale: f32,
    /// The series salt, which seeds the material grain per bar.
    seed: u32,
    /// Half a slot along the position axis in data units, as a `(hi, lo)`
    /// pair: `(0.5, 0)`, half a category, for category bars and boxes; half
    /// a range bar series' `slot_width` on a numeric axis.
    slot_half: vec2<f32>,
    _reserved: vec2<u32>,
};

const CATEGORY_BAR_MODE_GROUPED: u32 = 0u;
const CATEGORY_BAR_MODE_STACKED: u32 = 1u;
const CATEGORY_BAR_MODE_PERCENT: u32 = 2u;
/// Vertices per bar: one quad.
const CATEGORY_BAR_VERTICES: u32 = 6u;
/// A stack's top that is not exposed shares its edge with the next bar.
const CATEGORY_EDGE_LEFT: u32 = 1u;
const CATEGORY_EDGE_RIGHT: u32 = 2u;
const CATEGORY_EDGE_TOP: u32 = 4u;
const CATEGORY_EDGE_BOTTOM: u32 = 8u;

@group(3) @binding(8) var<storage, read> category_pool: array<f32>;
@group(3) @binding(9) var<uniform> category_params: CategoryBarParams;
/// The series' lane table, sized by what it holds (SHADER_COMMON.md §5.13):
/// a category bar series' group value column lane bases in group order
/// (entry `k` member `k`'s), a box plot's record and outlier lanes (§5.15),
/// a radar series' value column (§5.17).
@group(3) @binding(10) var<storage, read> category_lanes: array<u32>;

/// The `(hi, lo)` pair of row `row` in the column at lane `base`.
fn category_pair(base: u32, row: u32) -> vec2<f32> {
    let lane = base + row * 2u;
    return vec2<f32>(category_pool[lane], category_pool[lane + 1u]);
}

fn category_value(member: u32, row: u32) -> vec2<f32> {
    return category_pair(category_lanes[member], row);
}

fn category_finite(v: vec2<f32>) -> bool {
    return bar_finite(v.x) && bar_finite(v.y) && bar_finite(v.x + v.y);
}

/// One bar as the draw, the pick and the selection see it, in chart-area
/// render pixels with y down.
struct CategoryBar {
    valid: bool,
    /// `(x0, y0, x1, y1)`.
    rect: vec4<f32>,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left.
    radii: vec4<f32>,
    /// `CATEGORY_EDGE_*` bits of the edges this bar shares with its stack
    /// neighbours; the bar owns those edges' pixels on its own side only.
    edges: u32,
    /// The value end of the bar along the value axis, in chart-area render
    /// pixels: where its value label is placed from.
    end_px: f32,
    /// Whether the value end is not covered by a later bar of its stack.
    exposed: bool,
};

fn category_invalid() -> CategoryBar {
    return CategoryBar(false, vec4<f32>(0.0), vec4<f32>(0.0), 0u, 0.0, false);
}

/// NDC to chart-area render pixels, y down.
fn category_px(ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((ndc.x + 1.0) / transform.pixel_to_ndc.x, (1.0 - ndc.y) / transform.pixel_to_ndc.y);
}

/// The position half a slot below (`side` -1) or above (`side` 1)
/// `position`, both `(hi, lo)` pairs: the slot's edge along the position
/// axis.
fn category_slot_edge(position: vec2<f32>, side: f32) -> vec2<f32> {
    return vec2<f32>(position.x + side * category_params.slot_half.x, position.y + side * category_params.slot_half.y);
}

/// The base style of the series' bars: the `Style` lanes (fill, outline
/// width and colour, material) and the layout lanes of the params (corner
/// radius and emphasis in `finish`), before the bar's overrides.
fn category_base_style() -> ResolvedBarStyle {
    return ResolvedBarStyle(
        style.color_premul,
        style.dash[0],
        max(style.line_width_px, 0.0),
        category_params.gap_px,
        category_params.group_width,
        style.dash[1],
        vec4<f32>(category_params.corner_radius_px, category_params.emphasis, 0.0, 0.0),
    );
}

/// The base style without the `Style` lanes, for the pick (which binds no
/// `Style`): only the geometry a bar's outline and fill never change.
fn category_geometry_style() -> ResolvedBarStyle {
    return ResolvedBarStyle(
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        0.0,
        category_params.gap_px,
        category_params.group_width,
        vec4<f32>(0.0),
        vec4<f32>(category_params.corner_radius_px, category_params.emphasis, 0.0, 0.0),
    );
}

/// The bar of row `row` of this series: its rect from the category position,
/// the group layout and the stack, its exposed corners rounded by the
/// resolved corner radius. A missing or zero value, a percent stack without
/// a positive total, or a slot too narrow for the group draws no bar.
fn category_bar(row: u32, resolved: ResolvedBarStyle) -> CategoryBar {
    if (row >= category_params.count) {
        return category_invalid();
    }
    let member = category_params.member;
    let v = category_value(member, row);
    let position = category_pair(category_params.positions, row);
    if (!category_finite(v) || !category_finite(position)) {
        return category_invalid();
    }
    let value = v.x + v.y;
    if (value == 0.0) {
        return category_invalid();
    }
    let mode = category_params.mode;
    var start = vec2<f32>(0.0);
    var end = v;
    var exposed = true;
    if (mode == CATEGORY_BAR_MODE_PERCENT) {
        // Shares of the category total, normalized by the largest value
        // first so the total stays finite.
        var largest = 0.0;
        for (var k = 0u; k < category_params.members; k = k + 1u) {
            let x = category_value(k, row);
            if (category_finite(x)) {
                largest = max(largest, x.x + x.y);
            }
        }
        var total = 0.0;
        for (var k = 0u; k < category_params.members; k = k + 1u) {
            let x = category_value(k, row);
            if (category_finite(x) && largest > 0.0) {
                total = total + (x.x + x.y) / largest;
            }
        }
        if (!(largest > 0.0) || !(total > 0.0) || value < 0.0) {
            return category_invalid();
        }
        var below = 0.0;
        for (var k = 0u; k < category_params.members; k = k + 1u) {
            let x = category_value(k, row);
            if (!category_finite(x) || !(x.x + x.y > 0.0)) {
                continue;
            }
            if (k < member) {
                below = below + (x.x + x.y) / largest / total * 100.0;
            } else if (k > member) {
                exposed = false;
            }
        }
        start = vec2<f32>(below, 0.0);
        // The top of a stack is exactly the whole: rounding never leaves it
        // short of 100.
        end = vec2<f32>(select(below + value / largest / total * 100.0, 100.0, exposed), 0.0);
    } else if (mode == CATEGORY_BAR_MODE_STACKED) {
        // Positive and negative values stack away from zero independently;
        // the pairs keep their low parts.
        var positive = vec2<f32>(0.0);
        var negative = vec2<f32>(0.0);
        for (var k = 0u; k < category_params.members; k = k + 1u) {
            let x = category_value(k, row);
            if (!category_finite(x)) {
                continue;
            }
            let xv = x.x + x.y;
            if (k < member) {
                if (xv < 0.0) {
                    negative = negative + x;
                } else if (xv > 0.0) {
                    positive = positive + x;
                }
            } else if (k > member && xv != 0.0 && (xv < 0.0) == (value < 0.0)) {
                exposed = false;
            }
        }
        start = select(positive, negative, value < 0.0);
        end = start + v;
    }

    let horizontal = category_params.horizontal != 0u;
    // The value axis: from the stack start to its end.
    var a: f32;
    var b: f32;
    var slot_a: f32;
    var slot_b: f32;
    let low = category_slot_edge(position, -1.0);
    let high = category_slot_edge(position, 1.0);
    if (horizontal) {
        a = category_px(data_to_ndc(transform, start, position)).x;
        b = category_px(data_to_ndc(transform, end, position)).x;
        slot_a = category_px(data_to_ndc(transform, start, low)).y;
        slot_b = category_px(data_to_ndc(transform, start, high)).y;
    } else {
        a = category_px(data_to_ndc(transform, position, start)).y;
        b = category_px(data_to_ndc(transform, position, end)).y;
        slot_a = category_px(data_to_ndc(transform, low, start)).x;
        slot_b = category_px(data_to_ndc(transform, high, start)).x;
    }
    // The category slot along the other axis: the group covers its centred
    // `group_width`, grouped bars side by side `gap_px` apart in series
    // order from the slot's left (or top) edge.
    let grouped = mode == CATEGORY_BAR_MODE_GROUPED;
    let per_slot = select(1.0, f32(category_params.members), grouped);
    let slot = abs(slot_b - slot_a);
    let span = slot * clamp(resolved.width_ratio, 0.0, 1.0);
    let gap = max(resolved.gap_px, 0.0);
    let thickness = (span - gap * (per_slot - 1.0)) / per_slot;
    if (!(thickness > 0.0) || !bar_finite(a) || !bar_finite(b)) {
        return category_invalid();
    }
    let across = (slot_a + slot_b) * 0.5 - span * 0.5
        + select(0.0, f32(member) * (thickness + gap), grouped);
    var rect: vec4<f32>;
    if (horizontal) {
        rect = vec4<f32>(min(a, b), across, max(a, b), across + thickness);
    } else {
        rect = vec4<f32>(across, min(a, b), across + thickness, max(a, b));
    }
    let extent = select(rect.w - rect.y, rect.z - rect.x, horizontal);
    if (!(extent > 0.0)) {
        return category_invalid();
    }

    // Only the exposed end is rounded, on its own side of the zero line.
    let r = min(min(max(resolved.finish.x, 0.0), thickness * 0.5), extent * 0.5);
    var radii = vec4<f32>(0.0);
    let positive_value = value >= 0.0;
    if (exposed) {
        if (horizontal) {
            radii = select(vec4<f32>(r, 0.0, 0.0, r), vec4<f32>(0.0, r, r, 0.0), positive_value);
        } else {
            radii = select(vec4<f32>(0.0, 0.0, r, r), vec4<f32>(r, r, 0.0, 0.0), positive_value);
        }
    }
    // A shared stack edge has one pixel owner: blending two partial
    // coverages there would let the background through the seam.
    var edges = 0u;
    if (!grouped) {
        var start_edge = CATEGORY_EDGE_BOTTOM;
        var end_edge = CATEGORY_EDGE_TOP;
        if (horizontal) {
            start_edge = select(CATEGORY_EDGE_RIGHT, CATEGORY_EDGE_LEFT, positive_value);
            end_edge = select(CATEGORY_EDGE_LEFT, CATEGORY_EDGE_RIGHT, positive_value);
        } else {
            start_edge = select(CATEGORY_EDGE_TOP, CATEGORY_EDGE_BOTTOM, positive_value);
            end_edge = select(CATEGORY_EDGE_BOTTOM, CATEGORY_EDGE_TOP, positive_value);
        }
        if (start.x + start.y != 0.0) {
            edges = edges | start_edge;
        }
        if (!exposed) {
            edges = edges | end_edge;
        }
    }
    return CategoryBar(true, rect, radii, edges, b, exposed);
}

/// The signed distance from `p` to the rounded rect of `bar`, in pixels:
/// `bounded_shading.wgsl`'s `sdf` of the bar's shape.
fn category_bar_distance(p: vec2<f32>, bar: CategoryBar) -> f32 {
    let shape = Bar(
        bar.rect,
        bar.radii,
        vec4<f32>(0.0),
        Material(vec4<f32>(0.0), vec4<f32>(0.0)),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
        vec4<f32>(0.0),
    );
    return sdf(p, shape);
}

/// Whether `p` is outside a shared edge `bar` does not own.
fn category_bar_unowned(p: vec2<f32>, bar: CategoryBar) -> bool {
    return ((bar.edges & CATEGORY_EDGE_LEFT) != 0u && p.x < bar.rect.x)
        || ((bar.edges & CATEGORY_EDGE_RIGHT) != 0u && p.x >= bar.rect.z)
        || ((bar.edges & CATEGORY_EDGE_TOP) != 0u && p.y < bar.rect.y)
        || ((bar.edges & CATEGORY_EDGE_BOTTOM) != 0u && p.y >= bar.rect.w);
}

/// A subpixel sample kept on its own side of the shared edges.
fn category_bar_sample(p: vec2<f32>, bar: CategoryBar) -> vec2<f32> {
    var s = p;
    if ((bar.edges & CATEGORY_EDGE_LEFT) != 0u) { s.x = max(s.x, bar.rect.x); }
    if ((bar.edges & CATEGORY_EDGE_RIGHT) != 0u) { s.x = min(s.x, bar.rect.z); }
    if ((bar.edges & CATEGORY_EDGE_TOP) != 0u) { s.y = max(s.y, bar.rect.y); }
    if ((bar.edges & CATEGORY_EDGE_BOTTOM) != 0u) { s.y = min(s.y, bar.rect.w); }
    return s;
}

// ───── Draw ─────

struct CategoryVsOut {
    @builtin(position) pos: vec4<f32>,
    /// The vertex in chart-area render pixels, y down.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) fill: vec4<f32>,
    @location(4) @interpolate(flat) outline_color: vec4<f32>,
    @location(5) @interpolate(flat) material: vec4<f32>,
    /// x = outline width, y = emphasis, z = grain seed, w = shared edges.
    @location(6) @interpolate(flat) metrics: vec4<f32>,
};

/// One corner of the bar's quad, one pixel past the rect so the edge's
/// analytic coverage has room; a bar that draws nothing collapses.
fn category_vertex(vi: u32, row: u32, resolved: ResolvedBarStyle) -> CategoryVsOut {
    var out: CategoryVsOut;
    let bar = category_bar(row, resolved);
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.rect = bar.rect;
    out.radii = bar.radii;
    out.fill = resolved.fill_color_premul;
    out.outline_color = resolved.border_color_premul;
    out.material = resolved.material;
    let seed = (category_params.seed ^ (row * 2654435761u)) % 4096u;
    out.metrics = vec4<f32>(resolved.border_width_px, resolved.finish.y, f32(seed), f32(bar.edges));
    if (!bar.valid) {
        return out;
    }
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[vi % 6u];
    let lo = bar.rect.xy - vec2<f32>(1.0);
    let hi = bar.rect.zw + vec2<f32>(1.0);
    let p = vec2<f32>(select(lo.x, hi.x, (corner & 1u) != 0u), select(lo.y, hi.y, corner >= 2u));
    out.local = p;
    out.pos = vec4<f32>(p.x * transform.pixel_to_ndc.x - 1.0, 1.0 - p.y * transform.pixel_to_ndc.y, 0.0, 1.0);
    return out;
}

@vertex
fn vs_category_bars(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> CategoryVsOut {
    return category_vertex(vi, row, resolve_bar_style(category_base_style(), row));
}

/// The flat body `bounded_shading.wgsl`'s `sdf` and `bar_surface` read
/// (SHADER_COMMON.md §5.9 bars): each module that splices that fragment
/// declares it. Here it lives in registers only: `rect` and `radii` the
/// bar's, `color` the straight fill, `material` §5.9, `outline` (flag,
/// width, 1 when horizontal, 0), `effects` (0, render pixels per logical
/// pixel, shared edges, 0); `outline_color` is not read by the shading.
struct Bar {
    rect: vec4<f32>,
    radii: vec4<f32>,
    color: vec4<f32>,
    material: Material,
    outline: vec4<f32>,
    outline_color: vec4<f32>,
    effects: vec4<f32>,
};

/// The bar from the fragment's flat inputs.
fn category_fragment_bar(in: CategoryVsOut) -> CategoryBar {
    return CategoryBar(true, in.rect, in.radii, u32(in.metrics.w), 0.0, true);
}

/// Four subpixel samples of the rounded rect: `(coverage, outline coverage)`.
fn category_coverage(p: vec2<f32>, bar: CategoryBar, band: f32) -> vec2<f32> {
    var coverage = 0.0;
    var outline = 0.0;
    for (var y = 0u; y < 2u; y = y + 1u) {
        for (var x = 0u; x < 2u; x = x + 1u) {
            let sample = category_bar_sample(p + (vec2<f32>(f32(x), f32(y)) + 0.5) * 0.5 - 0.5, bar);
            let d = category_bar_distance(sample, bar);
            coverage = coverage + select(0.0, 0.25, d <= 0.0);
            outline = outline + select(0.0, 0.25, d <= 0.0 && d >= -band && band > 0.0);
        }
    }
    return vec2<f32>(coverage, outline);
}

@fragment
fn fs_category_bars(in: CategoryVsOut) -> @location(0) vec4<f32> {
    let bar = category_fragment_bar(in);
    let p = in.local;
    if (category_bar_unowned(p, bar)) {
        discard;
    }
    let cover = category_coverage(p, bar, in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
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
        vec4<f32>(0.0, category_params.scale, in.metrics.w, 0.0),
    );
    let outline_alpha = in.outline_color.a;
    let outline_rgb = in.outline_color.rgb / max(outline_alpha, 1e-6);
    let c = mix(bar_surface(p, shaded), outline_rgb, outline_alpha * cover.y / max(cover.x, 0.001));
    return vec4<f32>(c * cover.x, cover.x);
}

/// `RadialStyle`'s default roughness, which a bar is shaded with
/// (`style.rs::FLAT_BODY_ROUGHNESS`).
const CATEGORY_BAR_ROUGHNESS: f32 = 0.35;

// ───── Selection ─────
//
// The picked highlight of one bar: an inside ring in the selection's colour
// (`style.color_premul`) and width (`style.line_width_px`), over the bar's
// own emphasis brightness added to what is drawn under it. The bar is the
// instance the caller draws; its overrides resolve by its row like the draw.

@vertex
fn vs_category_bar_selection(@builtin(vertex_index) vi: u32, @builtin(instance_index) row: u32) -> CategoryVsOut {
    let resolved = resolve_bar_style(category_base_style(), row);
    var out = category_vertex(vi, row, resolved);
    out.fill = style.color_premul;
    out.metrics.x = max(style.line_width_px, 0.0);
    return out;
}

@fragment
fn fs_category_bar_selection(in: CategoryVsOut) -> @location(0) vec4<f32> {
    let bar = category_fragment_bar(in);
    let p = in.local;
    if (category_bar_unowned(p, bar)) {
        discard;
    }
    let cover = category_coverage(p, bar, in.metrics.x);
    if (cover.x <= 0.0) {
        discard;
    }
    let ring = in.fill * cover.y;
    let brighten = vec3<f32>(max(in.metrics.y, 0.0) * max(cover.x - cover.y, 0.0));
    return vec4<f32>(ring.rgb + brighten, ring.a);
}

// ───── Exact data pick (compute entry) ─────
//
// The bar under the cursor: the nearest bar within the pick distance by the
// rounded rect's distance, from the same `category_bar` the draw builds.
// Answers as a histogram bin whose index is the row.

fn category_pick_bar(row: u32) -> DataPickCandidate {
    let resolved = resolve_bar_style(category_geometry_style(), row);
    let bar = category_bar(row, resolved);
    if (!bar.valid) {
        return data_pick_invalid();
    }
    let cursor = category_px(data_pick_query.cursor_ndc_t.xy);
    let distance_px = max(category_bar_distance(cursor, bar), 0.0);
    if (!data_pick_finite(distance_px)
        || !data_pick_finite(data_pick_query.limits.x)
        || distance_px > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        row,
        0u,
        0u,
        distance_px,
        row,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_category_bar(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var row = local_index;
    while (row < category_params.count) {
        let candidate = category_pick_bar(row);
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
