// The nearest-datum pick's 2D placement (#381): datum `i` of the
// series placed by the draw's own projection (`pick_data_px`: Cartesian,
// logarithmic, broken and probability axes and polar coordinates;
// `pick_ternary_px` on a ternary chart; a category series' raw point in
// its slot, jittered as drawn, `pick_category_jitter_px`), as its offset
// from the cursor in canvas pixels, `z` 1 when the draw places it and 0
// when it does not.

fn pick_nearest_offset(i: u32) -> vec4<f32> {
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    if pick_ternary() {
        if !pick_ternary_valid(i, x, y) {
            return vec4<f32>(0.0);
        }
        return vec4<f32>(pick_cursor_px() - pick_ternary_px(i, x, y), 1.0, 0.0);
    }
    if !pick_coordinate_is_valid(x, 0u) || !pick_coordinate_is_valid(y, 1u)
        || !polar_sample_valid(pick_query_params.transform, x, y) {
        return vec4<f32>(0.0);
    }
    var at = pick_data_px(x, y);
    if pick_query_params.category_slot.y != 0u {
        at = pick_category_jitter_px(i, x, y);
    }
    if !pick_is_finite(at.x) || !pick_is_finite(at.y) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(pick_cursor_px() - at, 1.0, 0.0);
}
