use crate::config::Config;
use crate::data_config::{
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape, SeriesConfig,
};
use crate::data_render::{self, ColumnId};

pub trait PointColumnLookup {
    fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointPickOptions {
    pub max_distance_px: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickedPoint {
    pub source_id: Option<String>,
    pub series_id: String,
    pub point_index: usize,
    pub distance_px: f32,
}

/// One exact data-primitive hit.
///
/// The result contains only source identity and stable indices. Data values,
/// coordinates, bar bounds, and reconstructed contour endpoints remain in the
/// registered columns / chart configuration and are never mirrored onto the
/// CPU by the GPU picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PickedData {
    Point {
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
        distance_px: f32,
    },
    HistogramBin {
        source_id: Option<String>,
        series_id: String,
        bin_index: usize,
        distance_px: f32,
    },
    MatrixCell {
        source_id: Option<String>,
        series_id: String,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
    ContourLevel {
        source_id: Option<String>,
        series_id: String,
        level_index: usize,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
}

impl PickedData {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. } => series_id,
        }
    }

    pub fn distance_px(&self) -> f32 {
        match self {
            Self::Point { distance_px, .. }
            | Self::HistogramBin { distance_px, .. }
            | Self::MatrixCell { distance_px, .. }
            | Self::ContourLevel { distance_px, .. } => *distance_px,
        }
    }

    pub fn to_ref(&self) -> crate::config::PickedDataRef {
        match self {
            Self::Point {
                source_id,
                series_id,
                point_index,
                ..
            } => crate::config::PickedDataRef::Point {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                point_index: *point_index,
            },
            Self::HistogramBin {
                source_id,
                series_id,
                bin_index,
                ..
            } => crate::config::PickedDataRef::HistogramBin {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                bin_index: *bin_index,
            },
            Self::MatrixCell {
                source_id,
                series_id,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::MatrixCell {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                x_index: *x_index,
                y_index: *y_index,
            },
            Self::ContourLevel {
                source_id,
                series_id,
                level_index,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::ContourLevel {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                level_index: *level_index,
                x_index: *x_index,
                y_index: *y_index,
            },
        }
    }

    pub fn into_point(self) -> Option<PickedPoint> {
        let Self::Point {
            source_id,
            series_id,
            point_index,
            distance_px,
        } = self
        else {
            return None;
        };
        Some(PickedPoint {
            source_id,
            series_id,
            point_index,
            distance_px,
        })
    }
}

impl From<PickedPoint> for PickedData {
    fn from(point: PickedPoint) -> Self {
        Self::Point {
            source_id: point.source_id,
            series_id: point.series_id,
            point_index: point.point_index,
            distance_px: point.distance_px,
        }
    }
}

pub fn pick_nearest_point<L: PointColumnLookup>(
    config: &Config,
    series: &[SeriesConfig],
    columns: &L,
    canvas_x: f32,
    canvas_y: f32,
    options: PointPickOptions,
) -> Option<PickedPoint> {
    if !canvas_x.is_finite()
        || !canvas_y.is_finite()
        || !options.max_distance_px.is_finite()
        || options.max_distance_px < 0.0
    {
        return None;
    }

    if let Ok(data_area) = config.data_area() {
        let r = data_area.0;
        let x0 = r.x as f32;
        let y0 = r.y as f32;
        let x1 = x0 + r.width as f32;
        let y1 = y0 + r.height as f32;
        if canvas_x < x0 || canvas_x > x1 || canvas_y < y0 || canvas_y > y1 {
            return None;
        }
    }

    let transform = data_render::scatter_transform_from_config(config);
    let max_dist_sq = options.max_distance_px * options.max_distance_px;
    let use_scatter_style_mapping = config.draw_style.is_precise();
    let mut best: Option<(usize, f32, PickedPoint)> = None;

    for (series_index, cfg) in series.iter().enumerate() {
        let Some(xs) = columns.get_f32_column(&cfg.x_column) else {
            continue;
        };
        let Some(ys) = columns.get_f32_column(&cfg.y_column) else {
            continue;
        };

        if let Some(scatter) = extract_scatter(&cfg.render_type) {
            let style_index = if use_scatter_style_mapping {
                scatter
                    .point_style_index_column
                    .as_ref()
                    .and_then(|column| columns.get_f32_column(column))
            } else {
                None
            };
            let size_values = if use_scatter_style_mapping {
                scatter
                    .point_size_map
                    .as_ref()
                    .and_then(|map| columns.get_f32_column(&map.column))
            } else {
                None
            };
            for i in 0..xs.len().min(ys.len()) {
                let radius_px = scatter_visual_radius_px(
                    scatter,
                    style_index,
                    size_values,
                    i,
                    use_scatter_style_mapping,
                );
                if radius_px <= 0.0 {
                    continue;
                }
                pick_anchor(
                    &mut best,
                    series_index,
                    cfg,
                    config,
                    &transform,
                    i,
                    (xs[i], ys[i]),
                    radius_px,
                    (canvas_x, canvas_y),
                    max_dist_sq,
                );
            }
        }

        // A point vector is picked at its anchors with the head's radius. It
        // draws only in precise mode, so it is picked only there.
        if let Some(style) = extract_vector_points(&cfg.render_type)
            && use_scatter_style_mapping
        {
            let radius_px = vector_anchor_radius_px(style);
            let DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points { u, v },
                ..
            } = &cfg.render_type
            else {
                unreachable!("extract_vector_points matched the points form");
            };
            let count = xs
                .len()
                .min(ys.len())
                .min(columns.get_f32_column(u).map_or(0, |u| u.len()))
                .min(columns.get_f32_column(v).map_or(0, |v| v.len()));
            if radius_px > 0.0 {
                for i in 0..count {
                    pick_anchor(
                        &mut best,
                        series_index,
                        cfg,
                        config,
                        &transform,
                        i,
                        (xs[i], ys[i]),
                        radius_px,
                        (canvas_x, canvas_y),
                        max_dist_sq,
                    );
                }
            }
        }

        let Some(line) = extract_line(&cfg.render_type) else {
            continue;
        };
        let half_width = (line.line_width.max(0.0) * 0.5).max(0.0);
        for i in 0..xs.len().min(ys.len()).saturating_sub(1) {
            let ax = xs[i];
            let ay = ys[i];
            let bx = xs[i + 1];
            let by = ys[i + 1];
            if !ax.is_finite() || !ay.is_finite() || !bx.is_finite() || !by.is_finite() {
                continue;
            }
            // Both endpoints must be placeable before any piece is tested —
            // the same gate the GPU picker applies to the segment.
            if project_data_to_canvas_px(config, &transform, ax, ay).is_none()
                || project_data_to_canvas_px(config, &transform, bx, by).is_none()
            {
                continue;
            }

            // A step mode walks the same axis-aligned pieces the line shader
            // draws; a linear segment is its single piece.
            let (pieces, piece_count) =
                crate::line_step::step_pieces([ax, ay], [bx, by], line.interpolation);
            let mut pieces_px = [([0.0f32; 2], [0.0f32; 2]); 3];
            let mut projected = true;
            for (piece, out) in pieces[..piece_count].iter().zip(pieces_px.iter_mut()) {
                let Some(a) = project_data_to_canvas_px(config, &transform, piece.a[0], piece.a[1])
                else {
                    projected = false;
                    break;
                };
                let Some(b) = project_data_to_canvas_px(config, &transform, piece.b[0], piece.b[1])
                else {
                    projected = false;
                    break;
                };
                *out = ([a.0, a.1], [b.0, b.1]);
            }
            if !projected {
                continue;
            }
            let Some((dist_sq, point_index)) = line_segment_pick(
                [canvas_x, canvas_y],
                &pieces_px[..piece_count],
                half_width,
                i,
            ) else {
                continue;
            };
            if !dist_sq.is_finite() || dist_sq > max_dist_sq {
                continue;
            }

            maybe_replace_best(
                &mut best,
                series_index,
                dist_sq,
                PickedPoint {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    point_index,
                    distance_px: dist_sq.sqrt(),
                },
            );
        }
    }

    best.map(|(_, _, p)| p)
}

/// Score one anchor (a marker or a vector's tail) of `cfg` against the cursor
/// and keep it when it is the best so far: the data point `(x, y)` is
/// projected to the canvas, the hit distance is measured from the edge of its
/// `radius_px` disc, and anything past `max_dist_sq` is dropped. A non-finite
/// or unprojectable anchor is skipped.
#[allow(clippy::too_many_arguments)]
fn pick_anchor(
    best: &mut Option<(usize, f32, PickedPoint)>,
    series_index: usize,
    cfg: &SeriesConfig,
    config: &Config,
    transform: &data_render::ScatterTransform,
    index: usize,
    (x, y): (f32, f32),
    radius_px: f32,
    (canvas_x, canvas_y): (f32, f32),
    max_dist_sq: f32,
) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let Some((px, py)) = project_data_to_canvas_px(config, transform, x, y) else {
        return;
    };
    let dx = px - canvas_x;
    let dy = py - canvas_y;
    let center_dist = dx.mul_add(dx, dy * dy).sqrt();
    let hit_dist = (center_dist - radius_px).max(0.0);
    let dist_sq = hit_dist * hit_dist;
    if !dist_sq.is_finite() || dist_sq > max_dist_sq {
        return;
    }
    maybe_replace_best(
        best,
        series_index,
        dist_sq,
        PickedPoint {
            source_id: cfg.source_id.clone(),
            series_id: cfg.series_id.clone(),
            point_index: index,
            distance_px: hit_dist,
        },
    );
}

fn maybe_replace_best(
    best: &mut Option<(usize, f32, PickedPoint)>,
    series_index: usize,
    dist_sq: f32,
    picked: PickedPoint,
) {
    let replace = match best {
        None => true,
        Some((best_series, best_dist_sq, _)) => {
            dist_sq < *best_dist_sq || (dist_sq == *best_dist_sq && series_index > *best_series)
        }
    };
    if replace {
        *best = Some((series_index, dist_sq, picked));
    }
}

/// The pick radius of a vector series' anchor, in logical pixels: half the
/// head width, or half the shaft width when the shaft is wider. The GPU
/// picker and the selection ring use the same number.
pub(crate) fn vector_anchor_radius_px(style: &crate::data_config::VectorStyle) -> f32 {
    (style.head_width_px.max(style.width_px) * 0.5).max(0.0)
}

/// The style of a point vector series: its anchors are picked like markers.
/// A grid vector's cells are a separate query, like a heatmap's.
fn extract_vector_points(rt: &DataRenderType) -> Option<&crate::data_config::VectorStyle> {
    match rt {
        DataRenderType::Vector {
            components: crate::data_config::VectorComponents::Points { .. },
            style,
        } => Some(style),
        _ => None,
    }
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        // Exhaustive on purpose: a new render type must fail to compile here so
        // somebody decides whether picking should hit-test a line for it.
        //
        // The four field / bar types are not point-pickable: a bar has no
        // vertex to snap to, and a matrix' `(x_column, y_column)` are grid
        // coordinates, so a "nearest point" would be a grid intersection the
        // user never plotted. Picking a cell or a level is a separate query.
        DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        // A vector series is picked at its anchors through `extract_vector`.
        | DataRenderType::Vector { .. } => None,
    }
}

fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

/// The drawn radius of one point, in logical pixels; zero for a hidden point.
///
/// Precedence per channel is the shader's: sparse override > size map >
/// style table slot > series base. A size map resolves the radius from the
/// point's value column (`size_values`, the same column the GPU draws from);
/// a non-finite value hides the point unless an override sets its radius.
fn scatter_visual_radius_px(
    scatter: &DataScatterStyleConfig,
    style_index: Option<&[f32]>,
    size_values: Option<&[f32]>,
    point_index: usize,
    use_style_mapping: bool,
) -> f32 {
    let mut radius = scatter.point_size;
    let mut shape = &scatter.point_shape;
    let mut hidden = false;

    if use_style_mapping {
        if let (Some(indices), Some(table)) = (style_index, scatter.point_style_table.as_deref())
            && let Some(idx) = indices
                .get(point_index)
                .copied()
                .and_then(valid_style_index)
                .filter(|idx| *idx < table.len())
        {
            let slot = &table[idx];
            if let Some(size) = slot.point_size {
                radius = size;
            }
            if let Some(slot_shape) = slot.point_shape.as_ref() {
                shape = slot_shape;
            }
        }

        if let Some(map) = scatter.point_size_map.as_ref() {
            match size_values
                .and_then(|values| values.get(point_index))
                .and_then(|value| map.radius_px(f64::from(*value)))
            {
                Some(mapped) => radius = mapped,
                None => hidden = true,
            }
        }

        if let Some(overrides) = scatter.point_style_overrides.as_deref() {
            for ov in overrides {
                if ov.index == point_index {
                    if let Some(size) = ov.style.point_size {
                        radius = size;
                        hidden = false;
                    }
                    if let Some(ov_shape) = ov.style.point_shape.as_ref() {
                        shape = ov_shape;
                    }
                }
            }
        }
    }

    if hidden {
        return 0.0;
    }
    shape_visual_radius_px(shape, radius.max(0.0))
}

fn valid_style_index(v: f32) -> Option<usize> {
    if (0.0..=16_777_216.0).contains(&v) && (v - v.round()).abs() <= 0.001 {
        Some(v.round() as usize)
    } else {
        None
    }
}

fn shape_visual_radius_px(shape: &ScatterShape, radius: f32) -> f32 {
    let scale = match shape {
        ScatterShape::Square | ScatterShape::SquareFilled => 0.886_226_95,
        ScatterShape::Triangle
        | ScatterShape::TriangleFilled
        | ScatterShape::TriangleDown
        | ScatterShape::TriangleLeft
        | ScatterShape::TriangleRight
        | ScatterShape::TriangleDownFilled
        | ScatterShape::TriangleLeftFilled
        | ScatterShape::TriangleRightFilled => 1.555_120_3,
        ScatterShape::Diamond | ScatterShape::DiamondFilled => 1.253_314_1,
        ScatterShape::Pentagon | ScatterShape::PentagonFilled => 1.149_139_9,
        ScatterShape::Hexagon | ScatterShape::HexagonFilled => 1.099_636_1,
        ScatterShape::Octagon | ScatterShape::OctagonFilled => 1.053_907_4,
        ScatterShape::Star | ScatterShape::StarFilled => 1.462_850_3,
        _ => 1.0,
    };
    radius * scale
}

/// Distance² from `cursor` to the stroke of one segment drawn as `pieces`
/// (projected canvas pixels, in draw order), and the source index the hit
/// snaps to.
///
/// Every piece is tested and the nearest wins (the earlier piece on a tie).
/// The snapped index follows the position of the hit along the whole stepped
/// path — 0 at the segment's first sample, 1 at its second — so a hit on a
/// riser resolves to the nearer of the two source indices, never to a corner
/// that is not a sample. A linear segment is its single piece and keeps the
/// straight-segment rule bit for bit. Twin of `pick_line_candidate` in
/// `gpu_pick.wgsl`.
fn line_segment_pick(
    cursor: [f32; 2],
    pieces: &[([f32; 2], [f32; 2])],
    half_width: f32,
    point_index: usize,
) -> Option<(f32, usize)> {
    let [x, y] = cursor;
    let mut coordinate_scale = 1.0f32;
    let mut total_len = 0.0f32;
    let mut prefix_len = 0.0f32;
    let mut best: Option<(f32, f32, f32, f32)> = None; // (dist_sq, t, prefix, len)
    for &([ax, ay], [bx, by]) in pieces {
        coordinate_scale = coordinate_scale
            .max(ax.abs())
            .max(ay.abs())
            .max(bx.abs())
            .max(by.abs());
        let sx = bx - ax;
        let sy = by - ay;
        let len_sq = sx.mul_add(sx, sy * sy);
        if !len_sq.is_finite() {
            return None;
        }
        // A zero-length piece (a riser between equal values, or a degenerate
        // linear segment) has no stroke to hit.
        if len_sq <= f32::EPSILON {
            continue;
        }
        let len = len_sq.sqrt();
        let t = (((x - ax) * sx + (y - ay) * sy) / len_sq).clamp(0.0, 1.0);
        // Match GPU picking without reconstructing a rounded closest point.
        // Interior distance comes from the perpendicular; outside the segment
        // it comes from the nearest endpoint, preserving the finite-segment test.
        let center_dist = if t <= 0.0 {
            let dx = x - ax;
            let dy = y - ay;
            dx.mul_add(dx, dy * dy).sqrt()
        } else if t >= 1.0 {
            let dx = x - bx;
            let dy = y - by;
            dx.mul_add(dx, dy * dy).sqrt()
        } else {
            (x - ax).mul_add(sy, -(y - ay) * sx).abs() / len
        };
        if !center_dist.is_finite() {
            return None;
        }
        let hit_dist = (center_dist - half_width.max(0.0)).max(0.0);
        let dist_sq = hit_dist * hit_dist;
        if dist_sq.is_finite() && best.is_none_or(|(best_sq, ..)| dist_sq < best_sq) {
            best = Some((dist_sq, t, prefix_len, len));
        }
        prefix_len += len;
        total_len += len;
    }
    let (dist_sq, t, prefix, len) = best?;
    // Position along the whole stepped path. A linear segment keeps `t`
    // itself: `(t · len) / len` is not bit-exact.
    let position = if pieces.len() > 1 {
        (prefix + t * len) / total_len
    } else {
        t
    };
    // Match the GPU's lower-index tie rule within projected f32 uncertainty.
    let midpoint_tolerance = (f32::EPSILON * coordinate_scale / total_len).min(0.25);
    let snapped_index = if position <= 0.5 + midpoint_tolerance {
        point_index
    } else {
        point_index + 1
    };
    Some((dist_sq, snapped_index))
}

fn project_data_to_canvas_px(
    config: &Config,
    transform: &data_render::ScatterTransform,
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    if (transform.scale_log[0] != 0.0 && x <= 0.0) || (transform.scale_log[1] != 0.0 && y <= 0.0) {
        return None;
    }
    let axis_t = |value: f32, axis: usize| {
        let min_hi = transform.data_min[axis];
        let max_hi = transform.data_max[axis];
        let min_lo = transform.data_min_lo[axis];
        let max_lo = transform.data_max_lo[axis];
        let linear_num = (value - min_hi) - min_lo;
        let range = (max_hi - min_hi) + (max_lo - min_lo);
        let log_num = (maybe_log(value, transform.scale_log[axis]) - min_hi) - min_lo;
        let data_t = linear_num * (1.0 - transform.scale_log[axis]) / range
            + log_num * transform.scale_log[axis] / range;
        transform.data_to_panel_offset[axis] + data_t * transform.data_to_panel_scale[axis]
    };
    let tx = axis_t(x, 0);
    let ty = axis_t(y, 1);
    let ndc_x = tx * 2.0 - 1.0;
    let ndc_y = ty * 2.0 - 1.0;
    if !ndc_x.is_finite() || !ndc_y.is_finite() {
        return None;
    }

    let ca = config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;
    let px = ca.x as f32 + (ndc_x + 1.0) * 0.5 * chart_w;
    let py = ca.y as f32 + (1.0 - ndc_y) * 0.5 * chart_h;
    if px.is_finite() && py.is_finite() {
        Some((px, py))
    } else {
        None
    }
}

fn maybe_log(v: f32, is_log: f32) -> f32 {
    let lv = v.max(1e-30).log10();
    v * (1.0 - is_log) + lv * is_log
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::color::Color;
    use crate::config::AxisScale;
    use crate::data_config::{
        DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterStyleConfig, ErrorRef, ScatterShape,
    };
    use crate::layout::{ChartArea, Rect};
    use crate::line::LineStylePreset;

    #[test]
    fn segment_distance_preserves_centerlines_and_finite_endpoints() {
        for (a, b, cursor, expected_sq, expected_index) in [
            ([10.0, 50.0], [90.0, 50.0], [50.0, 50.0], 0.0, 0),
            ([10.0, 50.0], [90.0, 50.0], [70.0, 50.0], 0.0, 1),
            ([10.0, 50.0], [90.0, 50.0], [5.0, 50.0], 25.0, 0),
            ([10.0, 50.0], [90.0, 50.0], [95.0, 50.0], 25.0, 1),
            ([10.0, 50.0], [90.0, 50.0], [50.0, 53.0], 9.0, 0),
            ([50.0, 10.0], [50.0, 90.0], [50.0, 30.0], 0.0, 0),
            ([0.0, 0.0], [30.0, 40.0], [11.0, 23.0], 25.0, 0),
        ] {
            let (distance_sq, index) = line_segment_pick(cursor, &[(a, b)], 0.0, 0).unwrap();
            assert!((distance_sq - expected_sq).abs() < 0.0001);
            assert_eq!(index, expected_index);
        }
        assert_eq!(
            line_segment_pick([50.0, 53.0], &[([10.0, 50.0], [90.0, 50.0])], 1.0, 0),
            Some((4.0, 0))
        );
        assert_eq!(
            line_segment_pick([1.0, 1.0], &[([0.0, 0.0], [0.0, 0.0])], 0.0, 0),
            None
        );
    }

    // A stepped segment is picked along its pieces: the nearest piece wins,
    // a zero-length riser is skipped rather than invalidating the segment,
    // and the snapped index follows the position along the whole path.
    #[test]
    fn stepped_segment_distance_walks_every_piece_and_snaps_by_path_position() {
        // step-post from (10, 10) to (90, 50): run at y = 10, riser at x = 90.
        let post = [([10.0, 10.0], [90.0, 10.0]), ([90.0, 10.0], [90.0, 50.0])];
        // On the run, a quarter along the path: index 0.
        assert_eq!(
            line_segment_pick([30.0, 10.0], &post, 0.0, 4),
            Some((0.0, 4))
        );
        // On the riser: past the midpoint of the 120 px path, index 1.
        assert_eq!(
            line_segment_pick([90.0, 40.0], &post, 0.0, 4),
            Some((0.0, 5))
        );
        // Far from the run but near the riser: the riser's distance wins.
        let (dist_sq, index) = line_segment_pick([95.0, 45.0], &post, 0.0, 4).unwrap();
        assert!((dist_sq - 25.0).abs() < 1e-4);
        assert_eq!(index, 5);
        // The straight chord between the endpoints is NOT part of the path.
        let (dist_sq, _) = line_segment_pick([50.0, 30.0], &post, 0.0, 4).unwrap();
        assert!((dist_sq - 400.0).abs() < 1e-3);
        // A riser between equal values has zero length and is skipped.
        let flat = [([10.0, 10.0], [90.0, 10.0]), ([90.0, 10.0], [90.0, 10.0])];
        assert_eq!(
            line_segment_pick([90.0, 10.0], &flat, 0.0, 0),
            Some((0.0, 1))
        );
        // Every piece degenerate: nothing to hit.
        let point = [([5.0, 5.0], [5.0, 5.0]), ([5.0, 5.0], [5.0, 5.0])];
        assert_eq!(line_segment_pick([5.0, 5.0], &point, 0.0, 0), None);
    }

    struct Columns(HashMap<ColumnId, Vec<f32>>);

    impl Columns {
        fn new(entries: &[(&str, &[f32])]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(id, values)| ((*id).to_string(), values.to_vec()))
                    .collect(),
            )
        }
    }

    impl PointColumnLookup for Columns {
        fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]> {
            self.0.get(id).map(Vec::as_slice)
        }
    }

    fn config() -> Config {
        let mut c = crate::default::default_config();
        c.chart_area = ChartArea(Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 100,
        });
        c.bottom_x.min = 0.0;
        c.bottom_x.max = 10.0;
        c.left_y.min = 0.0;
        c.left_y.max = 10.0;
        c.chart_title.top_margin = 0.0;
        for axis in [&mut c.top_x, &mut c.bottom_x, &mut c.left_y, &mut c.right_y] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }
        c
    }

    fn line_series(id: &str, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }
    }

    fn scatter_series(id: &str, x: &str, y: &str, source_id: Option<&str>) -> SeriesConfig {
        scatter_series_with_size(id, x, y, source_id, 3.0)
    }

    fn scatter_series_with_size(
        id: &str,
        x: &str,
        y: &str,
        source_id: Option<&str>,
        point_size: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: source_id.map(str::to_string),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::Circle,
                    point_size,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        }
    }

    fn scatter_errorbar_y_series(
        id: &str,
        x: &str,
        y: &str,
        err_y: &str,
        point_size: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::Circle,
                    point_size,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
                err_y: ErrorRef::Symmetric {
                    column: err_y.into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 6.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        }
    }

    fn pick(
        config: &Config,
        series: &[SeriesConfig],
        columns: &Columns,
        x: f32,
        y: f32,
        max_distance_px: f32,
    ) -> Option<PickedPoint> {
        pick_nearest_point(
            config,
            series,
            columns,
            x,
            y,
            PointPickOptions { max_distance_px },
        )
    }

    #[test]
    fn picks_nearest_point_across_series() {
        let config = config();
        let columns = Columns::new(&[
            ("x0", &[1.0, 5.0]),
            ("y0", &[1.0, 5.0]),
            ("x1", &[7.0]),
            ("y1", &[7.0]),
        ]);
        let series = [
            scatter_series("farther", "x0", "y0", Some("src-a")),
            scatter_series("nearest", "x1", "y1", None),
        ];

        let picked = pick(&config, &series, &columns, 81.0, 50.0, 20.0).unwrap();

        assert_eq!(picked.series_id, "nearest");
        assert_eq!(picked.source_id, None);
        assert_eq!(picked.point_index, 0);
    }

    #[test]
    fn single_point_line_series_is_not_pickable_without_visible_segment() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [line_series("single-line", "x", "y")];

        assert_eq!(pick(&config, &series, &columns, 60.0, 70.0, 0.0), None);
    }

    #[test]
    fn max_distance_miss_returns_none() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [scatter_series("s", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 70.0, 70.0, 5.0), None);
    }

    #[test]
    fn line_series_picks_nearest_endpoint_from_segment_hit() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[0.0, 10.0])]);
        let series = [line_series("line", "x", "y")];

        let picked = pick(&config, &series, &columns, 80.0, 50.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "line");
        assert_eq!(picked.point_index, 1);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn line_width_counts_as_pickable_stroke_area() {
        let mut config = config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[5.0, 5.0])]);
        let mut series = line_series("wide", "x", "y");
        let DataRenderType::Line { line } = &mut series.render_type else {
            unreachable!();
        };
        line.line_width = 10.0;

        let picked = pick(&config, &[series], &columns, 80.0, 74.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "wide");
        assert_eq!(picked.point_index, 1);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn scatter_series_does_not_pick_between_points() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[0.0, 10.0])]);
        let series = [scatter_series("scatter", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 80.0, 50.0, 0.0), None);
    }

    #[test]
    fn scatter_pick_uses_marker_radius_when_distance_limit_is_zero() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [scatter_series_with_size("big", "x", "y", None, 8.0)];

        let picked = pick(&config, &series, &columns, 67.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "big");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn zero_size_scatter_errorbar_is_not_pickable_at_data_center() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0]), ("ey", &[2.0])]);
        let series = [scatter_errorbar_y_series("err", "x", "y", "ey", 0.0)];

        assert_eq!(pick(&config, &series, &columns, 60.0, 70.0, 0.0), None);
    }

    #[test]
    fn scatter_pick_uses_per_point_size_overrides() {
        let config = config();
        let columns = Columns::new(&[
            ("x", &[5.0, 8.0]),
            ("y", &[5.0, 5.0]),
            ("style", &[0.0, 1.0]),
        ]);
        let mut series = scatter_series_with_size("mapped", "x", "y", None, 0.0);
        let DataRenderType::Scatter { scatter } = &mut series.render_type else {
            unreachable!();
        };
        scatter.point_style_index_column = Some("style".into());
        scatter.point_style_table = Some(vec![
            DataScatterPointStyleConfig {
                point_size: Some(0.0),
                ..Default::default()
            },
            DataScatterPointStyleConfig {
                point_size: Some(10.0),
                ..Default::default()
            },
        ]);

        assert_eq!(
            pick(&config, &[series.clone()], &columns, 60.0, 70.0, 0.0),
            None
        );
        let picked = pick(&config, &[series], &columns, 93.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "mapped");
        assert_eq!(picked.point_index, 1);
    }

    fn vector_points_series(id: &str, u: &str, v: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points {
                    u: u.into(),
                    v: v.into(),
                },
                style: crate::data_config::VectorStyle {
                    color: Color::BLACK,
                    width_px: 4.0,
                    scale: crate::data_config::VectorScale::Pixels { px_per_unit: 1.0 },
                    pivot: crate::data_config::VectorPivot::Tail,
                    head_length_px: 10.0,
                    head_width_px: 10.0,
                    min_length_px: 0.0,
                    color_map: None,
                },
            },
        }
    }

    /// A point vector is picked at its anchor with half the wider of head and
    /// shaft as its radius, and only as far as its component columns reach.
    #[test]
    fn vector_points_pick_at_their_anchors_with_the_glyph_radius() {
        let config = config();
        let columns = Columns::new(&[
            ("x", &[2.0, 8.0]),
            ("y", &[5.0, 5.0]),
            ("u", &[1.0, 1.0]),
            ("v", &[0.0, 0.0]),
            ("u_short", &[1.0]),
        ]);
        let series = [vector_points_series("arrows", "u", "v")];

        // Anchors at (30, 70) and (90, 70); the 10 px head gives a 5 px radius.
        let inside = pick(&config, &series, &columns, 34.5, 70.0, 0.0).unwrap();
        assert_eq!((inside.point_index, inside.distance_px), (0, 0.0));
        assert_eq!(pick(&config, &series, &columns, 36.0, 70.0, 0.0), None);
        let near = pick(&config, &series, &columns, 37.0, 70.0, 3.0).unwrap();
        assert_eq!(near.point_index, 0);
        assert!((near.distance_px - 2.0).abs() < 1e-4);
        assert_eq!(
            pick(&config, &series, &columns, 90.0, 70.0, 0.0)
                .unwrap()
                .point_index,
            1
        );

        // A component column shorter than the anchors hides the surplus.
        let short = [vector_points_series("arrows", "u_short", "v")];
        assert_eq!(
            pick(&config, &short, &columns, 30.0, 70.0, 0.0)
                .unwrap()
                .point_index,
            0
        );
        assert_eq!(pick(&config, &short, &columns, 90.0, 70.0, 0.0), None);
    }

    #[test]
    fn outside_valid_data_area_returns_none() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0]), ("y", &[0.0])]);
        let series = [scatter_series("s", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 9.0, 20.0, 100.0), None);
    }

    #[test]
    fn log_transform_matches_visible_position() {
        let mut config = config();
        config.bottom_x.scale = AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        let columns = Columns::new(&[("x", &[10.0]), ("y", &[5.0])]);
        let series = [scatter_series("log", "x", "y", None)];

        let picked = pick(&config, &series, &columns, 60.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "log");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn inverted_axes_match_visible_position() {
        let mut config = config();
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let columns = Columns::new(&[("x", &[2.0]), ("y", &[8.0])]);
        let series = [scatter_series("inverted", "x", "y", Some("src-inv"))];

        let picked = pick(&config, &series, &columns, 90.0, 100.0, 0.0).unwrap();

        assert_eq!(picked.source_id, Some("src-inv".into()));
        assert_eq!(picked.series_id, "inverted");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn exact_tie_prefers_later_series_and_lower_index_within_series() {
        let config = config();
        let columns = Columns::new(&[
            ("x0", &[5.0]),
            ("y0", &[5.0]),
            ("x1", &[5.0, 5.0]),
            ("y1", &[5.0, 5.0]),
        ]);
        let series = [
            scatter_series("early", "x0", "y0", None),
            scatter_series("late", "x1", "y1", Some("src-b")),
        ];

        let picked = pick(&config, &series, &columns, 60.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "late");
        assert_eq!(picked.source_id, Some("src-b".into()));
        assert_eq!(picked.point_index, 0);
    }
}
