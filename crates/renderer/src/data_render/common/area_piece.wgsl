// ───────────────────────────── area fill pieces ─────────────────────────────
// The trapezoids a line's fill draws (SHADER_COMMON.md §2.2): one per step
// piece of a segment a → b, the line piece on top and its base piece below,
// both produced by the same `step_piece` (common/line_step.wgsl) so their x
// corners agree. The base is a second series' y column (fill to a series)
// or a constant baseline. `vs_area` (line_columnar.wgsl) emits the corners
// and the picker's `gpu_pick/area.wgsl` tests the cursor against the same
// corners, so a fill picks exactly where it paints.

// `Style.primitive_flags` bit 4: the base is a second series, not the
// baseline in `Style.dash[1].xy`.
const AREA_FILL_TO_SERIES: u32 = 16u;

// Whether segment a → b has a fill: both line samples are valid, and so are
// the second series' samples (a gap in either line breaks the fill exactly
// where it breaks the line) or the baseline is finite.
fn area_segment_drawn(t: Transform, xa: vec2<f32>, ya: vec2<f32>, xb: vec2<f32>, yb: vec2<f32>,
    base_a: vec2<f32>, base_b: vec2<f32>, to_series: bool, baseline: vec2<f32>) -> bool {
    if (!sample_valid(t, xa, ya) || !sample_valid(t, xb, yb)) {
        return false;
    }
    if (to_series && (!sample_valid(t, xa, base_a) || !sample_valid(t, xb, base_b))) {
        return false;
    }
    if (!to_series && (!sample_finite(baseline.x) || !sample_finite(baseline.y))) {
        return false;
    }
    return true;
}

// Piece `k` of the fill under segment a → b: the line's piece on top and the
// base path's piece below. The base path steps with the same x corners: a
// baseline is constant, a second series follows this series' interpolation.
struct AreaPiece {
    top: StepPiece,
    bottom: StepPiece,
};

fn area_piece(xa: vec2<f32>, ya: vec2<f32>, xb: vec2<f32>, yb: vec2<f32>, base_a: vec2<f32>,
    base_b: vec2<f32>, to_series: bool, baseline: vec2<f32>, mode: u32, k: u32) -> AreaPiece {
    let top = step_piece(xa, ya, xb, yb, mode, k);
    let base_ya = select(baseline, base_a, to_series);
    let base_yb = select(baseline, base_b, to_series);
    let bottom = step_piece(xa, base_ya, xb, base_yb, mode, k);
    return AreaPiece(top, bottom);
}

// A baseline at or below zero on a logarithmic y axis has no position. The
// line's samples would be masked there; masking the baseline would delete the
// whole area instead, so it is clamped to the axis minimum (data_t = 0). This
// is the documented exception to the sample_valid rule.
fn area_baseline_ndc_y(t: Transform, base: vec2<f32>) -> f32 {
    let value = base.x + base.y;
    if (t.scale_log.y != 0.0 && !(value > 0.0)) {
        return t.data_to_panel_offset.y * 2.0 - 1.0;
    }
    return data_to_ndc(t, vec2<f32>(1.0, 0.0), base).y;
}

// The NDC of one corner of `piece`: at end A or B, on the line or on its
// base. The strip `vs_area` emits per piece is (A top, A base, B top,
// B base), triangles (A top, A base, B top) and (A base, B top, B base).
fn area_corner_ndc(t: Transform, piece: AreaPiece, at_b: bool, on_base: bool, to_series: bool) -> vec2<f32> {
    let x = select(piece.top.x0, piece.top.x1, at_b);
    let y_top = select(piece.top.y0, piece.top.y1, at_b);
    let y_base = select(piece.bottom.y0, piece.bottom.y1, at_b);
    var ndc = data_to_ndc(t, x, y_top);
    if (on_base) {
        // A polar baseline is a ring (the inner ring at or below the axis
        // start, which is the centre when `inner_radius_fraction` is 0), so
        // the base corner projects like a sample at the same angle.
        if (to_series || t.polar_angle.x > 0.5) {
            ndc = data_to_ndc(t, x, y_base);
        } else {
            ndc.y = area_baseline_ndc_y(t, y_base);
        }
    }
    return ndc;
}
// ─────────────────────────── end area fill pieces ───────────────────────────
