// The on-demand pick ID pass (#381 prototype): the series of one panel
// drawn into a 1×1 `Rgba32Uint` target with a 1×1 depth target, the
// cursor's pixel, only when a pick is requested. The draw pipelines are not
// touched.
//
// Each primitive (a point, or a segment from a point to the next) is one
// vertex of a point list at the centre of the target, so it covers the
// target's one pixel. Its vertex stage applies the
// point picker's per-axis gates (`gate_entries.wgsl`) to a gated series and
// culls the primitive when they reject it; its fragment, the cursor's
// pixel, measures the primitive with the exact module's own candidate
// functions (`pick_scatter_candidate`, `pick_line_candidate`) and writes
// the hit through the pick ID output (`pick_id_output.wgsl`), whose depth
// rule resolves the hits: covering beats near, the topmost covering hit
// wins, then the nearest near one.
//
// The module is the exact module of one column kind with these entries in
// place of the exact compute entry, and the default `PICK_VARIANT` (every
// path), so one pipeline serves every style of that kind.

// The centre of the 1×1 target, where a point-list vertex covers its pixel.
const PICK_ID_CENTRE: vec2<f32> = vec2<f32>(0.0, 0.0);

// Whether a series draws something the per-axis gates do not bound, so
// the point picker scans it directly whatever its size
// (`prepare_series_buffer`): point vectors, error bars, stems, an area
// fill, a ternary or region series, a closed line.
fn pick_id_ungated() -> bool {
    let q = pick_query_params;
    return pick_ternary() || pick_region() || (q.style.x & PICK_FLAG_VECTOR) != 0u
        || q.errorbar.x != 0u || q.stem_info.x != 0u || q.area.y != 0u || q.line.z != 0u;
}

// Whether the gates would keep point `i` as a marker: every point of an
// ungated series; otherwise the X and Y gates' tests. They are
// conservative, so they also apply to a series small enough for the exact
// pass to scan directly.
fn pick_id_point_gate(i: u32, x: vec2<f32>, y: vec2<f32>) -> bool {
    if pick_id_ungated() {
        return true;
    }
    if !pick_pair_is_valid(x) || !pick_pair_is_valid(y) {
        return false;
    }
    if !pick_scatter_projected_may_hit(pick_project_axis_pair(x, 0u), 0u)
        || !pick_scatter_projected_may_hit(pick_project_axis_pair(y, 1u), 1u) {
        return false;
    }
    return !pick_polar() || pick_polar_scatter_may_hit(i, y);
}

@vertex
fn pick_id_vs_points(@builtin(vertex_index) i: u32) -> PickIdVarying {
    let series = pick_query_params.series.y;
    if (pick_query_params.style.x & PICK_FLAG_SCATTER) == 0u
        || i >= pick_query_params.data.x
        || i >= pick_query_params.stream.y {
        return pick_id_culled(series, i);
    }
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    if !pick_id_point_gate(i, x, y) {
        return pick_id_culled(series, i);
    }
    return pick_id_varying(series, i, PICK_ID_CENTRE);
}

fn pick_id_write(candidate: PickCandidate) -> PickIdOutput {
    return pick_id_output(
        candidate.primitive_kind,
        candidate.series_order,
        candidate.point_index,
        candidate.primitive_index,
        candidate.distance_px,
        pick_query_params.chart_limits.z,
        candidate.primitive_kind == PICK_KIND_FILL,
    );
}

@fragment
fn pick_id_fs_points(input: PickIdVarying) -> PickIdOutput {
    let i = input.pick_id.y;
    let x = pick_read_pair(pick_query_params.data.y, i);
    let y = pick_read_pair(pick_query_params.data.z, i);
    let candidate = pick_scatter_candidate(i, x, y);
    if candidate.valid == 0u {
        discard;
    }
    return pick_id_write(candidate);
}
