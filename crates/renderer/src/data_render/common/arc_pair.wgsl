// Arc pairs (SHADER_COMMON.md §5.6): a line's cumulative pixel arc as a
// 64-bit fixed-point integer, `ARC_UNITS_PER_PX` units per pixel, held as
// `vec2<u32>(low word, high word)`. Integer sums are exact and associative,
// so the prefix keeps 1/65536 px resolution up to 2^48 px whatever order a
// scan adds in, and no compiler can fold them away the way it may fold an
// error-free float sum (#107). The scan (`line_arc.wgsl`) writes them; the
// stroke, the arc slots and the pick read a segment's arc as the pair at A
// plus a small f32 offset.

const ARC_UNITS_PER_PX: f32 = 65536.0;

/// A non-negative pixel length as an arc pair, rounded to the nearest unit.
/// Negative and non-finite lengths are zero; past 2^48 px it saturates.
fn arc_from_px(px: f32) -> vec2<u32> {
    if (!(px > 0.0)) {
        return vec2<u32>(0u, 0u);
    }
    let units = round(px * ARC_UNITS_PER_PX);
    if (!(units < 18446744073709551616.0)) {
        return vec2<u32>(0xFFFFFFFFu, 0xFFFFFFFFu);
    }
    // Both steps are exact: `high` is an integer f32 and `units - high *
    // 2^32` keeps only the bits below 2^32 of an integer `units`.
    let high = floor(units / 4294967296.0);
    return vec2<u32>(u32(units - high * 4294967296.0), u32(high));
}

/// `a + b`.
fn arc_add(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let low = a.x + b.x;
    return vec2<u32>(low, a.y + b.y + select(0u, 1u, low < a.x));
}

/// `a - b` for `a >= b`.
fn arc_sub(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2<u32>(a.x - b.x, a.y - b.y - select(0u, 1u, a.x < b.x));
}

/// `a < b`.
fn arc_less(a: vec2<u32>, b: vec2<u32>) -> bool {
    return a.y < b.y || (a.y == b.y && a.x < b.x);
}

/// An arc pair in pixels as one f32: exact enough for a segment's length or
/// an offset along it.
fn arc_to_px(a: vec2<u32>) -> f32 {
    return f32(a.y) * (4294967296.0 / ARC_UNITS_PER_PX) + f32(a.x) / ARC_UNITS_PER_PX;
}

/// `a - b` in pixels as one f32, signed.
fn arc_delta(a: vec2<u32>, b: vec2<u32>) -> f32 {
    if (arc_less(a, b)) {
        return -arc_to_px(arc_sub(b, a));
    }
    return arc_to_px(arc_sub(a, b));
}

/// A positive pixel length `px` as a divisor in arc units: at least one
/// unit, below 2^31 (32768 px) so the long division cannot overflow.
fn arc_divisor(px: f32) -> u32 {
    return u32(clamp(round(px * ARC_UNITS_PER_PX), 1.0, 2147483520.0));
}

/// The 64-bit product `a * b` of two words.
fn arc_mul_wide(a: u32, b: u32) -> vec2<u32> {
    let al = a & 0xFFFFu;
    let ah = a >> 16u;
    let bl = b & 0xFFFFu;
    let bh = b >> 16u;
    let ll = al * bl;
    let lh = al * bh;
    let hl = ah * bl;
    let mid = lh + hl;
    let mid_carry = select(0u, 0x10000u, mid < lh);
    let low = ll + (mid << 16u);
    let low_carry = select(0u, 1u, low < ll);
    return vec2<u32>(low, ah * bh + (mid >> 16u) + mid_carry + low_carry);
}

/// The quotient and remainder of an arc pair divided by `divisor` units
/// (`arc_divisor`), by long division.
struct ArcDivision {
    quotient: vec2<u32>,
    remainder: u32,
};

fn arc_divide(a: vec2<u32>, divisor: u32) -> ArcDivision {
    var out: ArcDivision;
    out.quotient = vec2<u32>(0u, 0u);
    var rest = 0u;
    for (var bit = 64u; bit > 0u; bit = bit - 1u) {
        let i = bit - 1u;
        let word = select(a.x, a.y, i >= 32u);
        rest = (rest << 1u) | ((word >> (i & 31u)) & 1u);
        if (rest >= divisor) {
            rest = rest - divisor;
            if (i >= 32u) {
                out.quotient.y = out.quotient.y | (1u << (i - 32u));
            } else {
                out.quotient.x = out.quotient.x | (1u << i);
            }
        }
    }
    out.remainder = rest;
    return out;
}

/// The cell of an arc in steps of `step_px`: the floor of `arc / step`,
/// wrapped into `u32` the way `sketch_noise` wraps its lattice index, and
/// the fraction in [0, 1).
struct ArcCell {
    index: u32,
    fraction: f32,
};

fn arc_cell(a: vec2<u32>, step_px: f32) -> ArcCell {
    let divisor = arc_divisor(step_px);
    let division = arc_divide(a, divisor);
    var cell: ArcCell;
    cell.index = division.quotient.x;
    cell.fraction = min(f32(division.remainder) / f32(divisor), 0.99999994);
    return cell;
}

/// The arc of slot `slot` on a grid of `pitch_px`: `slot` whole steps of the
/// pitch in arc units (`arc_divisor`, the unit `star_indirect` counts in) and
/// `offset` of one more, exact for every `u32` slot.
fn arc_slot_position(slot: u32, offset: f32, pitch_px: f32) -> vec2<u32> {
    let step = arc_divisor(pitch_px);
    return arc_add(arc_mul_wide(slot, step), arc_from_px(offset * f32(step) / ARC_UNITS_PER_PX));
}
