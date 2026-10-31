// Contour label render — one quad per anchor, textured with a pre-baked label.
//
// The labels' *content* is known before the frame starts: a label reads
// `levels[i]`, and the levels are config. So the CPU bakes each level's string
// into one cell of a 2D label atlas with the ordinary text stack
// (`measure_plain_text` / `draw_plain_text` / `FontPolicy`), colour and
// background included, and this shader only has to *place* that image.
//
// **A label image atlas, not a glyph atlas.** Whole strings are baked, so
// per-character font fallback, the sketch handwriting face and `FontPolicy` all
// stay in the single existing text stack — there is no second one. That is the
// whole reason the pipeline can be one-directional: no GPU→CPU readback, no
// ticket, and everything finishes inside the frame.
//
// The quad and the atlas sampling are the shared label quad
// (`common/label_quad.wgsl`), which the value labels place their quads with
// too.
//
// The anchor record arrives as an **instance vertex buffer**, which is the same
// 32 B layout whether `anchor_select` wrote it on the GPU or the renderer wrote a
// host's `ContourLabelConfig::anchors` override into it. One record format, one
// draw path.
//
// The screen angle is the anchor's data-space tangent projected as a finite
// difference — exact on a linear axis, the local direction on a logarithmic one
// — flipped by 180° when it would leave the text reading right-to-left. The
// frame and `LABEL_EDGE_PX` are the fragment `common/contour_label_box.wgsl`.

@group(1) @binding(0) var<uniform> lp: LabelParams;

@group(1) @binding(2) var atlas: texture_2d<f32>;
@group(1) @binding(3) var atlas_samp: sampler;

/// One quad per anchor. CPU twin of the vertex count: `gpu_contour::LABEL_VERTICES`.
///
/// Attributes 0..4 are the anchor record's fields in order. The final width is
/// copied from the atlas table when the anchor is selected, so this quad and the
/// contour-gap pass consume the same exact dimension.
@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @location(0) a_x: vec2<f32>,
    @location(1) a_y: vec2<f32>,
    @location(2) a_dir: vec2<f32>,
    @location(3) a_level: u32,
    @location(4) a_width_px: f32,
) -> LabelOut {
    // The frame (centre, reading direction, upright turn) is the label box's
    // (`common/contour_label_box.wgsl`), which the contour gap tests too.
    let frame = contour_label_frame(transform, a_x, a_y, a_dir);
    return label_quad(transform, lp, vid, frame, a_level, a_width_px, false);
}

@fragment
fn fs_main(in: LabelOut) -> @location(0) vec4<f32> {
    // textureSampleLevel, not textureSample: every figgy texture is single-mip
    // and a bare sample is rejected in non-uniform control flow by browser WGSL
    // (`no_bare_texture_sample_in_any_shader` enforces this).
    //
    // The atlas comes from tiny-skia, which is premultiplied, and the pipeline
    // blends PREMULTIPLIED_ALPHA — so the baked colour and the baked background
    // arrive exactly as the CPU drew them.
    return textureSampleLevel(atlas, atlas_samp, in.uv, 0.0);
}
