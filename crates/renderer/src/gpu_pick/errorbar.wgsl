// Exact fragment: the error bars a marker series draws (SHADER_COMMON.md
// §5.11), measured on the quads `common/errorbar_segment.wgsl` builds for
// the draw, as part of the series' scatter primitive.

// One quad's hit distance at the cursor in the draw's y-up pixel frame: the
// rectangle between the quad's ends A and B (no extension past them), its
// half stroke either side, and under Sketch each end shifted along the
// expansion axis by its hashed offset (`disp_a`, `disp_b`), which shears
// the rectangle into the parallelogram the two triangles cover. Zero inside.
fn pick_errorbar_quad_distance(a_up: vec2<f32>, b_up: vec2<f32>, perp: vec2<f32>, half_stroke: f32,
    disp_a: f32, disp_b: f32) -> f32 {
    let delta = b_up - a_up;
    let len = length(delta);
    // The corners lie `±half_stroke` either side, whatever its sign. A
    // zero-length or zero-width quad has no area and draws nothing.
    let half = abs(half_stroke);
    if !(len > 1e-6) || !(half > 0.0) {
        return PICK_F32_MAX;
    }
    let dir = delta / len;
    let rel = pick_flip(pick_query_params.cursor_chart.xy) - a_up;
    let along = dot(rel, dir);
    let shift = mix(disp_a, disp_b, clamp(along / len, 0.0, 1.0));
    let side = dot(rel, perp) - shift;
    let d = vec2<f32>(max(-along, along - len), abs(side) - half);
    return max(length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0), 0.0);
}

// The bar's style at `point_index`, as `resolve_errorbar_mapped_style`
// resolves it, in logical pixels: the base, then the style-table slot its
// index column names, then every sparse override. Colour is not carried:
// `color_premul.w` is 1 where the resolved colour paints (alpha above
// zero) and 0 where it does not (`renderer::picking`'s error-bar rows).
fn pick_resolve_errorbar_style(point_index: u32) -> ResolvedErrorBarStyle {
    let eb = pick_query_params.errorbar;
    let rows = pick_query_params.errorbar_rows;
    let px = pick_query_params.errorbar_px;
    var out = ResolvedErrorBarStyle(vec4<f32>(0.0, 0.0, 0.0, px.w), px.x, px.y, px.z);
    if point_index < eb.w {
        let style_index = bitcast<f32>(pick_pool_words[eb.z + point_index * 2u]);
        if valid_errorbar_style_index(style_index) {
            let slot_index = u32(round(style_index));
            if slot_index < rows.y {
                let slot = pick_style_slots[rows.x + slot_index];
                out = apply_errorbar_style_slot(out, ErrorBarStyleSlot(slot.color_premul, slot.params));
            }
        }
    }
    for (var i = 0u; i < rows.w; i = i + 1u) {
        let row = pick_style_overrides[rows.z + i];
        if row.point_index == point_index + pick_query_params.stream.x {
            out = apply_errorbar_style_slot(out, ErrorBarStyleSlot(row.color_premul, row.params));
        }
    }
    return out;
}

// The error bar of point `point_index` (anchor `x`, `y`): the nearest of
// its six quads under the chart's draw style. Precise draws the quads
// (`vs_main` / `vs_mapped`), Sketch the quads with their ends displaced by
// the hashed corner wobble (`vs_sketch`), and a Milkyway jet (`vs_jet`)
// paints a gaussian beam inside its stem quad and a knot inside the disc its
// cap quad inscribes, so those envelopes pick. Constellation draws no error
// bars. A bar whose colour does not paint picks nothing. The draw masks no
// sample (a log axis clamps), so neither does the pick: a quad whose ends
// do not project to finite pixels is not drawn and does not pick.
fn pick_errorbar_candidate(point_index: u32, x: vec2<f32>, y: vec2<f32>) -> PickCandidate {
    let series_order = pick_query_params.series.y;
    let eb = pick_query_params.errorbar;
    let mode = pick_query_params.styled.x;
    let styled = pick_variant(PICK_VARIANT_STYLED_MARKER);
    if point_index >= eb.x || (styled && mode == PICK_MARKER_STAR) {
        return pick_invalid_candidate(series_order);
    }
    let bar_style = pick_resolve_errorbar_style(point_index);
    if !(bar_style.color_premul.w > 0.0) {
        return pick_invalid_candidate(series_order);
    }
    // Only the directions the series draws have columns; a disabled one is
    // never measured.
    let lanes = pick_query_params.errorbar_lanes;
    var sample = ErrorBarSample(x, y, vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0), vec2<f32>(0.0));
    if (eb.y & ERRORBAR_HAS_Y) != 0u {
        sample.err_y_lo = pick_read_pair(lanes.x, point_index);
        sample.err_y_hi = pick_read_pair(lanes.y, point_index);
    }
    if (eb.y & ERRORBAR_HAS_X) != 0u {
        sample.err_x_lo = pick_read_pair(lanes.z, point_index);
        sample.err_x_hi = pick_read_pair(lanes.w, point_index);
    }
    // The drawn sizes: the style's logical pixels at the display scale.
    let scale = pick_query_params.scatter_line.z;
    let jet = styled && mode == PICK_MARKER_PLANET;
    let stem_px = bar_style.line_width_px * scale;
    let cap_px = bar_style.cap_half_px * scale;
    var stroke = errorbar_stroke(stem_px, cap_px, bar_style.cap_width_px * scale);
    if jet {
        stroke = errorbar_jet_stroke(stem_px, cap_px);
    }
    let style_params = pick_query_params.transform.style_params[0];
    let amp = max(style_params.x, 0.0);
    let seed = (u32(style_params.z) ^ pick_query_params.styled.z) + pick_styled_index(point_index);
    var best = PICK_F32_MAX;
    for (var seg = 0u; seg < 6u; seg = seg + 1u) {
        if !errorbar_segment_enabled(eb.y, seg) {
            continue;
        }
        let g = errorbar_segment(seg, sample, stroke);
        let a_up = pick_flip(pick_project_pair(g.a.x, g.a.y)) + g.a_px;
        let b_up = pick_flip(pick_project_pair(g.b.x, g.b.y)) + g.b_px;
        if !pick_is_finite(a_up.x) || !pick_is_finite(a_up.y) || !pick_is_finite(b_up.x) || !pick_is_finite(b_up.y) {
            continue;
        }
        var d = PICK_F32_MAX;
        if jet && seg != 0u && seg != 3u {
            // A knot fades to zero at the disc its square quad inscribes.
            d = max(length(pick_flip(pick_query_params.cursor_chart.xy) - (a_up + b_up) * 0.5) - g.half_stroke, 0.0);
        } else if styled && mode == PICK_MARKER_SKETCH {
            d = pick_errorbar_quad_distance(a_up, b_up, g.perp, g.half_stroke,
                errorbar_sketch_offset(amp, seed, seg, true), errorbar_sketch_offset(amp, seed, seg, false));
        } else {
            d = pick_errorbar_quad_distance(a_up, b_up, g.perp, g.half_stroke, 0.0, 0.0);
        }
        best = min(best, d);
    }
    let hit_distance = max(best, pick_polar_clip_distance());
    let distance_sq = hit_distance * hit_distance;
    let maximum_sq = pick_query_params.chart_limits.z * pick_query_params.chart_limits.z;
    if !pick_is_finite(hit_distance) || !pick_is_finite(distance_sq) || distance_sq > maximum_sq {
        return pick_invalid_candidate(series_order);
    }
    return PickCandidate(
        1u,
        series_order,
        pick_source_row(point_index),
        PICK_KIND_ERRORBAR,
        pick_source_row(point_index),
        distance_sq,
        hit_distance,
        0u,
    );
}
