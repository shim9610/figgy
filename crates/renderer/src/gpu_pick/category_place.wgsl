// A category series' jittered raw point placed in the pick's pixel frame:
// shared by the exact marker pick and the nearest-datum pick (#381), so
// both place a box plot's or a violin's raw point where `vs_box_points`
// draws it. Needs `category_jitter.wgsl` and `sketch_noise.wgsl`.

// A box plot's raw point in the draw's pixel frame (y up): its member's
// middle in the category slot of its position, jittered across it by
// `category_jitter_cross` as `vs_box_points` draws it, at its value along
// the other axis. The slot and the jitter are measured in chart-area render
// pixels with y down, as the bar module measures them.
fn pick_category_jitter_px(point_index: u32, x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
    let tr = pick_query_params.transform;
    let slot = pick_query_params.category_slot;
    let placement = pick_query_params.category;
    let horizontal = slot.z != 0u;
    let position = select(x, y, horizontal);
    let value = select(y, x, horizontal);
    let low = vec2<f32>(position.x - 0.5, position.y);
    let high = vec2<f32>(position.x + 0.5, position.y);
    // The slot's low and high edges and the point's position, from one
    // projection call in a loop: a software GPU compiler pays for every
    // inlined copy (#363).
    var frames = array<vec2<f32>, 3>();
    for (var i = 0u; i < 3u; i = i + 1u) {
        let at = select(select(low, high, i == 1u), position, i == 2u);
        frames[i] = category_frame_px(
            data_to_ndc(tr, select(at, value, horizontal), select(value, at, horizontal)),
            tr.pixel_to_ndc,
        );
    }
    let slot_a = select(frames[0].x, frames[0].y, horizontal);
    let slot_b = select(frames[1].x, frames[1].y, horizontal);
    let along = select(frames[2].y, frames[2].x, horizontal);
    let m = category_member(slot_a, slot_b, slot.y, slot.x, placement.x, placement.y * pick_query_params.scatter_line.z);
    if (m.x < 0.5) {
        return vec2<f32>(PICK_F32_MAX);
    }
    let middle = m.y + m.z * 0.5;
    let cross = category_jitter_cross(middle, m.z, placement.w, placement.z, pick_source_row(point_index), slot.w, horizontal);
    let frame = select(vec2<f32>(cross, along), vec2<f32>(along, cross), horizontal);
    // Chart-area render pixels (y down) to the draw's frame (y up).
    return vec2<f32>(frame.x - 1.0 / tr.pixel_to_ndc.x, 1.0 / tr.pixel_to_ndc.y - frame.y);
}
