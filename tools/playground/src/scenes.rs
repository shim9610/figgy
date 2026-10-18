//! Scene catalogue: synthetic data and the starting `Config` / series of every
//! Cartesian scene. Bounded scenes build their models in `bounded.rs`.

use renderer::config::{Config, append_legend_entry, series_symbol_segments};
use renderer::data::Column;
use renderer::data_config::{
    BarOrientation, ContourConfig, ContourLabelConfig, DataBarStyleConfig, DataErrorBarStyleConfig,
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ErrorRef, FieldFillConfig,
    FillMode, GridLayout, LineFill, LineInterpolation, MatrixOrientation, MatrixRef, PointColorMap,
    PointSizeMap, ScatterShape, SeriesConfig, Shading, SizeScale, VectorColorMap, VectorComponents,
    VectorPivot, VectorScale, VectorStyle,
};
use renderer::format::LabelFormat;
use renderer::layout::{ChartArea, Rect};
use renderer::line::LineStylePreset;
use renderer::text::RichText;
use renderer::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle, ArrowStyle, CellRef,
    Chart, Color, ColorMap, ColumnSource, FiggyError, LayerRef, OverlayLayer, Renderer, Subplot,
    default,
};

/// Logical size of every Cartesian document; the window fits it uniformly.
pub const DOC_W: u32 = 1100;
pub const DOC_H: u32 = 720;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    Lines,
    Scatter,
    ValueMaps,
    Histogram,
    Field,
    VectorField,
    LogAxes,
    TwinAxes,
    Subplots,
    Annotations,
    BigData,
    Radial,
    Categorical,
    BoxPlot,
}

impl SceneKind {
    pub const ALL: [SceneKind; 14] = [
        SceneKind::Lines,
        SceneKind::Scatter,
        SceneKind::ValueMaps,
        SceneKind::Histogram,
        SceneKind::Field,
        SceneKind::VectorField,
        SceneKind::LogAxes,
        SceneKind::TwinAxes,
        SceneKind::Subplots,
        SceneKind::Annotations,
        SceneKind::BigData,
        SceneKind::Radial,
        SceneKind::Categorical,
        SceneKind::BoxPlot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SceneKind::Lines => "Lines · areas · steps",
            SceneKind::Scatter => "Scatter · error bars",
            SceneKind::ValueMaps => "Scatter · value maps",
            SceneKind::Histogram => "Histogram",
            SceneKind::Field => "Heatmap · contours",
            SceneKind::VectorField => "Vector field (quiver)",
            SceneKind::LogAxes => "Log axes",
            SceneKind::TwinAxes => "Twin Y axes (overlay)",
            SceneKind::Subplots => "Subplots (linked X)",
            SceneKind::Annotations => "Annotations",
            SceneKind::BigData => "1M-point line",
            SceneKind::Radial => "Radial (pie / donut)",
            SceneKind::Categorical => "Categorical bars",
            SceneKind::BoxPlot => "Box plot",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            SceneKind::Lines => "Area fill to baseline, a band between two series, step modes.",
            SceneKind::Scatter => "Marker shapes, XY error bars, a line through the points.",
            SceneKind::ValueMaps => {
                "Colour by a value column through the colourbar; size by another (bubbles). \
                 Pick a point to read both values. NaN sizes hide the point."
            }
            SceneKind::Histogram => "Pre-binned edges + counts. Try horizontal orientation.",
            SceneKind::Field => "Interpolated field, labelled isolines and a colourbar.",
            SceneKind::VectorField => {
                "Arrows from (u, v) grids over a speed heatmap, coloured through the \
                 colourbar, plus pixel-scaled probe arrows. Pick an arrow or a cell to \
                 read u / v."
            }
            SceneKind::LogAxes => "Logarithmic Y with decade ticks and dotted minor grid.",
            SceneKind::TwinAxes => "Switch the active layer to Overlay 0 to edit the right axis.",
            SceneKind::Subplots => {
                "Upper-right X is linked to the parent. Activate a subplot to edit it."
            }
            SceneKind::Annotations => {
                "Active layer = Annotations: drag lines, spans, text, arrows."
            }
            SceneKind::BigData => "One million points, every one drawn. Zoom and pan it.",
            SceneKind::Radial => "Click a slice; drag the title or legend.",
            SceneKind::Categorical => "Click a bar; grouped / stacked / percent modes.",
            SceneKind::BoxPlot => "Click a box; notches, means, outliers.",
        }
    }

    pub fn is_bounded(self) -> bool {
        matches!(
            self,
            SceneKind::Radial | SceneKind::Categorical | SceneKind::BoxPlot
        )
    }
}

pub struct CartesianScene {
    pub config: Config,
    pub series: Vec<SeriesConfig>,
}

// ---------------------------------------------------------------------------
// Data helpers
// ---------------------------------------------------------------------------

fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) -> Result<(), FiggyError> {
    if r.handle_for(id).is_ok() {
        return Ok(());
    }
    let (min, max) = data
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    r.add_column(id, &Column { data, min, max }).map(|_| ())
}

/// Deterministic pseudo-random numbers in [-1, 1] (no rand dependency).
fn noise(seed: u64, i: usize) -> f64 {
    let mut x = seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64) * 2.0 - 1.0
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
        .collect()
}

pub fn line_style(color: Color, width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: LineStylePreset::Solid,
        line_color: color,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

pub fn scatter_style(color: Color, shape: ScatterShape, size: f32) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: color,
        point_shape: shape,
        point_size: size,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type,
    }
}

fn base_config(title: &str, x_title: &str, y_title: &str) -> Config {
    let mut config = default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: DOC_W,
        height: DOC_H,
    });
    Chart::new(config)
        .with_title(title)
        .with_x_title(x_title)
        .with_y_title(y_title)
        .config()
        .clone()
}

/// Rebuild the parent legend from the series the parent draws (those no
/// subplot claims), and each visible subplot legend from its own series.
pub fn rebuild_legends(config: &mut Config, series: &[SeriesConfig]) {
    let claimed: Vec<&str> = config
        .subplots
        .iter()
        .flat_map(|s| s.series.iter().map(String::as_str))
        .collect();
    let fill = |content: &mut RichText, ids: &mut dyn Iterator<Item = &SeriesConfig>| {
        content.segments.clear();
        for s in ids {
            append_legend_entry(content, series_symbol_segments(s), &s.series_id);
        }
    };
    fill(
        &mut config.legend.content,
        &mut series
            .iter()
            .filter(|s| !claimed.contains(&s.series_id.as_str())),
    );
    for sub in &mut config.subplots {
        if sub.config.legend.visible {
            let own = sub.series.clone();
            fill(
                &mut sub.config.legend.content,
                &mut series.iter().filter(|s| own.contains(&s.series_id)),
            );
        }
    }
}

fn with_legend(mut config: Config, series: &[SeriesConfig]) -> Config {
    config.legend.visible = true;
    rebuild_legends(&mut config, series);
    config
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

pub fn build(r: &mut Renderer, kind: SceneKind) -> Result<CartesianScene, FiggyError> {
    match kind {
        SceneKind::Lines => lines(r),
        SceneKind::Scatter => scatter(r),
        SceneKind::ValueMaps => value_maps(r),
        SceneKind::Histogram => histogram(r),
        SceneKind::Field => field(r),
        SceneKind::VectorField => vector_field(r),
        SceneKind::LogAxes => log_axes(r),
        SceneKind::TwinAxes => twin_axes(r),
        SceneKind::Subplots => subplots(r),
        SceneKind::Annotations => annotations(r),
        SceneKind::BigData => big_data(r),
        _ => unreachable!("bounded scenes are built in bounded.rs"),
    }
}

fn lines(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let x = linspace(0.0, 10.0, 400);
    upload(r, "ln_x", x.clone())?;
    upload(r, "ln_sin", x.iter().map(|v| v.sin()).collect())?;
    upload(
        r,
        "ln_cos",
        x.iter().map(|v| 0.8 * (v * 0.7).cos()).collect(),
    )?;
    let mid: Vec<f64> = x.iter().map(|v| -1.6 + 0.3 * (v * 0.9).sin()).collect();
    upload(
        r,
        "ln_hi",
        mid.iter()
            .zip(&x)
            .map(|(m, v)| m + 0.25 + 0.1 * (v * 2.0).cos())
            .collect(),
    )?;
    upload(
        r,
        "ln_lo",
        mid.iter()
            .zip(&x)
            .map(|(m, v)| m - 0.25 - 0.1 * (v * 2.0).cos())
            .collect(),
    )?;
    let sx = linspace(0.0, 10.0, 11);
    upload(r, "ln_step_x", sx.clone())?;
    upload(
        r,
        "ln_step_y",
        (0..sx.len()).map(|i| 1.6 + 0.4 * noise(7, i)).collect(),
    )?;

    let blue = Color::from_rgb8(31, 119, 180);
    let orange = Color::from_rgb8(255, 127, 14);
    let green = Color::from_rgb8(44, 160, 44);
    let purple = Color::from_rgb8(148, 103, 189);
    let mut area = line_style(blue, 2.0);
    area.fill = Some(LineFill::ToBaseline(0.0));
    let mut dashed = line_style(orange, 2.0);
    dashed.line_style = LineStylePreset::Dash;
    let mut band = line_style(green, 1.5);
    band.fill = Some(LineFill::ToSeries("band_lo".into()));
    let mut step = line_style(purple, 2.5);
    step.interpolation = LineInterpolation::StepMid;
    let series = vec![
        series(
            "sin_area",
            "ln_x",
            "ln_sin",
            DataRenderType::Line { line: area },
        ),
        series(
            "cos_dashed",
            "ln_x",
            "ln_cos",
            DataRenderType::Line { line: dashed },
        ),
        series(
            "band_hi",
            "ln_x",
            "ln_hi",
            DataRenderType::Line { line: band },
        ),
        series(
            "band_lo",
            "ln_x",
            "ln_lo",
            DataRenderType::Line {
                line: line_style(green, 1.5),
            },
        ),
        series(
            "step_mid",
            "ln_step_x",
            "ln_step_y",
            DataRenderType::Line { line: step },
        ),
    ];
    let mut chart = Chart::new(base_config("Lines, areas and steps", "x", "y"));
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-2.2, 2.4);
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn scatter(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let n = 25;
    let x: Vec<f64> = (0..n).map(|i| i as f64 * 0.8).collect();
    let y: Vec<f64> = x
        .iter()
        .enumerate()
        .map(|(i, v)| 2.0 + 0.5 * v + 0.8 * noise(1, i))
        .collect();
    upload(r, "sc_x", x.clone())?;
    upload(r, "sc_y", y)?;
    upload(
        r,
        "sc_ex",
        (0..n).map(|i| 0.15 + 0.1 * noise(2, i).abs()).collect(),
    )?;
    upload(
        r,
        "sc_ey",
        (0..n).map(|i| 0.3 + 0.4 * noise(3, i).abs()).collect(),
    )?;
    upload(r, "sc_fit", x.iter().map(|v| 2.0 + 0.5 * v).collect())?;
    let cloud = 600;
    upload(
        r,
        "sc_cx",
        (0..cloud)
            .map(|i| 10.0 + 4.0 * noise(4, i) * noise(5, i).abs().sqrt())
            .collect(),
    )?;
    upload(
        r,
        "sc_cy",
        (0..cloud)
            .map(|i| 4.0 + 2.5 * noise(6, i) * noise(8, i).abs().sqrt())
            .collect(),
    )?;

    let red = Color::from_rgb8(214, 39, 40);
    let teal = Color::from_rgb8(23, 190, 207);
    let gray = Color::from_rgb8(90, 90, 90);
    let err = DataErrorBarStyleConfig {
        error_bar_color: red,
        error_bar_width: 1.2,
        error_bar_cap_size: 6.0,
        cap_width: 1.2,
        error_bar_style_table: None,
        error_bar_style_index_column: None,
        error_bar_style_overrides: None,
    };
    let mut fit = line_style(gray, 1.5);
    fit.line_style = LineStylePreset::DashDot;
    let series = vec![
        series(
            "cloud",
            "sc_cx",
            "sc_cy",
            DataRenderType::Scatter {
                scatter: scatter_style(
                    Color::from_rgba(0.09, 0.75, 0.81, 0.55),
                    ScatterShape::DiamondFilled,
                    5.0,
                ),
            },
        ),
        series("fit", "sc_x", "sc_fit", DataRenderType::Line { line: fit }),
        series(
            "measured",
            "sc_x",
            "sc_y",
            DataRenderType::ScatterErrorbarXY {
                scatter: scatter_style(red, ScatterShape::CircleFilled, 7.0),
                err_x: ErrorRef::Symmetric {
                    column: "sc_ex".into(),
                },
                err_y: ErrorRef::Symmetric {
                    column: "sc_ey".into(),
                },
                err_style: err,
            },
        ),
    ];
    let _ = teal;
    let mut chart = Chart::new(base_config("Scatter and error bars", "dose", "response"));
    chart.auto_fit_x_union(r.pool(), &["sc_x", "sc_cx"], 0.05)?;
    chart.auto_fit_y_union(r.pool(), &["sc_y", "sc_cy"], 0.1)?;
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

/// Two clouds on one colourbar. The left cloud is coloured by `depth` at a
/// fixed size; the right cloud is coloured by `depth` and sized by
/// `amplitude` (a bubble chart). The size domain comes from the amplitude
/// statistics this scene already has, never from the renderer.
fn value_maps(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let n = 320;
    let depth = |x: f64, y: f64| 10.0 * y + 4.0 * (x - 5.0).abs();
    let cloud = |seed: u64, cx: f64| -> (Vec<f64>, Vec<f64>) {
        let x: Vec<f64> = (0..n).map(|i| cx + 1.8 * noise(seed, i)).collect();
        let y: Vec<f64> = (0..n).map(|i| 5.0 + 4.2 * noise(seed + 1, i)).collect();
        (x, y)
    };
    let (x1, y1) = cloud(31, 2.6);
    let (x2, y2) = cloud(41, 7.4);
    let depth1: Vec<f64> = x1.iter().zip(&y1).map(|(x, y)| depth(*x, *y)).collect();
    let depth2: Vec<f64> = x2.iter().zip(&y2).map(|(x, y)| depth(*x, *y)).collect();
    // Every 40th amplitude is NaN: a non-finite size hides the point and
    // removes it from picking.
    let amplitude: Vec<f64> = (0..n)
        .map(|i| {
            if i % 40 == 20 {
                f64::NAN
            } else {
                1.0 + 9.0 * noise(43, i).abs()
            }
        })
        .collect();
    let (amp_min, amp_max) = amplitude
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    upload(r, "vm_x1", x1)?;
    upload(r, "vm_y1", y1)?;
    upload(r, "vm_depth1", depth1)?;
    upload(r, "vm_x2", x2)?;
    upload(r, "vm_y2", y2)?;
    upload(r, "vm_depth2", depth2)?;
    upload(r, "vm_amp", amplitude)?;

    let mut coloured = scatter_style(
        Color::from_rgb8(31, 119, 180),
        ScatterShape::CircleFilled,
        7.0,
    );
    coloured.point_color_map = Some(PointColorMap {
        column: "vm_depth1".into(),
    });
    let mut bubbles = scatter_style(
        Color::from_rgba(0.13, 0.41, 0.78, 0.6),
        ScatterShape::CircleFilled,
        6.0,
    );
    bubbles.point_color_map = Some(PointColorMap {
        column: "vm_depth2".into(),
    });
    let mut size_map = PointSizeMap {
        column: "vm_amp".into(),
        domain_min: 0.0,
        domain_max: 1.0,
        min_px: 2.0,
        max_px: 16.0,
        scale: SizeScale::Area,
    };
    size_map.fit_domain(amp_min, amp_max);
    bubbles.point_size_map = Some(size_map);
    let series = vec![
        series(
            "depth",
            "vm_x1",
            "vm_y1",
            DataRenderType::Scatter { scatter: coloured },
        ),
        series(
            "bubbles",
            "vm_x2",
            "vm_y2",
            DataRenderType::Scatter { scatter: bubbles },
        ),
    ];
    let mut config = base_config("Scatter: colour and size value maps", "x", "y");
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    config.colorbar = Some(bar);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 10.0);
    chart.auto_fit_colorbar_union(r.pool(), &["vm_depth1", "vm_depth2"], 0.0)?;
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn histogram(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let bins = 30;
    let edges = linspace(-3.0, 3.0, bins + 1);
    let mut counts = vec![0.0; bins];
    for i in 0..5000 {
        // Sum of uniforms ≈ normal.
        let v: f64 = (0..4).map(|k| noise(10 + k, i)).sum::<f64>() * 0.85;
        let b = ((v + 3.0) / 6.0 * bins as f64).floor();
        if b >= 0.0 && (b as usize) < bins {
            counts[b as usize] += 1.0;
        }
    }
    upload(r, "hist_edges", edges)?;
    upload(r, "hist_counts", counts)?;
    let bar = DataBarStyleConfig {
        fill_color: Color::from_rgb8(86, 140, 200),
        border_color: Color::from_rgb8(30, 60, 110),
        border_width: 1.0,
        baseline: 0.0,
        gap_px: 1.0,
        width_ratio: 1.0,
        orientation: BarOrientation::Vertical,
        bar_style_overrides: None,
    };
    let series = vec![series(
        "samples",
        "hist_edges",
        "hist_counts",
        DataRenderType::Histogram { bar },
    )];
    let mut chart = Chart::new(base_config("Histogram of 5000 samples", "value", "count"));
    chart.set_x_range(-3.0, 3.0);
    chart.auto_fit_y(r.pool(), "hist_counts", 0.08)?;
    chart.with_axis_range_change(|c| {
        c.left_y.min = 0.0;
        c.right_y.min = 0.0;
    });
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn field(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    const G: usize = 48;
    let coords = linspace(0.0, 1.0, G);
    let z = |x: f64, y: f64| {
        let a = (-((x - 0.35).powi(2) + (y - 0.6).powi(2)) / 0.03).exp();
        let b = 0.7 * (-((x - 0.7).powi(2) + (y - 0.3).powi(2)) / 0.02).exp();
        a + b + 0.1 * (6.0 * x).sin() * (5.0 * y).cos()
    };
    let ids: Vec<String> = (0..G).map(|c| format!("fld_z{c}")).collect();
    if r.handle_for("fld_x").is_err() {
        let grid: Vec<Column<f64>> = coords
            .iter()
            .map(|x| {
                let data: Vec<f64> = coords.iter().map(|y| z(*x, *y)).collect();
                let (min, max) = data
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                        (l.min(*v), h.max(*v))
                    });
                Column { data, min, max }
            })
            .collect();
        let coord = Column {
            data: coords.clone(),
            min: 0.0,
            max: 1.0,
        };
        let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![("fld_x", &coord), ("fld_y", &coord)];
        for (id, column) in ids.iter().zip(&grid) {
            batch.push((id.as_str(), column as &dyn ColumnSource));
        }
        r.add_columns(&batch)?;
    }
    let mut config = base_config("Field: heatmap + labelled contours", "x", "y");
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    let mut bar = default::default_colorbar_options();
    bar.axis.min = -0.2;
    bar.axis.max = 1.2;
    bar.axis.major_spacing = 0.2;
    config.colorbar = Some(bar);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 1.0);
    chart.set_y_range(0.0, 1.0);
    let ink = Color::from_rgb8(20, 20, 20);
    let series = vec![series(
        "field",
        "fld_x",
        "fld_y",
        DataRenderType::HeatmapContour {
            matrix: MatrixRef {
                columns: ids,
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout: GridLayout::Centers,
            },
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 1.0,
            },
            contour: ContourConfig {
                levels: vec![0.0, 0.2, 0.4, 0.6, 0.8],
                line: line_style(ink, 1.2),
                per_level_color: None,
                labels: Some(ContourLabelConfig {
                    visible: true,
                    font_size: 11.0,
                    color: ink,
                    format: LabelFormat::Decimal,
                    significant_digits: 1,
                    spacing_px: 140.0,
                    anchors: Vec::new(),
                    bg_color: None,
                    bg_padding_px: 2.0,
                }),
            },
        },
    )];
    Ok(CartesianScene {
        config: chart.config().clone(),
        series,
    })
}

fn vector_field(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    // A vortex with a mild drift, sampled on a 24 x 16 grid: the u / v
    // components and the speed are three matrices over the same coordinates.
    const NX: usize = 24;
    const NY: usize = 16;
    let xs = linspace(0.0, 10.0, NX);
    let ys = linspace(0.0, 6.0, NY);
    let flow = |x: f64, y: f64| {
        let (dx, dy) = (x - 5.0, y - 3.0);
        let r2 = dx * dx + dy * dy;
        (-dy / (1.0 + 0.3 * r2) + 0.3, dx / (1.0 + 0.3 * r2))
    };
    let u_ids: Vec<String> = (0..NX).map(|c| format!("vf_u{c}")).collect();
    let v_ids: Vec<String> = (0..NX).map(|c| format!("vf_v{c}")).collect();
    let s_ids: Vec<String> = (0..NX).map(|c| format!("vf_s{c}")).collect();
    // Twelve probe anchors on a ring, with their own (u, v) columns. Their
    // bounds size the probe arrows: host statistics, not a renderer read.
    let probes: Vec<(f64, f64)> = (0..12)
        .map(|i| {
            let a = i as f64 / 12.0 * std::f64::consts::TAU;
            (5.0 + 3.2 * a.cos(), 3.0 + 2.0 * a.sin())
        })
        .collect();
    let probe_uv: Vec<(f64, f64)> = probes.iter().map(|p| flow(p.0, p.1)).collect();
    let bounds = |pick: fn(&(f64, f64)) -> f64| {
        probe_uv
            .iter()
            .map(pick)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                (l.min(v), h.max(v))
            })
    };
    if r.handle_for("vf_x").is_err() {
        let column = |data: Vec<f64>| {
            let (min, max) = data
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                    (l.min(*v), h.max(*v))
                });
            Column { data, min, max }
        };
        let mut grids = Vec::new();
        for x in &xs {
            let samples: Vec<(f64, f64)> = ys.iter().map(|y| flow(*x, *y)).collect();
            grids.push(column(samples.iter().map(|s| s.0).collect()));
            grids.push(column(samples.iter().map(|s| s.1).collect()));
            grids.push(column(samples.iter().map(|s| s.0.hypot(s.1)).collect()));
        }
        let gx = column(xs.clone());
        let gy = column(ys.clone());
        let px = column(probes.iter().map(|p| p.0).collect());
        let py = column(probes.iter().map(|p| p.1).collect());
        let pu = column(probe_uv.iter().map(|p| p.0).collect());
        let pv = column(probe_uv.iter().map(|p| p.1).collect());
        let mut batch: Vec<(&str, &dyn ColumnSource)> = vec![
            ("vf_x", &gx),
            ("vf_y", &gy),
            ("vf_px", &px),
            ("vf_py", &py),
            ("vf_pu", &pu),
            ("vf_pv", &pv),
        ];
        for (c, chunk) in grids.chunks(3).enumerate() {
            batch.push((u_ids[c].as_str(), &chunk[0] as &dyn ColumnSource));
            batch.push((v_ids[c].as_str(), &chunk[1] as &dyn ColumnSource));
            batch.push((s_ids[c].as_str(), &chunk[2] as &dyn ColumnSource));
        }
        r.add_columns(&batch)?;
    }
    let grid = |ids: &[String]| MatrixRef {
        columns: ids.to_vec(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    };
    let speed = series(
        "speed",
        "vf_x",
        "vf_y",
        DataRenderType::Heatmap {
            matrix: grid(&s_ids),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Interpolated,
                opacity: 0.3,
            },
        },
    );
    // The gradient grid: data-scaled, so the tips follow the axes when the
    // view pans or zooms, and coloured by the speed grid.
    let quiver = series(
        "quiver",
        "vf_x",
        "vf_y",
        DataRenderType::Vector {
            components: VectorComponents::Grid {
                u: grid(&u_ids),
                v: grid(&v_ids),
            },
            style: VectorStyle {
                color: Color::from_rgb8(20, 20, 20),
                width_px: 1.6,
                scale: VectorScale::Data { factor: 0.3 },
                pivot: VectorPivot::Tail,
                head_length_px: 7.0,
                head_width_px: 6.0,
                min_length_px: 2.0,
                color_map: Some(VectorColorMap::Grid(grid(&s_ids))),
            },
        },
    );
    // Probe arrows: pixel-scaled, so they keep their size under zoom.
    let mut probe_style = VectorStyle {
        color: Color::from_rgb8(200, 40, 60),
        width_px: 2.5,
        scale: VectorScale::Pixels { px_per_unit: 1.0 },
        pivot: VectorPivot::Middle,
        head_length_px: 10.0,
        head_width_px: 9.0,
        min_length_px: 0.0,
        color_map: None,
    };
    probe_style.fit_scale(bounds(|p| p.0), bounds(|p| p.1), 48.0);
    let probes = series(
        "probes",
        "vf_px",
        "vf_py",
        DataRenderType::Vector {
            components: VectorComponents::Points {
                u: "vf_pu".into(),
                v: "vf_pv".into(),
            },
            style: probe_style,
        },
    );
    let series = vec![speed, quiver, probes];
    let mut config = base_config("Vector field: vortex flow", "x", "y");
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    config.colorbar = Some(bar);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(0.0, 6.0);
    let speed_refs: Vec<&str> = s_ids.iter().map(String::as_str).collect();
    chart.auto_fit_colorbar_union(r.pool(), &speed_refs, 0.0)?;
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn log_axes(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let (es, sigmas) = renderer::demo::cross_section_data(1024);
    upload(r, "log_e", es)?;
    upload(r, "log_sigma", sigmas)?;
    let mut config = base_config("Cross-section σ(E)", "E [keV]", "σ [barn]");
    config.left_y.scale = renderer::config::AxisScale::Logarithmic;
    config.right_y.scale = renderer::config::AxisScale::Logarithmic;
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;
    let mut chart = Chart::new(config);
    chart.auto_fit_x(r.pool(), "log_e", 0.0)?;
    chart.auto_fit_y(r.pool(), "log_sigma", 0.1)?;
    let series = vec![series(
        "sigma",
        "log_e",
        "log_sigma",
        DataRenderType::Line {
            line: line_style(Color::from_rgb8(60, 160, 90), 2.5),
        },
    )];
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn day_columns(r: &mut Renderer) -> Result<(), FiggyError> {
    let hour = linspace(0.0, 24.0, 97);
    let wave = |phase: f64| -> Vec<f64> {
        hour.iter()
            .map(|h| (h / 24.0 * std::f64::consts::TAU + phase).sin())
            .collect()
    };
    upload(r, "day_hour", hour.clone())?;
    upload(
        r,
        "day_temp",
        wave(-1.2).iter().map(|v| 18.0 + 8.0 * v).collect(),
    )?;
    upload(
        r,
        "day_press",
        wave(0.0)
            .iter()
            .enumerate()
            .map(|(i, v)| 1008.0 + 6.0 * v + 0.6 * noise(20, i))
            .collect(),
    )?;
    upload(
        r,
        "day_hum",
        wave(2.0).iter().map(|v| 60.0 + 25.0 * v).collect(),
    )?;
    Ok(())
}

fn twin_axes(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    day_columns(r)?;
    let mut config = base_config("Temperature and pressure", "Hour", "Temperature (°C)");
    config.right_y.line_visible = false;
    config.right_y.tick = renderer::config::TickVisibility::None;
    let rust = Color::from_rgb8(176, 64, 32);
    let mut overlay = OverlayLayer {
        series: vec!["pressure".into()],
        ..Default::default()
    };
    overlay.y_axis.out_margin = 110.0;
    overlay.y_axis.title_option.visible = true;
    overlay.y_axis.title_option.text = RichText::plain("Pressure (hPa)", rust, 18.0, "");
    overlay.y_axis.label_style.color = rust;
    overlay.y_axis.line_color = rust;
    config.overlays.push(overlay);
    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(r.pool(), "day_temp", 0.1)?;
    chart.auto_fit_overlay_y_union(r.pool(), 0, &["day_press"], 0.1)?;
    let series = vec![
        series(
            "temperature",
            "day_hour",
            "day_temp",
            DataRenderType::Line {
                line: line_style(Color::from_rgb8(34, 104, 198), 2.5),
            },
        ),
        series(
            "pressure",
            "day_hour",
            "day_press",
            DataRenderType::ScatterLine {
                scatter: scatter_style(rust, ScatterShape::Circle, 5.0),
                line: line_style(rust, 1.5),
            },
        ),
    ];
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

fn subplots(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    day_columns(r)?;
    let (w, h) = (DOC_W / 2, DOC_H / 2);
    let cell = |x: u32, y: u32, title: &str, xt: &str, yt: &str| -> Config {
        let mut c = base_config(title, xt, yt);
        c.chart_area = ChartArea(Rect {
            x,
            y,
            width: w,
            height: h,
        });
        c
    };
    let sub = |config: Config, ids: &[&str]| Subplot {
        config,
        series: ids.iter().map(|s| s.to_string()).collect(),
        link_x: None,
        link_y: None,
    };
    let rust = Color::from_rgb8(176, 64, 32);
    let green = Color::from_rgb8(40, 140, 90);
    let mut config = cell(0, 0, "Temperature", "Hour", "°C");
    let mut pressure = sub(
        cell(w, 0, "Pressure (X linked)", "Hour", "hPa"),
        &["pressure"],
    );
    pressure.link_x = Some(CellRef::Parent);
    let humid = sub(
        cell(0, h, "Humidity vs temperature", "°C", "%"),
        &["humidity_t"],
    );
    let mut day = sub(
        cell(w, h, "Daytime + twin axis", "Hour", "hPa"),
        &["daytime"],
    );
    let mut twin = OverlayLayer {
        series: vec!["daytime_humidity".into()],
        ..Default::default()
    };
    twin.y_axis.out_margin = 90.0;
    twin.y_axis.title_option.visible = true;
    twin.y_axis.title_option.text = RichText::plain("%", green, 18.0, "");
    twin.y_axis.label_style.color = green;
    day.config.overlays.push(twin);
    config.subplots = vec![pressure, humid, day];

    let mut chart = Chart::new(config);
    chart.set_x_range(0.0, 24.0);
    chart.auto_fit_y(r.pool(), "day_temp", 0.1)?;
    chart.auto_fit_subplot_y_union(r.pool(), 0, &["day_press"], 0.1)?;
    chart.auto_fit_subplot_x_union(r.pool(), 1, &["day_temp"], 0.05)?;
    chart.auto_fit_subplot_y_union(r.pool(), 1, &["day_hum"], 0.1)?;
    chart.set_subplot_x_range(2, 6.0, 18.0);
    chart.auto_fit_subplot_y_union(r.pool(), 2, &["day_press"], 0.1)?;
    chart.with_axis_range_change(|c| {
        let y = &mut c.subplots[2].config.overlays[0].y_axis;
        y.min = 30.0;
        y.max = 90.0;
        y.major_spacing = 10.0;
    });
    let series = vec![
        series(
            "temperature",
            "day_hour",
            "day_temp",
            DataRenderType::Line {
                line: line_style(Color::from_rgb8(34, 104, 198), 2.5),
            },
        ),
        series(
            "pressure",
            "day_hour",
            "day_press",
            DataRenderType::Line {
                line: line_style(rust, 2.0),
            },
        ),
        series(
            "humidity_t",
            "day_temp",
            "day_hum",
            DataRenderType::Scatter {
                scatter: scatter_style(green, ScatterShape::TriangleFilled, 6.0),
            },
        ),
        series(
            "daytime",
            "day_hour",
            "day_press",
            DataRenderType::Line {
                line: line_style(rust, 2.0),
            },
        ),
        series(
            "daytime_humidity",
            "day_hour",
            "day_hum",
            DataRenderType::Line {
                line: line_style(green, 2.0),
            },
        ),
    ];
    Ok(CartesianScene {
        config: chart.config().clone(),
        series,
    })
}

fn annotations(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let (ts, vs) = renderer::demo::rc_data(512);
    upload(r, "ann_t", ts)?;
    upload(r, "ann_v", vs)?;
    let mut chart = Chart::new(base_config(
        "RC charging with annotations",
        "t [s]",
        "V [V]",
    ));
    chart.auto_fit_x(r.pool(), "ann_t", 0.02)?;
    chart.auto_fit_y(r.pool(), "ann_v", 0.08)?;
    let (x0, x1) = (chart.config().bottom_x.min, chart.config().bottom_x.max);
    let (y0, y1) = (chart.config().left_y.min, chart.config().left_y.max);
    let red = Color::from_rgb8(200, 40, 40);
    let items = vec![
        Annotation::new(AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: x0 + 0.1 * (x1 - x0),
            to: x0 + 0.25 * (x1 - x0),
            fill: Color::from_rgba(1.0, 0.8, 0.2, 0.3),
        }),
        Annotation::new(AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::Y,
            value: y0 + 0.63 * (y1 - y0),
            style: AnnotationLineStyle {
                color: red,
                width: 1.5,
                style: LineStylePreset::Dash,
            },
            label: Some(RichText::plain("63 % (τ)", red, 14.0, "")),
        }),
        Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 0.55, y: 0.35 },
            text: RichText::plain("drag me", Color::BLACK, 16.0, ""),
            box_style: Some(Default::default()),
        }),
        Annotation::new(AnnotationKind::Arrow {
            from: Anchor::Ratio { x: 0.55, y: 0.45 },
            to: Anchor::Data {
                x: x0 + 0.2 * (x1 - x0),
                y: y0 + 0.6 * (y1 - y0),
            },
            style: ArrowStyle::default(),
            text: None,
        }),
    ];
    let mut config = chart.config().clone();
    config.annotations.items = items;
    config.set_active_layer(LayerRef::Annotations);
    let series = vec![series(
        "charge",
        "ann_t",
        "ann_v",
        DataRenderType::Line {
            line: line_style(Color::from_rgb8(220, 90, 60), 2.0),
        },
    )];
    Ok(CartesianScene {
        config: with_legend(config, &series),
        series,
    })
}

fn big_data(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    const N: usize = 1_000_000;
    if r.handle_for("big_x").is_err() {
        let x: Vec<f64> = (0..N).map(|i| i as f64).collect();
        let mut acc = 0.0;
        let y: Vec<f64> = (0..N)
            .map(|i| {
                acc += noise(99, i);
                acc + 40.0 * (i as f64 / 20_000.0).sin()
            })
            .collect();
        upload(r, "big_x", x)?;
        upload(r, "big_y", y)?;
    }
    let mut chart = Chart::new(base_config(
        "Random walk, 1 000 000 points",
        "sample",
        "value",
    ));
    chart.auto_fit_x(r.pool(), "big_x", 0.0)?;
    chart.auto_fit_y(r.pool(), "big_y", 0.05)?;
    let series = vec![series(
        "walk",
        "big_x",
        "big_y",
        DataRenderType::Line {
            line: line_style(Color::from_rgb8(31, 119, 180), 1.0),
        },
    )];
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}
