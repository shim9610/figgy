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

/// The declaration-order mask of block `block`'s levels that can reach a
/// fragment whose value spans `search_lo..=search_hi`: one binary search
/// per bound over the block's sorted lookup copy. A single mask restores
/// the block's declaration order, which preserves duplicate-level colour
/// and source-over behaviour without walking all 32 levels when only a few
/// can reach the fragment. The heatmap and the triangle mesh contours
/// (#210) search through this one function.
fn contour_block_candidates(block: u32, search_lo: f32, search_hi: f32) -> u32 {
    let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
    let count = contour_lookup_metadata[block].finite_count;
    let first = contour_lower_bound(start, count, search_lo);
    let last = contour_upper_bound(start, count, search_hi);
    var candidates = 0u;
    for (var sorted = first; sorted < last; sorted = sorted + 1u) {
        let record = contour_search_record(start + sorted);
        let original = u32(record.y) - start;
        candidates = candidates | (1u << original);
    }
    return candidates;
}

fn vec4_f32_is_finite(v: vec4<f32>) -> bool {
    return vec2_f32_is_finite(v.xy) && vec2_f32_is_finite(v.zw);
}

/// The stroke colour of contour level `i`, premultiplied.
///
/// A level past the end of the table takes the last entry rather than nothing:
/// the host writes one entry per level, and a short table is a mismatch to draw
/// through with the last declared colour, not a reason to drop the contour.
fn level_color(i: u32) -> vec4<f32> {
    if (field.level_color_count == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return level_colors[min(i, field.level_color_count - 1u)];
}

/// What the contour strokes paint at a fragment: the composited colour and
/// the declaration index of the topmost level painted there, or
/// `0xffffffff` where none is. The draw takes the colour; a 3D chart's
/// floor ID pick (`fs_floor_contour_pick`) the level, so the two agree.
struct ContourPaint {
    color: vec4<f32>,
    level: u32,
};

fn contour_paint_none() -> ContourPaint {
    var out: ContourPaint;
    out.color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    out.level = 0xffffffffu;
    return out;
}

/// `paint` with level `i`'s stroke composited over it at the unclamped
/// coverage `coverage` (half the width minus the distance plus half a
/// pixel); a level painted at half coverage or more becomes the topmost.
fn contour_level_over(paint: ContourPaint, i: u32, coverage: f32) -> ContourPaint {
    var out = paint;
    let cov = clamp(coverage, 0.0, 1.0);
    if (!f32_is_finite(coverage) || !f32_is_finite(cov) || cov <= 0.0) {
        return out;
    }
    let c = level_color(i) * cov;
    let composited = c + paint.color * (1.0 - c.a);
    if (!vec4_f32_is_finite(c) || !vec4_f32_is_finite(composited)) {
        return out;
    }
    out.color = composited;
    if (cov >= 0.5) {
        out.level = i;
    }
    return out;
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
    var out = field_fill_sample(z00, z10, z01, z11, fc, fr);
    if (!out.placeable) {
        return out;
    }
    out.t = z_ramp_t(out.z);
    // Unplaceable on the colourbar: `nan_color` as well.
    out.placeable = out.t >= 0.0;
    return out;
}

/// The shaded value before the colourbar: the located cell's pair, or the
/// bilinear blend of the four surrounding sample points when interpolated;
/// `placeable` is false for a bounds miss or a non-finite value, `t` is
/// left at -1.
fn field_fill_sample(z00: GridValue, z10: GridValue, z01: GridValue,
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
    out.placeable = true;
    return out;
}

/// One channel of an image at the located cell (`FIELD_RGB`, #139): the
/// channel's table `offset` entries on, shaded as the fill's value is
/// (flat or bilinear), mapped linearly from `z_min..z_max` to 0..1 and
/// clamped. `placeable` is false where any read is unplaceable.
fn field_image_channel(c: u32, r: u32, fc: f32, fr: f32, offset: u32) -> FieldFillValue {
    var v = field_fill_sample(
        grid_value_in(c, r, offset),
        grid_value_in(c + 1u, r, offset),
        grid_value_in(c, r + 1u, offset),
        grid_value_in(c + 1u, r + 1u, offset),
        fc,
        fr,
    );
    // A flat sample is placeable before the ramp reads it; a missing
    // channel (a non-finite value) leaves the cell unpainted and unpicked.
    if (!v.placeable || !vec2_f32_is_finite(v.z)) {
        v.placeable = false;
        return v;
    }
    let lo = field.z_min.x + field.z_min.y;
    let span = (field.z_max.x - field.z_min.x) + (field.z_max.y - field.z_min.y);
    v.t = clamp((v.z.x + v.z.y - lo) / span, 0.0, 1.0);
    return v;
}

/// The premultiplied colour of an image at the located cell: the three
/// channels at `field.opacity`, or nothing where a channel is unplaceable.
fn field_image_color(c: u32, r: u32, fc: f32, fr: f32) -> vec4<f32> {
    var rgb = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k = k + 1u) {
        let v = field_image_channel(c, r, fc, fr, k * field.image.x);
        if (!v.placeable) {
            return vec4<f32>(0.0);
        }
        rgb[k] = v.t;
    }
    return vec4<f32>(rgb * field.opacity, field.opacity);
}

/// Whether an image paints at the located cell: every channel placeable and
/// the opacity positive.
fn field_image_paints(c: u32, r: u32, fc: f32, fr: f32) -> bool {
    for (var k = 0u; k < 3u; k = k + 1u) {
        if (!field_image_channel(c, r, fc, fr, k * field.image.x).placeable) {
            return false;
        }
    }
    return field.opacity > 0.0;
}

// Resident storage and bounded replay share all final field colour arithmetic.
// `nan_color` is the premultiplied colour of an unplaceable sample.
fn field_fill_color(z00: GridValue, z10: GridValue, z01: GridValue,
    z11: GridValue, fc: f32, fr: f32, nan_color: vec4<f32>) -> vec4<f32> {
    return field_value_color(field_fill_value(z00, z10, z01, z11, fc, fr), nan_color);
}

/// A shaded value `z` placed on the colourbar: unplaceable where `z` is
/// not finite or the ramp cannot place it. The triangle mesh (#210) shades
/// its barycentric blend through this, the heatmap its cell's value or
/// bilinear blend through `field_fill_value`.
fn field_placed_value(z: vec2<f32>) -> FieldFillValue {
    var out: FieldFillValue;
    out.placeable = false;
    out.z = z;
    out.t = -1.0;
    if (!vec2_f32_is_finite(z)) {
        return out;
    }
    out.t = z_ramp_t(z);
    out.placeable = out.t >= 0.0;
    return out;
}

/// The premultiplied colour of a shaded value: the ramp (continuous or
/// banded) at the fill's opacity, or `nan_color` where it is unplaceable.
/// The heatmap and the triangle mesh colour through this one function.
fn field_value_color(v: FieldFillValue, nan_color: vec4<f32>) -> vec4<f32> {
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
