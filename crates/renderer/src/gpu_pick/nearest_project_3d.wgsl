// The nearest-datum pick's 3D placement (#381): datum `i` placed
// through the panel's camera with the draw's shared 3D projection
// (`common/project_3d.wgsl`: `view3d_point_valid`, `view3d_point_t`,
// `project_box`), its height read from the pool at `size_map.z` (its
// logical length `size_map.w`). The camera is the draw's premultiplied by
// the map from the cursor's pixel onto clip space, as the 3D ID pick's
// (`Renderer::pick_surface`): the cursor's pixel spans clip -1..1, so a
// clip offset is two pixels per unit.
//
// `project_3d.wgsl` reads the draw's `transform` uniform; the pick has it
// in `pick_query_params`, so it is copied into a private of that name.

var<private> transform: Transform;

fn pick_nearest_offset(i: u32) -> vec4<f32> {
    transform = pick_query_params.transform;
    if i >= pick_query_params.size_map.w {
        return vec4<f32>(0.0);
    }
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    let z = pick_read_pair(pick_query_params.size_map.z, i);
    if !view3d_point_valid(transform, x, y, z) {
        return vec4<f32>(0.0);
    }
    let clip = project_box(view3d_point_t(x, y, z));
    if !(clip.w > 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(-clip.xy / clip.w * 0.5, 1.0, 0.0);
}
