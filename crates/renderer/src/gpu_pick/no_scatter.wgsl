
// Pick fragment for an exact variant without a scatter primitive: the
// entry's scatter mask is never set for such a series.
fn pick_scatter_candidate(
    point_index: u32,
    x: vec2<f32>,
    y: vec2<f32>,
) -> PickCandidate {
    return pick_invalid_candidate(pick_query_params.series.y);
}
