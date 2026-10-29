// Category slot placement (SHADER_COMMON.md §2.7): where a series' member
// sits across a category slot, and where a box plot's raw point is
// jittered inside it. Spliced into the bar module, whose box plot, range
// bar and violin fragments place their members with it, and into the
// point pick's marker module, whose `PICK_VARIANT_CATEGORY_JITTER` path
// places a raw point exactly where the draw put it. Both measure in
// chart-area render pixels with y down (`category_frame_px`).

/// NDC to chart-area render pixels, y down, for a target whose
/// `Transform.pixel_to_ndc` is `pixel_to_ndc`.
fn category_frame_px(ndc: vec2<f32>, pixel_to_ndc: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((ndc.x + 1.0) / pixel_to_ndc.x, (1.0 - ndc.y) / pixel_to_ndc.y);
}

fn category_member_finite(v: f32) -> bool {
    return v == v && abs(v) <= 3.4028235e38;
}

/// Member `member` of `members` in the slot between `slot_a` and `slot_b`
/// along the category axis: the group covers the slot's centred
/// `width_ratio`, its members side by side `gap_px` apart in order.
/// `(valid, first edge, thickness, slot centre)`; `valid` is 0 for a slot
/// too narrow for the group or a non-finite edge.
fn category_member(slot_a: f32, slot_b: f32, members_in: u32, member: u32, width_ratio: f32, gap_px: f32) -> vec4<f32> {
    let members = f32(max(members_in, 1u));
    let slot = abs(slot_b - slot_a);
    let span = slot * clamp(width_ratio, 0.0, 1.0);
    let gap = max(gap_px, 0.0);
    let thickness = (span - gap * (members - 1.0)) / members;
    if (!(thickness > 0.0) || !category_member_finite(slot_a) || !category_member_finite(slot_b)) {
        return vec4<f32>(0.0);
    }
    let centre = (slot_a + slot_b) * 0.5;
    let cross0 = centre - span * 0.5 + f32(member) * (thickness + gap);
    return vec4<f32>(1.0, cross0, thickness, centre);
}

/// Where raw point `index` sits across its member of `thickness` centred
/// at `middle`: its centre line `offset` half thicknesses from the middle,
/// moved by up to `jitter` half thicknesses either way by a hash of the
/// index and the series `seed`. The same on every draw and pick, so a point
/// never moves while the chart is panned, resized or exported.
fn category_jitter_cross(middle: f32, thickness: f32, offset: f32, jitter: f32, index: u32, seed: u32) -> f32 {
    let spread = (sketch_hash01(index, seed) * 2.0 - 1.0) * jitter;
    return middle + (offset + spread) * thickness * 0.5;
}
