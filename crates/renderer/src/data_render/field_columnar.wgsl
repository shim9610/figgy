// Columnar field shader — heatmaps and band (filled-contour) fields.
//
// The grid is a bundle of pool columns declared by `MatrixRef`: one column per
// x (or y) position, each holding that position's values along the other axis.
// Coordinates come from the ordinary `x_column` / `y_column`; nothing here is
// stored twice.
//
// **One quad for the whole field, not one per cell.** The cell a fragment
// belongs to is found per fragment by binary search over the coordinate
// columns. Two reasons, both decisive:
//
//   1. A 5000 x 5000 grid is 25M cells. As instances that is 150M vertices a
//      frame; as one quad it is 6, and the cost becomes the data area's pixel
//      count — which is what a heatmap's cost should be.
//   2. Adjacent per-cell quads seam under MSAA. Two quads meeting on an edge
//      resolve as `bg*(1-cA)*(1-cB) + ...`, so up to 25% of the background
//      shows through along every shared edge. One quad cannot seam against
//      itself.
//
// The search runs in axis `t` space (`axis_pair_to_t`), so logarithmic and
// inverted axes need no special case: the same monotone comparison works for
// all four combinations.
//
// Cell geometry follows `GridLayout`. `Edges` reads the coordinate columns as
// cell boundaries (n + 1 for n cells); `Centers` reads them as cell midpoints
// and derives boundaries as the midpoints between them, mirroring the outermost
// half-cells. A single centre has no neighbour to measure against, so its cell
// has zero width and draws nothing — the data does not say how wide it is, and
// inventing a width would be a number nobody wrote.
//
// `Shading::Flat` gives each cell its own colour from its own z.
// `Shading::Interpolated` interpolates z bilinearly between the four
// surrounding sample points, so the quads span point-to-point rather than
// cell-to-cell (matplotlib's `flat` vs `gouraud`).
//
// `FillMode::Bands` quantizes z into the intervals between the contour levels,
// which makes exactly the filled-contour bands the contour lines will be drawn
// over. `Continuous` uses the ramp directly.
//
// The colour ramp is the colourmap's own control points, and the mapping is a
// byte-for-byte reimplementation of `model::colormap::sample` — see `ramp()`.
// That is what makes the CPU-drawn colourbar strip and this shader agree: same
// stops, same formula, same `t`.

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
    return bitcast<f32>(bitcast<u32>(a + b));
}

fn grid_rounded_subtract(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a - b));
}

/// Normalize a finite pair with error-free addition. The bitcasts expose every
/// f32 rounding boundary so field rendering and fit reduction cannot be
/// reassociated differently by their backends.
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
    // `count` is bounded by the pool's column length, so 32 halvings settle it.
    for (var step = 0u; step < 32u; step = step + 1u) {
        if (hi - lo <= 1u) {
            break;
        }
        let mid = (lo + hi) / 2u;
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
    let span = b - a;
    if (!f32_is_finite(a) || !f32_is_finite(b) || !f32_is_finite(span)) {
        return out;
    }
    out.index = lo;
    // A zero-width quad (a lone centre) has no inside; treat it as its start.
    var frac = 0.0;
    if (span != 0.0) {
        frac = (t - a) / span;
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
// ───── END common block ─────

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

/// `z` mapped to ramp position, or a negative number when it cannot be placed.
///
/// CPU twin: `ColorBarOptions::normalized_z`. "Unplaceable" and "smallest" are
/// different facts, so NaN, a non-positive z on a logarithmic bar, and a
/// degenerate range all report the miss instead of clamping to an end.
fn z_ramp_t(zv: vec2<f32>) -> f32 {
    // Both halves are checked before they are added: NaN propagation through
    // arithmetic is not guaranteed either.
    if (!f32_is_finite(zv.x) || !f32_is_finite(zv.y)) {
        return -1.0;
    }
    let raw = zv.x + zv.y;
    if (!f32_is_finite(raw)) {
        return -1.0;
    }
    let log_z = field_flag(FIELD_LOG_Z);
    if (log_z && raw <= 0.0) {
        return -1.0;
    }
    let is_log = select(0.0, 1.0, log_z);
    // Same shape as `axis_pair_to_t`: the linear form keeps the hi/lo split so
    // a large absolute z still resolves small deltas.
    let linear_num = (zv.x - field.z_min.x) + (zv.y - field.z_min.y);
    let log_num = (maybe_log(raw, 1.0) - field.z_min.x) - field.z_min.y;
    let span = (field.z_max.x - field.z_min.x) + (field.z_max.y - field.z_min.y);
    if (!f32_is_finite(linear_num) || !f32_is_finite(log_num)
        || !f32_is_finite(span) || span <= 0.0) {
        return -1.0;
    }
    let numerator = mix(linear_num, log_num, is_log);
    let normalized = numerator / span;
    if (!f32_is_finite(numerator) || !f32_is_finite(normalized)) {
        return -1.0;
    }
    return clamp(normalized, 0.0, 1.0);
}

fn vec4_f32_is_finite(v: vec4<f32>) -> bool {
    return vec2_f32_is_finite(v.xy) && vec2_f32_is_finite(v.zw);
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

/// The colourmap at `t`. Byte-for-byte twin of `model::colormap::sample`,
/// including the `n - 2` index clamp and the exact-endpoint short circuits —
/// `mix` is *not* used, because `a + (b - a) * t` is what the CPU computes and
/// the two paths have to agree at the ends.
fn ramp(t: f32) -> vec4<f32> {
    let n = field.stop_count;
    if (n == 0u) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    if (n == 1u) {
        return stops[0];
    }
    let tc = select(clamp(t, 0.0, 1.0), 0.0, !f32_is_finite(t));
    let scaled = tc * f32(n - 1u);
    let index = min(u32(floor(scaled)), n - 2u);
    let frac = scaled - f32(index);
    if (frac <= 0.0) {
        return stops[index];
    }
    if (frac >= 1.0) {
        return stops[index + 1u];
    }
    let a = stops[index];
    let b = stops[index + 1u];
    return a + (b - a) * frac;
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

struct FieldOut {
    @builtin(position) pos: vec4<f32>,
    /// Axis `t` of this fragment, i.e. its position in the chart area — the
    /// same quantity `axis_pair_to_t` returns for a data value, which is what
    /// lets the cell search compare the two directly.
    @location(0) axis_t: vec2<f32>,
};

/// One full-clip-space quad. `Transform` already maps the data range onto
/// NDC +/-1 across the chart area, and the caller's scissor trims it to the
/// data area, so the quad needs no data-dependent extent — the fragment stage
/// reports a miss wherever the field is not.
@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> FieldOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    let ndc = corners[vid];
    var out: FieldOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.axis_t = (ndc + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

@fragment
fn fs_main(in: FieldOut) -> @location(0) vec4<f32> {
    let clear = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    // The constituent index runs along x unless the declaration says otherwise;
    // the within-column index runs along the other axis. Never inferred from
    // the lengths.
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(field.x_base, field.x_len, quad_count(along_cells, LATTICE_QUADS), 0u, in.axis_t.x, LATTICE_QUADS);
    let y = locate(field.y_base, field.y_len, quad_count(across_cells, LATTICE_QUADS), 1u, in.axis_t.y, LATTICE_QUADS);
    if (!x.hit || !y.hit) {
        return clear;
    }
    let c = select(x.index, y.index, columns_are_y);
    let r = select(y.index, x.index, columns_are_y);
    let fc = select(x.frac, y.frac, columns_are_y);
    let fr = select(y.frac, x.frac, columns_are_y);

    var z: vec2<f32>;
    if (field_flag(FIELD_INTERPOLATED)) {
        // Bilinear over the four surrounding sample points. Done on the axis
        // `t` fractions, so on a logarithmic axis the interpolation is in
        // screen space, matching the weak nonlinearity of common plotting tools.
        let z00 = grid_value(c, r);
        let z10 = grid_value(c + 1u, r);
        let z01 = grid_value(c, r + 1u);
        let z11 = grid_value(c + 1u, r + 1u);
        // Bounds misses and actual non-finite data are both unplaceable, but
        // they remain distinct until this consumer applies the paint policy.
        if (!z00.valid || !z10.valid || !z01.valid || !z11.valid) {
            return style.color_premul;
        }
        if (!vec2_f32_is_finite(z00.pair) || !vec2_f32_is_finite(z10.pair)
            || !vec2_f32_is_finite(z01.pair) || !vec2_f32_is_finite(z11.pair)) {
            return style.color_premul;
        }
        let low = z00.pair + (z10.pair - z00.pair) * fc;
        let high = z01.pair + (z11.pair - z01.pair) * fc;
        if (!vec2_f32_is_finite(low) || !vec2_f32_is_finite(high)) {
            return style.color_premul;
        }
        z = low + (high - low) * fr;
        if (!vec2_f32_is_finite(z)) {
            return style.color_premul;
        }
    } else {
        let sample = grid_value(c, r);
        if (!sample.valid) {
            return style.color_premul;
        }
        z = sample.pair;
    }

    let t = z_ramp_t(z);
    if (t < 0.0) {
        // Unplaceable: `nan_color`, already premultiplied by the caller. Not
        // multiplied by `opacity` — that multiplies the *ramp*'s alpha.
        return style.color_premul;
    }
    let position = select(t, band_t(z.x + z.y), field_flag(FIELD_BANDS));
    let color = ramp(position);
    let alpha = color.a * field.opacity;
    return vec4<f32>(color.rgb * alpha, alpha);
}

/// Screen-space differential terms shared by contour painting and contour
/// picking. Keeping the quadratic distance root behind this one helper is what
/// prevents the visible stroke and its hit target from drifting apart.
struct ContourMetric {
    valid: bool,
    g_len: f32,
    kappa: f32,
    gradient_square: f32,
};

fn contour_metric(s: ContourSample) -> ContourMetric {
    var out: ContourMetric;
    out.valid = false;
    out.g_len = 0.0;
    out.kappa = 0.0;
    out.gradient_square = 0.0;
    if (!s.hit) {
        return out;
    }
    let g = grad_px(s.dz);
    let g_len = length(g);
    // The Hessian in pixels. `grad_px` scales each axis by `pixel_to_ndc / 2`, so
    // the mixed second derivative takes that factor once per axis.
    let hxy = s.d2 * transform.pixel_to_ndc.x * transform.pixel_to_ndc.y * 0.25;
    if (!vec2_f32_is_finite(g) || !f32_is_finite(g_len) || !f32_is_finite(hxy)) {
        return out;
    }
    // Normal curvature of the level set. With `H = [[0, hxy], [hxy, 0]]` the
    // quadratic form collapses to this.
    var kappa = 0.0;
    if (g_len > 0.0) {
        let n = g / g_len;
        kappa = 2.0 * hxy * n.x * n.y;
    }
    let gradient_square = g_len * g_len;
    if (!f32_is_finite(kappa) || !f32_is_finite(gradient_square)) {
        return out;
    }
    if (g_len <= 0.0 && kappa == 0.0) {
        // A plateau: no gradient and no curvature. Its level set is the whole
        // flat region, and painting/picking that solid would invent a line.
        return out;
    }
    out.valid = true;
    out.g_len = g_len;
    out.kappa = kappa;
    out.gradient_square = gradient_square;
    return out;
}

/// Distance in pixels to one level along the bilinear field's gradient normal.
/// Returns -1 for an unplaceable level/root.
fn contour_level_distance_px(s: ContourSample, metric: ContourMetric, level: f32) -> f32 {
    if (!metric.valid || !f32_is_finite(level)) {
        return -1.0;
    }
    // z is bilinear, so along a straight line it is exactly quadratic. Written
    // rationalized so it degrades to `f / |grad|` as curvature tends to zero.
    let f = s.z - level;
    let curvature_term = 2.0 * metric.kappa * f;
    let disc = metric.gradient_square - curvature_term;
    if (!f32_is_finite(f) || !f32_is_finite(curvature_term)
        || !f32_is_finite(disc) || disc < 0.0) {
        return -1.0;
    }
    let root = sqrt(disc);
    let denom = metric.g_len + root;
    if (!f32_is_finite(root) || !f32_is_finite(denom) || denom <= 0.0) {
        return -1.0;
    }
    let distance_to_level = abs((2.0 * f) / denom);
    if (!f32_is_finite(distance_to_level)) {
        return -1.0;
    }
    return distance_to_level;
}

/// Contour lines, drawn as the level set of the bilinear field.
///
/// Same quad and same fragment machinery as the fill, so the two agree by
/// construction — the band boundary a `FillMode::Bands` fill paints and the line
/// drawn here come out of one interpolation, including on a logarithmic axis
/// without materializing or tracing a marching-squares segment list.
///
/// Coverage uses the quadratic root obtained by restricting the current cell's
/// bilinear polynomial to the fragment's gradient-normal line. That root is
/// algebraically exact for that polynomial and line; it is not asserted to stay
/// inside the cell or to be the globally shortest Euclidean distance to the
/// piecewise-bilinear isoline. The analytic gradient is the bilinear form's own
/// derivative, not a `dpdx` finite difference.
///
/// A point with no gradient can still have mixed curvature, which the quadratic
/// handles. Only a plateau (`|grad z| == 0` and zero mixed curvature) is left
/// blank: its level set is the whole flat region, so painting it solid would
/// state a line position the data does not contain.
///
/// Cost is the data area's pixel count, not the cell count: a 5000 x 5000 grid
/// costs what a 100 x 100 one does. Per fragment, each 32-level block is searched
/// by z range and only reachable candidates run the coverage math.
fn contour_color(in: FieldOut) -> vec4<f32> {
    var acc = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let s = contour_sample(in.axis_t);
    if (!s.hit) {
        return acc;
    }
    let metric = contour_metric(s);
    if (!metric.valid) {
        return acc;
    }
    if (!f32_is_finite(field.line_width_px)) {
        return acc;
    }
    let half_w = max(field.line_width_px, 0.0) * 0.5;
    // The reject below must keep a neighbouring cell's line, which legitimately
    // reaches into this fragment: `margin` is how much z changes over half a
    // stroke plus a pixel of antialiasing.
    let margin = (half_w + 1.0) * metric.g_len;
    let search_lo = s.z_lo - margin;
    let search_hi = s.z_hi + margin;
    if (!f32_is_finite(half_w) || !f32_is_finite(margin)
        || !f32_is_finite(search_lo) || !f32_is_finite(search_hi)) {
        return acc;
    }
    for (var block = 0u; block < contour_level_block_count(); block = block + 1u) {
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
        // A single mask restores the block's declaration order. That preserves
        // duplicate-level colour and source-over behavior without walking all 32
        // levels when only a few can reach this cell.
        while (candidates != 0u) {
            let original = firstTrailingBit(candidates);
            candidates = candidates & ~(1u << original);
            let i = start + original;
            let lv = levels[i];
            if (!f32_is_finite(lv)) {
                continue;
            }
            let distance_to_level = contour_level_distance_px(s, metric, lv);
            let coverage_unclamped = half_w - distance_to_level + 0.5;
            if (distance_to_level < 0.0 || !f32_is_finite(coverage_unclamped)) {
                continue;
            }
            let cov = clamp(coverage_unclamped, 0.0, 1.0);
            if (!f32_is_finite(cov) || cov <= 0.0) {
                continue;
            }
            let c = level_color(i) * cov;
            let composited = c + acc * (1.0 - c.a);
            if (!vec4_f32_is_finite(c) || !vec4_f32_is_finite(composited)) {
                continue;
            }
            acc = composited;
        }
    }
    return acc;
}

/// Whether this contour fragment sits under one of the exact label quads that
/// the next draw will composite. The count comes from the same indirect args
/// buffer as that draw, so automatic placement never needs a CPU readback and
/// the gap cannot disagree with the labels that actually appear.
fn contour_label_gap_contains(axis_t_value: vec2<f32>) -> bool {
    let count = min(label_gap_draw_args[1], 1024u);
    let fragment_ndc = axis_t_value * 2.0 - 1.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let anchor = label_gap_anchors[i];
        if (anchor.level >= label_gap_params.level_count) {
            continue;
        }
        let w = anchor.width_px;
        let h = label_gap_params.cell_stride_h - 2.0 * label_gap_params.gutter;
        if (!f32_is_finite(w) || !f32_is_finite(h) || w <= 0.0 || h <= 0.0) {
            continue;
        }
        let centre = data_to_ndc(anchor.x, anchor.y);
        let along = data_to_ndc(
            anchor.x + vec2<f32>(anchor.dir.x, 0.0),
            anchor.y + vec2<f32>(anchor.dir.y, 0.0),
        );
        let tangent_px = (along - centre) / transform.pixel_to_ndc;
        var right = vec2<f32>(1.0, 0.0);
        if (dot(tangent_px, tangent_px) > 0.0) {
            right = normalize(tangent_px);
        }
        let up = vec2<f32>(-right.y, right.x);
        let delta_px = (fragment_ndc - centre) / transform.pixel_to_ndc;
        let local = vec2<f32>(dot(delta_px, right), dot(delta_px, up));
        // Half a pixel covers the antialiased edge of both the stroke and the
        // atlas quad. `bg_padding_px` is already included in w/h, even when the
        // label background itself is transparent.
        if (abs(local.x) <= w * 0.5 + 0.5 && abs(local.y) <= h * 0.5 + 0.5) {
            return true;
        }
    }
    return false;
}

@fragment
fn fs_contour(in: FieldOut) -> @location(0) vec4<f32> {
    return contour_color(in);
}

@fragment
fn fs_contour_labelled(in: FieldOut) -> @location(0) vec4<f32> {
    let color = contour_color(in);
    // The anchor scan is paid only on pixels where a contour actually has
    // coverage, not across the field's whole rectangle.
    if (color.a > 0.0 && contour_label_gap_contains(in.axis_t)) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    return color;
}

// ───── Exact data picking (compute entry) ───────────────────────────────────

const DATA_PICK_FLAG_FILL: u32 = 1u;
const DATA_PICK_FLAG_CONTOUR: u32 = 2u;
const DATA_PICK_KIND_MATRIX_CELL: u32 = 2u;
const DATA_PICK_KIND_CONTOUR_LEVEL: u32 = 3u;

struct DataPickQuery {
    // xy = cursor NDC, zw = cursor axis-t.
    cursor_ndc_t: vec4<f32>,
    // x = max distance px, y = scaled bar gap px, z = contour width px.
    limits: vec4<f32>,
    // x = flags, y = source paint order, z/w are primitive-specific.
    data: vec4<u32>,
    bases: vec4<u32>,
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
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

fn field_pick_invalid() -> DataPickCandidate {
    return DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
}

fn field_pick_cell(t: vec2<f32>) -> DataPickCandidate {
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(
        field.x_base,
        field.x_len,
        quad_count(along_cells, LATTICE_QUADS),
        0u,
        t.x,
        LATTICE_QUADS,
    );
    let y = locate(
        field.y_base,
        field.y_len,
        quad_count(across_cells, LATTICE_QUADS),
        1u,
        t.y,
        LATTICE_QUADS,
    );
    if (!x.hit || !y.hit) {
        return field_pick_invalid();
    }
    return DataPickCandidate(
        1u,
        data_pick_query.data.y,
        DATA_PICK_KIND_MATRIX_CELL,
        x.index,
        y.index,
        0u,
        0.0,
        0u,
    );
}

fn field_pick_contour(t: vec2<f32>) -> DataPickCandidate {
    var best = field_pick_invalid();
    let s = contour_sample(t);
    let metric = contour_metric(s);
    if (!metric.valid || !f32_is_finite(data_pick_query.limits.x)
        || !f32_is_finite(data_pick_query.limits.z)) {
        return best;
    }
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x = locate(
        field.x_base,
        field.x_len,
        quad_count(along_cells, LATTICE_SAMPLES),
        0u,
        t.x,
        LATTICE_SAMPLES,
    );
    let y = locate(
        field.y_base,
        field.y_len,
        quad_count(across_cells, LATTICE_SAMPLES),
        1u,
        t.y,
        LATTICE_SAMPLES,
    );
    if (!x.hit || !y.hit) {
        return best;
    }
    let half_width = max(data_pick_query.limits.z, 0.0) * 0.5;
    let max_distance = max(data_pick_query.limits.x, 0.0);
    var level_index = 0u;
    while (level_index < field.level_count) {
        let center_distance = contour_level_distance_px(s, metric, levels[level_index]);
        if (center_distance >= 0.0) {
            let hit_distance = max(center_distance - half_width, 0.0);
            if (f32_is_finite(hit_distance) && hit_distance <= max_distance
                && (best.valid == 0u || hit_distance < best.distance_px
                    || (hit_distance == best.distance_px && level_index > best.index0))) {
                best = DataPickCandidate(
                    1u,
                    data_pick_query.data.y,
                    DATA_PICK_KIND_CONTOUR_LEVEL,
                    level_index,
                    x.index,
                    y.index,
                    hit_distance,
                    level_index,
                );
            }
        }
        level_index = level_index + 1u;
    }
    return best;
}

@compute @workgroup_size(1)
fn pick_field_data() {
    let flags = data_pick_query.data.x;
    let t = data_pick_query.cursor_ndc_t.zw;
    // Contours paint over their fill. A contour within the configured pick
    // tolerance therefore wins before the containing heatmap cell.
    if ((flags & DATA_PICK_FLAG_CONTOUR) != 0u) {
        let contour = field_pick_contour(t);
        if (contour.valid != 0u) {
            data_pick_output = contour;
            return;
        }
    }
    if ((flags & DATA_PICK_FLAG_FILL) != 0u) {
        data_pick_output = field_pick_cell(t);
        return;
    }
    data_pick_output = field_pick_invalid();
}

// ───── Typed selection overlay (render entry) ───────────────────────────────

struct DataSelection {
    color_premul: vec4<f32>,
    // x = cell/bar outline width, y = selected contour extra width.
    metrics: vec4<f32>,
    // x = kind, y/z/w = kind-specific indices.
    indices: vec4<u32>,
};

@group(1) @binding(4) var<uniform> data_selection: DataSelection;

fn selection_color(coverage: f32) -> vec4<f32> {
    if (!f32_is_finite(coverage) || coverage <= 0.0
        || !vec4_f32_is_finite(data_selection.color_premul)) {
        return vec4<f32>(0.0);
    }
    return data_selection.color_premul * clamp(coverage, 0.0, 1.0);
}

fn selected_cell_coverage(t: vec2<f32>) -> f32 {
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    let along_cells = select(field.cols, field.rows, columns_are_y);
    let across_cells = select(field.rows, field.cols, columns_are_y);
    let x_count = quad_count(along_cells, LATTICE_QUADS);
    let y_count = quad_count(across_cells, LATTICE_QUADS);
    let x_index = data_selection.indices.z;
    let y_index = data_selection.indices.w;
    let width_px = data_selection.metrics.x;
    if (x_index >= x_count || y_index >= y_count
        || !f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    let x0 = boundary_t(field.x_base, field.x_len, x_index, 0u, LATTICE_QUADS);
    let x1 = boundary_t(field.x_base, field.x_len, x_index + 1u, 0u, LATTICE_QUADS);
    let y0 = boundary_t(field.y_base, field.y_len, y_index, 1u, LATTICE_QUADS);
    let y1 = boundary_t(field.y_base, field.y_len, y_index + 1u, 1u, LATTICE_QUADS);
    if (!f32_is_finite(x0) || !f32_is_finite(x1)
        || !f32_is_finite(y0) || !f32_is_finite(y1)
        || t.x < min(x0, x1) || t.x > max(x0, x1)
        || t.y < min(y0, y1) || t.y > max(y0, y1)) {
        return 0.0;
    }
    let pixel_t = transform.pixel_to_ndc * 0.5;
    if (!vec2_f32_is_finite(pixel_t) || pixel_t.x <= 0.0 || pixel_t.y <= 0.0) {
        return 0.0;
    }
    let edge_distance_px = min(
        min(abs(t.x - x0), abs(t.x - x1)) / pixel_t.x,
        min(abs(t.y - y0), abs(t.y - y1)) / pixel_t.y,
    );
    return width_px - edge_distance_px + 0.5;
}

fn selected_contour_coverage(t: vec2<f32>) -> f32 {
    let level_index = data_selection.indices.y;
    let width_px = max(field.line_width_px, 0.0) + max(data_selection.metrics.y, 0.0);
    if (level_index >= field.level_count
        || !f32_is_finite(width_px) || width_px <= 0.0) {
        return 0.0;
    }
    let s = contour_sample(t);
    let metric = contour_metric(s);
    let distance_px = contour_level_distance_px(s, metric, levels[level_index]);
    if (distance_px < 0.0) {
        return 0.0;
    }
    return max(width_px, 0.0) * 0.5 - distance_px + 0.5;
}

/// Cell outlines and whole-level contour highlights from the same grid,
/// lattice and implicit-contour functions as the normal field draw.
@fragment
fn fs_data_selection(in: FieldOut) -> @location(0) vec4<f32> {
    let kind = data_selection.indices.x;
    if (kind == DATA_PICK_KIND_MATRIX_CELL) {
        return selection_color(selected_cell_coverage(in.axis_t));
    }
    if (kind == DATA_PICK_KIND_CONTOUR_LEVEL) {
        return selection_color(selected_contour_coverage(in.axis_t));
    }
    return vec4<f32>(0.0);
}
