//! The playground's scene catalogue: synthetic data and the starting model
//! of every scene, Cartesian and polar (`Config` and series) and the
//! DataLess radial chart (in [`radial`]).
//!
//! The file lives with the renderer's tests so the decoration oracle
//! (`tests/decoration_oracle.rs`) sweeps exactly the scenes the playground
//! shows: `tools/playground` includes it back with `#[path]`, so a scene added
//! here is covered without a test of its own. It uses only the renderer's
//! public API.
#![allow(dead_code)]
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

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
use renderer::polar::{PolarInterpolation, PolarOptions};
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
    Polar,
    CategoryAxis,
    BigData,
    Radial,
    Categorical,
    BoxPlot,
    Radar,
    LongLabels,
}

impl SceneKind {
    pub const ALL: [SceneKind; 18] = [
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
        SceneKind::Polar,
        SceneKind::CategoryAxis,
        SceneKind::BigData,
        SceneKind::Radial,
        SceneKind::Categorical,
        SceneKind::BoxPlot,
        SceneKind::Radar,
        SceneKind::LongLabels,
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
            SceneKind::Polar => "Polar (angle, radius)",
            SceneKind::CategoryAxis => "Category axis",
            SceneKind::BigData => "Large data (resident)",
            SceneKind::Radial => "Radial (pie / donut)",
            SceneKind::Categorical => "Categorical bars",
            SceneKind::BoxPlot => "Box plot",
            SceneKind::Radar => "Radar (spider)",
            SceneKind::LongLabels => "Long labels (auto layout)",
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
            SceneKind::LogAxes => {
                "Logarithmic Y with decade ticks, a logarithmic X under one decade with \
                 adaptive 1..9 ticks (AxisOptions.log_ticks), dotted minor grid."
            }
            SceneKind::TwinAxes => "Switch the active layer to Overlay 0 to edit the right axis.",
            SceneKind::Subplots => {
                "Upper-right X is linked to the parent. Activate a subplot to edit it."
            }
            SceneKind::Annotations => {
                "Active layer = Annotations: drag lines, spans, text, arrows."
            }
            SceneKind::Polar => {
                "Config.polar: bottom_x is the angle in degrees, left_y the radius. A \
                 filled pattern and a marker rose with arc segments; pick the markers."
            }
            SceneKind::CategoryAxis => {
                "AxisOptions.categories on X: a label per category, thinned when they \
                 crowd (one is long). Drag the tick labels; pick the markers."
            }
            SceneKind::BigData => {
                "Up to 100 million resident points as a line, markers or both, every one \
                 drawn. Set the size, upload, then zoom, pan, pick or run the benchmark."
            }
            SceneKind::Radial => "Click a slice; drag the title or legend.",
            SceneKind::Categorical => {
                "A categorical document opened as category bars on a categorical \
                 axis: pick a bar, step with the arrows, switch grouped / stacked / \
                 percent in the chart's bar groups."
            }
            SceneKind::BoxPlot => {
                "A box plot document opened as box plot series on a categorical \
                 axis: pick a box, a whisker or an outlier, step with the arrows; \
                 notches, means and outliers come from the document's style."
            }
            SceneKind::LongLabels => {
                "Labels such as 0.00300 under the Y title, negative decimals on the right, \
                 a logarithmic top axis, timestamps and a colourbar, in bands set by hand. \
                 Press \"Auto layout\" (layout::auto_layout) to fit bands, title offsets \
                 and label formats."
            }
            SceneKind::Radar => {
                "A radar document opened as radar series on a polar chart whose \
                 angle axis holds the categories: pick a vertex, an edge or a \
                 fill, step with the arrows; grid shape, markers and fill."
            }
        }
    }

    /// Whether the scene is the radial chart, which its own renderer draws.
    pub fn is_radial(self) -> bool {
        matches!(self, SceneKind::Radial)
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
pub fn noise(seed: u64, i: usize) -> f64 {
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

pub fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type,
    }
}

pub fn base_config(title: &str, x_title: &str, y_title: &str) -> Config {
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

/// `text` written as TeX (`RichText::from_tex`, #45) in the style of
/// `like`: its colour, size and font.
pub fn math_text(text: &str, like: &RichText) -> RichText {
    RichText::from_tex(text, like.color, like.font_size, like.font.clone())
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// Set the chart title of `config` to the formula `text` and grow the title
/// band to fit it, as `Chart::with_title` does for plain text.
pub fn with_math_title(mut config: Config, text: &str) -> Config {
    config.chart_title.text = math_text(text, &config.chart_title.text);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    renderer::layout::fit_chart_title_margin(&mut config, &measure);
    config
}

/// Rebuild the parent legend from the series the parent draws (those no
/// subplot claims), and each visible subplot legend from its own series.
pub fn rebuild_legends(config: &mut Config, series: &[SeriesConfig]) {
    let claimed: Vec<&str> = config
        .subplots
        .iter()
        .flat_map(|s| s.series.iter().map(String::as_str))
        .collect();
    // A heatmap's mark is a swatch of its chart's colourbar map.
    let fill = |content: &mut RichText,
                colormap: Option<&ColorMap>,
                ids: &mut dyn Iterator<Item = &SeriesConfig>| {
        content.segments.clear();
        for s in ids {
            append_legend_entry(content, series_symbol_segments(s, colormap), &s.series_id);
        }
    };
    fill(
        &mut config.legend.content,
        config.colorbar.as_ref().map(|bar| &bar.colormap),
        &mut series
            .iter()
            .filter(|s| !claimed.contains(&s.series_id.as_str())),
    );
    for sub in &mut config.subplots {
        if sub.config.legend.visible {
            let own = sub.series.clone();
            fill(
                &mut sub.config.legend.content,
                sub.config.colorbar.as_ref().map(|bar| &bar.colormap),
                &mut series.iter().filter(|s| own.contains(&s.series_id)),
            );
        }
    }
}

pub fn with_legend(mut config: Config, series: &[SeriesConfig]) -> Config {
    config.legend.visible = true;
    rebuild_legends(&mut config, series);
    config
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// The starting model of Cartesian scene `kind`, its columns uploaded to
/// `r`. The large-data scene generates `big_data_points` samples.
pub fn build(
    r: &mut Renderer,
    kind: SceneKind,
    big_data_points: usize,
) -> Result<CartesianScene, FiggyError> {
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
        SceneKind::Polar => polar(r),
        SceneKind::CategoryAxis => category_axis(r),
        SceneKind::Categorical => categorical(r),
        SceneKind::BoxPlot => box_plot(r),
        SceneKind::Radar => radar(r),
        SceneKind::LongLabels => long_labels(r),
        SceneKind::BigData => big_data::build(r, big_data_points),
        SceneKind::Radial => unreachable!("the radial scene is built by `radial::radial_chart`"),
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
    let mut config = with_math_title(
        base_config("", "x", "y"),
        r"Lines, areas and steps: $y = \sin x$, $\int_0^{x} \cos t\,dt$, $\sqrt{x^2+y^2}$",
    );
    config.left_y.title_option.text = math_text(
        r"$y(x) = \frac{\Delta s}{\Delta t}$",
        &config.left_y.title_option.text,
    );
    let mut chart = Chart::new(config);
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

fn polar(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    // An antenna-like pattern over the full circle and a three-leaf rose
    // sampled every five degrees. Both columns are the host's (angle,
    // radius); the renderer projects them.
    let degrees: Vec<f64> = (0..=360).map(f64::from).collect();
    let pattern: Vec<f64> = degrees
        .iter()
        .map(|d| {
            let t = d.to_radians();
            (0.6 + 2.4 * (2.0 * t).cos().abs()) * (0.7 + 0.3 * t.cos().max(0.0))
        })
        .collect();
    let rose_deg: Vec<f64> = (0..72).map(|i| f64::from(i) * 5.0).collect();
    let rose: Vec<f64> = rose_deg
        .iter()
        .map(|d| 2.0 + (3.0 * d.to_radians()).cos())
        .collect();
    upload(r, "polar_deg", degrees)?;
    upload(r, "polar_pattern", pattern)?;
    upload(r, "polar_rose_deg", rose_deg)?;
    upload(r, "polar_rose", rose)?;
    let blue = Color::from_rgb8(30, 110, 210);
    let red = Color::from_rgb8(200, 60, 40);
    let mut filled = line_style(blue, 2.5);
    filled.fill = Some(LineFill::ToBaseline(0.0));
    filled.fill_color = Some(Color::from_rgba(0.12, 0.43, 0.82, 0.25));
    let series = vec![
        series(
            "pattern",
            "polar_deg",
            "polar_pattern",
            DataRenderType::Line { line: filled },
        ),
        series(
            "rose",
            "polar_rose_deg",
            "polar_rose",
            DataRenderType::ScatterLine {
                scatter: scatter_style(red, ScatterShape::CircleFilled, 6.0),
                line: line_style(red, 1.5),
            },
        ),
    ];
    let mut config = base_config("Polar: pattern and rose", "Angle", "Radius");
    config.bottom_x.min = 0.0;
    config.bottom_x.max = 360.0;
    config.bottom_x.major_spacing = 30.0;
    config.bottom_x.label_style.format = LabelFormat::Degrees;
    config.left_y.min = 0.0;
    config.left_y.max = 3.0;
    config.left_y.major_spacing = 0.5;
    config.polar = Some(PolarOptions {
        interpolation: PolarInterpolation::Arc { pieces: 6 },
        radial_label_angle_degrees: 90.0,
        ..PolarOptions::default()
    });
    rebuild_legends(&mut config, &series);
    Ok(CartesianScene { config, series })
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
    // The energy range spans less than one decade: whole decades would label
    // 100 alone, the adaptive policy labels the 1..9 ladder (#42).
    for axis in [&mut config.bottom_x, &mut config.top_x] {
        axis.scale = renderer::config::AxisScale::Logarithmic;
        axis.log_ticks = renderer::config::LogTickPolicy::Adaptive;
    }
    config.grid.show_minor_x = true;
    config.grid.show_minor_y = true;
    config.grid.minor_x_style = LineStylePreset::Dot;
    config.grid.minor_y_style = LineStylePreset::Dot;
    let mut chart = Chart::new(config);
    chart.auto_fit_x(r.pool(), "log_e", 0.0)?;
    chart.auto_fit_y(r.pool(), "log_sigma", 0.1)?;
    // Energies read as plain numbers (50, 60, … 100) rather than powers.
    let fitted = chart.config_mut();
    for axis in [&mut fitted.bottom_x, &mut fitted.top_x] {
        axis.label_style.format = LabelFormat::Decimal;
    }
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

/// A Cartesian chart whose X axis is categorical: seven departments, one
/// with a long name so the labels are thinned at the document size, and
/// values at the category positions `0..7`.
fn category_axis(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    use renderer::config::Category;
    let names = [
        ("sales", "Sales"),
        ("support", "Support"),
        ("rnd", "Research and development (all sites)"),
        ("ops", "Operations"),
        ("legal", "Legal"),
        ("design", "Design"),
        ("qa", "Quality"),
    ];
    let x: Vec<f64> = (0..names.len()).map(|i| i as f64).collect();
    upload(r, "cat_x", x)?;
    upload(
        r,
        "cat_budget",
        (0..names.len())
            .map(|i| 4.0 + 2.5 * (i as f64 * 0.9).sin() + 0.6 * noise(31, i))
            .collect(),
    )?;
    upload(
        r,
        "cat_spent",
        (0..names.len())
            .map(|i| 3.5 + 2.0 * (i as f64 * 0.9 + 0.4).sin() + 0.8 * noise(32, i))
            .collect(),
    )?;
    let mut config = base_config("Budget by department", "Department", "Budget (M€)");
    let categories: Vec<Category> = names
        .iter()
        .map(|(id, label)| Category::new(*id, *label))
        .collect();
    config.bottom_x.categories = Some(categories.clone());
    config.top_x.categories = Some(categories);
    config.grid.show_major_x = true;
    let mut chart = Chart::new(config);
    chart.auto_fit_y_union(r.pool(), &["cat_budget", "cat_spent"], 0.15)?;
    let blue = Color::from_rgb8(31, 119, 180);
    let orange = Color::from_rgb8(255, 127, 14);
    let series = vec![
        series(
            "budget",
            "cat_x",
            "cat_budget",
            DataRenderType::ScatterLine {
                scatter: scatter_style(blue, ScatterShape::CircleFilled, 9.0),
                line: line_style(blue, 2.0),
            },
        ),
        series(
            "spent",
            "cat_x",
            "cat_spent",
            DataRenderType::Scatter {
                scatter: scatter_style(orange, ScatterShape::Diamond, 11.0),
            },
        ),
    ];
    Ok(CartesianScene {
        config: with_legend(chart.config().clone(), &series),
        series,
    })
}

/// The overlap `layout::auto_layout` resolves (#44), as a user reported it
/// and left it: small optical depths whose `0.00300` labels run under the Y
/// title, with every other kind of label that needs room beside them:
/// negative long decimals on the right, a logarithmic top axis with adaptive
/// labels printed as decimals, timestamps along the bottom and a colourbar
/// with long labels under a math title. The bands are the ones a host set by
/// hand; the playground's "Auto layout" action and the decoration oracle's
/// second sweep apply `auto_layout` to it.
fn long_labels(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    use renderer::config::{AxisScale, LogTickPolicy};
    use renderer::format::TimestampLabelFormat;
    // 2023-11-14 22:13:20 UTC, then every 45 minutes for three days.
    const START: f64 = 1_700_000_000.0;
    const HOURS: f64 = 72.0;
    let n = 97;
    let t: Vec<f64> = (0..n)
        .map(|i| START + i as f64 * HOURS * 3600.0 / (n - 1) as f64)
        .collect();
    let tau: Vec<f64> = (0..n)
        .map(|i| 0.0016 + 0.0011 * (i as f64 * 0.21).sin() + 0.0001 * noise(51, i))
        .collect();
    let albedo: Vec<f64> = (0..n)
        .map(|i| 0.0024 + 0.0016 * (i as f64 * 0.13 + 0.7).cos())
        .collect();
    upload(r, "ll_t", t)?;
    upload(r, "ll_tau", tau)?;
    upload(r, "ll_albedo", albedo)?;

    let blue = Color::from_rgb8(31, 119, 180);
    let mut points = scatter_style(blue, ScatterShape::CircleFilled, 7.0);
    points.point_color_map = Some(PointColorMap {
        column: "ll_albedo".into(),
    });
    let series = vec![series(
        "optical_depth",
        "ll_t",
        "ll_tau",
        DataRenderType::ScatterLine {
            scatter: points,
            line: line_style(blue, 1.5),
        },
    )];

    let mut config = base_config("Aerosol optical depth over three days", "Time (UTC)", "");
    config.left_y.title_option.text = math_text(
        r"Optical depth $\tau_{550}$",
        &config.left_y.title_option.text,
    );
    config.bottom_x.label_style.format = LabelFormat::Timestamp(TimestampLabelFormat::default());
    let mut bar = default::default_colorbar_options();
    bar.colormap = ColorMap::Viridis;
    bar.axis.title_option.visible = true;
    bar.axis.title_option.text = math_text(
        r"Albedo $\omega_0 - 10^{-3}$",
        &config.left_y.title_option.text,
    );
    // Room the host gave the bar's labels, not its title.
    bar.axis.out_margin = 80.0;
    config.colorbar = Some(bar);
    let mut chart = Chart::new(config);
    chart.set_x_range(START, START + HOURS * 3600.0);
    chart.set_y_range(0.0, 0.003);
    chart.auto_fit_colorbar_union(r.pool(), &["ll_albedo"], 0.0)?;
    let mut config = chart.config().clone();

    // A second Y scale on the right: the same depths as a negative forcing.
    let right = &mut config.right_y;
    right.min = -0.0045;
    right.max = -0.0005;
    right.major_spacing = 0.0005;
    right.minor_count = 4;
    right.label_style.label_visible = true;
    right.label_style.significant_digits = 3;
    right.title_option.visible = true;
    right.title_option.text = math_text(r"Forcing $\Delta F$", &config.left_y.title_option.text);
    right.out_margin = 110.0;

    // A wavelength scale on top, logarithmic with adaptive majors, its
    // labels printed as decimals.
    let top = &mut config.top_x;
    top.scale = AxisScale::Logarithmic;
    top.min = 1.0e-6;
    top.max = 1.0e3;
    top.log_ticks = LogTickPolicy::Adaptive;
    top.minor_count = 8;
    top.label_style.format = LabelFormat::Decimal;
    top.label_style.significant_digits = 1;
    top.label_style.label_visible = true;
    top.title_option.visible = true;
    top.title_option.text = math_text(
        r"Wavelength $\lambda$ (m)",
        &config.bottom_x.title_option.text,
    );
    top.out_margin = 40.0;

    Ok(CartesianScene {
        config: with_legend(config, &series),
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
    overlay.y_axis.title_option.text =
        RichText::from_tex(r"Pressure $p_{\mathrm{atm}}$ (hPa)", rust, 18.0, "")
            .expect("valid TeX");
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
        with_math_title(
            cell(w, 0, "", "Hour", "hPa"),
            r"Pressure $\frac{dp}{dt} \approx 0$ (X linked)",
        ),
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
            text: RichText::from_tex(
                r"drag me: $V = V_0\left(1 - e^{-t/\tau}\right)$",
                Color::BLACK,
                16.0,
                "",
            )
            .expect("valid TeX"),
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

// ---------------------------------------------------------------------------
// Large data
// ---------------------------------------------------------------------------

/// The data half of the resident large-data scene: generated points drawn
/// as a line, as markers or as both.
///
/// The data is resident: both columns live in the GPU column pool and every
/// point is drawn on every frame. The generator writes each value straight
/// into the pool's mapped staging range through `HiLoColumnSource`, so no
/// dataset-sized CPU buffer exists at any point. The playground keeps the
/// bench (sizes, timings, the pan benchmark) in its own `big_data.rs`.
pub mod big_data {
    use renderer::config::Config;
    use renderer::data::split_f64_to_f32_pair;
    use renderer::data_config::{DataRenderType, ScatterShape, SeriesConfig};
    use renderer::{
        Chart, Color, ColumnPairWriter, ColumnUploadStats, FiggyError, HiLoColumnSource, Renderer,
    };

    use super::CartesianScene;

    const X_ID: &str = "big_x";
    const Y_ID: &str = "big_y";
    /// The X column spans `0..=X_SPAN`; hi/lo pairs keep 100 million samples
    /// distinct where a single f32 could not.
    const X_SPAN: f64 = 1000.0;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mode {
        Line,
        Scatter,
        Both,
    }

    impl Mode {
        pub const ALL: [Mode; 3] = [Mode::Line, Mode::Scatter, Mode::Both];

        pub fn label(self) -> &'static str {
            match self {
                Mode::Line => "Line",
                Mode::Scatter => "Scatter",
                Mode::Both => "Line + scatter",
            }
        }
    }

    /// One generated column, evaluated per index while it is written.
    struct Generated {
        len: usize,
        y: bool,
        min: f64,
        max: f64,
    }

    impl Generated {
        fn x(len: usize) -> Self {
            Self {
                len,
                y: false,
                min: 0.0,
                max: X_SPAN,
            }
        }

        /// The Y bounds are the exact extremes of the generated values: one pass
        /// that evaluates the signal and keeps two scalars.
        fn y(len: usize) -> Self {
            let (min, max) = (0..len).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), i| {
                let v = y_at(len, i);
                (lo.min(v), hi.max(v))
            });
            Self {
                len,
                y: true,
                min,
                max,
            }
        }

        fn value(&self, i: usize) -> f64 {
            if self.y {
                y_at(self.len, i)
            } else {
                x_at(self.len, i)
            }
        }
    }

    fn x_at(len: usize, i: usize) -> f64 {
        X_SPAN * i as f64 / (len.max(2) - 1) as f64
    }

    /// Two waves and per-sample noise: a line with structure at every zoom level
    /// and a scatter band with visible density.
    fn y_at(len: usize, i: usize) -> f64 {
        let x = x_at(len, i);
        30.0 * (x / 40.0).sin() + 8.0 * (0.9 * x).sin() + 4.0 * super::noise(7, i)
    }

    impl HiLoColumnSource for Generated {
        fn len(&self) -> usize {
            self.len
        }
        fn min(&self) -> f64 {
            self.min
        }
        fn max(&self) -> f64 {
            self.max
        }

        fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
            for (i, pair) in dst.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                let (hi, lo) = split_f64_to_f32_pair(self.value(i));
                pair[..4].copy_from_slice(&hi.to_le_bytes());
                pair[4..].copy_from_slice(&lo.to_le_bytes());
            }
        }

        fn write_f32_pair_le_into_with_stats(
            &self,
            mut dst: ColumnPairWriter<'_>,
        ) -> ColumnUploadStats {
            let mut min_positive = f64::INFINITY;
            for i in 0..self.len {
                let (hi, lo) = split_f64_to_f32_pair(self.value(i));
                dst.write_pair(i, hi, lo);
                let v = f64::from(hi) + f64::from(lo);
                if v > 0.0 && v.is_finite() {
                    min_positive = min_positive.min(v);
                }
            }
            ColumnUploadStats {
                min_positive: min_positive.is_finite().then_some(min_positive),
            }
        }
    }

    /// Upload `points` samples of both columns, replacing any earlier ones, and
    /// wait until the GPU holds them. Removing a column also removes the series
    /// that read it, so callers set the series again afterwards.
    pub fn upload(r: &mut Renderer, points: usize) -> Result<(), FiggyError> {
        r.remove_column(X_ID)?;
        r.remove_column(Y_ID)?;
        r.add_hilo_column(X_ID, &Generated::x(points))?;
        r.add_hilo_column(Y_ID, &Generated::y(points))?;
        // A failed wait only makes the upload time read short.
        if let Err(e) = r.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        }) {
            eprintln!("[big data] poll: {e}");
        }
        Ok(())
    }

    pub fn series(mode: Mode) -> Vec<SeriesConfig> {
        let line = super::line_style(Color::from_rgb8(31, 119, 180), 1.0);
        let scatter =
            super::scatter_style(Color::from_rgb8(214, 39, 40), ScatterShape::Circle, 2.0);
        let render_type = match mode {
            Mode::Line => DataRenderType::Line { line },
            Mode::Scatter => DataRenderType::Scatter { scatter },
            Mode::Both => DataRenderType::ScatterLine { scatter, line },
        };
        vec![super::series("samples", X_ID, Y_ID, render_type)]
    }

    fn title(points: usize) -> String {
        format!("Resident data, {} points", group_digits(points))
    }

    /// `5000000` as `5,000,000`.
    pub fn group_digits(n: usize) -> String {
        let digits = n.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }

    /// The chart for `points` samples, fitted to the generated columns.
    pub fn chart_config(
        r: &Renderer,
        points: usize,
        series: &[SeriesConfig],
    ) -> Result<Config, FiggyError> {
        let mut chart = Chart::new(super::base_config(&title(points), "x", "value"));
        chart.auto_fit_x(r.pool(), X_ID, 0.0)?;
        chart.auto_fit_y(r.pool(), Y_ID, 0.05)?;
        Ok(super::with_legend(chart.config().clone(), series))
    }

    /// The scene with `points` samples, drawn as a line.
    pub fn build(r: &mut Renderer, points: usize) -> Result<CartesianScene, FiggyError> {
        upload(r, points)?;
        let series = series(Mode::Line);
        Ok(CartesianScene {
            config: chart_config(r, points, &series)?,
            series,
        })
    }

    /// Release the columns when the scene goes away, so other scenes get the
    /// pool space back.
    pub fn release(r: &mut Renderer) {
        for id in [X_ID, Y_ID] {
            if let Err(e) = r.remove_column(id) {
                eprintln!("[big data] remove {id}: {e}");
            }
        }
    }
}

/// The legacy categorical document of the categorical scene: quarterly
/// revenue of three regions, Matte bars.
pub fn categorical_document() -> renderer::CategoricalChart {
    let mut chart = renderer::CategoricalChart {
        title: "Quarterly revenue".into(),
        categories: ["Q1", "Q2", "Q3", "Q4"]
            .into_iter()
            .map(|s| renderer::config::Category::new(s, s))
            .collect(),
        series: vec![
            renderer::BarSeries::new(
                "north",
                "North",
                [12., 18., 9., 15.].map(Some).into(),
                Color::from_rgb8(85, 143, 211),
            ),
            renderer::BarSeries::new(
                "south",
                "South",
                [18., 24., 14., 21.].map(Some).into(),
                Color::from_rgb8(255, 166, 105),
            ),
            renderer::BarSeries::new(
                "west",
                "West",
                [7., 11., 16., 13.].map(Some).into(),
                Color::from_rgb8(110, 190, 120),
            ),
        ],
        value_title: "Revenue (M$)".into(),
        ..Default::default()
    };
    chart.style.material = renderer::Material::Matte;
    chart
}

/// The categorical scene: [`categorical_document`] opened through
/// `Config::from_categorical` as a Cartesian chart of category bars. The
/// uploads borrow the document's values: the positions first, then the
/// value columns as one batch. The value axis is fitted to the bars and
/// keeps the document's label format.
fn categorical(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let document = categorical_document();
    let (mut config, series, uploads) = Config::from_categorical(&document);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: DOC_W,
        height: DOC_H,
    });
    let Some((positions, values)) = uploads.split_first() else {
        unreachable!("the reader writes the positions first");
    };
    if r.handle_for(&positions.column).is_err() {
        r.add_column(positions.column.clone(), &positions.values)?;
    }
    for upload in values {
        if r.handle_for(&upload.column).is_ok() {
            r.remove_column(&upload.column)?;
        }
    }
    let columns: Vec<(&str, &dyn ColumnSource)> = values
        .iter()
        .map(|upload| (upload.column.as_str(), &upload.values as &dyn ColumnSource))
        .collect();
    r.add_columns(&columns)?;
    let format = config.left_y.label_style.format.clone();
    let mut chart = Chart::new(config);
    chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
    let mut config = chart.config().clone();
    config.left_y.label_style.format = format;
    let (min, max) = (config.left_y.min, config.left_y.max);
    // Nice ticks when they exist; the fitted range stays otherwise.
    if config.left_y.auto_ticks(min, max, 5).is_err() {
        config.left_y.min = min;
        config.left_y.max = max;
    }
    Ok(CartesianScene { config, series })
}

/// The legacy box plot document of the box plot scene: responses by dose
/// of two trials, notches, means and outliers.
pub fn boxplot_document() -> renderer::BoxPlotChart {
    renderer::BoxPlotChart {
        title: "Response by dose".into(),
        categories: ["Control", "Low", "Mid", "High"]
            .into_iter()
            .map(|s| renderer::Category::new(s, s))
            .collect(),
        series: vec![
            renderer::BoxPlotSeries::new(
                "trial_a",
                "Trial A",
                vec![
                    box_summary(12., 30., 42., 55., 78., &[90.]),
                    box_summary(20., 42., 55., 68., 85., &[96., 4.]),
                    box_summary(25., 45., 60., 70., 88., &[]),
                    box_summary(8., 24., 36., 49., 67., &[82.]),
                ],
                Color::from_rgb8(65, 144, 208),
            ),
            renderer::BoxPlotSeries::new(
                "trial_b",
                "Trial B",
                vec![
                    box_summary(15., 28., 38., 50., 70., &[]),
                    box_summary(22., 40., 50., 62., 80., &[93.]),
                    box_summary(30., 50., 64., 74., 92., &[]),
                    box_summary(12., 30., 40., 52., 72., &[2.]),
                ],
                Color::from_rgb8(230, 169, 64),
            ),
        ],
        value_range: Some([0., 100.]),
        value_title: "Response".into(),
        // The median above every box: the scene sweeps value labels too.
        labels: renderer::BoxPlotLabels::Median,
        ..Default::default()
    }
}

fn box_summary(
    low: f64,
    q1: f64,
    med: f64,
    q3: f64,
    high: f64,
    outliers: &[f64],
) -> Option<renderer::BoxSummary> {
    let mut v = renderer::BoxSummary::new(low, q1, med, q3, high);
    v.outliers = outliers.to_vec();
    v.sample_count = Some(80);
    v.median_ci = Some([med - 4.0, med + 4.0]);
    v.mean = Some(med + 2.0);
    Some(v)
}

/// The box plot scene: [`boxplot_document`] opened through
/// `Config::from_boxplot` as a Cartesian chart of box plot series. The
/// uploads borrow the document's summaries: the positions, then each
/// series' record and outliers. The document fixes the value range.
fn box_plot(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    use renderer::{BOX_PLOT_RECORD, RaggedSource, RecordSource, UploadValues};
    let document = boxplot_document();
    let (mut config, series, uploads) = Config::from_boxplot(&document);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: DOC_W,
        height: DOC_H,
    });
    for upload in &uploads {
        match &upload.values {
            UploadValues::BoxRecord(_) => {
                let Some(fields) = upload.values.box_fields() else {
                    unreachable!("a record upload has fields");
                };
                let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                    (BOX_PLOT_RECORD[k].0, &fields[k] as &dyn ColumnSource)
                });
                r.upsert_record(
                    upload.column.clone(),
                    &RecordSource {
                        schema: BOX_PLOT_RECORD,
                        fields: &pairs,
                    },
                )?;
            }
            UploadValues::BoxOutliers(_) => {
                let Some((values, offsets)) = upload.values.box_outliers() else {
                    unreachable!("an outlier upload has values");
                };
                r.upsert_ragged(
                    upload.column.clone(),
                    &RaggedSource {
                        values: &values,
                        offsets: &offsets,
                    },
                )?;
            }
            values => {
                if r.handle_for(&upload.column).is_err() {
                    r.add_column(upload.column.clone(), values)?;
                }
            }
        }
    }
    Ok(CartesianScene { config, series })
}

/// The radar scene's document: three releases over six categories on a
/// fixed 0 to 10 axis.
pub fn radar_document() -> renderer::RadarChart {
    renderer::RadarChart {
        title: "Build quality".into(),
        categories: ["Speed", "Safety", "Size", "Docs", "Tests", "Ergonomics"]
            .into_iter()
            .map(|s| renderer::Category::new(s.to_lowercase(), s))
            .collect(),
        series: vec![
            renderer::RadarSeries::new(
                "v1",
                "Release 1",
                vec![6., 8., 4., 5., 7., 6.5],
                Color::from_rgb8(65, 144, 208),
            ),
            renderer::RadarSeries::new(
                "v2",
                "Release 2",
                vec![8., 7., 6., 8., 9., 7.],
                Color::from_rgb8(230, 120, 64),
            ),
            renderer::RadarSeries::new(
                "v3",
                "Release 3",
                vec![9., 8.5, 8., 9., 9.5, 8.],
                Color::from_rgb8(42, 168, 120),
            ),
        ],
        value_range: Some([0., 10.]),
        ring_spacing: Some(2.5),
        // A value label at every vertex: the scene sweeps value labels too.
        labels: renderer::RadarLabels::Values,
        ..Default::default()
    }
}

fn radar(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    let document = radar_document();
    let (mut config, series, uploads) = Config::from_radar(&document);
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: DOC_W,
        height: DOC_H,
    });
    let config = with_math_title(config, r"Build quality $\bar{q} = \frac{1}{n}\sum_{k} q_k$");
    for upload in &uploads {
        if r.handle_for(&upload.column).is_err() {
            r.add_column(upload.column.clone(), &upload.values)?;
        }
    }
    Ok(CartesianScene { config, series })
}

// ---------------------------------------------------------------------------
// The radial scene
// ---------------------------------------------------------------------------

/// The starting model of the radial scene; the playground's `radial.rs`
/// hosts it.
pub mod radial {
    use renderer::Color;
    use renderer::radial::*;

    pub fn radial_chart() -> RadialChart {
        let mut chart = RadialChart {
            title: "Traffic sources".into(),
            kind: RadialKind::Donut { inner_radius: 0.5 },
            slices: vec![
                RadialSlice::new("Search", 40.0, Color::from_rgb8(51, 132, 245)),
                RadialSlice::new("Direct", 25.0, Color::from_rgb8(165, 216, 72)),
                RadialSlice::new("Email", 15.0, Color::from_rgb8(80, 96, 121)),
                RadialSlice::new("Social", 12.0, Color::from_rgb8(255, 151, 62)),
                RadialSlice::new("Video", 8.0, Color::from_rgb8(48, 192, 230)),
            ],
            ..Default::default()
        };
        chart.style = RadialStyle::three_dimensional();
        chart.format_labels();
        chart
    }
}
