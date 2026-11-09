use super::*;
use crate::Config;
use crate::config::Category;
use crate::data_config::{BarOrientation, ScatterShape};

fn grid(prefix: &str) -> MatrixRef {
    MatrixRef::columns_are_x((0..3).map(|i| format!("{prefix}{i}")))
}

fn valid(config: &Config, series: &SeriesConfig) {
    if let Err(error) = config.validate_series(std::slice::from_ref(series)) {
        panic!("{:?}: {error:?}", series.render_type);
    }
}

fn xy(series: &SeriesConfig) -> &XySeries {
    match &series.render_type {
        DataRenderType::Xy(xy) => xy,
        other => panic!("an x-y series, not {other:?}"),
    }
}

// The line modifiers write the line's fill, its colour and its steps.
#[test]
fn line_modifiers_set_fill_and_steps() {
    let red = Color::from_rgb8(200, 30, 30);
    let series = SeriesConfig::line("a", "x", "y")
        .fill_to(0.0)
        .fill_color(red)
        .interpolation(LineInterpolation::StepMid);
    let line = xy(&series).line.as_ref().unwrap();
    assert_eq!(line.fill_color, Some(red));
    assert_eq!(line.fill, Some(LineFill::ToBaseline(0.0)));
    assert_eq!(line.interpolation, LineInterpolation::StepMid);
    valid(&Config::default(), &series);

    let band = SeriesConfig::line("hi", "x", "y").fill_between("lo");
    assert_eq!(
        xy(&band).line.as_ref().unwrap().fill,
        Some(LineFill::ToSeries("lo".into()))
    );
}

// A line through markers takes the markers' colour; a ternary series takes
// one the same way.
#[test]
fn with_line_joins_the_markers() {
    let blue = Color::from_rgb8(30, 60, 200);
    let series = SeriesConfig::scatter("a", "x", "y")
        .color(blue)
        .with_line(2.0);
    let line = xy(&series).line.as_ref().unwrap();
    assert_eq!((line.line_color, line.line_width), (blue, 2.0));
    valid(&Config::default(), &series);

    let mut ternary = Config::default();
    ternary.set_ternary(Some(crate::defaults::ternary_options()));
    let series = SeriesConfig::ternary("t", "p", "q", "r")
        .marker(ScatterShape::CircleFilled, 5.0)
        .with_line(1.5);
    let DataRenderType::Ternary { line, .. } = &series.render_type else {
        panic!("a ternary series");
    };
    assert_eq!(line.as_ref().unwrap().line_width, 1.5);
    valid(&ternary, &series);
}

// Stems, error bars and histogram bars take their own looks.
#[test]
fn stem_error_and_bar_modifiers() {
    let ink = Color::from_rgb8(10, 20, 30);
    let green = Color::from_rgb8(20, 160, 60);
    let series = SeriesConfig::scatter("a", "x", "y")
        .color(green)
        .with_stem(-1.0)
        .stem_color(ink)
        .horizontal()
        .y_errors("e")
        .error_bars(2.0, 9.0)
        .error_color(ink);
    let xy = xy(&series);
    let stem = xy.stem.as_ref().unwrap();
    assert_eq!(
        (stem.baseline, stem.color, stem.direction),
        (-1.0, ink, CategoryBarDirection::Horizontal)
    );
    let errors = &xy.errors.as_ref().unwrap().style;
    assert_eq!(
        (
            errors.error_bar_width,
            errors.cap_width,
            errors.error_bar_cap_size,
            errors.error_bar_color
        ),
        (2.0, 2.0, 9.0, ink)
    );
    valid(&Config::default(), &series);

    let bars = SeriesConfig::histogram("h", "edges", "counts")
        .bar_border(ink, 0.5)
        .bar_gap(3.0)
        .horizontal();
    let DataRenderType::Histogram { bar } = &bars.render_type else {
        panic!("a histogram");
    };
    assert_eq!(
        (
            bar.border_color,
            bar.border_width,
            bar.gap_px,
            bar.orientation.clone()
        ),
        (ink, 0.5, 3.0, BarOrientation::Horizontal)
    );
    valid(&Config::default(), &bars);
}

// Markers are coloured and sized by columns; a parallel series is coloured
// by one.
#[test]
fn point_maps() {
    let series =
        SeriesConfig::scatter("a", "x", "y")
            .color_by("c")
            .size_by("s", [0.0, 10.0], [2.0, 12.0]);
    let scatter = xy(&series).scatter.as_ref().unwrap();
    assert_eq!(
        scatter.point_color_map,
        Some(PointColorMap { column: "c".into() })
    );
    let size = scatter.point_size_map.as_ref().unwrap();
    assert_eq!(
        (
            size.column.as_str(),
            size.domain_min,
            size.domain_max,
            size.min_px,
            size.max_px
        ),
        ("s", 0.0, 10.0, 2.0, 12.0)
    );
    let mut config = Config::default();
    config.colorbar = Some(crate::defaults::color_bar_options());
    valid(&config, &series);

    let parallel = SeriesConfig::parallel("p", ["a", "b"]).color_by("c");
    let DataRenderType::Parallel { color_column, .. } = &parallel.render_type else {
        panic!("a parallel series");
    };
    assert_eq!(color_column.as_deref(), Some("c"));
}

// Fields and images take shading, opacity, a fill under contours and
// contour labels.
#[test]
fn field_modifiers() {
    let heat = SeriesConfig::heatmap("h", "x", "y", grid("z"))
        .shading(Shading::Interpolated)
        .opacity(0.5);
    let DataRenderType::Field(field) = &heat.render_type else {
        panic!("a field");
    };
    let fill = field.fill.as_ref().unwrap();
    assert_eq!(
        (fill.shading.clone(), fill.opacity),
        (Shading::Interpolated, 0.5)
    );

    let labels = ContourLabelConfig {
        font_size: 11.0,
        ..ContourLabelConfig::default()
    };
    let contours = SeriesConfig::contours("c", "x", "y", grid("z"), vec![0.5])
        .filled()
        .contour_labels(labels.clone());
    let DataRenderType::Field(field) = &contours.render_type else {
        panic!("a field");
    };
    assert!(field.fill.is_some());
    assert_eq!(field.contour.as_ref().unwrap().labels, Some(labels));
    let mut config = Config::default();
    config.colorbar = Some(crate::defaults::color_bar_options());
    valid(&config, &contours);

    let image = SeriesConfig::rgb_image("i", "x", "y", grid("r"), grid("g"), grid("b"))
        .shading(Shading::Interpolated)
        .opacity(0.8);
    let DataRenderType::RgbImage {
        shading, opacity, ..
    } = &image.render_type
    else {
        panic!("an RGB image");
    };
    assert_eq!((shading.clone(), *opacity), (Shading::Interpolated, 0.8));
}

// Vector constructors and glyph modifiers.
#[test]
fn vector_constructors_and_modifiers() {
    let arrows = SeriesConfig::vectors_grid("v", "x", "y", grid("u"), grid("w"))
        .arrow_head(12.0, 6.0)
        .pivot(VectorPivot::Middle)
        .style_vectors(|style| style.min_length_px = 2.0);
    let DataRenderType::Vector { components, style } = &arrows.render_type else {
        panic!("a vector series");
    };
    assert!(components.grid().is_some());
    assert_eq!(
        (
            style.head_length_px,
            style.head_width_px,
            style.pivot,
            style.min_length_px
        ),
        (12.0, 6.0, VectorPivot::Middle, 2.0)
    );
    valid(&Config::default(), &arrows);

    let quiver = SeriesConfig::vectors_3d("q", "x", "y", "z", "u", "v", "w");
    assert_eq!(quiver.z_column.as_deref(), Some("z"));
    let DataRenderType::Vector {
        components: VectorComponents::Points { w, .. },
        ..
    } = &quiver.render_type
    else {
        panic!("point vectors");
    };
    assert_eq!(w.as_deref(), Some("w"));
}

// The closures reach the sub-styles no modifier names.
#[test]
fn closures_reach_sub_styles() {
    let mut config = Config::default();
    config.bottom_x.categories = Some(
        (0..3)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    let bars = SeriesConfig::category_bars("b", "i", "v", Color::BLACK)
        .style_bars(|style| style.corner_radius = 3.0);
    let DataRenderType::CategoryBars { style, .. } = &bars.render_type else {
        panic!("category bars");
    };
    assert_eq!(style.corner_radius, 3.0);
    valid(&config, &bars);

    let boxes = SeriesConfig::box_plot("x", "i", "records", Color::BLACK)
        .style_boxes(|style| style.notched = true);
    let DataRenderType::BoxPlot { style, .. } = &boxes.render_type else {
        panic!("a box plot");
    };
    assert!(style.notched);

    let line = SeriesConfig::line("l", "x", "y").style_line(|line| line.line_width = 4.0);
    assert_eq!(xy(&line).line.as_ref().unwrap().line_width, 4.0);
    let markers = SeriesConfig::scatter("s", "x", "y")
        .style_markers(|scatter| scatter.point_shape = ScatterShape::Diamond);
    assert_eq!(
        xy(&markers).scatter.as_ref().unwrap().point_shape,
        ScatterShape::Diamond
    );
}

// A modifier that does not apply to a family leaves the series as it is.
#[test]
fn modifiers_leave_other_families_alone() {
    let bars = SeriesConfig::histogram("h", "edges", "counts");
    let same = bars
        .clone()
        .fill_color(Color::WHITE)
        .with_line(2.0)
        .error_bars(1.0, 1.0)
        .shading(Shading::Interpolated)
        .arrow_head(1.0, 1.0)
        .color_by("c");
    assert_eq!(same, bars);
}
