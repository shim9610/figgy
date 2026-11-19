//! The pick constants, the query parameters and the glyph extents.

use super::*;

/// Logical point/segment indices represented by one gate-mask word.
pub(in crate::gpu_pick) const GPU_PICK_GATE_WORD_POINTS: u32 = 32;
/// Compute workgroup width used by gating, exact testing, and reduction.
pub(in crate::gpu_pick) const GPU_PICK_WORKGROUP_SIZE: u32 = 64;
/// Up to 32 workgroups scan directly; larger series use the X/Y gates first.
pub(in crate::gpu_pick) const GPU_PICK_DIRECT_SCAN_POINTS: u32 =
    GPU_PICK_GATE_WORD_POINTS * GPU_PICK_WORKGROUP_SIZE * 32;

/// `QUAD_MARGIN_PX` of the shared marker shape (SHADER_COMMON.md §5.5): a
/// marker draws nothing farther than its visual radius plus this margin from
/// its centre on either axis.
pub(in crate::gpu_pick) const MARKER_QUAD_MARGIN_PX: f32 =
    crate::data_render::MARKER_QUAD_MARGIN_PX;

/// `CONSTELLATION_POINT_STAR_GAIN` times the largest per-point size jitter
/// of `constellation_star_radius` (`0.86 + 0.30`).
pub(in crate::gpu_pick) const CONSTELLATION_STAR_MAX_SCALE: f32 = 1.16;
/// `PLANET_QUAD_EXTENT`: a planet's quad half-extent per unit radius.
pub(crate) const PLANET_QUAD_EXTENT: f32 = 2.5;

/// The pick shader's per-query uniform for one registered series. `data` is
/// `(point count, x lane base, y lane base, style-index lane base)` and
/// `stream` the `(global offset, owned scatter count, owned segment count,
/// row-map base)` of the chunk; a resident series owns every point. `arc`
/// says whether the exact pass binds this series' arc prefix.
#[allow(clippy::too_many_arguments)]
pub(in crate::gpu_pick) fn query_params(
    series: &PickSeriesGpu,
    series_id: &str,
    query: &GpuPickQuery,
    display_scale: f32,
    series_order: u32,
    data: [u32; 4],
    stream: [u32; 4],
    arc: bool,
) -> PickQueryParamsGpu {
    let [cursor_x, cursor_y] = query.canvas_position_px;
    let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
    let style = query.draw_style;
    let params = query.transform.style_params;
    // Nothing a marker draws lies outside its quad: the visual radius plus
    // the quad margin on either axis, grown by the styled entry's own quad.
    let base_radius = series.base_radius_px * display_scale;
    // A raw point moves across its category slot: the gates reach past
    // its category position by one and a half slots, more than the group's
    // half span plus the largest offset and jitter (two half thicknesses).
    let jitter_reach = series.category_jitter.map_or(0.0, |jitter| {
        let axis = usize::from(jitter.horizontal);
        let t = &query.transform;
        let range = t.data_max[axis] - t.data_min[axis];
        let size = if jitter.horizontal {
            query.chart_rect_px[3]
        } else {
            query.chart_rect_px[2]
        };
        let slot = (size * t.data_to_panel_scale[axis] / range).abs();
        if slot.is_finite() { slot * 1.5 } else { 0.0 }
    });
    let marker_reach = jitter_reach
        + match style.marker {
            PICK_MARKER_SKETCH => {
                series.max_extent_px * display_scale
                    + MARKER_QUAD_MARGIN_PX
                    + params[0][0].max(0.0) * 0.5
            }
            PICK_MARKER_PLANET => base_radius.max(1.0) * PLANET_QUAD_EXTENT + MARKER_QUAD_MARGIN_PX,
            PICK_MARKER_STAR => {
                (base_radius * CONSTELLATION_STAR_MAX_SCALE).max(0.5) * 4.0 + MARKER_QUAD_MARGIN_PX
            }
            _ => series.max_extent_px * display_scale + MARKER_QUAD_MARGIN_PX,
        };
    // The line shader's `max(line_width_px, 1) / 2`.
    let half_width = (2.0 * series.line_half_width_px * display_scale).max(1.0) * 0.5;
    let ribbon = milkyway_line_half_extent(&params);
    let line_extent = match style.line {
        PICK_LINE_SKETCH => {
            half_width + crate::data_render::MARKER_STROKE_HALF_PX + params[0][0].max(0.0)
        }
        PICK_LINE_RIBBON => ribbon,
        _ => half_width + crate::data_render::MARKER_STROKE_HALF_PX,
    };
    let mut line_dash = [[0.0f32; 4]; 2];
    for (index, length) in series.line_dash.iter().enumerate() {
        line_dash[index / 4][index % 4] = length * display_scale;
    }
    // A milkyway ribbon is never dashed.
    let dash_count = if style.line == PICK_LINE_RIBBON {
        0
    } else {
        series.line_dash_len
    };
    // Streamline arrowheads (`pick_path_arrow_candidate`): the head in
    // canvas pixels through the vector lanes, and the gate's reach past the
    // centreline grows by a head's half diagonal, since a head is centred on
    // the path.
    let heads = series.path_arrows.map(|arrows| {
        let length = arrows.head_length_px * display_scale;
        let width = arrows.head_width_px * display_scale;
        (
            [length, 0.0, length, width],
            arrows.spacing_px * display_scale,
            (length * 0.5).hypot(width * 0.5),
        )
    });
    PickQueryParamsGpu {
        transform: query.transform,
        cursor_chart: [cursor_x, cursor_y, chart_x, chart_y],
        chart_limits: [
            chart_width,
            chart_height,
            query.max_distance_px,
            marker_reach,
        ],
        scatter_line: [
            series.base_radius_px,
            series.line_half_width_px,
            display_scale,
            if series.direct_scan { 1.0 } else { 0.0 },
        ],
        data,
        style: [
            series.flags,
            series.style_count,
            series.override_count,
            series.style_index_len,
        ],
        series: [
            series.gate_word_count,
            series_order,
            series.base_shape_id,
            series.dispatch_x,
        ],
        stream,
        line: [
            series.line_step_mode,
            u32::from(arc),
            u32::from(series.line_closed),
            0,
        ],
        size_map: [
            series.size_base,
            series.size_len,
            series.ternary.as_ref().map_or(0, |c| c.base),
            series.ternary.as_ref().map_or(0, |c| c.len),
        ],
        size_domain: series.size_domain,
        size_px: [series.size_px[0], series.size_px[1], 0.0, 0.0],
        // A square cap's corner reaches √2 times the half extent past the
        // centreline on either axis.
        line_px: [
            half_width,
            ribbon,
            line_extent * std::f32::consts::SQRT_2 + heads.map_or(0.0, |(_, _, reach)| reach),
            0.0,
        ],
        line_dash,
        styled: [
            style.marker,
            style.line,
            crate::sketch::fnv1a(series_id),
            dash_count,
        ],
        vector: [
            series.vector_base[0],
            series.vector_base[1],
            series.vector_count,
            series.vector.as_ref().map_or(0, |vector| {
                vector.pivot
                    | if vector.scale_pixels {
                        VECTOR_SCALE_PIXELS
                    } else {
                        0
                    }
            }),
        ],
        vector_px: series
            .vector
            .as_ref()
            .map_or(heads.map_or([0.0; 4], |(px, ..)| px), |vector| {
                [
                    if vector.scale_pixels {
                        vector.scale * display_scale
                    } else {
                        vector.scale
                    },
                    vector.width_px * display_scale,
                    vector.head_length_px * display_scale,
                    vector.head_width_px * display_scale,
                ]
            }),
        vector_min: [
            series
                .vector
                .as_ref()
                .map_or(0.0, |vector| vector.min_length_px * display_scale),
            heads.map_or(0.0, |(_, spacing, _)| spacing),
            0.0,
            0.0,
        ],
        errorbar: series.errorbar.as_ref().map_or([0; 4], |bars| bars.params),
        errorbar_lanes: series.errorbar.as_ref().map_or([0; 4], |bars| bars.lanes),
        errorbar_rows: series.errorbar.as_ref().map_or([0; 4], |bars| bars.rows),
        errorbar_px: series.errorbar.as_ref().map_or([0.0; 4], |bars| bars.px),
        area: series.area.as_ref().map_or([0; 4], |fill| fill.params),
        area_baseline: series.area.as_ref().map_or([0.0; 4], |fill| fill.baseline),
        stem: series.stem.as_ref().map_or([0.0; 4], |stem| {
            let (hi, lo) = crate::data::split_f64_to_f32_pair(stem.baseline);
            [hi, lo, stem.width_px, 0.0]
        }),
        stem_dash: series.stem.as_ref().map_or([[0.0; 4]; 2], |stem| {
            let mut dash = [[0.0; 4]; 2];
            for (slot, length) in dash.iter_mut().flatten().zip(stem.dash.pattern()) {
                *slot = length * display_scale;
            }
            dash
        }),
        stem_info: series.stem.as_ref().map_or([0; 4], |stem| {
            [
                1,
                if stem.horizontal {
                    crate::data_render::ERRORBAR_HAS_X
                } else {
                    crate::data_render::ERRORBAR_HAS_Y
                },
                stem.dash.pattern().len().min(8) as u32,
                u32::from(stem.paints),
            ]
        }),
        category: series.category_jitter.map_or([0.0; 4], |jitter| {
            [
                jitter.group_width,
                jitter.gap_px,
                jitter.jitter,
                jitter.offset,
            ]
        }),
        category_slot: series.category_jitter.map_or([0; 4], |jitter| {
            [
                jitter.member,
                jitter.members.max(1),
                u32::from(jitter.horizontal),
                jitter.seed,
            ]
        }),
    }
}

/// The half extent of what a Milkyway line draws around its centreline, in
/// canvas pixels, from the packed style parameters (`pack_milkyway_params`):
/// the ribbon strip reaches one full ribbon width either side
/// (`vs_ribbon`'s `max(ribbon_width_px, 1)`), and a star of the arc-driven
/// star pass (`vs_stars`) lies within its perpendicular scatter
/// (`1.7 * spread_px`), a binary companion's separation (`4 *
/// structure_scale`) and its quad (`4 * star_scale * 4.3` at full
/// brightness) of the path. The star positions are hashed per slot, so this
/// envelope, not each star, is what picks.
pub(crate) fn milkyway_line_half_extent(params: &[[f32; 4]; 3]) -> f32 {
    let ribbon = params[0][1].max(1.0);
    let star_scale = params[1][0].max(0.0);
    let spread = params[1][1].max(0.0);
    let structure = params[2][0].max(1e-3);
    let stars = if params[0][0] > 0.0 {
        spread * 1.7 + 4.0 * structure + star_scale * (0.9 + 3.4) * 4.0
    } else {
        0.0
    };
    ribbon.max(stars)
}
