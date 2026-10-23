// The contour label box: the frame of one label quad and the test whether a
// point lies under it (SHADER_COMMON.md §5.10). The label draw
// (`contour_label.wgsl`) places its quad in this frame and the field module's
// stroke gap (`field_columnar.wgsl::contour_label_gap_contains`) tests it, so
// the gap and the label cannot disagree about where the label is. Spliced
// after `projection.wgsl`, whose `data_to_ndc` it calls.

/// How far the label quad extends past its measured content, in pixels. The
/// content-to-texel map has slope `(w - 1) / w`, so reaching the gutter's texel
/// centre takes a little over one pixel for short labels; two leaves margin.
/// Everything past that centre is clamped to it and is transparent.
const LABEL_EDGE_PX: f32 = 2.0;
/// How far the contour stroke gap under a label extends past its measured
/// content, in pixels. Half a pixel covers the antialiased edge of both the
/// stroke and the atlas quad; it stays inside `LABEL_EDGE_PX`, so the gap lies
/// inside the quad.
const LABEL_GAP_EDGE_PX: f32 = 0.5;

/// One label's frame: its centre in NDC and its unit reading direction and
/// up direction in pixels, y up.
struct ContourLabelFrame {
    centre: vec2<f32>,
    right: vec2<f32>,
    up: vec2<f32>,
};

/// The frame of the label anchored at the data pair `(x, y)` with data-space
/// tangent `dir`, under the transform `t`.
///
/// The tangent is data-space, so it is projected the same way the point is:
/// one finite difference through the live transform — exact on a linear axis,
/// the local direction on a logarithmic one. `dir` is added to the pair's high
/// lane only: it is a delta, and the split exists to keep the *base* precise.
fn contour_label_frame(
    t: Transform,
    x: vec2<f32>,
    y: vec2<f32>,
    dir: vec2<f32>,
) -> ContourLabelFrame {
    let centre = data_to_ndc(t, x, y);
    let along = data_to_ndc(t, x + vec2<f32>(dir.x, 0.0), y + vec2<f32>(dir.y, 0.0));
    // Work in pixels with y up, so the rotation is not skewed by the panel's
    // aspect: `pixel_to_ndc` converts px to NDC, so its reciprocal goes back.
    let d_px = (along - centre) / t.pixel_to_ndc;
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
    // choice for that tie. It is also the answer matplotlib gives a vertical
    // `clabel`.
    if (right.x < 0.0 || (right.x == 0.0 && right.y < 0.0)) {
        right = -right;
    }
    return ContourLabelFrame(centre, right, vec2<f32>(-right.y, right.x));
}

/// Whether the point `ndc` lies in the box of `content_px` (the measured label
/// size, background padding included) grown by `edge_px` on every side, in
/// `frame`. `pixel_to_ndc` is the transform's pixel-to-NDC ratio.
fn contour_label_covers(
    frame: ContourLabelFrame,
    pixel_to_ndc: vec2<f32>,
    ndc: vec2<f32>,
    content_px: vec2<f32>,
    edge_px: f32,
) -> bool {
    let delta_px = (ndc - frame.centre) / pixel_to_ndc;
    let local = vec2<f32>(dot(delta_px, frame.right), dot(delta_px, frame.up));
    return abs(local.x) <= content_px.x * 0.5 + edge_px
        && abs(local.y) <= content_px.y * 0.5 + edge_px;
}
