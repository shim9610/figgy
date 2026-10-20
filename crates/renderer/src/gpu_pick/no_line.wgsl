
// Pick fragment for an exact variant without a line primitive: the entry's
// line mask is never set for such a series.
fn pick_line_candidate(
    segment_index: u32,
    ax: vec2<f32>,
    ay: vec2<f32>,
    bx: vec2<f32>,
    by: vec2<f32>,
) -> PickCandidate {
    return pick_invalid_candidate(pick_query_params.series.y);
}
