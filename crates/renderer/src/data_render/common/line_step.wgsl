// ──────────── step interpolation (NOT part of the common block) ────────────
// A step mode draws every instance (segment a → b) as axis-aligned pieces.
// Piece corners live in DATA space so every consumer projects the same
// points: the line, area, sketch, ribbon and star entries here, the
// arc-length scan (line_arc.wgsl, `params.mode`) and the GPU picker
// (gpu_pick/line.wgsl, in its own pick_* ABI). `step_piece_count`, `step_piece`
// and `step_piece_len_px` must stay byte-identical between line_columnar.wgsl
// and line_arc.wgsl (shader_consistency checks); the CPU twin is
// `line_step::step_pieces` in the renderer crate.
const LINE_STEP_SHIFT: u32 = 2u;
const LINE_STEP_MASK: u32 = 3u;
const LINE_STEP_LINEAR: u32 = 0u;
const LINE_STEP_PRE: u32 = 1u;
const LINE_STEP_POST: u32 = 2u;
const LINE_STEP_MID: u32 = 3u;
const LINE_ARC_SHIFT: u32 = 4u;

// The mode word: bits 0-1 the step mode, bits 4 and up the polar arc pieces
// per segment (0 for chords). An arc overrides the step mode; the renderer
// rejects the two together.
fn step_piece_count(mode: u32) -> u32 {
    let arc = mode >> LINE_ARC_SHIFT;
    if (arc > 0u) {
        return arc;
    }
    let step = mode & LINE_STEP_MASK;
    if (step == LINE_STEP_PRE || step == LINE_STEP_POST) {
        return 2u;
    }
    if (step == LINE_STEP_MID) {
        return 3u;
    }
    return 1u;
}

// Data-space (hi, lo) corners of piece `k` of the stepped segment a → b.
struct StepPiece {
    x0: vec2<f32>,
    y0: vec2<f32>,
    x1: vec2<f32>,
    y1: vec2<f32>,
};

fn step_piece(xa: vec2<f32>, ya: vec2<f32>, xb: vec2<f32>, yb: vec2<f32>, mode: u32, k: u32) -> StepPiece {
    var p: StepPiece;
    p.x0 = xa;
    p.y0 = ya;
    p.x1 = xb;
    p.y1 = yb;
    let arc = mode >> LINE_ARC_SHIFT;
    if (arc > 0u) {
        // Polar arc: piece k of `arc` equal subdivisions of a -> b in data
        // space, so a constant radius follows its ring once projected.
        let t0 = f32(k) / f32(arc);
        let t1 = f32(k + 1u) / f32(arc);
        p.x0 = mix(xa, xb, t0);
        p.y0 = mix(ya, yb, t0);
        p.x1 = mix(xa, xb, t1);
        p.y1 = mix(ya, yb, t1);
        return p;
    }
    let step = mode & LINE_STEP_MASK;
    if (step == LINE_STEP_PRE) {
        // Riser at x_a, then the run at y_b.
        if (k == 0u) { p.x1 = xa; } else { p.y0 = yb; }
    } else if (step == LINE_STEP_POST) {
        // Run at y_a, then the riser at x_b.
        if (k == 0u) { p.y1 = ya; } else { p.x0 = xb; }
    } else if (step == LINE_STEP_MID) {
        // The riser sits at the DATA-space midpoint of the two x values, not
        // the pixel midpoint — the two differ on a logarithmic axis, and the
        // CPU picker places it in data units too.
        let xm = (xa + xb) * 0.5;
        if (k == 0u) {
            p.x1 = xm;
            p.y1 = ya;
        } else if (k == 1u) {
            p.x0 = xm;
            p.x1 = xm;
        } else {
            p.x0 = xm;
            p.y0 = yb;
        }
    }
    return p;
}

// Pixel length of one piece, with the arc scan's projection.
fn step_piece_len_px(p: StepPiece) -> f32 {
    let a_px = data_to_ndc(p.x0, p.y0) / transform.pixel_to_ndc;
    let b_px = data_to_ndc(p.x1, p.y1) / transform.pixel_to_ndc;
    return distance(a_px, b_px);
}
// ─────────────────────── end step interpolation ───────────────────────
