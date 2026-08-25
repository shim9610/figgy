// Columnar bar shader — histograms from host-binned data.
//
// Binning is the host's job. The renderer is handed two columns: bin `edges`
// (n + 1 values) and `counts` (n). The edge column is bound twice, the second
// time shifted by one logical value (COLUMN_VALUE_BYTES), so instance i sees
// `edges[i]`, `edges[i + 1]`, and `counts[i]` — the same trick the line shader
// uses for its segment endpoints. Instance count is decided by the caller as
// `min(edges - 1, counts)`; a length mismatch draws fewer bars, never garbage.
//
// `style.shape_id` picks which axis the edges run along, so one shader draws
// both orientations. The caller binds the edge column to slots 0/1 and the
// count column to slot 2 accordingly; nothing here infers the roles from the
// column lengths.
//
// Each instance is 5 axis-aligned quads = 30 vertices on a TriangleList:
// the fill plus the four border edges, which tile the bar exactly with no
// overlap (so a translucent fill never blends over its own border). Pixel
// dimensions — the inter-bar gap and the border width — are applied as NDC
// offsets via `transform.pixel_to_ndc`, the same construction
// `errorbar_columnar.wgsl` uses.
//
// Log axes need no special case: `maybe_log` already floors non-positive
// values at 1e-30, so a bar based at 0 on a logarithmic count axis extends
// past the bottom of the data area and is clipped by the scissor — which is
// what a bar with no lower bound looks like. Nothing produces NaN.

// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    style_params: array<vec4<f32>, 3>,
};  // 112 B (vec4 array at offset 64, stride 16)

@group(0) @binding(0) var<uniform> transform: Transform;

struct Style {
    color_premul: vec4<f32>,
    line_width_px: f32,
    point_radius_px: f32,
    cap_half_px: f32,
    cap_width_px: f32,
    shape_id: u32,
    dash_len: u32,
    // Per-series decorrelation salt (FNV-1a of series_id). Styled entries
    // (sketch/milkyway/constellation) XOR it into their hash seeds so two series never
    // share a star/wobble pattern; precise entries never read it.
    series_salt: u32,
    // Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    // for X; every other primitive ignores this field.
    primitive_flags: u32,
    dash: array<vec4<f32>, 2>,
};

@group(1) @binding(0) var<uniform> style: Style;

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = (v.x - min_hi) + (v.y - min_lo);
    let range = (max_hi - min_hi) + (max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return panel_offset + data_t * panel_scale;
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}
// ───── END common block ─────

// ───── Style reinterpretation (bars only) ─────
//
// The `Style` uniform is byte-identical across every data shader; a bar reads
// the fields it needs and reinterprets the ones it has no use for. The mapping
// below is written by `PrimitiveStyle::from_bar`:
//
//   color_premul   fill colour, premultiplied
//   line_width_px  border width, px
//   cap_half_px    gap between neighbouring bars, px (half on each side)
//   cap_width_px   fraction of the bin occupied by the bar, 0..1
//   shape_id       0 = edges run along x (vertical bars), 1 = along y
//   dash[0]        border colour, premultiplied
//   dash[1].xy     baseline as the pool's (hi, lo) f32 pair
//
// `point_radius_px`, `dash_len`, `dash[1].zw` are unused.

const BAR_ORIENT_HORIZONTAL: u32 = 1u;
/// Quads per instance: fill + four border edges.
const BAR_SEGMENTS: u32 = 5u;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    // Bin bounds — the same column bound twice, one logical value apart.
    @location(0) edge_lo: vec2<f32>,
    @location(1) edge_hi: vec2<f32>,
    // The bar's magnitude (the counts column).
    @location(2) value: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color_premul: vec4<f32>,
};

fn bar_baseline() -> vec2<f32> {
    return vec2<f32>(style.dash[1].x, style.dash[1].y);
}

/// The bar's outer rect in NDC as `(lo.x, lo.y, hi.x, hi.y)`, gap applied.
///
/// Built from the two opposite data-space corners and then min/max'd, so the
/// result is axis-aligned regardless of which way the value runs from the
/// baseline or whether the axis is inverted.
fn bar_outer_ndc_values(
    edge_lo: vec2<f32>,
    edge_hi: vec2<f32>,
    value: vec2<f32>,
    base: vec2<f32>,
    horizontal: bool,
    gap_px: f32,
    width_ratio: f32,
) -> vec4<f32> {
    var a: vec2<f32>;
    var b: vec2<f32>;
    if (horizontal) {
        a = data_to_ndc(base, edge_lo);
        b = data_to_ndc(value, edge_hi);
    } else {
        a = data_to_ndc(edge_lo, base);
        b = data_to_ndc(edge_hi, value);
    }
    var lo = min(a, b);
    var hi = max(a, b);

    // Width ratio: retain a centred fraction of the raw bin span. It is a
    // geometric ratio, so unlike the pixel gap it scales under zoom/export.
    let ratio = clamp(width_ratio, 0.0, 1.0);
    if (horizontal) {
        let d = (hi.y - lo.y) * (1.0 - ratio) * 0.5;
        lo.y = lo.y + d;
        hi.y = hi.y - d;
    } else {
        let d = (hi.x - lo.x) * (1.0 - ratio) * 0.5;
        lo.x = lo.x + d;
        hi.x = hi.x - d;
    }

    // Gap: shrink along the edge axis only, half on each side, and never past
    // the point where the bar would invert.
    let half_gap_px = max(gap_px, 0.0) * 0.5;
    if (horizontal) {
        let d = min(half_gap_px * transform.pixel_to_ndc.y, (hi.y - lo.y) * 0.5);
        lo.y = lo.y + d;
        hi.y = hi.y - d;
    } else {
        let d = min(half_gap_px * transform.pixel_to_ndc.x, (hi.x - lo.x) * 0.5);
        lo.x = lo.x + d;
        hi.x = hi.x - d;
    }
    return vec4<f32>(lo, hi);
}

fn bar_outer_ndc(in: VsIn, horizontal: bool, gap_px: f32, width_ratio: f32) -> vec4<f32> {
    return bar_outer_ndc_values(
        in.edge_lo,
        in.edge_hi,
        in.value,
        bar_baseline(),
        horizontal,
        gap_px,
        width_ratio,
    );
}

/// The fill rect: the outer rect inset by the border width on all four sides,
/// clamped so a border wider than the bar collapses the fill instead of
/// turning it inside out.
fn bar_inner_ndc(outer: vec4<f32>, border_width_px: f32) -> vec4<f32> {
    let border = max(border_width_px, 0.0) * transform.pixel_to_ndc;
    let half_span = (outer.zw - outer.xy) * 0.5;
    let inset = min(border, half_span);
    return vec4<f32>(outer.xy + inset, outer.zw - inset);
}

/// Sub-rect for one segment: 0 = fill, 1..4 = left / right / low / high border.
/// Together they tile `outer` exactly.
fn bar_segment_rect(seg: u32, outer: vec4<f32>, inner: vec4<f32>) -> vec4<f32> {
    if (seg == 0u) {
        return inner;
    } else if (seg == 1u) {
        return vec4<f32>(outer.x, outer.y, inner.x, outer.w);
    } else if (seg == 2u) {
        return vec4<f32>(inner.z, outer.y, outer.z, outer.w);
    } else if (seg == 3u) {
        return vec4<f32>(inner.x, outer.y, inner.z, inner.y);
    }
    return vec4<f32>(inner.x, inner.w, inner.z, outer.w);
}

// ───── Sparse per-bin style overrides (bars only) ─────

struct BarStyleSlot {
    fill_color_premul: vec4<f32>,
    border_color_premul: vec4<f32>,
    // x = border width, y = gap, z = width ratio, w = mask bits.
    params: vec4<f32>,
};

struct BarStyleOverride {
    bin_index: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    fill_color_premul: vec4<f32>,
    border_color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct BarStyleMapMeta {
    style_count: u32,
    override_count: u32,
    has_index: u32,
    _pad: u32,
};

@group(2) @binding(5) var<storage, read> bar_style_slots: array<BarStyleSlot>;
@group(2) @binding(6) var<storage, read> bar_style_overrides: array<BarStyleOverride>;
@group(2) @binding(7) var<uniform> bar_style_meta: BarStyleMapMeta;

struct ResolvedBarStyle {
    fill_color_premul: vec4<f32>,
    border_color_premul: vec4<f32>,
    border_width_px: f32,
    gap_px: f32,
    width_ratio: f32,
};

const BAR_STYLE_MASK_FILL: u32 = 1u;
const BAR_STYLE_MASK_BORDER_COLOR: u32 = 2u;
const BAR_STYLE_MASK_BORDER_WIDTH: u32 = 4u;
const BAR_STYLE_MASK_GAP: u32 = 8u;
const BAR_STYLE_MASK_WIDTH_RATIO: u32 = 16u;

fn apply_bar_style_slot(base: ResolvedBarStyle, slot: BarStyleSlot) -> ResolvedBarStyle {
    let mask = u32(slot.params.w);
    var out = base;
    if ((mask & BAR_STYLE_MASK_FILL) != 0u) {
        out.fill_color_premul = slot.fill_color_premul;
    }
    if ((mask & BAR_STYLE_MASK_BORDER_COLOR) != 0u) {
        out.border_color_premul = slot.border_color_premul;
    }
    if ((mask & BAR_STYLE_MASK_BORDER_WIDTH) != 0u) {
        out.border_width_px = max(slot.params.x, 0.0);
    }
    if ((mask & BAR_STYLE_MASK_GAP) != 0u) {
        out.gap_px = max(slot.params.y, 0.0);
    }
    if ((mask & BAR_STYLE_MASK_WIDTH_RATIO) != 0u) {
        out.width_ratio = clamp(slot.params.z, 0.0, 1.0);
    }
    return out;
}

fn resolve_bar_style(base: ResolvedBarStyle, inst: u32) -> ResolvedBarStyle {
    var out = base;
    // Declaration order is meaningful: later partial records for one bin
    // layer over earlier ones, matching the model contract.
    for (var i = 0u; i < bar_style_meta.override_count; i = i + 1u) {
        let ov = bar_style_overrides[i];
        if (ov.bin_index == inst) {
            out = apply_bar_style_slot(
                out,
                BarStyleSlot(ov.fill_color_premul, ov.border_color_premul, ov.params),
            );
        }
    }
    return out;
}

fn base_bar_style() -> ResolvedBarStyle {
    return ResolvedBarStyle(
        style.color_premul,
        style.dash[0],
        max(style.line_width_px, 0.0),
        max(style.cap_half_px, 0.0),
        clamp(style.cap_width_px, 0.0, 1.0),
    );
}

fn bar_vertex(in: VsIn, resolved: ResolvedBarStyle) -> VsOut {
    let seg = in.vi / 6u;
    let horizontal = style.shape_id == BAR_ORIENT_HORIZONTAL;

    var out: VsOut;
    out.color_premul = select(
        resolved.border_color_premul,
        resolved.fill_color_premul,
        seg == 0u,
    );

    let outer = bar_outer_ndc(in, horizontal, resolved.gap_px, resolved.width_ratio);
    // A bin of zero width, or one the gap closed entirely, draws nothing.
    if (!(outer.z > outer.x) || !(outer.w > outer.y)) {
        out.pos = vec4<f32>(outer.xy, 0.0, 1.0);
        return out;
    }
    let rect = bar_segment_rect(seg, outer, bar_inner_ndc(outer, resolved.border_width_px));

    // Triangle-list corner map [0,1,2, 2,1,3] over
    // {0: (x0,y0), 1: (x1,y0), 2: (x0,y1), 3: (x1,y1)}.
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let x = select(rect.x, rect.z, (corner & 1u) != 0u);
    let y = select(rect.y, rect.w, corner >= 2u);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    return bar_vertex(in, base_bar_style());
}

@vertex
fn vs_mapped(in: VsIn, @builtin(instance_index) inst: u32) -> VsOut {
    return bar_vertex(in, resolve_bar_style(base_bar_style(), inst));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color_premul;
}

// ───── Exact data picking (compute entry) ───────────────────────────────────
//
// The compute entry calls `bar_outer_ndc_values`, the same function as the
// render vertex entry. CPU code supplies only lane bases, counts and style
// scalars; it never reconstructs or stores bar rectangles.

const DATA_PICK_KIND_HISTOGRAM_BIN: u32 = 1u;
const DATA_PICK_WORKGROUP_SIZE: u32 = 64u;

struct DataPickQuery {
    // xy = cursor NDC, zw = cursor axis-t.
    cursor_ndc_t: vec4<f32>,
    // x = max distance px, y = scaled bar gap px, z = bar width ratio,
    // w = contour width px.
    limits: vec4<f32>,
    // x = flags, y = source paint order, z = item count, w = orientation.
    data: vec4<u32>,
    // x = edge-column f32 lane base, y = value-column lane base.
    bases: vec4<u32>,
    // xy = histogram baseline (hi, lo).
    baseline: vec4<f32>,
};

// Thirty-two bytes, byte-for-byte with Rust's `DataPickCandidateGpu`.
struct DataPickCandidate {
    valid: u32,
    paint_order: u32,
    kind: u32,
    index0: u32,
    index1: u32,
    index2: u32,
    distance_px: f32,
    primitive_order: u32,
};

@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(2) var<storage, read> data_pick_pool: array<f32>;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

var<workgroup> data_pick_shared: array<DataPickCandidate, DATA_PICK_WORKGROUP_SIZE>;

fn data_pick_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

fn data_pick_pair(base: u32, index: u32) -> vec2<f32> {
    let lane = base + index * 2u;
    return vec2<f32>(data_pick_pool[lane], data_pick_pool[lane + 1u]);
}

fn data_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(
        0u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        0u,
        0u,
        0u,
        0.0,
        0u,
    );
}

fn data_pick_better(candidate: DataPickCandidate, incumbent: DataPickCandidate) -> bool {
    if (candidate.valid == 0u) {
        return false;
    }
    if (incumbent.valid == 0u) {
        return true;
    }
    if (candidate.distance_px != incumbent.distance_px) {
        return candidate.distance_px < incumbent.distance_px;
    }
    // Later bars are painted later when malformed/reversed bins overlap.
    return candidate.primitive_order > incumbent.primitive_order;
}

fn data_pick_bar(bin_index: u32) -> DataPickCandidate {
    let edge_lo = data_pick_pair(data_pick_query.bases.x, bin_index);
    let edge_hi = data_pick_pair(data_pick_query.bases.x, bin_index + 1u);
    let value = data_pick_pair(data_pick_query.bases.y, bin_index);
    if (!all(vec4<bool>(
        data_pick_finite(edge_lo.x),
        data_pick_finite(edge_lo.y),
        data_pick_finite(edge_hi.x),
        data_pick_finite(edge_hi.y),
    )) || !data_pick_finite(value.x) || !data_pick_finite(value.y)) {
        return data_pick_invalid();
    }
    let resolved = resolve_bar_style(
        ResolvedBarStyle(
            vec4<f32>(0.0),
            vec4<f32>(0.0),
            0.0,
            data_pick_query.limits.y,
            data_pick_query.limits.z,
        ),
        bin_index,
    );
    let outer = bar_outer_ndc_values(
        edge_lo,
        edge_hi,
        value,
        data_pick_query.baseline.xy,
        data_pick_query.data.w == BAR_ORIENT_HORIZONTAL,
        resolved.gap_px,
        resolved.width_ratio,
    );
    if (!all(vec4<bool>(
        data_pick_finite(outer.x),
        data_pick_finite(outer.y),
        data_pick_finite(outer.z),
        data_pick_finite(outer.w),
    )) || !(outer.z > outer.x) || !(outer.w > outer.y)) {
        return data_pick_invalid();
    }
    let cursor = data_pick_query.cursor_ndc_t.xy;
    let outside_ndc = max(max(outer.xy - cursor, cursor - outer.zw), vec2<f32>(0.0));
    let outside_px = outside_ndc / transform.pixel_to_ndc;
    let distance_px = length(outside_px);
    if (!data_pick_finite(distance_px)
        || !data_pick_finite(data_pick_query.limits.x)
        || distance_px > max(data_pick_query.limits.x, 0.0)) {
        return data_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_HISTOGRAM_BIN,
        bin_index,
        0u,
        0u,
        distance_px,
        bin_index,
    );
}

@compute @workgroup_size(DATA_PICK_WORKGROUP_SIZE)
fn pick_histogram_bin(@builtin(local_invocation_index) local_index: u32) {
    var best = data_pick_invalid();
    var index = local_index;
    while (index < data_pick_query.data.z) {
        let candidate = data_pick_bar(index);
        if (data_pick_better(candidate, best)) {
            best = candidate;
        }
        index = index + DATA_PICK_WORKGROUP_SIZE;
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

// ───── Typed selection overlay (render entry) ───────────────────────────────

struct DataSelection {
    color_premul: vec4<f32>,
    // x = cell/bar outline width, y = selected contour extra width.
    metrics: vec4<f32>,
    // x = kind, y/z/w = kind-specific indices.
    indices: vec4<u32>,
};

@group(2) @binding(4) var<uniform> data_selection: DataSelection;

/// Four outline quads around the exact rendered bar outer rectangle. The
/// caller draws only the selected bin instance, so no coordinate or rectangle
/// is stored in selection state.
@vertex
fn vs_bar_selection(in: VsIn) -> VsOut {
    let horizontal = style.shape_id == BAR_ORIENT_HORIZONTAL;
    let outer = bar_outer_ndc_values(
        in.edge_lo,
        in.edge_hi,
        in.value,
        style.dash[1].xy,
        horizontal,
        style.cap_half_px,
        style.cap_width_px,
    );
    var out: VsOut;
    out.color_premul = data_selection.color_premul;
    if (!(outer.z > outer.x) || !(outer.w > outer.y)) {
        out.pos = vec4<f32>(outer.xy, 0.0, 1.0);
        return out;
    }
    let border = max(data_selection.metrics.x, 0.0) * transform.pixel_to_ndc;
    let inset = min(border, (outer.zw - outer.xy) * 0.5);
    let inner = vec4<f32>(outer.xy + inset, outer.zw - inset);
    let segment = in.vi / 6u + 1u;
    let rect = bar_segment_rect(segment, outer, inner);
    var corner_map = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corner_map[in.vi % 6u];
    let x = select(rect.x, rect.z, (corner & 1u) != 0u);
    let y = select(rect.y, rect.w, corner >= 2u);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}
