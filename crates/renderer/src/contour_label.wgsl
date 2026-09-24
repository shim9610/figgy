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
// Atlas layout: every cell has the same 2D stride and at least one transparent
// texel of gutter on every side. Level `i` maps to `(i % columns, i / columns)`
// and reports its content width `w_i` separately, so the quad keeps its measured
// size. Sampling stays between the first and last texel centres of that content;
// linear filtering therefore cannot blend in a neighbouring label.
//
// The anchor record arrives as an **instance vertex buffer**, which is the same
// 32 B layout whether `anchor_select` wrote it on the GPU or the renderer wrote a
// host's `ContourLabelConfig::anchors` override into it. One record format, one
// draw path.
//
// The screen angle is the anchor's data-space tangent projected as a finite
// difference — exact on a linear axis, the local direction on a logarithmic one
// — flipped by 180° when it would leave the text reading right-to-left.
// ───── BEGIN common block (SHADER_COMMON.md) ─────
struct Transform {
    data_min: vec2<f32>,
    data_max: vec2<f32>,
    data_min_lo: vec2<f32>,
    data_max_lo: vec2<f32>,
    scale_log: vec2<f32>,
    pixel_to_ndc: vec2<f32>,
    data_to_panel_scale: vec2<f32>,
    data_to_panel_offset: vec2<f32>,
    // Generic per-panel style parameter slots. Interpretation belongs to the
    // ACTIVE style's shader entries; the precise entries never read them.
    // sketch:        [0] = (amplitude_px, wavelength_px, seed(f32), 0)
    // milkyway:      [0] = (star_density, ribbon_width_px, ribbon_intensity,
    //                seed(f32)), [1] = (star_scale, spread_px, faint_bias, planet_rim),
    //                [2] = (structure_scale, star_brightness, 0, 0) — multiplier on the
    //                style's px-denominated structure constants (clump
    //                wavelength, binary separation); keeps the star texture
    //                resolution-invariant under DPI/export scaling.
    // constellation: [0] = (star_opacity, line_opacity, 0, 0)
    // All styles reserve [2].z for the global point-base u32 BIT PATTERN.
    // Resident draws write zero; streamed point/errorbar draws bitcast it.
    style_params: array<vec4<f32>, 3>,
};  // 112 B (vec4 array at offset 64, stride 16)

@group(0) @binding(0) var<uniform> transform: Transform;

fn styled_point_index(local_index: u32) -> u32 {
    return bitcast<u32>(transform.style_params[2].z) + local_index;
}

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = log(max(v, 1e-30)) / log(10.0);
    return mix(v, lv, is_log);
}

fn axis_pair_to_t(v: vec2<f32>, min_hi: f32, max_hi: f32, min_lo: f32, max_lo: f32, is_log: f32, panel_scale: f32, panel_offset: f32) -> f32 {
    let raw = v.x + v.y;
    let linear_num = (v.x - min_hi) + (v.y - min_lo);
    let range = (max_hi - min_hi) + (max_lo - min_lo);
    let log_num = (maybe_log(raw, is_log) - min_hi) - min_lo;
    let data_t = mix(linear_num / range, log_num / range, is_log);
    return panel_offset + data_t * panel_scale;
}

fn data_to_ndc(xv: vec2<f32>, yv: vec2<f32>) -> vec2<f32> {
    let tx = axis_pair_to_t(xv, transform.data_min.x, transform.data_max.x, transform.data_min_lo.x, transform.data_max_lo.x, transform.scale_log.x, transform.data_to_panel_scale.x, transform.data_to_panel_offset.x);
    let ty = axis_pair_to_t(yv, transform.data_min.y, transform.data_max.y, transform.data_min_lo.y, transform.data_max_lo.y, transform.scale_log.y, transform.data_to_panel_scale.y, transform.data_to_panel_offset.y);
    return vec2<f32>(tx, ty) * 2.0 - 1.0;
}
// ───── END common block ─────
/// CPU twin: `gpu_contour::LabelParamsGpu` (32 B, 16 B uniform alignment).
struct LabelParams {
    /// Atlas dimensions in texels, at the render scale it was baked for.
    atlas_w: f32,
    atlas_h: f32,
    /// Uniform grid-cell stride in texels, including both gutters.
    cell_stride_w: f32,
    cell_stride_h: f32,
    columns: u32,
    rows: u32,
    /// Transparent texels on each side of the cell content.
    gutter: f32,
    /// Levels the atlas actually holds. An anchor naming a level beyond this is
    /// stale rather than fatal: its quad collapses and nothing draws.
    level_count: u32,
};

@group(1) @binding(0) var<uniform> lp: LabelParams;
@group(1) @binding(2) var atlas: texture_2d<f32>;
@group(1) @binding(3) var atlas_samp: sampler;

struct LabelOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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
    // Corner offsets in cell fractions, centred on the anchor.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(-0.5, 0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    let corner = corners[vid];

    var out: LabelOut;
    // A level the atlas does not hold, or an empty cell, draws nothing. Both
    // collapse to a degenerate quad rather than clamping to a neighbour's row,
    // which would put the wrong number on the line.
    if (a_level >= lp.level_count || lp.atlas_w <= 0.0 || lp.atlas_h <= 0.0 ||
        lp.columns == 0u || lp.rows == 0u) {
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.uv = vec2<f32>(0.0, 0.0);
        return out;
    }
    let w = a_width_px;
    let cell_h = lp.cell_stride_h - 2.0 * lp.gutter;
    let column = a_level % lp.columns;
    let row = a_level / lp.columns;
    if (w <= 0.0 || cell_h <= 0.0 || row >= lp.rows) {
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.uv = vec2<f32>(0.0, 0.0);
        return out;
    }

    let centre = data_to_ndc(a_x, a_y);
    // The tangent is data-space, so it is projected the same way the point is:
    // one finite difference through the live transform. `a_dir` is added to the
    // pair's high lane only — it is a delta, and the split exists to keep the
    // *base* precise.
    let along = data_to_ndc(a_x + vec2<f32>(a_dir.x, 0.0), a_y + vec2<f32>(a_dir.y, 0.0));
    // Work in pixels with y up, so the rotation is not skewed by the panel's
    // aspect: `pixel_to_ndc` converts px to NDC, so its reciprocal goes back.
    let d_px = (along - centre) / transform.pixel_to_ndc;
    var right = vec2<f32>(1.0, 0.0);
    if (dot(d_px, d_px) > 0.0) {
        right = normalize(d_px);
    }
    // Keep the text upright. A tangent pointing left would draw the number
    // mirrored end-to-end; 180° is the same line and reads correctly.
    //
    // The `y` clause is the vertical-tangent tie-break. An exactly vertical
    // tangent has `right.x == 0`, so the sign test alone would leave the label
    // facing either way after projection. Bottom-to-top is the deterministic
    // choice for that tie.
    // It is also the answer matplotlib
    // gives a vertical `clabel`.
    if (right.x < 0.0 || (right.x == 0.0 && right.y < 0.0)) {
        right = -right;
    }
    let up = vec2<f32>(-right.y, right.x);

    let local = corner * vec2<f32>(w, cell_h);
    let offset_px = right * local.x + up * local.y;
    out.pos = vec4<f32>(centre + offset_px * transform.pixel_to_ndc, 0.0, 1.0);

    // `f` is the corner in [0, 1] over the measured content. Sampling starts and
    // ends at texel centres, then clamps to this cell's inner edge. The gutter
    // remains outside the sampled interval and absorbs linear-filter support.
    let f = corner + vec2<f32>(0.5, 0.5);
    let cell_origin = vec2<f32>(
        f32(column) * lp.cell_stride_w,
        f32(row) * lp.cell_stride_h,
    );
    let content_origin = cell_origin + vec2<f32>(lp.gutter);
    let sample_lo = content_origin + vec2<f32>(0.5);
    let cell_sample_hi = cell_origin
        + vec2<f32>(lp.cell_stride_w, lp.cell_stride_h)
        - vec2<f32>(lp.gutter + 0.5);
    let content_sample_hi = content_origin
        + max(vec2<f32>(w, cell_h) - vec2<f32>(0.5), vec2<f32>(0.5));
    let sample_hi = min(content_sample_hi, cell_sample_hi);
    // Texture v grows downward while `local.y` grows upward.
    let sample_px = clamp(
        mix(sample_lo, sample_hi, vec2<f32>(f.x, 1.0 - f.y)),
        sample_lo,
        sample_hi,
    );
    out.uv = sample_px / vec2<f32>(lp.atlas_w, lp.atlas_h);
    return out;
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
