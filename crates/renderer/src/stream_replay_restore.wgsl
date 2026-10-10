@group(0) @binding(3) var<storage, read> tiles: array<u32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}

// Exactly ONE image owns every sample. Do not alpha-over the approximate and
// exact data images: translucent crossings and antialiased edges would darken.
fn restore_pixel(p: vec2<i32>, sample: i32) -> vec4<f32> {
    let size = textureDimensions(preview);
    let tile_width = (size.x + 7u) / 8u;
    // Only replace pixels reached by exact data. Dilating this mask, even by
    // one pixel for AA, erases the unfinished continuation of a thin line.
    let u = vec2<u32>(p);
    let tile = u / vec2(8u);
    let bit = (u.y % 8u) * 8u + u.x % 8u;
    let index = (tile.y * tile_width + tile.x) * 2u + bit / 32u;
    if (tiles[index] & (1u << (bit % 32u))) != 0u { discard; }
    return textureLoad(preview, p, sample);
}
// The fragment entry point is appended for one sample or @sample_index MSAA.
