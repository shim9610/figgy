// ── Ternary region fill (group 1) ───────────────────────────────────────────
//
// The closed regions a ternary series outlines, filled by stencil-then-cover
// with the even-odd rule (#49, #55). The host gives each polygon's vertices
// in order in the series' x, y and c columns, a NaN row separating polygons;
// the renderer closes each polygon from its last vertex to its first and
// creates no vertex of its own (rule 1).
//
// `vs_stencil` draws one triangle per edge, `(anchor, vᵢ, vᵢ₊₁)`, the anchor
// the triangle's centre: an even-odd fan's anchor is arbitrary, and a fixed
// one needs nothing stored per polygon (the maintainer's decision on #49).
// Every triangle inverts the stencil and writes no colour. `vs_cover` then
// draws the whole ternary triangle once per series where the stencil is
// non-zero and resets it to zero, so one series' polygons combine by
// even-odd: a polygon inside another is a hole, two that overlap leave their
// overlap unfilled. The vertices are read from the pool as storage; no
// vertex slot is bound.
//
// CPU twin: `data_render/ternary_region.rs::TernaryRegionParamsGpu`
// (`#[repr(C)]`, 32 B).
struct TernaryRegionParams {
    /// Premultiplied: the fill colour at its alpha times the opacity.
    color_premul: vec4<f32>,
    /// f32-lane bases of the x, y and c columns in the pool.
    x_base: u32,
    y_base: u32,
    c_base: u32,
    /// Vertices: the shortest of the three columns.
    count: u32,
};

@group(1) @binding(0) var<storage, read> region_pool: array<f32>;
@group(1) @binding(1) var<uniform> region: TernaryRegionParams;

fn region_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(region_pool[j], region_pool[j + 1u]);
}

/// Whether vertex `i` can be placed: inside the column and a valid sample.
fn region_valid(i: u32) -> bool {
    if (i >= region.count) {
        return false;
    }
    return ternary_sample_valid(
        region_pair(region.x_base, i),
        region_pair(region.y_base, i),
        region_pair(region.c_base, i),
    );
}

fn region_ndc(i: u32) -> vec2<f32> {
    return ternary_ndc(
        transform,
        region_pair(region.x_base, i),
        region_pair(region.y_base, i),
        region_pair(region.c_base, i),
    );
}

/// The first vertex of the ring that holds vertex `i`: back from `i` to
/// the vertex after the previous separator, or the column's first.
fn region_ring_start(i: u32) -> u32 {
    var first = i;
    loop {
        if (first == 0u || !region_valid(first - 1u)) {
            break;
        }
        first = first - 1u;
    }
    return first;
}

/// The vertex edge `i` runs to: the next vertex, or the ring's first past
/// the ring's last. `0xffffffff` when `i` starts no edge.
fn region_edge_end(i: u32) -> u32 {
    if (!region_valid(i)) {
        return 0xffffffffu;
    }
    if (region_valid(i + 1u)) {
        return i + 1u;
    }
    let first = region_ring_start(i);
    if (first == i) {
        return 0xffffffffu;
    }
    return first;
}

const REGION_HIDDEN: vec4<f32> = vec4<f32>(2.0, 2.0, 0.0, 1.0);

@vertex
fn vs_stencil(@builtin(instance_index) i: u32, @builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let end = region_edge_end(i);
    if (end == 0xffffffffu) {
        return REGION_HIDDEN;
    }
    let t = transform;
    let anchor = (t.ternary_lt.xy + t.ternary_lt.zw + t.ternary_r.xy) / 3.0;
    var p = anchor;
    if (vertex == 1u) {
        p = region_ndc(i);
    } else if (vertex == 2u) {
        p = region_ndc(end);
    }
    return vec4<f32>(p, 0.0, 1.0);
}

/// Colour writes are masked off: the triangle only inverts the stencil.
@fragment
fn fs_stencil() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

/// The whole ternary triangle, one triangle.
@vertex
fn vs_cover(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
    let t = transform;
    var p = t.ternary_lt.xy;
    if (vertex == 1u) {
        p = t.ternary_lt.zw;
    } else if (vertex == 2u) {
        p = t.ternary_r.xy;
    }
    return vec4<f32>(p, 0.0, 1.0);
}

@fragment
fn fs_cover() -> @location(0) vec4<f32> {
    return region.color_premul;
}
