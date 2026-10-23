// Exact fragment: line strokes (SHADER_COMMON.md §5.6), their step pieces,
// dash phase and styled extrusion, and the segment candidate.

// Data-space corners of step piece `k` of the segment a → b. Same rules as
// `step_piece` in line_columnar.wgsl (data-space midpoint for step-mid), in
// this shader's own pick_* ABI; the CPU twin is `line_step::step_pieces`.
struct PickStepPiece {
    x0: vec2<f32>,
    y0: vec2<f32>,
    x1: vec2<f32>,
    y1: vec2<f32>,
};

// The mode word: bits 0-1 the step mode, bits 4 and up the polar arc
// pieces per segment (`transform.polar_extent.z`), as `step_piece_count` in
// line_columnar.wgsl reads it.
const PICK_ARC_SHIFT: u32 = 4u;

fn pick_line_mode() -> u32 {
    let arc = u32(max(pick_query_params.transform.polar_extent.z, 0.0));
    return (pick_query_params.line.x & 3u) | (arc << PICK_ARC_SHIFT);
}

fn pick_step_piece_count(mode: u32) -> u32 {
    let arc = mode >> PICK_ARC_SHIFT;
    if arc > 0u {
        return arc;
    }
    let step = mode & 3u;
    if step == PICK_STEP_PRE || step == PICK_STEP_POST {
        return 2u;
    }
    if step == PICK_STEP_MID {
        return 3u;
    }
    return 1u;
}

fn pick_step_piece(ax: vec2<f32>, ay: vec2<f32>, bx: vec2<f32>, by: vec2<f32>, mode: u32, k: u32) -> PickStepPiece {
    var p: PickStepPiece;
    p.x0 = ax;
    p.y0 = ay;
    p.x1 = bx;
    p.y1 = by;
    let arc = mode >> PICK_ARC_SHIFT;
    if arc > 0u {
        let t0 = f32(k) / f32(arc);
        let t1 = f32(k + 1u) / f32(arc);
        p.x0 = mix(ax, bx, t0);
        p.y0 = mix(ay, by, t0);
        p.x1 = mix(ax, bx, t1);
        p.y1 = mix(ay, by, t1);
        return p;
    }
    let step = mode & 3u;
    if step == PICK_STEP_PRE {
        if k == 0u { p.x1 = ax; } else { p.y0 = by; }
    } else if step == PICK_STEP_POST {
        if k == 0u { p.y1 = ay; } else { p.x0 = bx; }
    } else if step == PICK_STEP_MID {
        let xm = (ax + bx) * 0.5;
        if k == 0u {
            p.x1 = xm;
            p.y1 = ay;
        } else if k == 1u {
            p.x0 = xm;
            p.x1 = xm;
        } else {
            p.x0 = xm;
            p.y0 = by;
        }
    }
    return p;
}

// ───── shared line stroke (SHADER_COMMON.md §5.6; `shader_consistency`
// compares these function bodies and constants with the line shader) ─────
