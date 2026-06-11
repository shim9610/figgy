// Fullscreen textured-quad shader.
//
// Uses the single-triangle fullscreen trick: three NDC vertices at
// (-1,-1), (3,-1), (-1,3) cover [-1,1]^2 after the rasterizer clips the
// overhang. No vertex/index buffer; the pipeline is driven by `draw(3, 1)`.
// Avoids the diagonal seam cache miss that two triangles produce.
//
// UV mapping: wgpu textures use top-left origin (Y down), NDC uses bottom-
// left origin (Y up), so pos.y and uv.y are inverted.
//   idx 0 : pos (-1, -1)  →  uv (0, 1)   # bottom-left
//   idx 1 : pos ( 3, -1)  →  uv (2, 1)   # right overhang, clipped
//   idx 2 : pos (-1,  3)  →  uv (0, -1)  # top overhang, clipped
// After clipping, uv interpolates linearly across exactly [0,1]^2.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOut {
    // Constant arrays indexed by vertex_index.
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );

    var out: VsOut;
    out.pos = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = uvs[idx];
    return out;
}

@group(0) @binding(0) var src_texture: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(src_texture, src_sampler, in.uv);
}
