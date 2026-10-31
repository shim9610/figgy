//! Tier 2 of #46: leaf mutations against [`ResidentDisplayKey`]. The key
//! names everything the cached series bodies are drawn from; a leaf whose
//! edit changes the bodies while the key still matches leaves the panel
//! showing bodies drawn for the previous state.
//!
//! For every leaf of the chart's `Config` and of each `SeriesConfig`, the
//! edit goes through `set_chart_state`, the view is refreshed as a host
//! would, and the panel is prepared. When the prepare draws the bodies
//! again, the key changed and the property holds. When it does not, the
//! frame is painted with the kept bodies, the cache is emptied, and the
//! same state is prepared and painted again with bodies drawn from it: the
//! two frames differ only in the bodies, so any difference is a key gap.
use super::*;
use crate::data_config::{
    BarOrientation, ContourConfig, ContourLabelConfig, DataBarStyleConfig, DataErrorBarStyleConfig,
    DataLineStyleConfig, ErrorRef, FieldFillConfig, FillMode, GridLayout, LineFill,
    MatrixOrientation, MatrixRef, PointColorMap, PointSizeMap, Shading, SizeScale, VectorColorMap,
    VectorComponents, VectorPivot, VectorScale, VectorStyle,
};
use crate::renderer::config_mutation::{Outcome, Tally, run};

const SIZE: (u32, u32) = (160, 120);
const RECT: Rect = Rect {
    x: 0,
    y: 0,
    width: SIZE.0,
    height: SIZE.1,
};
const GRID: usize = 6;

/// Valid replacements for string leaves: series ids, column ids of the same
/// length as the ones they replace, and a bundled font family other than
/// the default. The first is also the value a synthesized string takes.
const STRINGS: &[&str] = &[
    "line",
    "x",
    "a",
    "b",
    "u",
    "v",
    "m",
    "z1",
    "Comic Neue",
    "rec_b",
];

fn add_columns(r: &mut Renderer) {
    let n = POINTS;
    let ramp = |scale: f32, offset: f32| {
        column(
            (0..n)
                .map(|i| offset + scale * i as f32 / (n - 1) as f32)
                .collect(),
        )
    };
    r.add_column("e", &ramp(0.04, 0.01)).unwrap();
    r.add_column("u", &wave(0.4, 0.08)).unwrap();
    r.add_column("v", &wave(1.9, 0.08)).unwrap();
    r.add_column("m", &ramp(1.0, 0.0)).unwrap();
    let axis = |len: usize| {
        column(
            (0..len)
                .map(|i| 0.05 + 0.9 * i as f32 / (len - 1) as f32)
                .collect(),
        )
    };
    r.add_column("gx", &axis(GRID)).unwrap();
    r.add_column("gy", &axis(GRID - 1)).unwrap();
    for i in 0..GRID {
        let values = (0..GRID - 1)
            .map(|j| ((i as f32 * 0.7 + j as f32 * 0.4).sin() * 0.5 + 0.5).clamp(0.0, 1.0))
            .collect();
        r.add_column(&format!("z{i}"), &column(values)).unwrap();
    }
    r.add_column("he", &axis(9)).unwrap();
    r.add_column(
        "hc",
        &column(vec![0.1, 0.3, 0.5, 0.2, 0.6, 0.4, 0.25, 0.15]),
    )
    .unwrap();
    // Category bars: the positions, then the two value columns as one batch.
    r.add_column("cpos", &column(vec![0.0, 1.0, 2.0])).unwrap();
    r.add_columns(&[
        (
            "cva",
            &column(vec![0.6, -0.4, 0.9]) as &dyn crate::ColumnSource,
        ),
        (
            "cvb",
            &column(vec![0.5, -0.3, 0.2]) as &dyn crate::ColumnSource,
        ),
    ])
    .unwrap();
    // Box plots: two records of three boxes and one outlier column.
    let field = |values: [f32; 3]| column(values.to_vec());
    for (id, shift) in [("rec_a", 0.0), ("rec_b", 0.1)] {
        let columns = [
            field([0.5 + shift, 0.6, 0.45]),
            field([0.3 + shift, 0.4, 0.3]),
            field([0.7 + shift, 0.8, 0.6]),
            field([0.1, 0.2, 0.1]),
            field([0.9, 1.0, 0.8]),
            field([0.55, 0.65, 0.5]),
            field([0.45, 0.55, 0.4]),
            field([0.55, 0.65, 0.5]),
        ];
        let names = [
            "median",
            "q1",
            "q3",
            "whisker_low",
            "whisker_high",
            "mean",
            "ci_low",
            "ci_high",
        ];
        let fields: [(&str, &dyn crate::ColumnSource); 8] =
            std::array::from_fn(|k| (names[k], &columns[k] as &dyn crate::ColumnSource));
        r.upsert_record(
            id,
            &crate::RecordSource {
                schema: crate::BOX_PLOT_RECORD,
                fields: &fields,
            },
        )
        .unwrap();
    }
    // Radar polygons: the category indices and two value columns.
    r.add_column("rcat", &column(vec![0.0, 1.0, 2.0, 3.0]))
        .unwrap();
    r.add_column("rva", &column(vec![0.9, 0.5, 0.7, 0.4]))
        .unwrap();
    r.add_column("rvb", &column(vec![0.3, 0.6, 0.2, 0.5]))
        .unwrap();
    r.upsert_ragged(
        "out_a",
        &crate::RaggedSource {
            values: &column(vec![0.05, 1.1, 1.2]),
            offsets: &column(vec![0.0, 1.0, 1.0, 3.0]),
        },
    )
    .unwrap();
}

fn chart_config() -> Config {
    let mut c = base_config();
    c.chart_area = ChartArea(RECT);
    c.chart_title.text = text("Bodies");
    c.chart_title.top_margin = 14.0;
    c.legend.content = text("all");
    for a in [&mut c.bottom_x, &mut c.left_y, &mut c.top_x, &mut c.right_y] {
        // Positive, so a logarithmic scale is a valid edit.
        a.min = 0.01;
        a.out_margin = 18.0;
        a.major_spacing = 0.5;
        a.label_style.font_size = 8.0;
    }
    let mut bar = crate::default::default_colorbar_options();
    bar.axis.min = 0.01;
    bar.axis.max = 1.0;
    bar.axis.major_spacing = 0.5;
    bar.thickness_px = 8.0;
    bar.axis.out_margin = 14.0;
    c.colorbar = Some(bar);
    c
}

/// The panel with the vector series moved to an overlay layer with its own
/// Y range: two panels, each with its own cached bodies.
fn overlay_config() -> Config {
    let mut c = chart_config();
    c.overlays.push(overlay(0.05, 1.5, "vec"));
    c.overlays[0].y_axis.out_margin = 18.0;
    c
}

/// The panel with the filled line and the error-bar line stacked as offset
/// spectra (`Config::line_offsets`): each member is a panel of its own with
/// its own cached bodies, drawn at its shift.
fn offset_config() -> Config {
    let mut c = chart_config();
    c.line_offsets = Some(crate::data_config::LineOffsets {
        series: vec!["err".into(), "line".into()],
        step_y: 0.1,
        step_x: 0.05,
    });
    c
}

/// The panel in the upper half of a taller surface and a subplot in the
/// lower half drawing the histogram on its own axes.
fn subplot_config() -> (Config, (u32, u32)) {
    let mut c = chart_config();
    let mut chart = c.clone();
    chart.colorbar = None;
    chart.chart_area = ChartArea(Rect {
        x: 0,
        y: SIZE.1,
        width: SIZE.0,
        height: SIZE.1,
    });
    chart.chart_title.text = text("Below");
    chart.legend.visible = false;
    c.subplots.push(crate::Subplot {
        config: chart,
        series: vec!["hist".into()],
        link_x: None,
        link_y: None,
    });
    (c, (SIZE.0, SIZE.1 * 2))
}

/// The panel with a categorical bottom axis whose line crosses the value
/// axis at zero, and stacked category bars over it.
fn category_config() -> Config {
    let mut c = chart_config();
    c.colorbar = None;
    c.bottom_x.categories = Some(vec![
        crate::config::Category::new("a", "A"),
        crate::config::Category::new("b", "B"),
        crate::config::Category::new("c", "C"),
    ]);
    c.bottom_x.cross_at = Some(0.0);
    c.left_y.min = -1.0;
    c.left_y.max = 2.0;
    c.bar_groups = Some(crate::data_config::BarGroups {
        mode: crate::data_config::CategoryBarMode::Stacked,
        direction: crate::data_config::CategoryBarDirection::Vertical,
        group_width: 0.8,
        bar_gap: 2.0,
    });
    c
}

/// Two category bar series of one group: a Matte series with an outline
/// and rounded corners, and a flat one with a restyled, recoloured bar.
fn category_series() -> Vec<SeriesConfig> {
    use crate::data_config::{CategoryBarOverride, CategoryBarStyle, CategoryBarTarget};
    let bars = |id: &str, values: &str, color: Color, style: CategoryBarStyle| {
        declared(
            id,
            "cpos",
            values,
            DataRenderType::CategoryBars {
                color,
                style,
                labels: Default::default(),
                label_format: Default::default(),
                overrides: None,
            },
        )
    };
    let mut first = bars(
        "bars",
        "cva",
        Color::new(0.2, 0.4, 0.8, 1.0),
        CategoryBarStyle {
            material: crate::Material::Matte,
            corner_radius: 4.0,
            outline: true,
            outline_width: 1.5,
            texture_strength: 0.6,
            ..CategoryBarStyle::default()
        },
    );
    let mut second = bars(
        "more",
        "cvb",
        Color::new(0.9, 0.5, 0.1, 1.0),
        CategoryBarStyle::default(),
    );
    let DataRenderType::CategoryBars { overrides, .. } = &mut second.render_type else {
        unreachable!()
    };
    *overrides = Some(vec![CategoryBarOverride {
        target: CategoryBarTarget::new("c", "more"),
        color: Some(Color::new(0.1, 0.7, 0.3, 1.0)),
        style: Some(CategoryBarStyle {
            corner_radius: 6.0,
            outline: true,
            ..CategoryBarStyle::default()
        }),
        labels: None,
        label_format: None,
    }]);
    let DataRenderType::CategoryBars { labels, .. } = &mut first.render_type else {
        unreachable!()
    };
    labels.text = vec!["0.6".into(), "-0.4".into(), "0.9".into()];
    vec![first, second]
}

/// The category panel with grouped boxes over it.
fn box_config() -> Config {
    let mut c = category_config();
    c.bottom_x.cross_at = None;
    c.left_y.min = -0.2;
    c.left_y.max = 1.4;
    c.bar_groups = Some(crate::data_config::BarGroups {
        mode: crate::data_config::CategoryBarMode::Grouped,
        direction: crate::data_config::CategoryBarDirection::Vertical,
        group_width: 0.8,
        bar_gap: 2.0,
    });
    c
}

/// Two box plot series of one group: a notched Matte series with a mean,
/// outliers and an outline, and a flat one with a restyled, recoloured box.
fn box_series() -> Vec<SeriesConfig> {
    use crate::data_config::{BoxPlotOverride, BoxPlotStyle, BoxPlotTarget};
    let boxes = |id: &str, record: &str, color: Color, style: BoxPlotStyle| {
        declared(
            id,
            "cpos",
            record,
            DataRenderType::BoxPlot {
                color,
                style,
                labels: Default::default(),
                outliers: None,
                overrides: None,
                violin: None,
                points: None,
            },
        )
    };
    let mut first = boxes(
        "boxes",
        "rec_a",
        Color::new(0.2, 0.4, 0.8, 1.0),
        BoxPlotStyle {
            material: crate::Material::Matte,
            notched: true,
            show_mean: true,
            corner_radius: 2.0,
            texture_strength: 0.6,
            ..BoxPlotStyle::default()
        },
    );
    let DataRenderType::BoxPlot {
        outliers, labels, ..
    } = &mut first.render_type
    else {
        unreachable!()
    };
    *outliers = Some("out_a".into());
    labels.text = vec!["0.5".into(), "0.6".into(), "0.45".into()];
    let mut second = boxes(
        "more",
        "rec_b",
        Color::new(0.9, 0.5, 0.1, 1.0),
        BoxPlotStyle {
            material: crate::Material::Flat,
            ..BoxPlotStyle::default()
        },
    );
    let DataRenderType::BoxPlot { overrides, .. } = &mut second.render_type else {
        unreachable!()
    };
    *overrides = Some(vec![BoxPlotOverride {
        target: BoxPlotTarget::new("c", "more"),
        color: Some(Color::new(0.1, 0.7, 0.3, 1.0)),
        style: Some(BoxPlotStyle {
            corner_radius: 3.0,
            show_outliers: false,
            ..BoxPlotStyle::default()
        }),
        labels: None,
    }]);
    vec![first, second]
}

/// A polar panel whose angle axis holds four categories, with a polygon
/// grid, and two radar series over it.
fn radar_config() -> Config {
    let mut c = chart_config();
    c.colorbar = None;
    c.bottom_x.categories = Some(vec![
        crate::config::Category::new("a", "A"),
        crate::config::Category::new("b", "B"),
        crate::config::Category::new("c", "C"),
        crate::config::Category::new("d", "D"),
    ]);
    c.left_y.min = 0.0;
    c.left_y.max = 1.0;
    c.polar = Some(crate::polar::PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        grid_shape: Some(crate::polar::GridShape::Polygon),
        ..Default::default()
    });
    c
}

/// A filled radar series with square markers and an outline-only one.
fn radar_series() -> Vec<SeriesConfig> {
    use crate::data_config::{RadarLabels, RadarMarker, RadarSeriesLabels, RadarSeriesStyle};
    let first = declared(
        "radar",
        "rcat",
        "rva",
        DataRenderType::Radar {
            color: Color::new(0.8, 0.2, 0.1, 1.0),
            style: RadarSeriesStyle {
                line_width_px: Some(2.0),
                fill: None,
                marker: Some(RadarMarker::Square),
                marker_size_px: Some(6.0),
                fill_opacity: Some(0.4),
            },
            labels: RadarSeriesLabels {
                placement: RadarLabels::Values,
                decimals: 1,
                value_suffix: String::new(),
                text: vec!["0.9".into(), "0.5".into(), "0.7".into(), "0.4".into()],
                offsets: vec![[1.0, -2.0]; 4],
            },
        },
    );
    let second = declared(
        "more",
        "rcat",
        "rvb",
        DataRenderType::Radar {
            color: Color::new(0.1, 0.3, 0.9, 1.0),
            style: RadarSeriesStyle {
                fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
                marker: Some(RadarMarker::Circle),
                ..RadarSeriesStyle::default()
            },
            labels: RadarSeriesLabels::default(),
        },
    );
    vec![first, second]
}

fn line_style(color: Color) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Dash,
        line_color: color,
        line_width: 2.0,
        fill: None,
        fill_color: None,
        interpolation: crate::data_config::LineInterpolation::Linear,
    }
}

fn scatter_style() -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: Color::new(0.1, 0.3, 0.9, 0.8),
        point_shape: crate::ScatterShape::Circle,
        point_size: 5.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn matrix() -> MatrixRef {
    MatrixRef {
        columns: (0..GRID).map(|i| format!("z{i}")).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    }
}

fn declared(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type,
    }
}

/// One panel drawing every resident body kind: a filled dashed line, an
/// error-bar series with mapped point colour and size, a histogram, a
/// labelled filled contour and a colour-mapped, data-scaled point vector.
fn chart_series() -> Vec<SeriesConfig> {
    let mut line = line_style(Color::new(0.8, 0.2, 0.1, 1.0));
    line.fill = Some(LineFill::ToBaseline(0.2));
    line.fill_color = Some(Color::new(0.8, 0.2, 0.1, 0.3));
    let mut scatter = scatter_style();
    scatter.point_color_map = Some(PointColorMap { column: "m".into() });
    scatter.point_size_map = Some(PointSizeMap {
        column: "m".into(),
        domain_min: 0.0,
        domain_max: 1.0,
        min_px: 2.0,
        max_px: 7.0,
        scale: SizeScale::Area,
    });
    let contour = ContourConfig {
        levels: vec![0.3, 0.6],
        line: line_style(Color::BLACK),
        per_level_color: None,
        labels: Some(ContourLabelConfig {
            visible: true,
            font_size: 9.0,
            color: Color::BLACK,
            format: crate::format::LabelFormat::Decimal,
            significant_digits: 2,
            spacing_px: 40.0,
            anchors: Vec::new(),
            bg_color: Some(Color::new(1.0, 1.0, 0.8, 1.0)),
            bg_padding_px: 1.0,
        }),
    };
    vec![
        declared("line", "x", "a", DataRenderType::Line { line }),
        declared(
            "err",
            "x",
            "c",
            DataRenderType::LineScatterErrorbarXY {
                scatter,
                line: line_style(Color::new(0.1, 0.6, 0.2, 1.0)),
                err_x: ErrorRef::Symmetric { column: "e".into() },
                err_y: ErrorRef::Asymmetric {
                    lower: "e".into(),
                    upper: "m".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::new(0.3, 0.3, 0.3, 1.0),
                    error_bar_width: 1.0,
                    error_bar_cap_size: 3.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        ),
        declared(
            "hist",
            "he",
            "hc",
            DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: Color::new(0.5, 0.5, 0.9, 0.4),
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 0.8,
                    orientation: BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
        ),
        declared(
            "field",
            "gx",
            "gy",
            DataRenderType::HeatmapContour {
                matrix: matrix(),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Flat,
                    opacity: 0.5,
                },
                contour,
                surface: None,
            },
        ),
        declared(
            "vec",
            "x",
            "b",
            DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                },
                style: VectorStyle {
                    color: Color::new(0.6, 0.1, 0.6, 1.0),
                    width_px: 1.5,
                    scale: VectorScale::Data { factor: 1.0 },
                    pivot: VectorPivot::Tail,
                    head_length_px: 4.0,
                    head_width_px: 4.0,
                    min_length_px: 0.0,
                    color_map: Some(VectorColorMap::Column("m".into())),
                },
            },
        ),
    ]
}

/// Make the bundled handwritten face resolvable by family name, so a
/// `label_font` edit to "Comic Neue" draws different glyphs on every host.
/// Other tests register the same bytes; registration is idempotent.
fn register_comic_neue() {
    crate::text_render::register_font_bytes(
        include_bytes!("../../fonts/ComicNeue-Regular.ttf").to_vec(),
    )
    .unwrap();
}

/// A renderer holding the chart, one view and the panel's bodies drawn.
struct Panel {
    r: Renderer,
    id: ChartId,
    view: ChartView,
    /// The view's panel rectangle (the parent chart's area).
    rect: Rect,
    /// The painted target, holding every panel of the document.
    size: (u32, u32),
}

impl Panel {
    fn new(config: &Config, series: &[SeriesConfig], size: (u32, u32)) -> Self {
        let mut r = renderer(wgpu::TextureFormat::Rgba8Unorm, 1);
        add_columns(&mut r);
        let id = r.register_chart(config.clone(), series.to_vec()).unwrap();
        let rect = config.chart_area.0;
        let view = r
            .create_chart_view(&Chart::new(config.clone()), rect)
            .unwrap();
        let mut panel = Self {
            r,
            id,
            view,
            rect,
            size,
        };
        panel.prepare();
        assert!(
            panel.serials().iter().all(Option::is_some),
            "every baseline panel caches its bodies"
        );
        panel
    }

    /// The draw serial of each panel's cached bodies, base panel first.
    fn serials(&self) -> Vec<Option<u64>> {
        std::iter::once(&self.view)
            .chain(&self.view.overlay_views)
            .map(|view| self.r.resident_display_serial(view))
            .collect()
    }

    fn refresh(&mut self) {
        let config = self.r.chart_config(self.id).unwrap().clone();
        self.r
            .refresh_axis(&mut self.view, &Chart::new(config), self.rect)
            .unwrap();
    }

    fn prepare(&mut self) -> PreparedFrame {
        prepare(&mut self.r, self.id, &self.view)
    }

    fn try_prepare(&mut self) -> Option<PreparedFrame> {
        self.r
            .prepare_registered(&[RegisteredChartDrawItem {
                chart_id: self.id,
                view: &self.view,
            }])
            .ok()
    }

    fn paint(&self, frame: &PreparedFrame) -> Vec<u8> {
        try_paint(&self.r, frame, 1, self.size).unwrap()
    }

    /// Apply `config` and `series`, report whether the kept bodies still
    /// match bodies drawn from the new state, and restore the baseline.
    fn check(
        &mut self,
        config: Config,
        series: Vec<SeriesConfig>,
        baseline: (&Config, &[SeriesConfig]),
    ) -> Outcome {
        if self.r.set_chart_state(self.id, config, series).is_err() {
            return Outcome::Rejected;
        }
        let outcome = self.compare();
        self.restore(baseline.0, baseline.1);
        outcome
    }

    fn compare(&mut self) -> Outcome {
        let drawn = self.serials();
        // A refresh or prepare that fails is the renderer rejecting the
        // edited state (validation that runs at draw time).
        let config = self.r.chart_config(self.id).unwrap().clone();
        if self
            .r
            .refresh_axis(&mut self.view, &Chart::new(config), self.rect)
            .is_err()
        {
            return Outcome::Rejected;
        }
        let Some(kept) = self.try_prepare() else {
            return Outcome::Rejected;
        };
        let now = self.serials();
        let kept_panels: Vec<usize> = (0..drawn.len().min(now.len()))
            .filter(|&i| now[i].is_some() && now[i] == drawn[i])
            .collect();
        if drawn.len() == now.len() && kept_panels.is_empty() {
            // Every panel's key changed and its bodies were drawn again.
            return Outcome::Held;
        }
        let cached = kept
            .items
            .iter()
            .any(|item| item.resident_display.is_some());
        let stale = self.paint(&kept);
        drop(kept);
        self.r.resident_displays.clear();
        let fresh = self.prepare();
        let redrawn = self.paint(&fresh);
        if stale == redrawn {
            Outcome::Held
        } else {
            Outcome::Gap(format!(
                "the key of panel(s) {kept_panels:?} matched (bodies kept, cache {}), and \
                 the kept bodies differ from bodies drawn for the edited state in {} bytes",
                if cached { "in use" } else { "absent" },
                stale.iter().zip(&redrawn).filter(|(a, b)| a != b).count()
            ))
        }
    }

    fn restore(&mut self, config: &Config, series: &[SeriesConfig]) {
        self.r
            .set_chart_state(self.id, config.clone(), series.to_vec())
            .unwrap();
        self.refresh();
        // Whatever the edit left in the cache, the next edit starts from
        // bodies drawn for the baseline.
        self.r.resident_displays.clear();
        self.prepare();
    }
}

fn run_suite(name: &str, config: Config, series: Vec<SeriesConfig>, size: (u32, u32)) -> Tally {
    let mut panel = Panel::new(&config, &series, size);
    let mut tally = run(&format!("{name} config: "), &config, STRINGS, |_, next| {
        panel.check(next, series.clone(), (&config, &series))
    });
    eprintln!("{}", tally.summary(&format!("resident/{name} config")));
    for index in 0..series.len() {
        let part = run(
            &format!("{name} series[{index}] ({}): ", series[index].series_id),
            &series[index],
            STRINGS,
            |_, next| {
                let mut list = series.clone();
                list[index] = next;
                panel.check(config.clone(), list, (&config, &series))
            },
        );
        eprintln!(
            "{}",
            part.summary(&format!("resident/{name} series[{index}]"))
        );
        tally.absorb(part);
    }
    tally
}

#[test]
fn every_leaf_that_changes_the_resident_bodies_changes_the_key() {
    let _fonts = font_lock();
    register_comic_neue();
    let mut tally = run_suite("panel", chart_config(), chart_series(), SIZE);
    tally.absorb(run_suite("overlay", overlay_config(), chart_series(), SIZE));
    tally.absorb(run_suite("offsets", offset_config(), chart_series(), SIZE));
    let (config, size) = subplot_config();
    tally.absorb(run_suite("subplot", config, chart_series(), size));
    tally.absorb(run_suite(
        "category",
        category_config(),
        category_series(),
        SIZE,
    ));
    tally.absorb(run_suite("box", box_config(), box_series(), SIZE));
    tally.absorb(run_suite("radar", radar_config(), radar_series(), SIZE));
    eprintln!("{}", tally.summary("resident (all)"));
    eprintln!("skipped:\n{}", tally.skipped.join("\n"));
    // Release the font lock before the verdict, so a confirmed gap fails
    // this test alone instead of poisoning the lock for every later test.
    drop(_fonts);
    assert!(
        tally.gaps.is_empty(),
        "{} leaves change the cached series bodies without changing ResidentDisplayKey:\n{}",
        tally.gaps.len(),
        tally.gaps.join("\n")
    );
}

// The fit-token mutations reuse this panel's columns and series.
#[path = "fit_mutation_tests.rs"]
mod fit;
