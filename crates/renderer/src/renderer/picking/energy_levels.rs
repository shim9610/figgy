//! The CPU pick of energy-level series.

use super::*;

/// The best hit of the DataLess series of `series` (the active layer's, in
/// paint order: energy levels, #203, and Feynman diagrams, #234) under
/// `request` on the displayed panel `display`, picked on the CPU through the
/// placement the raster draws with. The
/// geometry is placed in logical pixels, as the raster is, and the distance
/// reported in display pixels. `document` resolves a view's source.
pub(in crate::renderer) fn pick_energy_levels(
    display: &Config,
    series: &[SeriesConfig],
    document: &[SeriesConfig],
    request: &GpuPickRequest,
) -> Result<Option<crate::gpu_data_pick::RankedPickedData>> {
    use crate::energy_levels::{LevelHit, LevelPlacement, drawn_records};
    use crate::pick::PickedData;
    let scale = if request.display_scale.is_finite() && request.display_scale > 0.0 {
        request.display_scale
    } else {
        1.0
    };
    let logical = |r: crate::layout::RectF| crate::layout::RectF {
        x: r.x / scale,
        y: r.y / scale,
        width: r.width / scale,
        height: r.height / scale,
    };
    let mut space = crate::annotation::pixel_space_for_config(display);
    space.panel = logical(space.panel);
    space.plot = logical(space.plot);
    let measure = crate::text_render::CpuTextMeasure::for_style(&display.draw_style);
    let cursor = [
        request.canvas_position_px[0] / scale,
        request.canvas_position_px[1] / scale,
    ];
    let mut best: Option<crate::gpu_data_pick::RankedPickedData> = None;
    for (order, cfg) in series.iter().enumerate() {
        // A Feynman diagram picks its vertices, propagators and labels
        // through the placement its raster draws with (#234).
        if let Some(diagram) = cfg.feynman() {
            use crate::feynman::FeynmanHit;
            // A layout that fails or memory that cannot be allocated is the
            // pick's error, as it is the raster's.
            let Some((hit, distance)) = diagram
                .pick(&space, &measure, cursor, request.max_distance_px / scale)
                .map_err(|error| super::feynman_error(&cfg.series_id, error))?
            else {
                continue;
            };
            let source_id = cfg.source_id.clone();
            let series_id = cfg.series_id.clone();
            let distance_px = distance * scale;
            let picked = match hit {
                FeynmanHit::Vertex(vertex_index) => PickedData::FeynmanVertex {
                    source_id,
                    series_id,
                    vertex_index,
                    distance_px,
                },
                FeynmanHit::Propagator(propagator_index) => PickedData::FeynmanPropagator {
                    source_id,
                    series_id,
                    propagator_index,
                    distance_px,
                },
                FeynmanHit::Label(part, index) => PickedData::FeynmanLabel {
                    source_id,
                    series_id,
                    part,
                    index,
                    distance_px,
                },
            };
            let candidate = crate::gpu_data_pick::RankedPickedData {
                picked,
                paint_order: u32::try_from(order).unwrap_or(u32::MAX),
            };
            if best
                .as_ref()
                .is_none_or(|b| candidate.picked.distance_px() <= b.picked.distance_px())
            {
                best = Some(candidate);
            }
            continue;
        }
        let Some(levels) = cfg.energy_levels() else {
            continue;
        };
        let Some(records) = drawn_records(levels, document) else {
            continue;
        };
        let placement = LevelPlacement {
            space: &space,
            categories: display.bottom_x.categories.as_deref().unwrap_or(&[]),
            view: levels.show,
            colorbar: display.colorbar.as_ref(),
            picked: None,
        };
        let Some((hit, distance)) = placement.pick(
            records,
            &levels.style,
            &measure,
            cursor,
            request.max_distance_px / scale,
        ) else {
            continue;
        };
        let source_id = cfg.source_id.clone();
        let series_id = cfg.series_id.clone();
        let distance_px = distance * scale;
        let picked = match hit {
            LevelHit::Level(bin_index) => PickedData::HistogramBin {
                source_id,
                series_id,
                bin_index,
                half: None,
                distance_px,
            },
            LevelHit::Transition(point_index) => PickedData::Point {
                source_id,
                series_id,
                point_index,
                distance_px,
            },
            LevelHit::Label(part, category_index) => PickedData::ValueLabel {
                source_id,
                series_id,
                category_index,
                part: Some(part),
                distance_px,
            },
        };
        let candidate = crate::gpu_data_pick::RankedPickedData {
            picked,
            paint_order: u32::try_from(order).unwrap_or(u32::MAX),
        };
        if best
            .as_ref()
            .is_none_or(|b| candidate.picked.distance_px() <= b.picked.distance_px())
        {
            best = Some(candidate);
        }
    }
    Ok(best)
}
