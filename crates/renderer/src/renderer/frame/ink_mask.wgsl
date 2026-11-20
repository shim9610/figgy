// The ink mask of one panel (#381): which pixels of the panel's data layer
// carry ink. The data layer is drawn into a transparent panel-sized target
// first; this pass reads it texel by texel and writes 1 where any channel
// is above 1/255 (a premultiplied body has its alpha there, an additive
// style its colour), 0 elsewhere. Coverage only: no colour leaves the pass.

@group(0) @binding(0) var data_layer: texture_2d<f32>;

@vertex
fn ink_mask_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the target.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn ink_mask_fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = textureLoad(data_layer, vec2<i32>(position.xy), 0);
    let ink = max(max(texel.r, texel.g), max(texel.b, texel.a)) > 1.0 / 255.0;
    return vec4<f32>(select(0.0, 1.0, ink), 0.0, 0.0, 1.0);
}
