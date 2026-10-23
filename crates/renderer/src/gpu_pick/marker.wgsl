// ───── end shared marker shape ─────

fn pick_nonnegative(v: f32) -> f32 {
    return select(0.0, v, v == v && v > 0.0);
}

fn pick_valid_style_index(v: f32) -> bool {
    return pick_is_finite(v) && v >= 0.0 && v <= 16777216.0 && abs(v - round(v)) <= 0.001;
}

fn pick_apply_style(radius_in: f32, shape_in: u32, slot: PickScatterStyleSlot) -> vec2<f32> {
    let mask = u32(slot.params.z);
    var radius = radius_in;
    var shape = shape_in;
    if (mask & PICK_STYLE_MASK_RADIUS) != 0u {
        radius = pick_nonnegative(slot.params.x);
    }
    if (mask & PICK_STYLE_MASK_SHAPE) != 0u {
        shape = u32(pick_nonnegative(slot.params.y));
    }
    return vec2<f32>(radius, f32(shape));
}

// ───── shared size-map radius (SHADER_COMMON.md §5.4; `shader_consistency`
// compares these function bodies with the scatter shader) ─────

/// A finite f32 by its exponent bits. Every ramp input is checked
/// explicitly: NaN and infinity propagation through arithmetic is not
/// portable in WGSL.
fn ramp_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

/// The marker radius for a size-map value `sv` (a hi/lo pair) over the
/// domain `domain_min..domain_max` (hi/lo pairs) onto `lo..hi` pixels, by
/// marker area when `area`, else by radius. Negative when the value is not
/// finite: the point is hidden. Twin of `PointSizeMap::radius_px`.
fn size_map_radius(sv: vec2<f32>, domain_min: vec2<f32>, domain_max: vec2<f32>, lo: f32, hi: f32, area: bool) -> f32 {
    if (!ramp_finite(sv.x) || !ramp_finite(sv.y) || !ramp_finite(sv.x + sv.y)) {
        return -1.0;
    }
    let num = (sv.x - domain_min.x) + (sv.y - domain_min.y);
    let span = (domain_max.x - domain_min.x) + (domain_max.y - domain_min.y);
    let t = clamp(num / span, 0.0, 1.0);
    if (area) {
        return sqrt(lo * lo + (hi * hi - lo * lo) * t);
    }
    return lo + (hi - lo) * t;
}

// The marker radius a size value maps to, or a negative number when the value
// is not finite and the point is hidden.
fn pick_size_map_radius(point_index: u32) -> f32 {
    if point_index >= pick_query_params.size_map.y {
        return -1.0;
    }
    let value = pick_read_pair(pick_query_params.size_map.x, point_index);
    let domain = pick_query_params.size_domain;
    return size_map_radius(
        value,
        domain.xy,
        domain.zw,
        pick_query_params.size_px.x,
        pick_query_params.size_px.y,
        (pick_query_params.style.x & PICK_FLAG_SIZE_AREA) != 0u,
    );
}

// One marker as the scatter shader resolves it before drawing: the radius
// in canvas pixels (the display scale applied, like `point_radius_px`), the
// shape code, and whether the size map hides the point.
struct PickMarker {
    radius: f32,
    shape: u32,
    hidden: bool,
};

// Precedence per channel, as in `resolve_mapped_style`: sparse override >
// size map > style table slot > series base. A hidden point stays hidden
// unless an override sets its radius.
fn pick_resolve_scatter_marker(point_index: u32) -> PickMarker {
    var radius = pick_nonnegative(pick_query_params.scatter_line.x);
    var shape = pick_query_params.series.z;
    let flags = pick_query_params.style.x;
    var hidden = false;
    if (flags & PICK_FLAG_STYLE_MAP) != 0u {
        if (flags & PICK_FLAG_STYLE_INDEX) != 0u && point_index < pick_query_params.style.w {
            let index_word = pick_query_params.data.w + point_index * 2u;
            let style_index = bitcast<f32>(pick_pool_words[index_word]);
            if pick_valid_style_index(style_index) {
                let slot_index = u32(round(style_index));
                if slot_index < pick_query_params.style.y {
                    let resolved = pick_apply_style(radius, shape, pick_style_slots[slot_index]);
                    radius = resolved.x;
                    shape = u32(resolved.y);
                }
            }
        }
    }
    if (flags & PICK_FLAG_SIZE_MAP) != 0u {
        let mapped = pick_size_map_radius(point_index);
        if mapped < 0.0 {
            hidden = true;
        } else {
            radius = mapped;
        }
    }
    if (flags & PICK_FLAG_STYLE_MAP) != 0u {
        for (var i = 0u; i < pick_query_params.style.z; i = i + 1u) {
            let override_row = pick_style_overrides[i];
            if override_row.point_index == point_index + pick_query_params.stream.x {
                let slot = PickScatterStyleSlot(override_row.color_premul, override_row.params);
                let resolved = pick_apply_style(radius, shape, slot);
                if (u32(slot.params.z) & PICK_STYLE_MASK_RADIUS) != 0u {
                    hidden = false;
                }
                radius = resolved.x;
                shape = u32(resolved.y);
            }
        }
    }
    return PickMarker(radius * pick_query_params.scatter_line.z, shape, hidden);
}

// ───── shared styled markers (SHADER_COMMON.md §5.7; `shader_consistency`
// compares these function bodies and constants with the scatter shader) ─────

const SKETCH_TAU: f32 = 6.28318530718;
// Six noise periods around the contour keep marker wobble visually local.
const SKETCH_CONTOUR_WOBBLES: f32 = 6.0;
const CONSTELLATION_POINT_STAR_GAIN: f32 = 1.0;
// Ring radii relative to the planet radius, and the apparent inclination of
// the ring plane (minor/major axis ratio of the projected ellipse).
const RING_INNER: f32 = 1.45;
const RING_OUTER: f32 = 2.3;
const RING_INCL: f32 = 0.32;
// Quad half-extent multiplier — room for the ring + rim glow + AA.
const PLANET_QUAD_EXTENT: f32 = 2.5;
// A planet fragment whose composed alpha is at or below this is discarded.
const PLANET_ALPHA_FLOOR: f32 = 0.003;

/// A sketch marker's contour wobble amplitude in pixels: half the style's
/// amplitude, at most 0.35 of the marker radius.
fn sketch_marker_wobble_px(amplitude_px: f32, radius_px: f32) -> f32 {
    return min(max(amplitude_px, 0.0) * 0.5, radius_px * 0.35);
}

/// A sketch marker's signed contour distance: the precise `shape_distance`
/// perturbed with angle-parameterized noise, d' = d + k·noise(θ/τ·C), seeded
/// per marker so every point gets its own hand-drawn outline. The noise
/// lattice has a seam at θ = ±π; its magnitude is ≤ wobble_px (≤ 0.75 px at
/// default amplitude) and reads as part of the hand-drawn look.
fn sketch_marker_distance(base: u32, p: vec2<f32>, r: f32, filled: bool, wobble_px: f32, seed: u32) -> f32 {
    let theta = atan2(p.y, p.x); // [-π, π]
    return shape_distance(base, p, r, filled)
        + wobble_px * sketch_noise(theta / SKETCH_TAU * SKETCH_CONTOUR_WOBBLES, seed);
}

/// A constellation marker star's core radius in pixels: the marker radius
/// with a per-point size jitter hashed from the point index, at least half a
/// pixel. The star's quad (and its PSF sprite) spans four times this plus
/// `QUAD_MARGIN_PX`.
fn constellation_star_radius(radius_px: f32, index: u32, seed: u32) -> f32 {
    let size_jitter = 0.86 + 0.30 * sketch_hash01(index, seed ^ 0x51A7u);
    return max(radius_px * CONSTELLATION_POINT_STAR_GAIN * size_jitter, 0.5);
}

// ScatterShape code → ring position angle. The first legacy shapes get hand-
// picked angles; newer marker codes share a deterministic fallback.
fn cons_ring_angle(shape_id: u32) -> f32 {
    switch shape_id {
        case 0u: { return 0.0; }          // Circle
        case 1u: { return 0.52; }         // Square        (+30°)
        case 2u: { return 1.05; }         // Triangle      (+60°)
        case 3u: { return -0.52; }        // Diamond       (−30°)
        case 4u: { return -1.05; }        // Cross         (−60°)
        case 5u: { return 0.26; }         // CircleFilled  (+15°)
        case 6u: { return 0.79; }         // SquareFilled  (+45°)
        case 7u: { return -0.26; }        // TriangleFilled(−15°)
        case 8u: { return -0.79; }        // DiamondFilled (−45°)
        default: {
            return -1.2 + f32(shape_id % 11u) * 0.24;
        }
    }
}

/// A planet's ring position angle: the series shape's tilt plus a small
/// per-point jitter (real systems aren't aligned).
fn planet_ring_phi(shape_id: u32, seed_inst: u32) -> f32 {
    let h_jit = sketch_hash01(seed_inst, 0x717Au);
    return cons_ring_angle(shape_id) + (h_jit - 0.5) * 0.12;
}

/// `p` rotated into the ring frame of position angle `phi`.
fn planet_ring_frame(p: vec2<f32>, phi: f32) -> vec2<f32> {
    let cphi = cos(phi);
    let sphi = sin(phi);
    return vec2<f32>(p.x * cphi + p.y * sphi, -p.x * sphi + p.y * cphi);
}

/// The ring-plane radius of ring-frame point `q`, in planet radii: the ring
/// band is drawn where it lies between `RING_INNER` and `RING_OUTER`.
fn planet_ring_rho(q: vec2<f32>, r_planet: f32) -> f32 {
    return length(vec2<f32>(q.x, q.y / RING_INCL)) / r_planet;
}

/// The atmospheric rim glow's alpha at distance `dist` from the planet's
/// centre: a soft exponential falloff outside the disc.
fn planet_rim_glow(dist: f32, r_planet: f32, rim_gain: f32) -> f32 {
    let rim_d = max(dist - r_planet, 0.0);
    return rim_gain * exp(-rim_d / max(r_planet * 0.10, 1.0));
}

// ───── end shared styled markers ─────

// A Milkyway planet's hit distance at `p_screen` (pixels from its centre, y
// up): `fs_planet`'s disc, the rim glow where it stays above
// `PLANET_ALPHA_FLOOR`, and the ring band between `RING_INNER` and
// `RING_OUTER`, all cut by the quad `vs_planet` emits. The band is the
// envelope of the baked ring texture, whose density is not a shader
// constant. `vs_planet` sizes the quad from the radius while `fs_planet`
// reconstructs positions from `max(radius, 1)`, so the fragment frame is
// scaled by their ratio.
fn pick_planet_distance(marker: PickMarker, p_screen: vec2<f32>, point_index: u32) -> f32 {
    let style_params = pick_query_params.transform.style_params;
    let r = marker.radius;
    let r_planet = max(r, 1.0);
    let quad = r * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX;
    let outside_quad = max(abs(p_screen.x), abs(p_screen.y)) - quad;
    let p = p_screen * ((r_planet * PLANET_QUAD_EXTENT + QUAD_MARGIN_PX) / quad);
    let dist = length(p);
    let rim_gain = clamp(style_params[1].w, 0.0, 2.0);
    var d = dist - r_planet;
    if planet_rim_glow(dist, r_planet, rim_gain) > PLANET_ALPHA_FLOOR {
        d = min(d, 0.0);
    } else if rim_gain > PLANET_ALPHA_FLOOR {
        let reach = max(r_planet * 0.10, 1.0) * log(rim_gain / PLANET_ALPHA_FLOOR);
        d = min(d, dist - r_planet - reach);
    }
    let seed_inst = (u32(style_params[0].w) ^ pick_query_params.styled.z) + pick_styled_index(point_index);
    let q = planet_ring_frame(p, planet_ring_phi(pick_query_params.series.z, seed_inst));
    let rho = planet_ring_rho(q, r_planet);
    // A band distance in ring-plane radii is at least that many minor-axis
    // pixels away.
    d = min(d, max(RING_INNER - rho, rho - RING_OUTER) * r_planet * RING_INCL);
    return max(max(d, outside_quad), 0.0);
}

// The hit distance of a marker drawn at `center_px` for the cursor: the
// scatter shader's own coverage rule (`shape_distance` and `marker_edge`,
// SHADER_COMMON.md §5.5, with the sketch wobble of §5.7), evaluated where
// the fragment at the cursor would evaluate it. The marker's local frame is
// y-up like NDC while canvas pixels run down, hence the flip. Zero exactly
// where the fragment's coverage is at least one half; the anti-aliasing
// fringe beyond it is a sub-pixel positive distance. A Milkyway planet and
// a Constellation star pick on their drawn extent instead.
// `PICK_SHAPE`: the one marker shape id a pipeline measures, for a series
// whose shape no style map changes per point (`PICK_SHAPE_ANY` otherwise), so
// the driver folds `shape_distance` to that shape at pipeline creation.
const PICK_SHAPE_ANY: u32 = 4294967295u;
override PICK_SHAPE: u32 = PICK_SHAPE_ANY;

fn pick_marker_distance(marker_in: PickMarker, center_px: vec2<f32>, point_index: u32) -> f32 {
    var marker = marker_in;
    if PICK_SHAPE != PICK_SHAPE_ANY {
        marker.shape = PICK_SHAPE;
    }
    let delta = pick_query_params.cursor_chart.xy - center_px;
    let p = vec2<f32>(delta.x, -delta.y);
    let mode = pick_query_params.styled.x;
    let style_params = pick_query_params.transform.style_params;
    let styled = pick_variant(PICK_VARIANT_STYLED_MARKER);
    if styled && mode == PICK_MARKER_PLANET {
        return pick_planet_distance(marker, p, point_index);
    }
    if styled && mode == PICK_MARKER_STAR {
        // A zero star opacity draws nothing. Otherwise the PSF sprite fades
        // to zero inside the disc its quad inscribes; the star's brightness
        // and tint are hashed per point and sampled from baked textures, so
        // that disc is the drawn envelope.
        if !(style_params[0].x > 0.0) {
            return PICK_F32_MAX;
        }
        let star = constellation_star_radius(marker.radius, pick_styled_index(point_index), pick_query_params.styled.z);
        return max(length(p) - (star * 4.0 + QUAD_MARGIN_PX), 0.0);
    }
    let filled = shape_is_filled(marker.shape);
    let base = base_shape_id(marker.shape);
    let visual_r = visual_shape_radius(base, marker.radius);
    var d: f32;
    if styled && mode == PICK_MARKER_SKETCH {
        let wobble = sketch_marker_wobble_px(style_params[0].x, marker.radius);
        let seed = (u32(style_params[0].z) ^ pick_query_params.styled.z) + pick_styled_index(point_index);
        d = sketch_marker_distance(base, p, visual_r, filled, wobble, seed);
    } else {
        d = shape_distance(base, p, visual_r, filled);
    }
    return max(marker_edge(d, filled), 0.0);
}


fn pick_scatter_candidate(
    point_index: u32,
    x: vec2<f32>,
    y: vec2<f32>,
) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    // The vertex stage's `sample_valid`; the sector and the rings are the
    // fragment clip's, applied at the cursor below.
    if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u) || !pick_polar_valid(x, y) {
        return pick_invalid_candidate(series_order);
    }
    let marker = pick_resolve_scatter_marker(point_index);
    if marker.hidden || !pick_is_finite(marker.radius) {
        return pick_invalid_candidate(series_order);
    }
    let point_px = pick_project_pair(x, y);
    if !pick_is_finite(point_px.x) || !pick_is_finite(point_px.y) {
        return pick_invalid_candidate(series_order);
    }
    let hit_distance = max(pick_marker_distance(marker, point_px, point_index), pick_polar_clip_distance());
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        0u,
        pick_source_row(point_index),
        distance_sq,
        hit_distance,
        0u,
    );
}
