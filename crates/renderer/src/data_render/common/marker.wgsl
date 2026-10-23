// The marker quad's margin past the shape's visual radius, in pixels:
// outline strokes and their anti-aliasing reach past the contour, and
// nothing a marker draws lies outside `visual_shape_radius + QUAD_MARGIN_PX`
// on either axis.
const QUAD_MARGIN_PX: f32 = 2.0;
// Half-width of the ~1.5 px contour stroke (outline shapes and Cross).
const STROKE_HALF_PX: f32 = 0.75;

const MARKER_PI: f32 = 3.14159265359;
const MARKER_HALF_PI: f32 = 1.57079632679;
const STAR_INNER_RATIO: f32 = 0.5;

fn shape_is_filled(shape_id: u32) -> bool {
    return (shape_id >= 5u && shape_id <= 8u) || shape_id >= 17u;
}

fn base_shape_id(shape_id: u32) -> u32 {
    switch shape_id {
        case 5u: { return 0u; }   // CircleFilled
        case 6u: { return 1u; }   // SquareFilled
        case 7u: { return 2u; }   // TriangleFilled
        case 8u: { return 3u; }   // DiamondFilled
        case 9u: { return 5u; }   // TriangleDown
        case 10u: { return 6u; }  // TriangleLeft
        case 11u: { return 7u; }  // TriangleRight
        case 12u: { return 8u; }  // Plus
        case 13u: { return 9u; }  // Pentagon
        case 14u: { return 10u; } // Hexagon
        case 15u: { return 11u; } // Octagon
        case 16u: { return 12u; } // Star
        case 17u: { return 5u; }  // TriangleDownFilled
        case 18u: { return 6u; }  // TriangleLeftFilled
        case 19u: { return 7u; }  // TriangleRightFilled
        case 20u: { return 8u; }  // PlusFilled
        case 21u: { return 4u; }  // CrossFilled
        case 22u: { return 9u; }  // PentagonFilled
        case 23u: { return 10u; } // HexagonFilled
        case 24u: { return 11u; } // OctagonFilled
        case 25u: { return 12u; } // StarFilled
        default: { return shape_id; }
    }
}

// Shape-local radius/half-extent relative to the circle reference radius.
// Filled polygon markers are area-matched to a circle of radius r.
fn visual_shape_radius(base: u32, r: f32) -> f32 {
    var scale = 1.0;
    switch base {
        case 1u: { scale = 0.88622695; }  // square sqrt(pi)/2
        case 2u, 5u, 6u, 7u: { scale = 1.55512030; } // triangles
        case 3u: { scale = 1.25331414; }  // diamond sqrt(pi/2)
        case 9u: { scale = 1.14913986; }  // pentagon
        case 10u: { scale = 1.09963611; } // hexagon
        case 11u: { scale = 1.05390737; } // octagon
        case 12u: { scale = 1.46285033; } // 5-point star, inner ratio 0.5
        default: {}
    }
    return r * scale;
}

fn rotate2(p: vec2<f32>, a: f32) -> vec2<f32> {
    let c = cos(a);
    let s = sin(a);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

// Unsigned distance from `p` to segment `ab`. The denominator guard keeps
// a zero-length segment (radius 0) from yielding NaN — it degenerates to
// point distance instead.
fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0);
}

fn regular_vertex(i: u32, n: u32, r: f32, rot: f32) -> vec2<f32> {
    let a = rot + f32(i) * (2.0 * MARKER_PI / f32(n));
    return vec2<f32>(cos(a), sin(a)) * r;
}

fn star_vertex(i: u32, r: f32) -> vec2<f32> {
    let outer = (i % 2u) == 0u;
    let rr = select(r * STAR_INNER_RATIO, r, outer);
    let a = MARKER_HALF_PI + f32(i) * MARKER_PI / 5.0;
    return vec2<f32>(cos(a), sin(a)) * rr;
}

fn sd_regular_polygon(p: vec2<f32>, n: u32, r: f32, rot: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = regular_vertex(n - 1u, n, r, rot);
    for (var i = 0u; i < n; i = i + 1u) {
        let curr = regular_vertex(i, n, r, rot);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_star(p: vec2<f32>, r: f32) -> f32 {
    var min_d = 1e20;
    var inside = false;
    var prev = star_vertex(9u, r);
    for (var i = 0u; i < 10u; i = i + 1u) {
        let curr = star_vertex(i, r);
        min_d = min(min_d, sd_segment(p, prev, curr));
        if ((curr.y > p.y) != (prev.y > p.y)) {
            let x_at_y = (prev.x - curr.x) * (p.y - curr.y) / (prev.y - curr.y) + curr.x;
            if (p.x < x_at_y) {
                inside = !inside;
            }
        }
        prev = curr;
    }
    return select(min_d, -min_d, inside);
}

fn sd_plus_filled(p: vec2<f32>, r: f32) -> f32 {
    let arm = max(r * 0.32, STROKE_HALF_PX);
    let h = sd_box(p, vec2<f32>(r, arm));
    let v = sd_box(p, vec2<f32>(arm, r));
    return min(h, v);
}

// Exact SDF of an equilateral point-up triangle centered on the origin,
// vertices on the circle of radius `r` (same inscribed sizing as the
// diamond). `p_in` is y-up (NDC orientation), so the apex points up on
// screen. Adapted from iq's sdEquilateralTriangle.
fn sd_triangle(p_in: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    let half_w = 0.5 * k * r; // half of the base edge
    var p = vec2<f32>(abs(p_in.x) - half_w, p_in.y + half_w / k);
    if (p.x + k * p.y > 0.0) {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) * 0.5;
    }
    p.x -= clamp(p.x, -2.0 * half_w, 0.0);
    return -length(p) * sign(p.y);
}

// Signed pixel distance from `p` to the contour of `base` shape. For open
// plus/cross, the distance is unsigned to the center strokes; the outline
// alpha path below turns that into a visible stroke.
fn shape_distance(base: u32, p: vec2<f32>, r: f32, filled: bool) -> f32 {
    var d: f32;
    switch base {
        case 0u: { d = length(p) - r; }
        case 1u: { d = max(abs(p.x), abs(p.y)) - r; }
        case 2u: { d = sd_triangle(p, r); }
        case 3u: { d = abs(p.x) + abs(p.y) - r; }
        case 4u: { // Cross / x
            let c = r;
            if (filled) {
                d = sd_plus_filled(rotate2(p, MARKER_PI * 0.25), c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, -c), vec2<f32>(c, c));
                let d2 = sd_segment(p, vec2<f32>(-c, c), vec2<f32>(c, -c));
                d = min(d1, d2);
            }
        }
        case 5u: { d = sd_triangle(-p, r); }
        case 6u: { d = sd_triangle(rotate2(p, -MARKER_HALF_PI), r); }
        case 7u: { d = sd_triangle(rotate2(p, MARKER_HALF_PI), r); }
        case 8u: { // Plus / +
            let c = r;
            if (filled) {
                d = sd_plus_filled(p, c);
            } else {
                let d1 = sd_segment(p, vec2<f32>(-c, 0.0), vec2<f32>(c, 0.0));
                let d2 = sd_segment(p, vec2<f32>(0.0, -c), vec2<f32>(0.0, c));
                d = min(d1, d2);
            }
        }
        case 9u: { d = sd_regular_polygon(p, 5u, r, MARKER_HALF_PI); }
        case 10u: { d = sd_regular_polygon(p, 6u, r, MARKER_HALF_PI); }
        case 11u: { d = sd_regular_polygon(p, 8u, r, MARKER_HALF_PI); }
        default: { // 12u: Star
            d = sd_star(p, r);
        }
    }
    return d;
}

/// The coverage edge of a marker at shape distance `d`: a filled marker
/// covers `d < 0`; an outline (and an open plus or cross) is a stroke of
/// half-width `STROKE_HALF_PX` centred on `d == 0`. A fragment draws where
/// this is below its anti-aliasing width, and the picker's hit distance is
/// `max(marker_edge(d, filled), 0)`.
fn marker_edge(d: f32, filled: bool) -> f32 {
    return select(abs(d) - STROKE_HALF_PX, d, filled);
}
