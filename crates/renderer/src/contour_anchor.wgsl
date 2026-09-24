// Contour label anchor selection — where each label goes, chosen on the GPU.
//
// The labels' *content* is baked on the CPU before the frame (it comes from
// `levels`, which is config); only the *position* is a GPU fact. This is that
// position, and it is computed from the field itself rather than from a list of
// traced segments. The implicit-field path replaced marching squares with
// form, and this shader is the label half of that change.
//
// **Seed lattice, then Newton.** The data area is divided into cells of about
// `spacing_px`. Each (cell, level) pair seeds one candidate at the cell centre
// and projects it onto that level's isoline:
//
//     p <- p - (z(p) - level) * grad z / |grad z|^2
//
// Four steps. The field is bilinear, so away from a stationary point that is two
// or three more than enough. The tangent falls out exactly as `perp(grad z)` —
// no finite difference through the transform, no polyline to walk.
//
// **Determinism is structural here, not a rule.** The candidate set is a fixed
// lattice and every candidate writes its own slot (`level * cells + cell`), so
// there is not one atomic operation in the pass. The previous design picked
// winners by `atomicMin` over `atomicAdd`-assigned segment indices, which was
// only stable because a software rasterizer happens to run serially.
//
// Two passes:
//
//   1. `anchor_project` — one invocation per (cell, level); writes a candidate
//      or marks the slot empty. Parallel, no communication.
//   2. `anchor_select`   — one invocation, serial: keeps candidates that clear
//      `spacing_px` and cannot overlap, then gives any level that came out empty
//      the candidate farthest from the labels already kept.
//
// A seed's projection is kept only if it lands back inside that seed's own
// lattice cell. That is what stops a dozen neighbouring seeds from converging on
// the same stretch of one isoline, and it needs no communication at all.
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
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
};  // 112 B (vec4 array at offset 64, stride 16)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}

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
/// CPU twin: `gpu_contour::AnchorParamsGpu` (`#[repr(C)]`, 64 B).
struct AnchorParams {
    /// Seed lattice cells along each screen axis, over the data area.
    lattice_x: u32,
    lattice_y: u32,
    level_count: u32,
    /// Anchors `anchor_select` may keep. CPU twin: `MAX_CONTOUR_LABELS_TOTAL`.
    kept_capacity: u32,
    /// **Chart area** in the panel's pixel frame. `axis_pair_to_t` returns `t`
    /// over the chart area (`scatter_transform_from_config` extends the axis
    /// range to it), so this is what converts between `t` and a pixel.
    area_x: f32,
    area_y: f32,
    area_w: f32,
    area_h: f32,
    /// **Data area** in the same frame — where the seeds go, and the only region
    /// whose labels survive the draw's scissor.
    clip_x: f32,
    clip_y: f32,
    clip_w: f32,
    clip_h: f32,
    /// Target minimum screen distance for the normal selection sweep. The
    /// per-level fallback may keep a closer candidate rather than omit a level.
    spacing_px: f32,
    /// Atlas cell height in pixels — the label box's height, for the overlap
    /// test. Widths are per level and come from `cell_w`.
    label_h_px: f32,
    /// Vertices per label instance. CPU twin: `gpu_contour::LABEL_VERTICES`.
    label_vertices: u32,
    _pad0: u32,
};

/// One label placement. CPU twin: `gpu_contour::LabelAnchorGpu` (32 B), and the
/// instance attributes `contour_label.wgsl` reads. A host that fills
/// `ContourLabelConfig::anchors` writes this same record, which is what keeps the
/// draw single-path.
struct LabelAnchor {
    x: vec2<f32>,
    y: vec2<f32>,
    dir: vec2<f32>,
    level: u32,
    width_px: f32,
};

/// An empty candidate slot. Also collapses the label quad in the render shader,
/// which is the same answer for a stale level index.
const NO_LEVEL: u32 = 0xffffffffu;

/// Labels `anchor_select` can hold in workgroup storage. CPU twin:
/// `gpu_contour::MAX_CONTOUR_LABELS_TOTAL`.
const MAX_KEPT: u32 = 1024u;

@group(1) @binding(0) var<uniform> ap: AnchorParams;
@group(1) @binding(1) var<storage, read_write> cand: array<LabelAnchor>;
@group(1) @binding(2) var<storage, read_write> anchors: array<LabelAnchor>;
@group(1) @binding(3) var<storage, read_write> label_indirect: array<u32>;
/// Content width of each level's atlas row, in pixels — the label box's width.
@group(1) @binding(4) var<storage, read> cell_w: array<f32>;

/// A pixel in the panel's frame to axis `t`. Screen y grows downward, `t` up.
fn px_to_t(px: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        (px.x - ap.area_x) / max(ap.area_w, 1.0),
        1.0 - (px.y - ap.area_y) / max(ap.area_h, 1.0),
    );
}

fn t_to_px(t: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(
        ap.area_x + t.x * ap.area_w,
        ap.area_y + (1.0 - t.y) * ap.area_h,
    );
}

/// Normalized data-axis `t` back to a data value, in the pool's `(hi, lo)` pair
/// form.
///
/// The inverse of `axis_pair_to_t`. On a linear axis each lane is interpolated
/// separately: the sum reproduces `min + t * (max - min)` exactly while each lane
/// stays in its own magnitude regime, which is the entire point of the split. A
/// logarithmic axis has to exponentiate, so its result is a single f32 — accuracy
/// on a log axis is relative anyway.
fn t_to_pair(t: f32, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32) -> vec2<f32> {
    if (is_log != 0.0) {
        let lo_v = min_hi + min_lo;
        let hi_v = max_hi + max_lo;
        return vec2<f32>(pow(10.0, lo_v + t * (hi_v - lo_v)), 0.0);
    }
    return vec2<f32>(min_hi + t * (max_hi - min_hi), min_lo + t * (max_lo - min_lo));
}

fn t_to_pair_x(t: f32) -> vec2<f32> {
    let data_t = (t - transform.data_to_panel_offset.x) / transform.data_to_panel_scale.x;
    return t_to_pair(data_t, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x);
}

fn t_to_pair_y(t: f32) -> vec2<f32> {
    let data_t = (t - transform.data_to_panel_offset.y) / transform.data_to_panel_scale.y;
    return t_to_pair(data_t, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y);
}

/// A `t` step as a data-space delta, through the same inverse.
///
/// Taken as a difference of two inverted points rather than a scaled span, so it
/// is right on a logarithmic axis too — there a fixed `t` step is a fixed
/// *ratio*, not a fixed amount.
fn t_step_to_data(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return (b.x + b.y) - (a.x + a.y);
}

struct LevelValue {
    value: f32,
    valid: bool,
};

/// This candidate's level with explicit declaration-bounds validity.
fn level_value(level: u32) -> LevelValue {
    var out: LevelValue;
    out.value = 0.0;
    out.valid = false;
    if (level >= field.level_count) {
        return out;
    }
    out.value = levels[level];
    out.valid = true;
    return out;
}

@compute @workgroup_size(64)
fn anchor_project(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cells = ap.lattice_x * ap.lattice_y;
    if (cells == 0u || gid.x >= cells * ap.level_count) {
        return;
    }
    var out: LabelAnchor;
    out.x = vec2<f32>(0.0, 0.0);
    out.y = vec2<f32>(0.0, 0.0);
    out.dir = vec2<f32>(0.0, 0.0);
    out.level = NO_LEVEL;
    out.width_px = 0.0;

    let level = gid.x / cells;
    let cell = gid.x % cells;
    let wanted = level_value(level);
    if (!wanted.valid || !f32_is_finite(wanted.value)) {
        cand[gid.x] = out;
        return;
    }
    let want = wanted.value;
    let step = vec2<f32>(ap.clip_w / f32(ap.lattice_x), ap.clip_h / f32(ap.lattice_y));
    let cell_lo = vec2<f32>(
        ap.clip_x + f32(cell % ap.lattice_x) * step.x,
        ap.clip_y + f32(cell / ap.lattice_x) * step.y,
    );

    var p = px_to_t(cell_lo + 0.5 * step);
    var s = contour_sample(p);
    for (var i = 0u; i < 4u; i = i + 1u) {
        if (!s.hit) {
            cand[gid.x] = out;
            return;
        }
        let g2 = dot(s.dz, s.dz);
        let residual = s.z - want;
        if (!f32_is_finite(g2) || g2 <= 0.0 || !f32_is_finite(residual)) {
            cand[gid.x] = out;
            return;
        }
        let correction = residual * s.dz / g2;
        if (!vec2_f32_is_finite(correction)) {
            cand[gid.x] = out;
            return;
        }
        p = p - correction;
        if (!vec2_f32_is_finite(p)) {
            cand[gid.x] = out;
            return;
        }
        s = contour_sample(p);
    }
    if (!s.hit) {
        cand[gid.x] = out;
        return;
    }
    let g2 = dot(s.dz, s.dz);
    let residual = s.z - want;
    let tolerance = 1e-3 * max(abs(want), 1.0);
    // A seed that did not converge, left the grid, or landed on a stationary
    // point has no isoline position to report. Silence, not a guess.
    if (!f32_is_finite(g2) || g2 <= 0.0
        || !f32_is_finite(residual) || !f32_is_finite(tolerance)
        || abs(residual) > tolerance) {
        cand[gid.x] = out;
        return;
    }
    let hit_px = t_to_px(p);
    if (!vec2_f32_is_finite(hit_px)
        || hit_px.x < cell_lo.x || hit_px.x >= cell_lo.x + step.x
        || hit_px.y < cell_lo.y || hit_px.y >= cell_lo.y + step.y) {
        cand[gid.x] = out;
        return;
    }

    let out_x = t_to_pair_x(p.x);
    let out_y = t_to_pair_y(p.y);
    // The isoline's tangent is perpendicular to the gradient — exactly, not as a
    // difference of two traced endpoints. Converted back into data units so the
    // record means what a host-written one means, and kept to a hundredth of the
    // range so the render shader's finite difference stays local on a log axis.
    let tangent = normalize(vec2<f32>(-s.dz.y, s.dz.x)) * 1e-2;
    let out_dir = vec2<f32>(
        t_step_to_data(out_x, t_to_pair_x(p.x + tangent.x)),
        t_step_to_data(out_y, t_to_pair_y(p.y + tangent.y)),
    );
    if (!vec2_f32_is_finite(out_x) || !vec2_f32_is_finite(out_y)
        || !vec2_f32_is_finite(tangent) || !vec2_f32_is_finite(out_dir)) {
        cand[gid.x] = out;
        return;
    }
    out.x = out_x;
    out.y = out_y;
    out.dir = out_dir;
    out.level = level;
    out.width_px = cell_w[level];
    cand[gid.x] = out;
}

/// Kept labels' screen positions and box radii, for the overlap test. Workgroup
/// storage rather than a buffer: `anchor_select` is a single invocation, so this
/// never crosses one.
var<workgroup> kept_px: array<vec2<f32>, 1024>;
var<workgroup> kept_r: array<f32, 1024>;

/// Half the diagonal of level `i`'s label box, in pixels.
fn label_radius(i: u32) -> f32 {
    let last = max(ap.level_count, 1u) - 1u;
    return 0.5 * length(vec2<f32>(cell_w[min(i, last)], ap.label_h_px));
}

fn anchor_px(a: LabelAnchor) -> vec2<f32> {
    return t_to_px((data_to_ndc(a.x, a.y) + vec2<f32>(1.0, 1.0)) * 0.5);
}

@compute @workgroup_size(1)
fn anchor_select() {
    let cells = ap.lattice_x * ap.lattice_y;
    let cap = min(ap.kept_capacity, MAX_KEPT);
    var kept = 0u;

    // Sweep A — rank-major: every level's k-th candidate is considered before any
    // level's (k+1)-th, so a level whose isoline crosses many cells cannot crowd
    // out one that crosses few.
    for (var k = 0u; k < cells && kept < cap; k = k + 1u) {
        for (var l = 0u; l < ap.level_count && kept < cap; l = l + 1u) {
            let c = cand[l * cells + k];
            if (c.level != l) {
                continue;
            }
            let px = anchor_px(c);
            let r = label_radius(l);
            var ok = true;
            for (var j = 0u; j < kept; j = j + 1u) {
                // Two boxes whose centres are farther apart than the sum of
                // their half-diagonals cannot overlap, whatever their rotation —
                // a sufficient condition, with `spacing_px` as the floor.
                if (distance(px, kept_px[j]) < max(ap.spacing_px, r + kept_r[j])) {
                    ok = false;
                    break;
                }
            }
            if (!ok) {
                continue;
            }
            anchors[kept] = c;
            kept_px[kept] = px;
            kept_r[kept] = r;
            kept = kept + 1u;
        }
    }

    // Sweep B — a level that came out with nothing takes a label anyway, because a
    // missing label is worse than a crowded one. Which one: the candidate
    // **farthest** from everything already kept.
    //
    // Not the lowest-ranked one. A level whose whole isoline fits inside
    // `spacing_px` — nested rings around a peak, at a wide spacing — cannot
    // satisfy the minimum at all, and taking its first candidate piles every such
    // level onto the same spot. Farthest-point spreads the unavoidable crowding
    // around the rings instead. Ties go to the lower index, so this stays
    // deterministic.
    for (var l = 0u; l < ap.level_count && kept < cap; l = l + 1u) {
        var has = false;
        for (var j = 0u; j < kept; j = j + 1u) {
            if (anchors[j].level == l) {
                has = true;
                break;
            }
        }
        if (has) {
            continue;
        }
        var best = 0u;
        var best_gap = -1.0;
        var found = false;
        for (var k = 0u; k < cells; k = k + 1u) {
            let c = cand[l * cells + k];
            if (c.level != l) {
                continue;
            }
            let px = anchor_px(c);
            var gap = 1e30;
            for (var j = 0u; j < kept; j = j + 1u) {
                gap = min(gap, distance(px, kept_px[j]));
            }
            if (gap > best_gap) {
                best_gap = gap;
                best = k;
                found = true;
            }
        }
        if (!found) {
            continue;
        }
        let c = cand[l * cells + best];
        anchors[kept] = c;
        kept_px[kept] = anchor_px(c);
        kept_r[kept] = label_radius(l);
        kept = kept + 1u;
    }

    label_indirect[0] = ap.label_vertices;
    label_indirect[1] = kept;
    label_indirect[2] = 0u;
    label_indirect[3] = 0u;
}
