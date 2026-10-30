// Columnar SDF scatter shader.
//
// slot 0: unit quad (per-vertex, ±1 NDC).
// slot 1: X column (per-instance, f32 hi/lo pair).
// slot 2: Y column (per-instance, f32 hi/lo pair).
//
// Marker shapes — style.shape_id is the stable code from data_render/style.rs::shape_id().
// Existing ids 0..8 are kept compatible; newer scientific-plot symbols append
// after them. Shape decoding below maps those ids to compact SDF base shapes
// plus an open/filled flag.

struct VsIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) polar_px: vec2<f32>,
    @location(2) @interpolate(flat) polar_limits: vec4<f32>,
    @location(3) @interpolate(flat) polar_sector: vec4<f32>,
};

// Quad half-extent in pixels = shape-specific axis extent + QUAD_MARGIN_PX.
// `point_radius_px` is the circle reference radius; non-circular shapes are
// normalized below so the same marker size has a similar visual weight.
// Outline
// strokes are centered on the shape contour, so they reach up to
// STROKE_HALF_PX plus the AA feather (~1 px) beyond the radius; without
// the margin the quad edge would clip them. fs_main must reconstruct
// pixel coordinates with this same expanded scale.

@vertex
fn vs_main(in: VsIn) -> VsOut {
    return scatter_vertex(in, sample_valid(transform, in.x, in.y), data_to_ndc(transform, in.x, in.y));
}

// A ternary chart's markers (`Config.ternary`): the third component at
// slot 3, the marker placed at `ternary_ndc`.
struct VsTernaryIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
    @location(3) c: vec2<f32>,
};

@vertex
fn vs_ternary(in: VsTernaryIn) -> VsOut {
    let base = VsIn(in.quad_pos, in.x, in.y);
    return scatter_vertex(base, ternary_sample_valid(in.x, in.y, in.c), ternary_ndc(transform, in.x, in.y, in.c));
}

// The precise marker's vertex at `center_ndc`, placed by its entry; a
// sample that is not `valid` is not drawn.
fn scatter_vertex(in: VsIn, valid: bool, center_ndc: vec2<f32>) -> VsOut {
    if (!valid) {
        var masked: VsOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let half_px =
        visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_main quad, so
    // one unit here is one screen pixel.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    // Filled: cover d < 0. Outline/Cross: ~1.5 px stroke centered on the
    // d == 0 contour.
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ──────────────── sketch mode ────────────────
// Hand-drawn entry points selected as a separate pipeline variant. The precise
// entries above never read the sketch Transform fields.

// Per-point style mapping. The default precise
// path above remains the three-slot fast path; this entry is selected only for
// scatter series with a style table, style-index column, or sparse overrides.
struct VsMappedIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
    @location(3) style_index: f32,
    // Value columns of the continuous maps (`point_color_map` /
    // `point_size_map`), per-instance hi/lo pairs read straight from the
    // column pool. A series without the map binds its X column here as inert
    // filler; the flags in `point_value_map` say which of them are real.
    @location(4) color_value: vec2<f32>,
    @location(5) size_value: vec2<f32>,
};

struct ScatterStyleSlot {
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleOverride {
    point_index: u32,
    // Keep this layout byte-for-byte with Rust's ScatterStyleOverrideGpu:
    // u32 + three scalar u32 pads, then two vec4<f32> fields (48 bytes).
    // A WGSL vec3<u32> would align to 16 bytes and shift color_premul.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    color_premul: vec4<f32>,
    params: vec4<f32>,
};

struct ScatterStyleMapMeta {
    style_count: u32,
    override_count: u32,
    has_index: u32,
    _pad: u32,
};

@group(2) @binding(5) var<storage, read> scatter_style_slots: array<ScatterStyleSlot>;
@group(2) @binding(6) var<storage, read> scatter_style_overrides: array<ScatterStyleOverride>;
@group(2) @binding(7) var<uniform> scatter_style_meta: ScatterStyleMapMeta;

// Continuous value maps (group 3). The colour
// side mirrors the field shader: the colourbar's bounds (already log10 when
// `VALUE_MAP_LOG_Z` is set), its `nan_color` and its ramp control points, so a
// colour-mapped point and a heatmap cell with the same z draw the same colour.
// The size side is `PointSizeMap`: value domain as hi/lo pairs and the pixel
// range already multiplied by the export scale. CPU twin:
// `data_render/value_map.rs::PointValueMapParamsGpu`, 64 B.
struct PointValueMapParams {
    z_min: vec2<f32>,
    z_max: vec2<f32>,
    size_domain_min: vec2<f32>,
    size_domain_max: vec2<f32>,
    // (min_px, max_px)
    size_px: vec2<f32>,
    flags: u32,
    stop_count: u32,
    nan_color_premul: vec4<f32>,
};

const VALUE_MAP_COLOR: u32 = 1u;
const VALUE_MAP_SIZE: u32 = 2u;
const VALUE_MAP_LOG_Z: u32 = 4u;
const VALUE_MAP_SIZE_AREA: u32 = 8u;

@group(3) @binding(0) var<uniform> point_value_map: PointValueMapParams;
@group(3) @binding(1) var<storage, read> point_value_stops: array<vec4<f32>>;

fn value_map_flag(bit: u32) -> bool {
    return (point_value_map.flags & bit) != 0u;
}


/// The colour-map value as a ramp position, or a negative number when it
/// cannot be placed.
fn value_ramp_t(zv: vec2<f32>) -> f32 {
    return ramp_t(zv, point_value_map.z_min, point_value_map.z_max, value_map_flag(VALUE_MAP_LOG_Z));
}

/// The colourbar's ramp at `t`, through the shared `ramp_segment` /
/// `ramp_blend`; only the stop lookup is this shader's.
fn value_ramp(t: f32) -> vec4<f32> {
    let n = point_value_map.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return point_value_stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(point_value_stops[index], point_value_stops[index + 1u], segment.y);
}

/// Premultiplied colour for a colour-map value; `nan_color` when unplaceable.
fn value_map_color(zv: vec2<f32>) -> vec4<f32> {
    let t = value_ramp_t(zv);
    if (t < 0.0) {
        return point_value_map.nan_color_premul;
    }
    let c = value_ramp(t);
    return vec4<f32>(c.rgb * c.a, c.a);
}

/// Marker radius for a size-map value, in pixels; negative hides the point.
fn value_map_radius(sv: vec2<f32>) -> f32 {
    return size_map_radius(
        sv,
        point_value_map.size_domain_min,
        point_value_map.size_domain_max,
        point_value_map.size_px.x,
        point_value_map.size_px.y,
        value_map_flag(VALUE_MAP_SIZE_AREA),
    );
}

struct ResolvedMappedStyle {
    color_premul: vec4<f32>,
    radius_px: f32,
    shape_id: u32,
};

struct VsMappedOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) color_premul: vec4<f32>,
    @location(2) radius_px: f32,
    @location(3) @interpolate(flat) shape_id: u32,
    @location(4) polar_px: vec2<f32>,
    @location(5) @interpolate(flat) polar_limits: vec4<f32>,
    @location(6) @interpolate(flat) polar_sector: vec4<f32>,
};

const STYLE_MASK_COLOR: u32 = 1u;
const STYLE_MASK_RADIUS: u32 = 2u;
const STYLE_MASK_SHAPE: u32 = 4u;

fn valid_style_index(v: f32) -> bool {
    return v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn apply_style_slot(base: ResolvedMappedStyle, slot: ScatterStyleSlot) -> ResolvedMappedStyle {
    let mask = u32(slot.params.z);
    var out = base;
    if ((mask & STYLE_MASK_COLOR) != 0u) {
        out.color_premul = slot.color_premul;
    }
    if ((mask & STYLE_MASK_RADIUS) != 0u) {
        out.radius_px = max(slot.params.x, 0.0);
    }
    if ((mask & STYLE_MASK_SHAPE) != 0u) {
        out.shape_id = u32(max(slot.params.y, 0.0));
    }
    return out;
}

// Precedence per channel: sparse override > continuous value map > style
// table slot > series base. A hidden point (non-finite size value) carries a
// negative radius; an override that sets the radius makes it visible again.
fn resolve_mapped_style(
    style_index: f32,
    inst: u32,
    color_value: vec2<f32>,
    size_value: vec2<f32>,
) -> ResolvedMappedStyle {
    var out = ResolvedMappedStyle(style.color_premul, style.point_radius_px, style.shape_id);
    let point_index = inst + scatter_style_meta._pad;

    if (scatter_style_meta.has_index != 0u && valid_style_index(style_index)) {
        let idx = u32(round(style_index));
        if (idx < scatter_style_meta.style_count) {
            out = apply_style_slot(out, scatter_style_slots[idx]);
        }
    }

    if (value_map_flag(VALUE_MAP_COLOR)) {
        out.color_premul = value_map_color(color_value);
    }
    if (value_map_flag(VALUE_MAP_SIZE)) {
        out.radius_px = value_map_radius(size_value);
    }

    for (var i = 0u; i < scatter_style_meta.override_count; i = i + 1u) {
        let ov = scatter_style_overrides[i];
        if (ov.point_index == point_index) {
            out = apply_style_slot(out, ScatterStyleSlot(ov.color_premul, ov.params));
        }
    }
    return out;
}

@vertex
fn vs_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsMappedOut {
    return mapped_vertex(in, inst, sample_valid(transform, in.x, in.y), data_to_ndc(transform, in.x, in.y));
}

// A ternary chart's mapped markers: the third component at slot 6.
struct VsTernaryMappedIn {
    @location(0) quad_pos: vec2<f32>,
    @location(1) x: vec2<f32>,
    @location(2) y: vec2<f32>,
    @location(3) style_index: f32,
    @location(4) color_value: vec2<f32>,
    @location(5) size_value: vec2<f32>,
    @location(6) c: vec2<f32>,
};

fn ternary_mapped_in(in: VsTernaryMappedIn) -> VsMappedIn {
    return VsMappedIn(in.quad_pos, in.x, in.y, in.style_index, in.color_value, in.size_value);
}

@vertex
fn vs_ternary_mapped(in: VsTernaryMappedIn, @builtin(instance_index) inst: u32) -> VsMappedOut {
    return mapped_vertex(ternary_mapped_in(in), inst, ternary_sample_valid(in.x, in.y, in.c), ternary_ndc(transform, in.x, in.y, in.c));
}

// The mapped marker's vertex at `center_ndc`, placed by its entry.
fn mapped_vertex(in: VsMappedIn, inst: u32, valid: bool, center_ndc: vec2<f32>) -> VsMappedOut {
    if (!valid) {
        var masked: VsMappedOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let resolved = resolve_mapped_style(in.style_index, inst, in.color_value, in.size_value);
    if (resolved.radius_px < 0.0) {
        var hidden: VsMappedOut;
        hidden.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return hidden;
    }
    let half_px =
        visual_shape_radius(base_shape_id(resolved.shape_id), resolved.radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsMappedOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    out.color_premul = resolved.color_premul;
    out.radius_px = resolved.radius_px;
    out.shape_id = resolved.shape_id;
    return out;
}

@fragment
fn fs_mapped(in: VsMappedOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let filled = shape_is_filled(in.shape_id);
    let base = base_shape_id(in.shape_id);
    let visual_r = visual_shape_radius(base, in.radius_px);
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX);

    let d = shape_distance(base, p, visual_r, filled);
    let aa = fwidth(d);
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return in.color_premul * alpha;
}

// The picked ring's marker mode (`Style.primitive_flags` of the ring entry):
// the draw style whose marker it surrounds, numbered like the pick shader's
// `PICK_MARKER_*` (`gpu_pick/types.rs::GpuPickDrawStyle`).
const RING_MARKER_PRECISE: u32 = 0u;
const RING_MARKER_SKETCH: u32 = 1u;
const RING_MARKER_PLANET: u32 = 2u;
const RING_MARKER_STAR: u32 = 3u;

/// The farthest a precise marker of `shape_id` and radius `r` covers from its
/// centre: the circumradius of its contour (`shape_distance` at the visual
/// radius, sized like `visual_shape_radius`), plus the stroke half-width for
/// an outline or an open plus or cross. `marker_edge <= 0` lies within it.
fn marker_extent_px(shape_id: u32, r: f32) -> f32 {
    let filled = shape_is_filled(shape_id);
    let base = base_shape_id(shape_id);
    let rv = visual_shape_radius(base, max(r, 0.0));
    var extent = rv;
    switch base {
        case 1u: { extent = rv * sqrt(2.0); } // square corner
        case 4u: { // cross: the diagonal strokes, or the rotated filled plus
            if (filled) {
                let arm = max(rv * 0.32, STROKE_HALF_PX);
                extent = sqrt(rv * rv + arm * arm);
            } else {
                extent = rv * sqrt(2.0);
            }
        }
        case 8u: { // plus: the arm ends, or the filled arms' corners
            if (filled) {
                let arm = max(rv * 0.32, STROKE_HALF_PX);
                extent = sqrt(rv * rv + arm * arm);
            }
        }
        default: {} // circle, triangles, diamond, polygons, star: vertices on rv
    }
    return extent + select(STROKE_HALF_PX, 0.0, filled);
}

/// How far the marker the chart's draw style paints at instance `inst`
/// reaches from its centre, on the envelope the picker picks (SHADER_COMMON.md
/// §5.7): the precise contour, grown by the sketch wobble; a Milkyway
/// planet's disc, rim glow and ring band; a Constellation star's sprite disc.
fn ring_marker_extent_px(shape_id: u32, r: f32, inst: u32) -> f32 {
    let mode = style.primitive_flags & 3u;
    if (mode == RING_MARKER_STAR) {
        let star = constellation_star_radius(r, styled_point_index(inst), style.series_salt);
        return star * 4.0 + QUAD_MARGIN_PX;
    }
    if (mode == RING_MARKER_PLANET) {
        // In the planet's own frame (`fs_planet` scales the quad to
        // `r_planet`): the ring band's major axis, or the rim glow's reach
        // above `PLANET_ALPHA_FLOOR`, inside the quad's corner.
        let r_planet = max(r, 1.0);
        let rim_gain = clamp(transform.style_params[1].w, 0.0, 2.0);
        var reach = r_planet * RING_OUTER;
        if (rim_gain > PLANET_ALPHA_FLOOR) {
            reach = max(reach, r_planet + max(r_planet * 0.10, 1.0) * log(rim_gain / PLANET_ALPHA_FLOOR));
        }
        let quad = r * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
        let screen = reach * quad / (r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX);
        return min(screen, quad * sqrt(2.0));
    }
    let extent = marker_extent_px(shape_id, r);
    if (mode == RING_MARKER_SKETCH) {
        return extent + sketch_marker_wobble_px(transform.style_params[0].x, r);
    }
    return extent;
}

struct VsPickRingOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) radius_px: f32,
    @location(2) polar_px: vec2<f32>,
    @location(3) @interpolate(flat) polar_limits: vec4<f32>,
    @location(4) @interpolate(flat) polar_sector: vec4<f32>,
};

// The picked ring: a circle `radius_extra_px` (`Style.cap_half_px`) outside
// the marker the chart draws at that point (`ring_marker_extent_px` of the
// marker radius `Style.point_radius_px` and shape `Style.shape_id`, both at
// the render scale).
@vertex
fn vs_pick_ring(in: VsIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    return ring_vertex(in, inst, sample_valid(transform, in.x, in.y), data_to_ndc(transform, in.x, in.y));
}

@vertex
fn vs_pick_ring_ternary(in: VsTernaryIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    let base = VsIn(in.quad_pos, in.x, in.y);
    return ring_vertex(base, inst, ternary_sample_valid(in.x, in.y, in.c), ternary_ndc(transform, in.x, in.y, in.c));
}

// The ring's vertex around the marker at `center_ndc`.
fn ring_vertex(in: VsIn, inst: u32, valid: bool, center_ndc: vec2<f32>) -> VsPickRingOut {
    if (!valid) {
        var masked: VsPickRingOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let radius = max(ring_marker_extent_px(style.shape_id, style.point_radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@vertex
fn vs_pick_ring_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    return mapped_ring_vertex(in, inst, sample_valid(transform, in.x, in.y), data_to_ndc(transform, in.x, in.y));
}

@vertex
fn vs_pick_ring_ternary_mapped(in: VsTernaryMappedIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    return mapped_ring_vertex(ternary_mapped_in(in), inst, ternary_sample_valid(in.x, in.y, in.c), ternary_ndc(transform, in.x, in.y, in.c));
}

// The mapped ring's vertex around the marker at `center_ndc`.
fn mapped_ring_vertex(in: VsMappedIn, inst: u32, valid: bool, center_ndc: vec2<f32>) -> VsPickRingOut {
    if (!valid) {
        var masked: VsPickRingOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let resolved = resolve_mapped_style(in.style_index, inst, in.color_value, in.size_value);
    if (resolved.radius_px < 0.0) {
        var hidden: VsPickRingOut;
        hidden.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return hidden;
    }
    // The ring entries use style.cap_half_px as radius_extra_px. The
    // errorbar shader is not involved in this pipeline, so the slot is local
    // to this decoration entry.
    let radius = max(ring_marker_extent_px(resolved.shape_id, resolved.radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@fragment
fn fs_pick_ring(in: VsPickRingOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let radius = max(in.radius_px, 0.0);
    let stroke = max(style.line_width_px, 0.5);
    let half_px = radius + stroke * 0.5 + QUAD_MARGIN_PX;
    let d = abs(length(in.local_pos * half_px) - radius) - stroke * 0.5;
    let aa = max(fwidth(d), 0.5);
    let alpha = 1.0 - smoothstep(-aa, aa, d);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// Sketch varyings. The fragment stage must not read `transform` (the shared
// transform bind group is vertex-only), so the vertex stage forwards the
// resolved per-marker seed and the clamped wobble amplitude as flat values.
struct VsSketchOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    // Global sketch seed + instance index — per-point shape variation.
    @location(1) @interpolate(flat) seed_inst: u32,
    // Contour wobble amplitude in px: min(amplitude·0.5, radius·0.35).
    @location(2) @interpolate(flat) wobble_px: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

// Same data→NDC placement as vs_main, with the quad grown by the wobble
// amplitude so the perturbed contour (and its stroke + AA feather) never
// clips at the quad edge. fs_sketch reconstructs pixel coordinates with the
// same expanded scale.
@vertex
fn vs_sketch(in: VsIn, @builtin(instance_index) inst: u32) -> VsSketchOut {
    if (!sample_valid(transform, in.x, in.y)) {
        var masked: VsSketchOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(transform, in.x, in.y);

    let wobble = sketch_marker_wobble_px(transform.style_params[0].x, style.point_radius_px);
    let half_px = visual_shape_radius(base_shape_id(style.shape_id), style.point_radius_px)
        + QUAD_MARGIN_PX
        + wobble;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsSketchOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].z) ^ style.series_salt) + styled_point_index(inst);
    out.wobble_px = wobble;
    return out;
}

// The sketch marker's contour is `sketch_marker_distance`'s: the precise
// shape perturbed by angle-parameterized noise, seeded per marker.
@fragment
fn fs_sketch(in: VsSketchOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r = style.point_radius_px;
    let filled = shape_is_filled(style.shape_id);
    let base = base_shape_id(style.shape_id);
    let visual_r = visual_shape_radius(base, r);
    // Pixel-space position — same expanded scale as the vs_sketch quad.
    let p = in.local_pos * (visual_r + QUAD_MARGIN_PX + in.wobble_px);

    let d = sketch_marker_distance(base, p, visual_r, filled, in.wobble_px, in.seed_inst);

    let aa = fwidth(d);
    let edge = marker_edge(d, filled);
    let alpha = 1.0 - smoothstep(-aa, aa, edge);
    if (alpha <= 0.0) {
        discard;
    }
    return style.color_premul * alpha;
}

// ────────────── constellation mode ───────────
//   - The ring's POSITION ANGLE encodes the series: the existing
//     ScatterShape SSoT maps to a tilt (see cons_ring_angle) — no new
//     per-series fields.
//   - The planet surface samples a baked procedural atlas (2×2 archetypes,
//     picked per point); shading is sphere lambert + limb darkening.
//   - The series point_color appears ONLY as a thin atmospheric rim glow,
//     like the line ribbon: bodies stay physical, identity stays visible.
// Blending is premultiplied alpha (planets occlude the star field), unlike
// the additive line passes.

@group(2) @binding(0) var cons_psf_tex: texture_2d<f32>; // R=core, G=halo
@group(2) @binding(1) var cons_lut_tex: texture_2d<f32>; // 256x1 blackbody
@group(2) @binding(2) var cons_samp: sampler;
@group(2) @binding(3) var cons_planet_atlas: texture_2d<f32>;
@group(2) @binding(4) var cons_ring_tex: texture_2d<f32>;

struct VsConstellationStarOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) brightness: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

@vertex
fn vs_constellation_star(in: VsIn, @builtin(instance_index) inst: u32) -> VsConstellationStarOut {
    if (!sample_valid(transform, in.x, in.y)) {
        var masked: VsConstellationStarOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(transform, in.x, in.y);

    let seed = style.series_salt;

    let global_inst = styled_point_index(inst);
    let u_b = sketch_hash01(global_inst, seed ^ 0x86A9u);
    let brightness = 0.22 + 0.78 * pow(u_b, 2.4);
    let star_radius = constellation_star_radius(style.point_radius_px, global_inst, seed);
    let half_px = star_radius * 4.0 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    let h_t = sketch_hash01(global_inst, seed ^ 0x7E47u);
    let t_norm = mix(0.08, 0.92, h_t);
    let tint = textureLoad(
        cons_lut_tex,
        vec2<i32>(i32(clamp(t_norm, 0.0, 1.0) * 255.0), 0),
        0,
    ).rgb;

    var out: VsConstellationStarOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.uv = in.quad_pos * 0.5 + vec2<f32>(0.5, 0.5);
    out.tint = tint;
    out.brightness = brightness;
    return out;
}

@fragment
fn fs_constellation_star(in: VsConstellationStarOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let s = textureSampleLevel(cons_psf_tex, cons_samp, in.uv, 0.0);
    let col = (vec3<f32>(1.0) * s.r + in.tint * s.g) * in.brightness;
    let a = clamp(max(col.r, max(col.g, col.b)), 0.0, 1.0);
    let marker_opacity = clamp(style.color_premul.a, 0.0, 1.0);
    let star_opacity = clamp(transform.style_params[0].x, 0.0, 1.0) * marker_opacity;
    let out_a = a * star_opacity;
    if (out_a <= 0.002) {
        discard;
    }
    return vec4<f32>(min(col, vec3<f32>(a)) * star_opacity, out_a);
}

struct VsPlanetOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local_pos: vec2<f32>,
    @location(1) @interpolate(flat) seed_inst: u32,
    // Atmospheric rim-glow strength (MilkywayOptions.planet_rim) —
    // forwarded by the vertex stage; the transform group is vertex-only.
    @location(2) @interpolate(flat) rim_gain: f32,
    @location(3) polar_px: vec2<f32>,
    @location(4) @interpolate(flat) polar_limits: vec4<f32>,
    @location(5) @interpolate(flat) polar_sector: vec4<f32>,
};

@vertex
fn vs_planet(in: VsIn, @builtin(instance_index) inst: u32) -> VsPlanetOut {
    if (!sample_valid(transform, in.x, in.y)) {
        var masked: VsPlanetOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(transform, in.x, in.y);

    let half_px = style.point_radius_px * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPlanetOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(transform, out.pos.xy);
    out.polar_limits = polar_clip_limits(transform);
    out.polar_sector = polar_clip_sector(transform);
    out.local_pos = in.quad_pos;
    out.seed_inst = (u32(transform.style_params[0].w) ^ style.series_salt) + styled_point_index(inst);
    out.rim_gain = clamp(transform.style_params[1].w, 0.0, 2.0);
    return out;
}

@fragment
fn fs_planet(in: VsPlanetOut) -> @location(0) vec4<f32> {
    if (!polar_fragment_visible(in.polar_px, in.polar_limits, in.polar_sector)) {
        discard;
    }
    let r_planet = max(style.point_radius_px, 1.0);
    let p = in.local_pos * (r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX);
    let dist = length(p);
    let aa = max(fwidth(dist), 0.5);

    // Per-point variation channels.
    let h_arch = sketch_hash01(in.seed_inst, 0xA2C4u);
    let h_spin = sketch_hash01(in.seed_inst, 0x591Eu);

    // Ring frame: rotate into the ring plane. Position angle = series shape
    // mapping ± a small per-point jitter (real systems aren't aligned).
    let phi = planet_ring_phi(style.shape_id, in.seed_inst);
    let cphi = cos(phi);
    let sphi = sin(phi);
    let q = planet_ring_frame(p, phi);

    // ── Physically consistent ring/planet frame ──
    // The ring lies in the planet's EQUATORIAL plane. With the projected
    // ellipse ratio RING_INCL = sin(i), the spin pole is
    //   P  = (0, cos i, sin i)   (screen-up in ring frame, tipped toward
    //                             the viewer — the north pole is visible)
    // and the equatorial basis is
    //   e1 = (1, 0, 0),  e2 = P × e1 = (0, sin i, −cos i).
    // e2's far hemisphere projects to +q.y, so the UPPER ring half is the
    // far (occluded) one and the lower half passes in front — and surface
    // latitude derives from the same pole, which is what makes the bands
    // bow with exactly the ring's curvature family (the Saturn look).
    let sin_i = RING_INCL;
    let cos_i = sqrt(1.0 - sin_i * sin_i);

    // Projected ring ellipse: radial coordinate in ring-plane units.
    let rho = planet_ring_rho(q, r_planet);
    let in_band = rho > RING_INNER && rho < RING_OUTER;
    var ring_a = 0.0;
    var ring_rgb = vec3<f32>(0.0);
    if (in_band) {
        let u = (rho - RING_INNER) / (RING_OUTER - RING_INNER);
        // textureSampleLevel, NOT textureSample: this branch is non-uniform
        // (fragment-position dependent), and browser WGSL (Tint) hard-rejects
        // implicit-derivative sampling there — the whole module fails to
        // compile, which blanks every scatter draw on wasm. The bake
        // textures are single-mip, so explicit LOD 0 is pixel-identical.
        let s = textureSampleLevel(cons_ring_tex, cons_samp, vec2<f32>(u, 0.5), 0.0);
        // Edge AA along the band borders.
        let band_aa = smoothstep(RING_INNER, RING_INNER + 0.06, rho)
            * (1.0 - smoothstep(RING_OUTER - 0.06, RING_OUTER, rho));
        ring_a = s.a * band_aa;
        ring_rgb = s.rgb;

        // Planet shadow on the ring — the REAL model: the shadow is the
        // planet's anti-light cylinder cutting the ring plane, not "the
        // whole back half" (a binary front/back gate puts a hard seam where
        // the ring crosses the major axis). Reconstruct the ring point in
        // 3D (ring plane: depth z = −u2·cos i with u2 = q.y / sin i), put
        // the light in the same rotated frame, and shade smoothly inside
        // the cylinder behind the body.
        let ring_p3 = vec3<f32>(q.x, q.y, -q.y * cos_i / max(sin_i, 1e-4)) / r_planet;
        let l0 = normalize(vec3<f32>(-0.55, 0.5, 0.62));
        let lq = vec3<f32>(
            l0.x * cphi + l0.y * sphi,
            -l0.x * sphi + l0.y * cphi,
            l0.z,
        );
        let t_axis = dot(ring_p3, lq);
        let d_perp = length(ring_p3 - lq * t_axis);
        let umbra = (1.0 - smoothstep(0.92, 1.18, d_perp))
            * smoothstep(0.05, 0.45, -t_axis);
        ring_rgb = ring_rgb * (1.0 - 0.8 * umbra);
    }
    // Lower half (q.y < 0) passes in FRONT of the planet, upper half behind.
    let ring_front = q.y < 0.0;

    // Planet disc + sphere shading.
    let disc = 1.0 - smoothstep(r_planet - aa, r_planet + aa, dist);
    var planet_rgb = vec3<f32>(0.0);
    if (disc > 0.0) {
        let pr = min(dist / r_planet, 0.9999);
        let z = sqrt(1.0 - pr * pr);
        let n = vec3<f32>(p.x / r_planet, p.y / r_planet, z);
        // Light from the upper-left, slightly toward the viewer.
        let l = normalize(vec3<f32>(-0.55, 0.5, 0.62));
        let diff = max(dot(n, l), 0.0);
        let limb = pow(max(z, 0.0), 0.45);

        // Sphere point in the ring frame (q.x, q.y, z toward viewer).
        let nq = vec3<f32>(q.x / r_planet, q.y / r_planet, z);
        // Latitude about the ring-plane pole; longitude in the equatorial
        // basis, spun per point.
        let lat = asin(clamp(nq.y * cos_i + nq.z * sin_i, -1.0, 1.0));
        let lon = atan2(nq.y * sin_i - nq.z * cos_i, nq.x);
        let arch = u32(h_arch * 4.0) % 4u;
        let tile = vec2<f32>(f32(arch % 2u), f32(arch / 2u));
        let inner_uv = vec2<f32>(
            fract(lon / 6.2831853 + 0.5 + h_spin),
            lat / 3.1415927 + 0.5,
        ) * 0.94 + vec2<f32>(0.03, 0.03);
        let uv = (tile + inner_uv) * 0.5;
        // Explicit LOD for the same Tint uniformity rule as the ring sample.
        let albedo = textureSampleLevel(cons_planet_atlas, cons_samp, uv, 0.0).rgb;

        planet_rgb = albedo * (0.18 + 0.88 * diff) * limb;
    }

    // Atmospheric rim glow — the series color's only appearance. Soft
    // exponential falloff outside the disc, not a hard shell.
    let rim = select(0.0, planet_rim_glow(dist, r_planet, in.rim_gain), dist > r_planet - aa);
    let rim_rgb = style.color_premul.rgb * rim;

    // Compose, premultiplied: back ring (occluded by the disc) → planet →
    // front ring over the planet → rim glow outside.
    var rgb = vec3<f32>(0.0);
    var a = 0.0;
    if (!ring_front) {
        let behind_a = ring_a * (1.0 - disc);
        rgb = ring_rgb * behind_a;
        a = behind_a;
    }
    rgb = rgb * (1.0 - disc) + planet_rgb * disc;
    a = a * (1.0 - disc) + disc;
    if (ring_front) {
        rgb = rgb * (1.0 - ring_a) + ring_rgb * ring_a;
        a = a * (1.0 - ring_a) + ring_a;
    }
    rgb = rgb + rim_rgb * (1.0 - a);
    a = a + rim * (1.0 - a);

    if (a <= PLANET_ALPHA_FLOOR) {
        discard;
    }
    return vec4<f32>(rgb, a);
}
