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
    /// Reserved ABI lane. The host dispatches exactly four correction steps.
    /// The legacy differential oracle reads this as its loop bound.
    projection_steps: u32,
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

/// Labels `anchor_select` can hold in invocation-private storage. CPU twin:
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

// A candidate record is private scratch until anchor_project finishes. Reuse
// its x lane for axis-t between dispatches; no extra buffer or binding is needed.
fn empty_anchor() -> LabelAnchor {
    var out: LabelAnchor;
    out.level = NO_LEVEL;
    return out;
}

@compute @workgroup_size(64)
fn anchor_seed(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cells = ap.lattice_x * ap.lattice_y;
    if (cells == 0u || gid.x >= cells * ap.level_count) { return; }
    var out = empty_anchor();
    let level = gid.x / cells;
    let wanted = level_value(level);
    if (wanted.valid && f32_is_finite(wanted.value)) {
        let cell = gid.x % cells;
        let step = vec2<f32>(ap.clip_w / f32(ap.lattice_x), ap.clip_h / f32(ap.lattice_y));
        let cell_lo = vec2<f32>(
            ap.clip_x + f32(cell % ap.lattice_x) * step.x,
            ap.clip_y + f32(cell / ap.lattice_x) * step.y,
        );
        out.x = px_to_t(cell_lo + 0.5 * step);
        out.level = level;
    }
    cand[gid.x] = out;
}

// One exact Newton correction. The host dispatches this same pipeline four
// times. Keeping the field search outside an enclosing shader loop prevents
// pathological optimizer expansion on software Vulkan compilers.
@compute @workgroup_size(64)
fn anchor_step(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cells = ap.lattice_x * ap.lattice_y;
    if (cells == 0u || gid.x >= cells * ap.level_count) { return; }
    var state = cand[gid.x];
    if (state.level == NO_LEVEL) { return; }
    let s = contour_sample(state.x);
    if (!s.hit) { cand[gid.x] = empty_anchor(); return; }
    let want = level_value(state.level).value;
    let g2 = dot(s.dz, s.dz);
    let residual = s.z - want;
    if (!f32_is_finite(g2) || g2 <= 0.0 || !f32_is_finite(residual)) {
        cand[gid.x] = empty_anchor(); return;
    }
    let correction = residual * s.dz / g2;
    if (!vec2_f32_is_finite(correction)) { cand[gid.x] = empty_anchor(); return; }
    let p = state.x - correction;
    if (!vec2_f32_is_finite(p)) { cand[gid.x] = empty_anchor(); return; }
    state.x = p;
    cand[gid.x] = state;
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

    // The four corrections were dispatched in order before this final sample.
    // Candidate x temporarily carries axis-t, never a published data coordinate.
    if (cand[gid.x].level == NO_LEVEL) {
        cand[gid.x] = out;
        return;
    }
    let p = cand[gid.x].x;
    let s = contour_sample(p);
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
    // A small residual alone is not an isoline. In a near-flat region a seed
    // can settle on a shallow extremum that is within `tolerance` of the level
    // without the level ever being reached there. A bilinear cell attains its
    // extremes at its corners, so the level set passes through this cell
    // exactly when the level lies within the corner range: the same condition
    // under which the stroke draws a line here.
    if (want < s.z_lo || want > s.z_hi) {
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

/// Kept labels' screen positions and box radii, for the overlap test.
/// Only the single invocation in `anchor_select` uses this scratch storage.
/// Private arrays retain WGSL initialization guarantees without shared-memory
/// initialization, which makes WARP's first dispatch disproportionately costly.
var<private> kept_px: array<vec2<f32>, 1024>;
var<private> kept_r: array<f32, 1024>;

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
