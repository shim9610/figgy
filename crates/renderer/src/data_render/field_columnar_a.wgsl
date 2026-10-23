// ── Field grid sampling (group 2) ───────────────────────────────────────────
//
// No textures and no vertex buffers: everything is read from storage. The pool
// is bound whole, with the coordinate columns located by `FieldParams` lane
// bases, exactly as the constellation star pass does it.

/// One constituent grid column: where it starts in the pool (as an f32 lane
/// index) and how many logical values it holds.
struct GridColumn {
    base: u32,
    len: u32,
};

/// Bit flags in `FieldParams.flags`.
const FIELD_COLUMNS_ARE_Y: u32 = 1u;
const FIELD_CENTERS: u32 = 2u;
const FIELD_INTERPOLATED: u32 = 4u;
const FIELD_BANDS: u32 = 8u;
const FIELD_LOG_Z: u32 = 16u;

/// Everything the field needs that is not already in `Transform` / `Style`.
///
/// `z_min` / `z_max` arrive **already log-transformed** when `FIELD_LOG_Z` is
/// set: the colourbar's bounds are f64 on the host, so taking their logarithm
/// there keeps the precision that doing it here would lose. Both are `(hi, lo)`
/// f32 pairs, the pool's own representation.
///
/// CPU twin: `mod.rs::FieldParamsGpu` (`#[repr(C)]`, 64 B).
struct FieldParams {
    x_base: u32,
    y_base: u32,
    x_len: u32,
    y_len: u32,
    cols: u32,
    rows: u32,
    level_count: u32,
    stop_count: u32,
    flags: u32,
    opacity: f32,
    /// Contour stroke width in pixels, already multiplied by the render scale.
    /// Read by `fs_contour`; the fill ignores it.
    line_width_px: f32,
    /// Entries in `level_colors`. A level past the end takes the last one.
    level_color_count: u32,
    z_min: vec2<f32>,
    z_max: vec2<f32>,
};

@group(2) @binding(0) var<storage, read> field_pool: array<f32>;
@group(2) @binding(1) var<storage, read> grid: array<GridColumn>;
@group(2) @binding(2) var<storage, read> levels: array<f32>;
/// The first `max(field.stop_count, 1)` entries are colourmap stops or the
/// required empty-table padding. The remaining entries preserve one record per
/// declared contour level. Each 32-record block starts with its finite keys,
/// sorted by x; y is the original declaration index as a normal numeric f32.
/// Non-finite slots are zero padding and binding 6 carries the searchable prefix
/// length. Binding 2 remains the positional SSoT.
@group(2) @binding(3) var<storage, read> stops: array<vec4<f32>>;
@group(2) @binding(4) var<uniform> field: FieldParams;
/// Per-level contour stroke colour, **premultiplied**. One entry per level; the
/// fill never reads it.
@group(2) @binding(5) var<storage, read> level_colors: array<vec4<f32>>;

fn field_flag(bit: u32) -> bool {
    return (field.flags & bit) != 0u;
}

fn pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(field_pool[j], field_pool[j + 1u]);
}

fn grid_rounded_add(a: f32, b: f32) -> f32 {
    return fma(1.0, a, b);
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return fma(1.0, a, -b);
}

/// Normalize a finite pair with error-free addition. Explicit fma additions
/// preserve rounding boundaries on Metal; round-trip bitcasts are eliminated
/// by optimizers and do not prevent reassociation.
fn normalize_grid_pair(v: vec2<f32>) -> vec2<f32> {
    let sum = grid_rounded_add(v.x, v.y);
    let b_virtual = grid_rounded_subtract(sum, v.x);
    let a_virtual = grid_rounded_subtract(sum, b_virtual);
    let b_roundoff = grid_rounded_subtract(v.y, b_virtual);
    let a_roundoff = grid_rounded_subtract(v.x, a_virtual);
    return vec2<f32>(sum, grid_rounded_add(a_roundoff, b_roundoff));
}

fn add_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let high = normalize_grid_pair(vec2<f32>(a.x, b.x));
    let low = grid_rounded_add(grid_rounded_add(a.y, b.y), high.y);
    return normalize_grid_pair(vec2<f32>(high.x, low));
}

fn subtract_grid_pairs(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return add_grid_pairs(a, -b);
}

fn scale_grid_pair(v: vec2<f32>, factor: f32) -> vec2<f32> {
    return normalize_grid_pair(vec2<f32>(v.x * factor, v.y * factor));
}

fn midpoint_grid_pair(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    // Scale first so two same-sign maximum f32 coordinates can still have a
    // finite midpoint.
    return add_grid_pairs(scale_grid_pair(a, 0.5), scale_grid_pair(b, 0.5));
}

/// `axis_pair_to_t` for whichever axis: 0 = x, 1 = y.
fn axis_t(pair: vec2<f32>, axis: u32) -> f32 {
    if (axis == 0u) {
        return axis_pair_to_t(pair, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    }
    return axis_pair_to_t(pair, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
}

/// Cell boundary `k` along one axis, in pool `(hi, lo)` pair form.
///
/// `Edges`: boundary k *is* coordinate k. `Centers`: boundary k is the midpoint
/// of coordinates k-1 and k, with the two outer boundaries mirrored out by the
/// adjacent half-cell. With one coordinate every boundary collapses onto it.
fn cell_edge_pair(base: u32, n: u32, k: u32) -> vec2<f32> {
    if (!field_flag(FIELD_CENTERS)) {
        return pool_pair(base, k);
    }
    if (k == 0u) {
        let c0 = pool_pair(base, 0u);
        let c1 = pool_pair(base, min(1u, n - 1u));
        return add_grid_pairs(c0, scale_grid_pair(subtract_grid_pairs(c0, c1), 0.5));
    }
    if (k >= n) {
        let last = pool_pair(base, n - 1u);
        let prev = pool_pair(base, max(n, 2u) - 2u);
        return add_grid_pairs(last, scale_grid_pair(subtract_grid_pairs(last, prev), 0.5));
    }
    return midpoint_grid_pair(pool_pair(base, k - 1u), pool_pair(base, k));
}

/// Sample point `i` along one axis — where a z value sits, as opposed to where
/// its cell ends. `Centers` samples are the coordinates themselves; `Edges`
/// samples sit at the middle of each cell.
fn sample_point_pair(base: u32, n: u32, i: u32) -> vec2<f32> {
    if (field_flag(FIELD_CENTERS)) {
        return pool_pair(base, i);
    }
    return midpoint_grid_pair(pool_pair(base, i), pool_pair(base, min(i + 1u, n - 1u)));
}

/// Which lattice a lookup walks.
///
/// Two different questions share one binary search. The **fill** asks about the
/// quads it draws, which follow `Shading`: cell-to-cell when flat, point-to-point
/// when interpolated. A **contour** always asks about the sample points, because
/// that is where z lives — an isoline does not move because the fill decided to
/// draw flat cells.
const LATTICE_QUADS: u32 = 0u;
const LATTICE_SAMPLES: u32 = 1u;

fn lattice_is_samples(lattice: u32) -> bool {
    return lattice == LATTICE_SAMPLES || field_flag(FIELD_INTERPOLATED);
}

/// Boundary `k` of one lattice along one axis.
fn boundary_t(base: u32, n: u32, k: u32, axis: u32, lattice: u32) -> f32 {
    if (lattice_is_samples(lattice)) {
        return axis_t(sample_point_pair(base, n, k), axis);
    }
    return axis_t(cell_edge_pair(base, n, k), axis);
}

/// Quads along one axis: one per cell on the quad lattice, one per gap between
/// sample points on the sample lattice.
fn quad_count(cells: u32, lattice: u32) -> u32 {
    if (lattice_is_samples(lattice)) {
        return max(cells, 1u) - 1u;
    }
    return cells;
}

/// Where a fragment landed along one axis.
struct CellHit {
    /// Quad index along this axis. Meaningless unless `hit`.
    index: u32,
    /// Position inside that quad, 0 at its low boundary, 1 at its high one.
    frac: f32,
    hit: bool,
};

/// Which quad along one axis contains `t`, and where inside it.
///
/// Boundaries are monotone in `t` but may descend (inverted axis), so the
/// direction is read off the two ends rather than assumed. The loop is
/// `O(log count)` — it stops as soon as the bracket is one wide.
fn locate(base: u32, n: u32, count: u32, axis: u32, t: f32, lattice: u32) -> CellHit {
    var out: CellHit;
    out.index = 0u;
    out.frac = 0.0;
    out.hit = false;
    if (count == 0u || n == 0u || !f32_is_finite(t)) {
        return out;
    }
    let first = boundary_t(base, n, 0u, axis, lattice);
    let last = boundary_t(base, n, count, axis, lattice);
    if (!f32_is_finite(first) || !f32_is_finite(last)) {
        return out;
    }
    if (t < min(first, last) || t > max(first, last)) {
        return out;
    }
    let ascending = last >= first;
    var lo = 0u;
    var hi = count;
    // Each step strictly shrinks the unsigned bracket, so a u32 count takes
    // at most 32 halvings. Keep the loop dynamic: nested fixed-trip searches
    // can make software GPU compilers expand contour projection excessively.
    while (hi - lo > 1u) {
        let mid = lo + (hi - lo) / 2u;
        let tm = boundary_t(base, n, mid, axis, lattice);
        if (!f32_is_finite(tm)) {
            return out;
        }
        let before = select(tm > t, tm <= t, ascending);
        if (before) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let a = boundary_t(base, n, lo, axis, lattice);
    let b = boundary_t(base, n, lo + 1u, axis, lattice);
    let span = grid_rounded_subtract(b, a);
    if (!f32_is_finite(a) || !f32_is_finite(b) || !f32_is_finite(span)) {
        return out;
    }
    out.index = lo;
    // A zero-width quad (a lone centre) has no inside; treat it as its start.
    var frac = 0.0;
    if (span != 0.0) {
        frac = grid_rounded_subtract(t, a) / span;
        if (!f32_is_finite(frac)) {
            return out;
        }
    }
    out.frac = clamp(frac, 0.0, 1.0);
    out.hit = true;
    return out;
}

struct GridValue {
    pair: vec2<f32>,
    valid: bool,
};

/// The grid's z at constituent column `c`, value `r`, as a pool pair plus
/// explicit bounds validity. The pair remains zero when the address is invalid.
fn grid_value(c: u32, r: u32) -> GridValue {
    var out: GridValue;
    out.pair = vec2<f32>(0.0, 0.0);
    out.valid = false;
    if (c >= field.cols) {
        return out;
    }
    let column = grid[c];
    if (r >= column.len) {
        return out;
    }
    let j = column.base + r * 2u;
    out.pair = vec2<f32>(field_pool[j], field_pool[j + 1u]);
    out.valid = true;
    return out;
}

/// Whether `v` is finite, decided from its bits.
///
/// WGSL does not guarantee IEEE NaN semantics, so `v != v` is not a NaN test —
/// a backend may lower it to an *ordered* comparison, which answers `false` for
/// NaN and lets a missing value be painted as the ramp's low end. The exponent
/// field is unambiguous: all ones means NaN or an infinity, and both are
/// unplaceable.
fn f32_is_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

fn vec2_f32_is_finite(v: vec2<f32>) -> bool {
    return f32_is_finite(v.x) && f32_is_finite(v.y);
}

/// z and its gradient at an axis-`t` point, on the **sample** lattice.
///
/// The one place a contour's geometry comes from. There is no segment list and
/// no marching-squares case table: z is a bilinear function inside each cell, so
/// its gradient is available in closed form and the isoline is `z == level`
/// exactly. A saddle is a hyperbola and needs no tie-break rule.
///
/// `dpdx`/`dpdy` are deliberately not used. Those are 2x2 quad finite
/// differences, so a quad straddling a cell boundary reports a gradient that
/// belongs to neither cell — one quad of wrong stroke width along every
/// boundary. The analytic derivative is exact within the cell.
struct ContourSample {
    hit: bool,
    z: f32,
    /// dz/dt in axis-`t` units, i.e. per unit of chart area.
    dz: vec2<f32>,
    /// The only non-zero second derivative a bilinear cell has: the cross term
    /// `d2z/dtx dty`. Both pure second derivatives are identically zero, so this
    /// one number *is* the Hessian, exactly — no finite difference involved.
    d2: f32,
    /// The cell's z range. A bilinear surface attains its extremes at the
    /// corners, so this bounds z over the whole cell and lets a level be
    /// rejected without evaluating anything.
    z_lo: f32,
    z_hi: f32,
};

fn contour_sample(t: vec2<f32>) -> ContourSample {
    var out: ContourSample;
    out.hit = false;
    out.z = 0.0;
    out.dz = vec2<f32>(0.0, 0.0);
    out.d2 = 0.0;
    out.z_lo = 0.0;
    out.z_hi = 0.0;
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along_cells, LATTICE_SAMPLES), 0u, t.x, LATTICE_SAMPLES);
    let y = locate(field.y_base, field.y_len, quad_count(across_cells, LATTICE_SAMPLES), 1u, t.y, LATTICE_SAMPLES);
    if (!x.hit || !y.hit) {
        return out;
    }
    let c = select(x.index, y.index, columns_are_y);
    let r = select(y.index, x.index, columns_are_y);
    let p00 = grid_value(c, r);
    let p10 = grid_value(c + 1u, r);
    let p01 = grid_value(c, r + 1u);
    let p11 = grid_value(c + 1u, r + 1u);
    // One missing corner makes the whole cell unplaceable — a contour through
    // invented data is worse than a gap.
    if (!p00.valid || !p10.valid || !p01.valid || !p11.valid) {
        return out;
    }
    // Bounds validity and stored-value finiteness are separate facts. Inspect
    // both pair lanes by bits before arithmetic so actual NaN/Infinity remains
    // a contour gap on backends that do not preserve IEEE NaN comparisons.
    if (!vec2_f32_is_finite(p00.pair) || !vec2_f32_is_finite(p10.pair)
        || !vec2_f32_is_finite(p01.pair) || !vec2_f32_is_finite(p11.pair)) {
        return out;
    }
    let z00 = p00.pair.x + p00.pair.y;
    let z10 = p10.pair.x + p10.pair.y;
    let z01 = p01.pair.x + p01.pair.y;
    let z11 = p11.pair.x + p11.pair.y;
    if (!f32_is_finite(z00) || !f32_is_finite(z10)
        || !f32_is_finite(z01) || !f32_is_finite(z11)) {
        return out;
    }
    let z_lo = min(min(z00, z10), min(z01, z11));
    let z_hi = max(max(z00, z10), max(z01, z11));
    // `u` runs along the constituent axis (the one the columns are laid out on),
    // `v` within a column — the same convention `grid_value(c, r)` uses.
    let u = select(x.frac, y.frac, columns_are_y);
    let v = select(y.frac, x.frac, columns_are_y);
    if (!f32_is_finite(u) || !f32_is_finite(v)) {
        return out;
    }
    let du0 = z10 - z00;
    let du1 = z11 - z01;
    let dv0 = z01 - z00;
    let dv1 = z11 - z10;
    if (!f32_is_finite(du0) || !f32_is_finite(du1)
        || !f32_is_finite(dv0) || !f32_is_finite(dv1)) {
        return out;
    }
    let low = z00 + du0 * u;
    let high = z01 + du1 * u;
    if (!f32_is_finite(low) || !f32_is_finite(high)) {
        return out;
    }
    let z_delta = high - low;
    let z = low + z_delta * v;
    let dz_du = du0 * (1.0 - v) + du1 * v;
    let dz_dv = dv0 * (1.0 - u) + dv1 * u;
    let d2_num = du1 - du0;
    if (!f32_is_finite(z_delta) || !f32_is_finite(z)
        || !f32_is_finite(dz_du) || !f32_is_finite(dz_dv)
        || !f32_is_finite(d2_num)) {
        return out;
    }
    // (u, v) are cell fractions; the chain rule needs the cell's span in axis
    // `t`. The span may be negative on an inverted axis, which flips the
    // gradient's sign exactly as it should.
    let x_span = boundary_t(field.x_base, field.x_len, x.index + 1u, 0u, LATTICE_SAMPLES)
        - boundary_t(field.x_base, field.x_len, x.index, 0u, LATTICE_SAMPLES);
    let y_span = boundary_t(field.y_base, field.y_len, y.index + 1u, 1u, LATTICE_SAMPLES)
        - boundary_t(field.y_base, field.y_len, y.index, 1u, LATTICE_SAMPLES);
    let u_span = select(x_span, y_span, columns_are_y);
    let v_span = select(y_span, x_span, columns_are_y);
    if (!f32_is_finite(u_span) || !f32_is_finite(v_span)
        || u_span == 0.0 || v_span == 0.0) {
        // A zero-width cell (a lone coordinate) has no interior to place a
        // contour in, the same answer the fill gives it.
        return out;
    }
    let dz_dtu = dz_du / u_span;
    let dz_dtv = dz_dv / v_span;
    // The cross term is symmetric in the two axes, so it needs no orientation
    // select: swapping u and v leaves it unchanged.
    let span_product = u_span * v_span;
    if (!f32_is_finite(span_product) || span_product == 0.0) {
        return out;
    }
    let d2 = d2_num / span_product;
    let dz = select(
        vec2<f32>(dz_dtu, dz_dtv),
        vec2<f32>(dz_dtv, dz_dtu),
        columns_are_y,
    );
    if (!vec2_f32_is_finite(dz) || !f32_is_finite(d2)) {
        return out;
    }
    out.z = z;
    out.dz = dz;
    out.d2 = d2;
    out.z_lo = z_lo;
    out.z_hi = z_hi;
    out.hit = true;
    return out;
}

/// An axis-`t` gradient in **pixels**: z units per pixel on each screen axis.
///
/// `t` spans the chart area, NDC spans 2 across it, and `pixel_to_ndc` is one
/// pixel in NDC — so one pixel is `pixel_to_ndc / 2` of `t`.
fn grad_px(dz: vec2<f32>) -> vec2<f32> {
    return dz * transform.pixel_to_ndc * 0.5;
}

// Label-gap inputs exist only in `fs_contour_labelled`'s call graph. The plain
// `fs_contour` entry therefore keeps the original three-group layout, while a
// labelled contour binds the exact selected-anchor buffers as group 3.
struct LabelGapAnchor {
    x: vec2<f32>,
    y: vec2<f32>,
    dir: vec2<f32>,
    level: u32,
    width_px: f32,
};

struct LabelGapParams {
    atlas_w: f32,
    atlas_h: f32,
    cell_stride_w: f32,
    cell_stride_h: f32,
    columns: u32,
    rows: u32,
    gutter: f32,
    level_count: u32,
};

@group(3) @binding(0) var<storage, read> label_gap_anchors: array<LabelGapAnchor>;
@group(3) @binding(1) var<storage, read> label_gap_draw_args: array<u32>;
@group(3) @binding(3) var<uniform> label_gap_params: LabelGapParams;

/// One block's portable lookup bounds. CPU twin:
/// `mod.rs::ContourLookupMetadataGpu` (`#[repr(C)]`, 8 B).
struct ContourLookupMetadata {
    finite_count: u32,
    negative_infinity_count: u32,
};

/// Field-fragment-only lookup metadata. The anchor pipeline shares group 2 but
/// never searches contour levels, so binding 6 is not compute-visible.
@group(2) @binding(6) var<storage, read> contour_lookup_metadata: array<ContourLookupMetadata>;

// ───── shared colour ramp (SHADER_COMMON.md §5.4; `shader_consistency` compares
// these function bodies with the scatter and vector shaders) ─────
