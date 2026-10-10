// Texture declarations and SAMPLE_COUNT are supplied for the surface's MSAA
// count. Each workgroup owns one 8x8 tile; no readback or per-source metadata.
@group(0) @binding(3) var<storage, read_write> tiles: array<u32>;
var<workgroup> changed: array<atomic<u32>, 2>;

@compute @workgroup_size(8, 8)
fn mask(@builtin(global_invocation_id) p: vec3<u32>,
        @builtin(local_invocation_index) lane: u32,
        @builtin(workgroup_id) tile: vec3<u32>) {
    if lane < 2u { atomicStore(&changed[lane], 0u); }
    workgroupBarrier();
    let size = textureDimensions(exact);
    if all(p.xy < size) {
        for (var sample = 0u; sample < SAMPLE_COUNT; sample++) {
            let a = textureLoad(exact, vec2<i32>(p.xy), i32(sample));
            let b = textureLoad(baseline, vec2<i32>(p.xy), i32(sample));
            if any(a != b) { atomicOr(&changed[lane / 32u], 1u << (lane % 32u)); }
        }
    }
    workgroupBarrier();
    if lane < 2u {
        let width = (size.x + 7u) / 8u;
        let index = (tile.y * width + tile.x) * 2u + lane;
        // Once revealed, a pixel never falls back to the approximate preview,
        // even when a later opaque primitive restores the background colour.
        tiles[index] = tiles[index] | atomicLoad(&changed[lane]);
    }
}
