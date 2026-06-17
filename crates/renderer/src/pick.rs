use crate::config::Config;
use crate::data_config::SeriesConfig;
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
    let mut best: Option<(usize, f32, PickedPoint)> = None;

    for (series_index, cfg) in series.iter().enumerate() {
        let Some(xs) = columns.get_f32_column(&cfg.x_column) else {
            continue;
        };
        let Some(ys) = columns.get_f32_column(&cfg.y_column) else {
            continue;
        };

        for i in 0..xs.len().min(ys.len()) {
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
            let dist_sq = dx.mul_add(dx, dy * dy);
            if !dist_sq.is_finite() || dist_sq > max_dist_sq {
                continue;
            }

            let replace = match &best {
                None => true,
                Some((best_series, best_dist_sq, _)) => {
                    dist_sq < *best_dist_sq
                        || (dist_sq == *best_dist_sq && series_index > *best_series)
                }
            };
            if replace {
                best = Some((
                    series_index,
                    dist_sq,
                    PickedPoint {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        point_index: i,
                        data_x: x,
                        data_y: y,
                        distance_px: dist_sq.sqrt(),
                    },
                ));
            }
        }
    }

    best.map(|(_, _, p)| p)
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
        DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape,
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
                    point_size: 3.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
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
            line_series("nearest", "x1", "y1"),
        ];

        let picked = pick(&config, &series, &columns, 81.0, 50.0, 20.0).unwrap();

        assert_eq!(picked.series_id, "nearest");
        assert_eq!(picked.source_id, None);
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.data_x, 7.0);
        assert_eq!(picked.data_y, 7.0);
    }

    #[test]
    fn max_distance_miss_returns_none() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [scatter_series("s", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 70.0, 70.0, 5.0), None);
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
