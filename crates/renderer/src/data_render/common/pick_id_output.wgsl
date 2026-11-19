// ───── pick ID output (#381) ─────
// What a fragment writes to a pick ID target: an `Rgba32Uint` colour, never
// blended, and a depth that orders the hits the way a pick resolves them.
//
// The colour is (kind + 1 | covering, series slot, point index, primitive
// index): zero in the first lane is empty space, so a cleared target reads
// as no hit, and `PICK_ID_COVERING` in it marks a hit inside its drawn shape.
// `kind` is the primitive kind the hit is on (`PICK_KIND_*` of the point
// picker); the point index is the logical index the pick reports, the
// primitive index the drawn element (for a segment, its first point).
//
// The depth (compared `LessEqual`, cleared to 1) puts every covering hit in
// front of every near miss:
// - a hit inside its drawn shape (distance 0) writes `PICK_ID_DEPTH_BODY`,
//   or `PICK_ID_DEPTH_FILL` for an area fill, which every series' body
//   covers. Equal depths pass `LessEqual`, so among covering hits the one
//   drawn last wins: the topmost, what the user sees.
// - a near miss within the hover reach writes a depth that grows with its
//   distance, so the nearest wins, and among equal distances the last drawn.

const PICK_ID_DEPTH_BODY: f32 = 0.0;
const PICK_ID_DEPTH_FILL: f32 = 0.125;
const PICK_ID_DEPTH_NEAR: f32 = 0.25;
const PICK_ID_COVERING: u32 = 0x100u;

struct PickIdOutput {
    @location(0) id: vec4<u32>,
    @builtin(frag_depth) depth: f32,
};

// The output for a hit of `kind` on series slot `series` at `distance_px`
// from the cursor, `reach_px` the farthest distance a hover accepts (a hit
// past it is discarded before this is called). `fill` marks an area fill.
fn pick_id_output(
    kind: u32,
    series: u32,
    point_index: u32,
    primitive_index: u32,
    distance_px: f32,
    reach_px: f32,
    fill: bool,
) -> PickIdOutput {
    var depth = select(PICK_ID_DEPTH_BODY, PICK_ID_DEPTH_FILL, fill);
    var tag = (kind + 1u) | PICK_ID_COVERING;
    if distance_px > 0.0 {
        tag = kind + 1u;
        let share = clamp(distance_px / max(reach_px, 1.0e-6), 0.0, 1.0);
        depth = PICK_ID_DEPTH_NEAR + (1.0 - PICK_ID_DEPTH_NEAR) * share * 0.999;
    }
    return PickIdOutput(vec4<u32>(tag, series, point_index, primitive_index), depth);
}
