@group(1) @binding(1) var<uniform> data_pick_query: DataPickQuery;
@group(1) @binding(3) var<storage, read_write> data_pick_output: DataPickCandidate;

/// The nearest drawn arrow of a grid vector series to the cursor, as the
/// matrix cell it is drawn for: every instance `vs_main` would draw, placed
/// as it places it, measured with `arrow_distance` in the draw's pixel frame
/// (`ndc_to_px` of the cursor). An arrow that is not drawn does not pick.
/// Arrows paint in instance order, so an exact tie goes to the later one.
@compute @workgroup_size(1)
fn pick_vector_cell() {
    var best = DataPickCandidate(0u, data_pick_query.data.y, 0u, 0u, 0u, 0u, 0.0, 0u);
    let cursor = ndc_to_px(data_pick_query.cursor_ndc_t.xy);
    let max_distance = data_pick_query.limits.x;
    let rows = vector_params.rows;
    if (rows == 0u || !arrow_finite(cursor.x) || !arrow_finite(cursor.y)
        || !arrow_finite(max_distance) || max_distance < 0.0) {
        data_pick_output = best;
        return;
    }
    let style = vector_arrow_style();
    let mode = vector_scale_mode();
    var i = 0u;
    loop {
        if (i >= vector_params.count) {
            break;
        }
        let arrow = project_arrow(vector_sample(i), mode, vector_params.scale);
        if (arrow.valid) {
            let placed = arrow_place(arrow.tail, arrow.tip, style);
            if (placed.valid) {
                let hit = max(arrow_distance(cursor, placed, vector_params.width_px), 0.0);
                if (arrow_finite(hit) && hit <= max_distance
                    && (best.valid == 0u || hit <= best.distance_px)) {
                    let col = i / rows;
                    let row = i % rows;
                    var x_index = col;
                    var y_index = row;
                    if (vector_flag(VECTOR_FLAG_COLUMNS_ARE_Y)) {
                        x_index = row;
                        y_index = col;
                    }
                    best = DataPickCandidate(
                        1u,
                        data_pick_query.data.y,
                        DATA_PICK_KIND_MATRIX_CELL,
                        x_index,
                        y_index,
                        0u,
                        hit,
                        i,
                    );
                }
            }
        }
        i = i + 1u;
    }
    data_pick_output = best;
}
