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
// The closing edge of a ring runs from its last vertex to its first. The
// first vertex's row is read from `region_start`, which the ring-start scan
// below writes before the draw (#49), so no invocation walks its ring: every
// vertex costs a bounded number of reads whatever the ring's length.
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
/// Per row: the row of its ring's first vertex (`ring_scan_*` below).
@group(1) @binding(2) var<storage, read> region_start: array<u32>;

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

/// The first vertex of the ring that holds vertex `i`: the vertex after the
/// previous invalid row, or the column's first, as the scan wrote it.
fn region_ring_start(i: u32) -> u32 {
    return region_start[i];
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

// ── Ring-start scan (compute, group 2) ─────────────────────────────────────
//
// For every row `i`, the row of the first vertex of the ring that holds it:
// the row after the nearest invalid row at or before `i`, or 0. That is an
// inclusive running maximum of `i + 1` over the invalid rows, computed in
// the arc scan's pattern (`line_arc.wgsl`): `ring_init` writes each row's
// candidate, `ring_scan_block` scans each block of 256 in workgroup memory
// and writes the block's maximum to the next level, the levels are scanned
// in turn, and `ring_apply` folds each scanned level into the one below.
// Every invocation does a bounded amount of work; the depth is log256 of the
// row count. A dispatch is two-dimensional, so a level may hold more blocks
// than one dimension allows. The scan reads only the pool and writes no
// value the user reads (rule 1).
//
// CPU twin: `data_render/ternary_region.rs::RingScanParamsGpu` (16 B).
struct RingScanParams {
    /// Entries in `ring_dst` at this level: rows at level 0, blocks above.
    len: u32,
    /// f32-lane bases of the x, y and c columns (`ring_init` only).
    x_base: u32,
    y_base: u32,
    c_base: u32,
};

@group(2) @binding(0) var<storage, read> ring_pool: array<f32>;
@group(2) @binding(1) var<storage, read_write> ring_dst: array<u32>;
@group(2) @binding(2) var<storage, read_write> ring_sums: array<u32>;
@group(2) @binding(3) var<uniform> ring_params: RingScanParams;

const RING_WG: u32 = 256u;
var<workgroup> ring_shared: array<u32, 256>;

// Windows DX12: bounded per-invocation zeroing avoids aggregate DXIL stores.
// Keep this override paired with gpu_compile::workgroup_options.
override FIGGY_EXPLICIT_WORKGROUP_ZERO: bool = false;

fn ring_zero_workgroup(local_index: u32) {
    if FIGGY_EXPLICIT_WORKGROUP_ZERO {
        ring_shared[local_index] = 0u;
        workgroupBarrier();
    }
}

/// The block a workgroup scans: dispatches are two-dimensional.
fn ring_block(wid: vec3<u32>, groups: vec3<u32>) -> u32 {
    return wid.y * groups.x + wid.x;
}

fn ring_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(ring_pool[j], ring_pool[j + 1u]);
}

/// Level 0: row `i`'s candidate, `i + 1` on an invalid row, else 0.
@compute @workgroup_size(256)
fn ring_init(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let i = ring_block(wid, groups) * RING_WG + lid;
    if (i >= ring_params.len) {
        return;
    }
    let valid = ternary_sample_valid(
        ring_pair(ring_params.x_base, i),
        ring_pair(ring_params.y_base, i),
        ring_pair(ring_params.c_base, i),
    );
    ring_dst[i] = select(i + 1u, 0u, valid);
}

/// The running maximum within each block of 256, in place; the block's
/// maximum to `ring_sums`.
@compute @workgroup_size(256)
fn ring_scan_block(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    ring_zero_workgroup(lid);
    let block = ring_block(wid, groups);
    let i = block * RING_WG + lid;
    var v = 0u;
    if (i < ring_params.len) {
        v = ring_dst[i];
    }
    ring_shared[lid] = v;
    workgroupBarrier();
    var offset = 1u;
    loop {
        if (offset >= RING_WG) {
            break;
        }
        var other = 0u;
        if (lid >= offset) {
            other = ring_shared[lid - offset];
        }
        workgroupBarrier();
        ring_shared[lid] = max(ring_shared[lid], other);
        workgroupBarrier();
        offset = offset << 1u;
    }
    if (i < ring_params.len) {
        ring_dst[i] = ring_shared[lid];
    }
    if (lid == RING_WG - 1u && block * RING_WG < ring_params.len) {
        ring_sums[block] = ring_shared[lid];
    }
}

/// Fold the scanned level above into this one: every block after the first
/// takes the maximum of the blocks before it.
@compute @workgroup_size(256)
fn ring_apply(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let block = ring_block(wid, groups);
    let i = block * RING_WG + lid;
    if (block == 0u || i >= ring_params.len) {
        return;
    }
    ring_dst[i] = max(ring_dst[i], ring_sums[block - 1u]);
}
