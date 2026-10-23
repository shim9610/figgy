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
    if (!sample_valid(in.x, in.y)) {
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
    let center_ndc = data_to_ndc(in.x, in.y);
    let half_px =
        visual_shape_radius(base_shape_id(resolved.shape_id), resolved.radius_px) + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsMappedOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
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
// `PICK_MARKER_*` (`gpu_pick.rs::GpuPickDrawStyle`).
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
    if (!sample_valid(in.x, in.y)) {
        var masked: VsPickRingOut;
        masked.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return masked;
    }
    let center_ndc = data_to_ndc(in.x, in.y);
    let radius = max(ring_marker_extent_px(style.shape_id, style.point_radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
    out.local_pos = in.quad_pos;
    out.radius_px = radius;
    return out;
}

@vertex
fn vs_pick_ring_mapped(in: VsMappedIn, @builtin(instance_index) inst: u32) -> VsPickRingOut {
    if (!sample_valid(in.x, in.y)) {
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
    let center_ndc = data_to_ndc(in.x, in.y);
    // The ring entries use style.cap_half_px as radius_extra_px. The
    // errorbar shader is not involved in this pipeline, so the slot is local
    // to this decoration entry.
    let radius = max(ring_marker_extent_px(resolved.shape_id, resolved.radius_px, inst) + style.cap_half_px, 0.0);
    let half_px = radius + max(style.line_width_px, 0.0) * 0.5 + QUAD_MARGIN_PX;
    let world = center_ndc + in.quad_pos * (half_px * transform.pixel_to_ndc);

    var out: VsPickRingOut;
    out.pos = vec4<f32>(world, 0.0, 1.0);
    out.polar_px = polar_clip_offset(out.pos.xy);
    out.polar_limits = polar_clip_limits();
    out.polar_sector = polar_clip_sector();
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
