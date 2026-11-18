// Arc slots: an instance per slot along a polyline's total pixel arc.
//
// An arc-driven pass binds no vertex buffers. Each instance is one slot at
// an arc position along the polyline's whole scanned arc (the prefix
// `line_arc.wgsl` writes), and the vertex stage finds the segment holding
// that position by binary search over the prefix, fetching the segment's
// endpoints straight from the column pool. The Milkyway star pass
// (`line_columnar.wgsl::vs_stars`) and the streamline arrowheads
// (`vector/vector_input_path.wgsl`) place their instances this way; the
// slot count is the indirect draw `line_arc.wgsl::star_indirect` writes.
//
// Group 3 (CPU layout `stylized.rs::create_star_data_bind_group_layout`,
// built by `line_arc.rs::ArcScratch::build`): the arc prefix, the pool and
// `StarVsParams` (CPU twin `line_arc.rs::StarVsParams`). The prefix holds
// arc pairs (§5.6 arc pairs, `arc_pair.wgsl`).

// Arc-driven star pass data (group 3). The star pass binds NO vertex
// buffers: each instance is one candidate slot along the polyline's TOTAL
// arc, and the VS locates its segment by binary search over the arc-length
// prefix, fetching endpoints straight from the column pool. Star density is
// therefore independent of how densely the data samples the line — the old
// per-segment quad budget saturated on sparse polylines (a single long
// segment capped at 24 stars no matter the requested density).
struct StarVsParams {
    n_points: u32,
    x_base: u32,
    y_base: u32,
    // Step interpolation mode of the polyline (LINE_STEP_*), so a star's arc
    // position lands on the same stepped piece the ribbon and scan used.
    mode: u32,
};
@group(3) @binding(0) var<storage, read> star_arc: array<vec2<u32>>;
@group(3) @binding(1) var<storage, read> star_pool: array<f32>;
@group(3) @binding(2) var<uniform> star_vsp: StarVsParams;

fn star_pool_pair(base: u32, i: u32) -> vec2<f32> {
    let j = base + i * 2u;
    return vec2<f32>(star_pool[j], star_pool[j + 1u]);
}

// The segment holding arc pair `arc_here` on a prefix of `n >= 2`
// points: the greatest `g` with `star_arc[g] <= arc_here`, clamped to
// `n - 2`, so the slot lies on segment (g, g + 1). A NaN gap's segment
// adds nothing to the prefix, so a slot on a gap boundary finds the
// zero-length gap segment or the drawn one after it; a caller drops a slot
// whose segment has no arc length.
fn arc_walk_segment(n: u32, arc_here: vec2<u32>) -> u32 {
    var lo = 0u;
    var hi = n - 1u;
    loop {
        if (hi - lo <= 1u) { break; }
        let mid = (lo + hi) >> 1u;
        if (!arc_less(arc_here, star_arc[mid])) { lo = mid; } else { hi = mid; }
    }
    return lo;
}
