// Entry layer, 3D ID pick (#211): a 3D chart picks through its 1×1 ID
// render (`surface_pick.rs`). Each arrow draws through the same
// `vector_vertex` placement as `vs_main`, with the depth test, and
// `fs_3d_pick` writes the series' pick id, kind 1 (a point) and the row
// where the arrow covers at least half the pixel: the shaft and the head
// pick where they paint, and an arrow behind a surface never answers.

struct VectorPickOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) px: vec2<f32>,
    @location(1) @interpolate(flat) tail: vec2<f32>,
    @location(2) @interpolate(flat) tip: vec2<f32>,
    @location(3) @interpolate(flat) head: vec2<f32>,
    @location(4) @interpolate(flat) index: u32,
};

@vertex
fn vs_3d_pick(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> VectorPickOut {
    let v = vector_vertex(vertex_index, instance_index);
    var out: VectorPickOut;
    out.pos = v.pos;
    out.px = v.px;
    out.tail = v.tail;
    out.tip = v.tip;
    out.head = v.head;
    out.index = instance_index;
    return out;
}

@fragment
fn fs_3d_pick(in: VectorPickOut) -> @location(0) vec4<u32> {
    var placed: ArrowPlacement;
    placed.tail = in.tail;
    placed.tip = in.tip;
    placed.head_length_px = in.head.x;
    placed.head_width_px = in.head.y;
    placed.valid = true;
    if (arrow_coverage(in.px, placed, vector_params.width_px, vector_params.aa_px) < 0.5) {
        discard;
    }
    return vec4<u32>(view3d_pick_id(), 1u, in.index, 0u);
}
