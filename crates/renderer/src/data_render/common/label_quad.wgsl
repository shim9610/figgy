// The label quad: one textured quad per label, placed in a label frame and
// sampled from one cell of a CPU-baked label atlas (SHADER_COMMON.md §5.16).
// The contour labels (`contour_label.wgsl`) and the value labels of the
// category bar, box plot and radar series (`value_label.wgsl`) place their
// quads with it, so both read the atlas the same way. Spliced after
// `contour_label_box.wgsl`, whose frame and `LABEL_EDGE_PX` it uses. It
// declares no binding: each module binds its own atlas.
//
// Atlas layout: every cell has the same 2D stride and at least one
// transparent texel of gutter on every side. Label `i` maps to
// `(i % columns, i / columns)` and reports its content width `w_i`
// separately, so the quad keeps its measured size. Sampling stays between the
// first and last texel centres of the cell; linear filtering therefore cannot
// blend in a neighbouring label.

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
    /// Labels the atlas actually holds. A label index beyond this is stale
    /// rather than fatal: its quad collapses and nothing draws.
    level_count: u32,
};

struct LabelOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

/// A quad that draws nothing.
fn label_quad_hidden() -> LabelOut {
    var out: LabelOut;
    out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0, 0.0);
    return out;
}

/// Corner `vid` of the quad of atlas cell `a_level`, `a_width_px` texels of
/// content wide, centred on `frame` under the transform `t`.
///
/// `texel_exact` maps one content pixel to one texel: an upright label whose
/// content origin sits on the pixel grid then samples every texel at its
/// centre, as the CPU drew it. Its quad reaches the cell's gutter and no
/// further, so it needs no clamp: a clamp at the corners would be
/// interpolated across the quad and compress the content. A rotated label
/// (a contour's) maps its content to the texel centres instead,
/// `(w - 1) / w` texels per pixel, on a quad `LABEL_EDGE_PX` past it.
fn label_quad(
    t: Transform,
    lp: LabelParams,
    vid: u32,
    frame: ContourLabelFrame,
    a_level: u32,
    a_width_px: f32,
    texel_exact: bool,
) -> LabelOut {
    // Corner offsets in cell fractions, centred on the frame.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(-0.5, 0.5),
        vec2<f32>(0.5, -0.5),
        vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, 0.5),
    );
    let corner = corners[vid % 6u];

    // A label the atlas does not hold, or an empty cell, draws nothing. Both
    // collapse to a degenerate quad rather than clamping to a neighbour's row,
    // which would put the wrong string on the label.
    if (a_level >= lp.level_count || lp.atlas_w <= 0.0 || lp.atlas_h <= 0.0 ||
        lp.columns == 0u || lp.rows == 0u) {
        return label_quad_hidden();
    }
    let w = a_width_px;
    let cell_h = lp.cell_stride_h - 2.0 * lp.gutter;
    let column = a_level % lp.columns;
    let row = a_level / lp.columns;
    if (w <= 0.0 || cell_h <= 0.0 || row >= lp.rows) {
        return label_quad_hidden();
    }

    var out: LabelOut;
    // The quad reaches `LABEL_EDGE_PX` past the measured content on every
    // side. A baked background box fills its content to the edge, so a quad
    // that ended there would make the box edge the rasterized quad edge:
    // stair-stepped once the label is rotated. Past the content the sample
    // below runs into the transparent gutter, so the texture filter fades the
    // box edge over one texel, exactly as it does for the glyphs inside it.
    let content = vec2<f32>(w, cell_h);
    let edge = select(LABEL_EDGE_PX, lp.gutter, texel_exact);
    let local = corner * (content + vec2<f32>(2.0 * edge));
    let offset_px = frame.right * local.x + frame.up * local.y;
    out.pos = vec4<f32>(frame.centre + offset_px * t.pixel_to_ndc, 0.0, 1.0);

    // `f` is the position in [0, 1] over the measured content, and runs past
    // that interval on the extended edge. Content maps to texel centres as it
    // always has; the same affine map continues into the gutter, and the clamp
    // below stops at this cell's outermost texel centres, which are gutter and
    // therefore transparent. Linear filtering never reaches a neighbour.
    let f = local / content + vec2<f32>(0.5, 0.5);
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
    let cell_edge_lo = cell_origin + vec2<f32>(0.5);
    // Texture v grows downward while `local.y` grows upward.
    if (texel_exact) {
        // The corners land on the cell's outer edges: every pixel centre
        // samples a texel centre of this cell, its gutter included.
        out.uv = (content_origin + content * vec2<f32>(f.x, 1.0 - f.y))
            / vec2<f32>(lp.atlas_w, lp.atlas_h);
        return out;
    }
    let sample_px = clamp(
        mix(sample_lo, sample_hi, vec2<f32>(f.x, 1.0 - f.y)),
        cell_edge_lo,
        cell_sample_hi + vec2<f32>(lp.gutter),
    );
    out.uv = sample_px / vec2<f32>(lp.atlas_w, lp.atlas_h);
    return out;
}
