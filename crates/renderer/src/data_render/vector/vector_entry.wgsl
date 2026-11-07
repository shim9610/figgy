// Entry layer: the vertex and fragment stages that call the other three.
//
// `vs_main` reads sample `instance_index` (input layer), projects it
// (projection layer), places the arrow and emits one corner of its quad (glyph
// layer). `fs_main` turns the interpolated pixel position into coverage with
// the same placement, carried flat. `draw(0..ARROW_VERTICES, 0..count)`.
// The grid form's arrow pick, `pick_vector_cell`, closes the grid input
// fragment and calls `vector_arrow_style` from here.

struct VectorVsOut {
    @builtin(position) pos: vec4<f32>,
    /// Chart-area pixel position of this fragment.
    @location(0) px: vec2<f32>,
    @location(1) @interpolate(flat) tail: vec2<f32>,
    @location(2) @interpolate(flat) tip: vec2<f32>,
    /// (head length, head width) after short-arrow scaling.
    @location(3) @interpolate(flat) head: vec2<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
};

fn vector_arrow_style() -> ArrowStyle {
    return ArrowStyle(
        vector_params.width_px,
        vector_params.head_length_px,
        vector_params.head_width_px,
        vector_params.min_length_px,
        vector_params.pivot,
    );
}

/// One corner of arrow `instance_index`'s quad: the body of `vs_main`, a
/// function so the 3D pick entry (`vector_entry_3d.wgsl`) runs the same
/// placement. A projection layer places the corner (`arrow_clip`): flat at
/// depth 0, or at the arrow's depth on a 3D chart.
fn vector_vertex(vertex_index: u32, instance_index: u32) -> VectorVsOut {
    var out: VectorVsOut;
    out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    out.px = vec2<f32>(0.0, 0.0);
    out.tail = vec2<f32>(0.0, 0.0);
    out.tip = vec2<f32>(0.0, 0.0);
    out.head = vec2<f32>(0.0, 0.0);
    out.color = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    let sample = vector_sample(instance_index);
    let arrow = project_arrow(sample, vector_scale_mode(), vector_params.scale);
    if (!arrow.valid) {
        return out;
    }
    let placed = arrow_place(arrow.tail, arrow.tip, vector_arrow_style());
    if (!placed.valid) {
        return out;
    }
    let margin = vector_params.aa_px + 1.0;
    let corner = arrow_quad_corner(vertex_index, placed, vector_params.width_px, margin);
    out.pos = arrow_clip(corner, arrow);
    out.px = corner;
    out.tail = placed.tail;
    out.tip = placed.tip;
    out.head = vec2<f32>(placed.head_length_px, placed.head_width_px);
    out.color = vector_color(sample);
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> VectorVsOut {
    return vector_vertex(vertex_index, instance_index);
}

@fragment
fn fs_main(in: VectorVsOut) -> @location(0) vec4<f32> {
    var placed: ArrowPlacement;
    placed.tail = in.tail;
    placed.tip = in.tip;
    placed.head_length_px = in.head.x;
    placed.head_width_px = in.head.y;
    placed.valid = true;
    let coverage = arrow_coverage(in.px, placed, vector_params.width_px, vector_params.aa_px);
    // An uncovered fragment of the quad's margin writes nothing: on a 3D
    // chart's depth-tested draw it must not write depth either. A flat draw
    // blended nothing there, so its pixels are unchanged.
    if (coverage <= 0.0) {
        discard;
    }
    return in.color * coverage;
}

