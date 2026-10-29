// ──────────────────────────── step interpolation ────────────────────────────
// A step mode draws every instance (segment a → b) as axis-aligned pieces.
// Piece corners live in DATA space so every consumer projects the same
// points: the line, area, sketch, ribbon and star entries here, the
// arc-length scan (line_arc.wgsl, `params.mode`) and the GPU picker
// (gpu_pick/line.wgsl). The line, arc and pick line modules splice this one
// fragment, so all of them split a segment into the same pieces; the CPU
// twin is `line_step::step_pieces` in the renderer crate.
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

// Data-space (hi, lo) corners of piece `k` of the stepped segment a → b,
// and the coordinate its two corners share: `STEP_AXIS_X` for a riser,
// `STEP_AXIS_Y` for a run, `STEP_AXIS_NONE` for a linear segment or an arc.
struct StepPiece {
    x0: vec2<f32>,
    y0: vec2<f32>,
    x1: vec2<f32>,
    y1: vec2<f32>,
    axis: u32,
};

const STEP_AXIS_NONE: u32 = 0u;
const STEP_AXIS_X: u32 = 1u;
const STEP_AXIS_Y: u32 = 2u;

fn step_piece(xa: vec2<f32>, ya: vec2<f32>, xb: vec2<f32>, yb: vec2<f32>, mode: u32, k: u32) -> StepPiece {
    var p: StepPiece;
    p.x0 = xa;
    p.y0 = ya;
    p.x1 = xb;
    p.y1 = yb;
    p.axis = STEP_AXIS_NONE;
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
        if (k == 0u) { p.x1 = xa; p.axis = STEP_AXIS_X; } else { p.y0 = yb; p.axis = STEP_AXIS_Y; }
    } else if (step == LINE_STEP_POST) {
        // Run at y_a, then the riser at x_b.
        if (k == 0u) { p.y1 = ya; p.axis = STEP_AXIS_Y; } else { p.x0 = xb; p.axis = STEP_AXIS_X; }
    } else if (step == LINE_STEP_MID) {
        // The riser sits at the DATA-space midpoint of the two x values, not
        // the pixel midpoint — the two differ on a logarithmic axis, and the
        // CPU picker places it in data units too.
        let xm = (xa + xb) * 0.5;
        if (k == 0u) {
            p.x1 = xm;
            p.y1 = ya;
            p.axis = STEP_AXIS_Y;
        } else if (k == 1u) {
            p.x0 = xm;
            p.x1 = xm;
            p.axis = STEP_AXIS_X;
        } else {
            p.x0 = xm;
            p.y0 = yb;
            p.axis = STEP_AXIS_Y;
        }
    }
    return p;
}

// The projected end `b` of piece `p` (NDC or pixels) given its projected
// start `a`, with the coordinate the corners share in data space copied from
// `a`. Projecting that coordinate twice need not round alike (Metal's
// compiler may contract one evaluation and not the other), and on a piece far
// shorter than a pixel, a riser between nearly equal values, one ulp across
// tilts the stroke's square-capped quad by about a degree (#63); a vertex stage that
// divides NDC into pixels copies it again after the division, which Metal may
// also round differently for equal inputs. A polar chart maps
// the data axes onto rings and spokes, so nothing is copied there.
fn step_piece_end(t: Transform, p: StepPiece, a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    if (t.polar_angle.x > 0.5) {
        return b;
    }
    if (p.axis == STEP_AXIS_X) {
        return vec2<f32>(a.x, b.y);
    }
    if (p.axis == STEP_AXIS_Y) {
        return vec2<f32>(b.x, a.y);
    }
    return b;
}

// The unit direction `dir` of piece `p` in pixels, exactly along its axis:
// a riser's x and a run's y are zero, keeping only the sign. Copying the
// shared coordinate (`step_piece_end`) is not enough in Metal's vertex stage,
// which can leave an ulp in the difference of the two equal coordinates;
// divided by a riser's length of 1e-4 px it tilted the quad by a degree
// (#63).
fn step_piece_dir(t: Transform, p: StepPiece, dir: vec2<f32>) -> vec2<f32> {
    if (t.polar_angle.x > 0.5) {
        return dir;
    }
    if (p.axis == STEP_AXIS_X) {
        return vec2<f32>(0.0, sign(dir.y));
    }
    if (p.axis == STEP_AXIS_Y) {
        return vec2<f32>(sign(dir.x), 0.0);
    }
    return dir;
}

// Pixel length of one piece, with the arc scan's projection.
fn step_piece_len_px(t: Transform, p: StepPiece) -> f32 {
    let a = data_to_px(t, p.x0, p.y0);
    return distance(a, step_piece_end(t, p, a, data_to_px(t, p.x1, p.y1)));
}
// ─────────────────────── end step interpolation ───────────────────────
