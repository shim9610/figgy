// The colour a field paints: a cell's value placed on the colourbar ramp,
// continuous or quantized into the bands between the contour levels. The
// heatmap fill and the 3D surface (`surface_mesh/`) colour through these one
// functions, so a surface is coloured exactly as the heatmap of the same grid.
// Needs the field group 2 (`field_grid.wgsl`) and `ramp.wgsl`.

/// One block's portable lookup bounds. CPU twin:
/// `data_render/contour.rs::ContourLookupMetadataGpu` (`#[repr(C)]`, 8 B).
struct ContourLookupMetadata {
    finite_count: u32,
    negative_infinity_count: u32,
};

/// Field-fragment-only lookup metadata. The anchor pipeline shares group 2 but
/// never searches contour levels, so binding 6 is not compute-visible.
@group(2) @binding(6) var<storage, read> contour_lookup_metadata: array<ContourLookupMetadata>;

/// `z` mapped to ramp position, or a negative number when it cannot be placed.
fn z_ramp_t(zv: vec2<f32>) -> f32 {
    return ramp_t(zv, field.z_min, field.z_max, field_flag(FIELD_LOG_Z));
}

const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;

fn contour_level_block_count() -> u32 {
    return (field.level_count + CONTOUR_LEVEL_BLOCK_SIZE - 1u) / CONTOUR_LEVEL_BLOCK_SIZE;
}

fn contour_search_record(sorted_index: u32) -> vec4<f32> {
    return stops[max(field.stop_count, 1u) + sorted_index];
}

/// First sorted position in one block whose value is >= `needle`.
fn contour_lower_bound(block_start: u32, block_len: u32, needle: f32) -> u32 {
    var lo = 0u;
    var hi = block_len;
    while (lo < hi) {
        let mid = lo + (hi - lo) / 2u;
        let value = contour_search_record(block_start + mid).x;
        if (value < needle) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return lo;
}

/// First sorted position in one block whose value is > `needle`.
fn contour_upper_bound(block_start: u32, block_len: u32, needle: f32) -> u32 {
    var lo = 0u;
    var hi = block_len;
    while (lo < hi) {
        let mid = lo + (hi - lo) / 2u;
        let value = contour_search_record(block_start + mid).x;
        if (value <= needle) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return lo;
}

/// Ramp position quantized into the bands between the contour levels.
///
/// The band index is how many levels `z` reaches. Each 32-entry source-order
/// block has a sorted lookup copy, so this is at most 32 binary searches for the
/// 1024-level ceiling. The original list remains untouched. The band's colour
/// is the ramp at its own midpoint, so `n` levels give `n + 1` flat bands and
/// none is an endpoint colour. Levels are compared in data units.
fn band_t(raw: f32) -> f32 {
    var reached = 0u;
    for (var block = 0u; block < contour_level_block_count(); block = block + 1u) {
        let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
        let metadata = contour_lookup_metadata[block];
        reached = reached + metadata.negative_infinity_count
            + contour_upper_bound(start, metadata.finite_count, raw);
    }
    return (f32(reached) + 0.5) / f32(field.level_count + 1u);
}

/// The colourmap at `t`. Twin of `model::colormap::sample` through the shared
/// `ramp_segment` / `ramp_blend`; only the stop lookup is this shader's.
fn ramp(t: f32) -> vec4<f32> {
    let n = field.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return stops[0];
    }
    let segment = ramp_segment(t, n);
    let index = u32(segment.x);
    return ramp_blend(stops[index], stops[index + 1u], segment.y);
}

/// The value the fill shades from its samples: the cell's value, or the
/// bilinear blend of the four, with `placeable` false where it paints
/// `nan_color` instead (a bounds miss, a non-finite value or blend).
struct FieldFillValue {
    placeable: bool,
    z: vec2<f32>,
    /// `z_ramp_t(z)`; negative is unplaceable.
    t: f32,
};

fn field_fill_value(z00: GridValue, z10: GridValue, z01: GridValue,
    z11: GridValue, fc: f32, fr: f32) -> FieldFillValue {
    var out: FieldFillValue;
    out.placeable = false;
    out.z = vec2<f32>(0.0, 0.0);
    out.t = -1.0;
    var z: vec2<f32>;
    if (field_flag(FIELD_INTERPOLATED)) {
        // Bilinear over the four surrounding sample points. Done on the axis
        // `t` fractions, so on a logarithmic axis the interpolation is in
        // screen space, matching the weak nonlinearity of common plotting tools.
        // Bounds misses and actual non-finite data are both unplaceable, but
        // they remain distinct until this consumer applies the paint policy.
        if (!z00.valid || !z10.valid || !z01.valid || !z11.valid) {
            return out;
        }
        if (!vec2_f32_is_finite(z00.pair) || !vec2_f32_is_finite(z10.pair)
            || !vec2_f32_is_finite(z01.pair) || !vec2_f32_is_finite(z11.pair)) {
            return out;
        }
        let low = z00.pair + (z10.pair - z00.pair) * fc;
        let high = z01.pair + (z11.pair - z01.pair) * fc;
        if (!vec2_f32_is_finite(low) || !vec2_f32_is_finite(high)) {
            return out;
        }
        z = low + (high - low) * fr;
        if (!vec2_f32_is_finite(z)) {
            return out;
        }
    } else {
        let sample = z00;
        if (!sample.valid) {
            return out;
        }
        z = sample.pair;
    }
    out.z = z;
    out.t = z_ramp_t(z);
    // Unplaceable on the colourbar: `nan_color` as well.
    out.placeable = out.t >= 0.0;
    return out;
}

// Resident storage and bounded replay share all final field colour arithmetic.
// `nan_color` is the premultiplied colour of an unplaceable sample.
fn field_fill_color(z00: GridValue, z10: GridValue, z01: GridValue,
    z11: GridValue, fc: f32, fr: f32, nan_color: vec4<f32>) -> vec4<f32> {
    let v = field_fill_value(z00, z10, z01, z11, fc, fr);
    if (!v.placeable) {
        // `nan_color`, already premultiplied by the caller. Not multiplied by
        // `opacity` — that multiplies the *ramp*'s alpha.
        return nan_color;
    }
    let position = select(v.t, band_t(v.z.x + v.z.y), field_flag(FIELD_BANDS));
    let color = ramp(position);
    let alpha = color.a * field.opacity;
    return vec4<f32>(color.rgb * alpha, alpha);
}

/// Whether the fill paints anything where it shades `v`: `nan_color`'s alpha
/// for an unplaceable value, else the ramp's alpha times the opacity. The
/// band position needs the contour lookup table, which the pick cannot bind
/// (it would put the anchor pass over the storage-binding limit), so a band
/// fill answers with the opacity alone: a transparent ramp stop under
/// `FillMode::Bands` still picks.
fn field_fill_paints(v: FieldFillValue, nan_color: vec4<f32>) -> bool {
    if (!v.placeable) {
        return nan_color.a > 0.0;
    }
    if (field_flag(FIELD_BANDS)) {
        return field.opacity > 0.0;
    }
    return ramp(v.t).a * field.opacity > 0.0;
}
