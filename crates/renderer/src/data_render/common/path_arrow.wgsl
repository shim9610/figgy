// Path arrows: an arrowhead's place on a drawn polyline segment.
//
// A streamline series' arrowheads sit at fixed arc positions along the
// drawn path (the arc prefix `line_arc.wgsl` scans, in render pixels) and
// point along it. Both the draw (`vector/vector_input_path.wgsl`, through
// the vector glyph) and the pick (`gpu_pick/line.wgsl`) place a head with
// this one function, so the head is picked where it is painted.
//
// The point is the data-space interpolation of the segment's two samples at
// the arc fraction, and the tangent the segment's unit data-space
// direction. On linear axes both are exact: the drawn segment is the
// projection of the data segment, and its arc grows linearly along it. On a
// logarithmic axis the data-space interpolation is a sub-pixel off the drawn
// chord (#51, D6), the same for the draw and the pick.

struct PathArrow {
    // The head's centre as the pool's hi/lo pairs.
    x: vec2<f32>,
    y: vec2<f32>,
    // The unit data-space direction of the segment, A to B.
    tangent: vec2<f32>,
    // The arc fraction along the segment, 0 at A and 1 at B.
    t: f32,
    valid: bool,
};

// The head at arc position `arc` on the segment from A = (`xa`, `ya`) at arc
// `arc_a` to B = (`xb`, `yb`) at arc `arc_b`: invalid when the arc lies
// outside [arc_a, arc_b), the segment has no arc length (a NaN gap or a
// degenerate segment) or no finite data-space direction.
fn path_arrow_on_segment(
    xa: vec2<f32>,
    ya: vec2<f32>,
    xb: vec2<f32>,
    yb: vec2<f32>,
    arc_a: f32,
    arc_b: f32,
    arc: f32,
) -> PathArrow {
    var out: PathArrow;
    out.x = xa;
    out.y = ya;
    out.tangent = vec2<f32>(0.0, 0.0);
    out.t = 0.0;
    out.valid = false;
    let span = arc_b - arc_a;
    if (!(span > 0.0) || !(arc >= arc_a) || !(arc < arc_b)) {
        return out;
    }
    let t = clamp((arc - arc_a) / span, 0.0, 1.0);
    // B - A in data units, each half subtracted before they are added.
    let dx = (xb.x - xa.x) + (xb.y - xa.y);
    let dy = (yb.x - ya.x) + (yb.y - ya.y);
    // Normalized through the larger component, so a segment of huge or
    // tiny extent keeps a finite direction.
    let reach = max(abs(dx), abs(dy));
    if (!arrow_finite(dx) || !arrow_finite(dy) || !(reach > 0.0)) {
        return out;
    }
    let scaled = vec2<f32>(dx, dy) / reach;
    out.x = pair_add(xa, t * dx);
    out.y = pair_add(ya, t * dy);
    out.tangent = scaled / length(scaled);
    out.t = t;
    out.valid = true;
    return out;
}
