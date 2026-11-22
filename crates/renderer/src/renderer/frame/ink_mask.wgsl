// The ink mask of one panel (#381): which series owns each pixel of the
// panel's data layer. Each series is drawn on its own into a transparent
// panel-sized target; this pass then reads that target texel by texel and
// writes the series' slot (its instance index, from 1) where any channel is
// above 1/255 (a premultiplied body has its alpha there, an additive style
// its colour), and leaves the texel alone elsewhere. The series are
// resolved in drawing order, so the slot left is the topmost owner; 0 is
// no ink. No colour leaves the pass.

@group(0) @binding(0) var data_layer: texture_2d<f32>;

struct InkVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) slot: u32,
}

@vertex
fn ink_mask_vs(
    @builtin(vertex_index) index: u32,
    @builtin(instance_index) slot: u32,
) -> InkVertex {
    // One triangle covering the target.
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return InkVertex(vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0), slot);
}

@fragment
fn ink_mask_fs(vertex: InkVertex) -> @location(0) u32 {
    let texel = textureLoad(data_layer, vec2<i32>(vertex.position.xy), 0);
    if max(max(texel.r, texel.g), max(texel.b, texel.a)) <= 1.0 / 255.0 {
        discard;
    }
    return vertex.slot;
}
