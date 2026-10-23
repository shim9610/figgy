// Independent radar chart primitives in physical pixels. No Cartesian shared
// blocks. Instance 0 is the background and the back raster (rings, spokes,
// value labels); instances 1..=n are the series primitives (polygon fills,
// outline segments and vertex markers, each one quad evaluated as a signed
// distance with four subsamples of edge AA); instance n + 1 is the front
// raster (text, title, legend, selection boxes).
struct Globals { size: vec4<f32>, background: vec4<f32>, clip: vec4<f32> };
struct Prim {
    kind: vec4<f32>,   // kind, vertex offset, vertex count, emphasis
    rect: vec4<f32>,   // left, top, right, bottom (physical pixels)
    points: vec4<f32>, // segment a.xy b.xy | marker center.xy radius 0
    color: vec4<f32>,  // straight RGBA
    stroke: vec4<f32>, // half width (segment), 0, 0, 0
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> prims: array<Prim>;
@group(0) @binding(2) var annotations: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> vertices: array<vec2<f32>>;
const KIND_POLYGON: f32 = 0.0;
const KIND_SEGMENT: f32 = 1.0;
const KIND_CIRCLE: f32 = 2.0;
const KIND_SQUARE: f32 = 3.0;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) @interpolate(flat) layer: u32 };
@vertex fn vs(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Vertex {
    let uv = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 1.0));
    var lo = vec2(0.0);
    var hi = globals.size.xy;
    if instance > 0u && instance <= u32(globals.size.z) {
        let p = prims[instance - 1u];
        lo = p.rect.xy - 1.0;
        hi = p.rect.zw + 1.0;
    }
    let p = mix(lo, hi, uv[vertex]);
    return Vertex(vec4(p.x / globals.size.x * 2.0 - 1.0, 1.0 - p.y / globals.size.y * 2.0, 0.0, 1.0), instance);
}
/// Signed distance to the closed polygon of `count` vertices from `offset`:
/// negative inside. Keep in step with `radar.rs::polygon_distance`.
fn polygon_sdf(p: vec2<f32>, offset: u32, count: u32) -> f32 {
    var inside = false;
    var distance = 1e30;
    for (var i = 0u; i < count; i++) {
        let a = vertices[offset + i];
        let b = vertices[offset + (i + 1u) % count];
        let edge = b - a;
        let q = p - a;
        let t = clamp(dot(q, edge) / max(dot(edge, edge), 1e-20), 0.0, 1.0);
        distance = min(distance, length(q - edge * t));
        if (a.y > p.y) != (b.y > p.y) {
            if p.x < a.x + (p.y - a.y) * edge.x / edge.y { inside = !inside; }
        }
    }
    return select(distance, -distance, inside);
}
fn prim_sdf(p: vec2<f32>, prim: Prim) -> f32 {
    let kind = prim.kind.x;
    if kind == KIND_POLYGON {
        return polygon_sdf(p, u32(prim.kind.y), u32(prim.kind.z));
    }
    if kind == KIND_SEGMENT {
        let a = prim.points.xy;
        let edge = prim.points.zw - a;
        let q = p - a;
        let t = clamp(dot(q, edge) / max(dot(edge, edge), 1e-20), 0.0, 1.0);
        return length(q - edge * t) - prim.stroke.x;
    }
    let d = p - prim.points.xy;
    if kind == KIND_SQUARE {
        return max(abs(d.x), abs(d.y)) - prim.points.z;
    }
    return length(d) - prim.points.z;
}
@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let p = v.position.xy;
    if v.layer == 0u {
        let ink = textureLoad(annotations, vec2<i32>(p), 0);
        let bg = vec4(globals.background.rgb * globals.background.a, globals.background.a);
        return output(ink + bg * (1.0 - ink.a));
    }
    if v.layer > u32(globals.size.z) {
        return output(textureLoad(annotations, vec2<i32>(p) + vec2(0, i32(globals.size.y)), 0));
    }
    if p.x < globals.clip.x || p.y < globals.clip.y || p.x >= globals.clip.z || p.y >= globals.clip.w { discard; }
    let prim = prims[v.layer - 1u];
    let rgb = clamp(prim.color.rgb + vec3(prim.kind.w), vec3(0.0), vec3(1.0));
    var coverage = 0.0;
    for (var y = 0u; y < 2u; y++) {
        for (var x = 0u; x < 2u; x++) {
            let sample = p + (vec2(f32(x), f32(y)) + 0.5) * 0.5 - 0.5;
            if prim_sdf(sample, prim) <= 0.0 { coverage += 0.25; }
        }
    }
    let alpha = prim.color.a * coverage;
    return output(vec4(rgb * alpha, alpha));
}
