// Single-lane column expansion (#317).
//
// A single-lane column (an f32 source, or a ragged column's offsets and a
// triangle column's indices) is staged with its first lane only, 4 bytes per
// value. This pass writes it into the pool's `(hi, lo)` layout: value `i`'s
// first lane is `src[i]` and its second lane is 0, so the pool holds exactly
// the bytes a pair upload writes. The pool layout and every shader that
// reads it are unchanged.
//
// `src` is bound as the column's compact values and `dst` as the column's
// pool region, both windows starting at the column, so `count` is
// `arrayLength(&src)` (the region's value count, its padding included, which
// the staging holds as zeros) and the region starts at lane 0. The dispatch
// is a 2D grid of 64-wide workgroups, `grid_x` of them per row, so any count
// fits under `max_compute_workgroups_per_dimension`.

@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(64)
fn expand_single(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) grid: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let count = arrayLength(&src);
    let i = (group.y * grid.x + group.x) * 64u + local;
    if i >= count {
        return;
    }
    dst[2u * i] = src[i];
    dst[2u * i + 1u] = 0u;
}
