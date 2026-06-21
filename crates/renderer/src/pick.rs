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
    pub data_x: f32,
    pub data_y: f32,
    pub distance_px: f32,
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
            for i in 0..xs.len().min(ys.len()) {
                let radius_px =
                    scatter_visual_radius_px(scatter, style_index, i, use_scatter_style_mapping);
                if radius_px <= 0.0 {
                    continue;
                }

                let x = xs[i];
                let y = ys[i];
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }

                let Some((px, py)) = project_data_to_canvas_px(config, &transform, x, y) else {
                    continue;
                };
                let dx = px - canvas_x;
                let dy = py - canvas_y;
                let center_dist = dx.mul_add(dx, dy * dy).sqrt();
                let hit_dist = (center_dist - radius_px).max(0.0);
                let dist_sq = hit_dist * hit_dist;
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
                        point_index: i,
                        data_x: x,
                        data_y: y,
                        distance_px: hit_dist,
                    },
                );
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

            let Some((a_px, a_py)) = project_data_to_canvas_px(config, &transform, ax, ay) else {
                continue;
            };
            let Some((b_px, b_py)) = project_data_to_canvas_px(config, &transform, bx, by) else {
                continue;
            };
            let Some((dist_sq, point_index)) =
                line_segment_pick(canvas_x, canvas_y, a_px, a_py, b_px, b_py, half_width, i)
            else {
                continue;
            };
            if !dist_sq.is_finite() || dist_sq > max_dist_sq {
                continue;
            }

            let data_x = xs[point_index];
            let data_y = ys[point_index];
            maybe_replace_best(
                &mut best,
                series_index,
                dist_sq,
                PickedPoint {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    point_index,
                    data_x,
                    data_y,
                    distance_px: dist_sq.sqrt(),
                },
            );
        }
    }

    best.map(|(_, _, p)| p)
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

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        _ => None,
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
        DataRenderType::Line { .. } => None,
    }
}

fn scatter_visual_radius_px(
    scatter: &DataScatterStyleConfig,
    style_index: Option<&[f32]>,
    point_index: usize,
    use_style_mapping: bool,
) -> f32 {
    let mut radius = scatter.point_size;
    let mut shape = &scatter.point_shape;

    if use_style_mapping {
        if let (Some(indices), Some(table)) = (style_index, scatter.point_style_table.as_deref()) {
            if let Some(idx) = indices
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
        }

        if let Some(overrides) = scatter.point_style_overrides.as_deref() {
            for ov in overrides {
                if ov.index == point_index {
                    if let Some(size) = ov.style.point_size {
                        radius = size;
                    }
                    if let Some(ov_shape) = ov.style.point_shape.as_ref() {
                        shape = ov_shape;
                    }
                }
            }
        }
    }

    shape_visual_radius_px(shape, radius.max(0.0))
}

fn valid_style_index(v: f32) -> Option<usize> {
    if v >= 0.0 && v <= 16_777_216.0 && (v - v.round()).abs() <= 0.001 {
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

fn line_segment_pick(
    x: f32,
    y: f32,
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    half_width: f32,
    point_index: usize,
) -> Option<(f32, usize)> {
    let sx = bx - ax;
    let sy = by - ay;
    let len_sq = sx.mul_add(sx, sy * sy);
    if !len_sq.is_finite() || len_sq <= f32::EPSILON {
        return None;
    }

    let t = (((x - ax) * sx + (y - ay) * sy) / len_sq).clamp(0.0, 1.0);
    let closest_x = ax + sx * t;
    let closest_y = ay + sy * t;
    let dx = closest_x - x;
    let dy = closest_y - y;
    let center_dist = dx.mul_add(dx, dy * dy).sqrt();
    if !center_dist.is_finite() {
        return None;
    }
    let hit_dist = (center_dist - half_width.max(0.0)).max(0.0);
    let snapped_index = if t <= 0.5 {
        point_index
    } else {
        point_index + 1
    };
    Some((hit_dist * hit_dist, snapped_index))
}

fn project_data_to_canvas_px(
    config: &Config,
    transform: &data_render::ScatterTransform,
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    let xv = maybe_log(x, transform.scale_log[0]);
    let yv = maybe_log(y, transform.scale_log[1]);
    let range_x = transform.data_max[0] - transform.data_min[0];
    let range_y = transform.data_max[1] - transform.data_min[1];
    let tx = (xv - transform.data_min[0]) / range_x;
    let ty = (yv - transform.data_min[1]) / range_y;
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
        assert_eq!(picked.data_x, 7.0);
        assert_eq!(picked.data_y, 7.0);
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
        assert_eq!(picked.data_x, 10.0);
        assert_eq!(picked.data_y, 10.0);
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
        assert_eq!(picked.data_x, 10.0);
        assert_eq!(picked.data_y, 5.0);
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
        assert_eq!(picked.data_x, 2.0);
        assert_eq!(picked.data_y, 8.0);
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
        assert_eq!(picked.data_x, 5.0);
        assert_eq!(picked.data_y, 5.0);
    }
}
