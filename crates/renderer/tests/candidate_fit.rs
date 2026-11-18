//! Candidate fits and style domain fits (#312): `Renderer::fit_config`,
//! `Renderer::fit_y_axes_config`, `Renderer::fit_point_size_domain` and
//! `Renderer::fit_vector_scale`, all from the resident columns' cached
//! statistics, with no chart installed or changed.
#![cfg(not(target_arch = "wasm32"))]

use renderer::config::{AxisOptions, Config};
use renderer::data_config::{MatrixRef, VectorScale};
use renderer::{
    Chart, Color, DataRenderType, FIT_RANGES_PADDING, FiggyError, Renderer, RuleKind, SeriesConfig,
};

/// A headless renderer with columns of eleven values: `t` 0..10, `a`
/// -1..1, `b` 100..200, `u` 20..30, `z` 5..10, `flat` all 3, `zero` all 0
/// and `nan` all NaN; and `pair`, 0 and 1.
fn fit_renderer() -> Renderer {
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let column = |from: f64, step: f64| -> Vec<f64> {
        (0..=10).map(|i| from + step * f64::from(i)).collect()
    };
    for (id, from, step) in [
        ("t", 0.0, 1.0),
        ("a", -1.0, 0.2),
        ("b", 100.0, 10.0),
        ("u", 20.0, 1.0),
        ("z", 5.0, 0.5),
        ("flat", 3.0, 0.0),
        ("zero", 0.0, 0.0),
        ("nan", f64::NAN, 0.0),
    ] {
        renderer.add_column(id, &column(from, step)).unwrap();
    }
    renderer
        .add_column("pair", &column(0.0, 1.0)[..2].to_vec())
        .unwrap();
    renderer
        .register_streamed_columns(vec![renderer::StreamColumn {
            id: "streamed".into(),
            len: 100,
            encoding: renderer::StreamEncoding::HiLoF32,
            replay: renderer::StreamReplay::Sequential,
            revision: 1,
            statistics: renderer::StreamStatistics::Unknown,
        }])
        .unwrap();
    renderer
}

/// `axis` runs from `min` to `max`, to rounding.
#[track_caller]
fn assert_range(axis: &AxisOptions, min: f64, max: f64) {
    assert!(
        (axis.min - min).abs() < 1e-9 && (axis.max - max).abs() < 1e-9,
        "{:?} is not {:?}",
        (axis.min, axis.max),
        (min, max)
    );
}

#[track_caller]
fn assert_rule<T: std::fmt::Debug>(result: Result<T, FiggyError>, rule: RuleKind) {
    match result {
        Err(
            FiggyError::InvalidSeriesConfig { kind, .. } | FiggyError::InvalidConfig { kind, .. },
        ) if kind == rule => {}
        other => panic!("expected {rule:?}, got {other:?}"),
    }
}

/// A parent with an overlay that has its own X axis, and a subplot with an
/// overlay that follows the subplot's axes: every layer kind the fit walks.
fn layered() -> (Config, Vec<SeriesConfig>) {
    let series = vec![
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::line("b", "u", "b"),
        SeriesConfig::scatter("c", "t", "b"),
        SeriesConfig::line("z", "u", "z"),
    ];
    let inner = Chart::placed(320, 0, 320, 240)
        .with_overlay(AxisOptions::overlay_y("Z", Color::BLACK), &["z"])
        .config()
        .clone();
    let chart = Chart::sized(320, 240)
        .with_overlay_xy(
            AxisOptions::overlay_y("U", Color::BLACK),
            AxisOptions::overlay_y("B", Color::BLACK),
            &["b"],
        )
        .with_subplot(renderer::Subplot::new(inner, &["c", "z"]));
    (chart.config().clone(), series)
}

// The candidate fit is the installed fit: at `FIT_RANGES_PADDING` it returns
// exactly what `fit_ranges` writes into a chart, for every layer, and the
// candidate it was given is left as it was.
#[test]
fn fit_config_is_the_fit_ranges_path() {
    let renderer = fit_renderer();
    let (config, series) = layered();
    let before = config.clone();
    let fitted = renderer
        .fit_config(&config, &series, FIT_RANGES_PADDING)
        .unwrap();
    assert_eq!(config, before, "the candidate is not changed");

    let mut chart = Chart::new(config.clone());
    renderer.fit_ranges(&mut chart, &series).unwrap();
    assert_eq!(&fitted, chart.config(), "one fit path");
    assert_range(&fitted.bottom_x, -0.5, 10.5);
    assert_range(&fitted.left_y, -1.1, 1.1);
    assert_range(&fitted.overlays[0].x_axis, 19.5, 30.5);
    assert_range(&fitted.overlays[0].y_axis, 95.0, 205.0);
    let cell = &fitted.subplots[0].config;
    assert_range(&cell.left_y, 95.0, 205.0);
    assert_range(&cell.overlays[0].y_axis, 4.75, 10.25);
    assert!(fitted.validate().is_ok());

    // Another padding: 10% of each span.
    let wide = renderer.fit_config(&config, &series, 0.1).unwrap();
    assert_range(&wide.bottom_x, -1.0, 11.0);
    assert_range(&wide.left_y, -1.2, 1.2);
    let none = renderer.fit_config(&config, &series, 0.0).unwrap();
    assert_range(&none.left_y, -1.0, 1.0);

    // A registered chart is untouched by a candidate fit.
    let mut renderer = renderer;
    let id = renderer
        .register_chart(config.clone(), series.clone())
        .unwrap();
    renderer.fit_config(&config, &series, 0.1).unwrap();
    assert_eq!(renderer.chart_config(id).unwrap(), &config);
}

// The Y-only fit keeps every X axis as given, ticks and format included,
// and fits every Y axis as the full fit does.
#[test]
fn fit_y_axes_config_keeps_every_x_axis() {
    let renderer = fit_renderer();
    let (mut config, series) = layered();
    config.bottom_x.min = 2.0;
    config.bottom_x.max = 3.0;
    config.bottom_x.major_spacing = 0.25;
    config.overlays[0].x_axis.min = -7.0;
    config.subplots[0].config.bottom_x.max = 99.0;
    let full = renderer.fit_config(&config, &series, 0.05).unwrap();
    let fitted = renderer.fit_y_axes_config(&config, &series, 0.05).unwrap();
    assert_eq!(fitted.bottom_x, config.bottom_x);
    assert_eq!(fitted.top_x, config.top_x);
    assert_eq!(fitted.overlays[0].x_axis, config.overlays[0].x_axis);
    let (cell, given) = (&fitted.subplots[0].config, &config.subplots[0].config);
    assert_eq!(cell.bottom_x, given.bottom_x);
    assert_eq!(cell.top_x, given.top_x);
    assert_eq!(cell.overlays[0].x_axis, given.overlays[0].x_axis);
    // The Y axes are the full fit's.
    assert_eq!(fitted.left_y, full.left_y);
    assert_eq!(fitted.right_y, full.right_y);
    assert_eq!(fitted.overlays[0].y_axis, full.overlays[0].y_axis);
    assert_eq!(cell.left_y, full.subplots[0].config.left_y);
    assert_eq!(
        cell.overlays[0].y_axis,
        full.subplots[0].config.overlays[0].y_axis
    );
    assert_range(&fitted.left_y, -1.1, 1.1);

    // A 3D chart's z axis is a Y-side axis: it is fitted.
    let mut cube = Chart::sized(320, 240);
    cube.config_mut()
        .set_view_3d(Some(renderer::View3d::default()));
    let points = [SeriesConfig::scatter("p", "t", "a").z("z")];
    let fitted = renderer
        .fit_y_axes_config(cube.config(), &points, 0.05)
        .unwrap();
    assert_eq!(fitted.bottom_x, cube.config().bottom_x);
    assert_range(&fitted.right_y, 4.75, 10.25);
}

// Every refusal is an error, and none changes anything.
#[test]
fn candidate_fits_refuse_what_they_cannot_fit() {
    let renderer = fit_renderer();
    let (config, series) = layered();
    for padding in [-0.1, f64::NAN, f64::INFINITY] {
        assert_rule(
            renderer.fit_config(&config, &series, padding),
            RuleKind::FitPadding,
        );
        assert_rule(
            renderer.fit_y_axes_config(&config, &series, padding),
            RuleKind::FitPadding,
        );
    }
    // A layer naming a series the chart has not registered: an overlay, a
    // subplot and a subplot's overlay.
    let mut ghost = config.clone();
    ghost.overlays[0].series.push("ghost".into());
    assert_rule(
        renderer.fit_config(&ghost, &series, 0.05),
        RuleKind::LayerSeriesUnknown,
    );
    let mut ghost = config.clone();
    ghost.subplots[0].series.push("ghost".into());
    assert_rule(
        renderer.fit_y_axes_config(&ghost, &series, 0.05),
        RuleKind::LayerSeriesUnknown,
    );
    let mut ghost = config.clone();
    ghost.subplots[0].config.overlays[0]
        .series
        .push("ghost".into());
    let error = renderer.fit_config(&ghost, &series, 0.05).unwrap_err();
    assert!(
        matches!(&error, FiggyError::InvalidSeriesConfig { series_id, kind: RuleKind::LayerSeriesUnknown } if series_id == "ghost"),
        "{error}"
    );
    // A column that was never registered, a streamed source and a
    // monotonic series.
    let missing = [SeriesConfig::line("a", "t", "nowhere")];
    assert!(matches!(
        renderer.fit_config(&Config::default(), &missing, 0.05),
        Err(FiggyError::UnknownColumn { id }) if id == "nowhere"
    ));
    let streamed = [SeriesConfig::line("a", "t", "streamed")];
    assert!(matches!(
        renderer.fit_config(&Config::default(), &streamed, 0.05),
        Err(FiggyError::ColumnNotResident { id }) if id == "streamed"
    ));
    let monotonic = [SeriesConfig::monotonic_line("m", "daq", 0)];
    assert_rule(
        renderer.fit_config(&Config::default(), &monotonic, 0.05),
        RuleKind::MonotonicMappingEdit,
    );
}

/// The point size map of a scatter series.
fn size_map(series: &SeriesConfig) -> &renderer::data_config::PointSizeMap {
    match &series.render_type {
        DataRenderType::Xy(renderer::XySeries {
            scatter: Some(scatter),
            ..
        }) => scatter.point_size_map.as_ref().expect("a size map"),
        other => panic!("not a scatter: {other:?}"),
    }
}

// The size domain takes the size column's extent; only the domain changes.
#[test]
fn fit_point_size_domain_takes_the_size_column_extent() {
    let renderer = fit_renderer();
    let mut series = SeriesConfig::scatter("s", "t", "a").size_by("b", [0.0, 1.0], [2.0, 12.0]);
    let before = series.clone();
    renderer.fit_point_size_domain(&mut series).unwrap();
    let map = size_map(&series);
    assert_eq!((map.domain_min, map.domain_max), (100.0, 200.0));
    assert_eq!((map.min_px, map.max_px), (2.0, 12.0));
    let mut expected = before.clone();
    if let DataRenderType::Xy(renderer::XySeries {
        scatter: Some(scatter),
        ..
    }) = &mut expected.render_type
        && let Some(map) = scatter.point_size_map.as_mut()
    {
        map.domain_min = 100.0;
        map.domain_max = 200.0;
    }
    assert_eq!(series, expected, "only the domain changes");
    // The fitted map gives the radii the shaders draw.
    assert_eq!(map.radius_px(100.0), Some(2.0));
    assert_eq!(map.radius_px(f64::NAN), None);

    // Refusals leave the series as it was.
    let mut plain = SeriesConfig::scatter("s", "t", "a");
    let kept = plain.clone();
    assert_rule(
        renderer.fit_point_size_domain(&mut plain),
        RuleKind::PointSizeFitNoMap,
    );
    assert_eq!(plain, kept);
    for (column, rule) in [
        ("flat", Some(RuleKind::PointSizeFitExtent)),
        ("nan", Some(RuleKind::PointSizeFitExtent)),
        ("nowhere", None),
        ("streamed", None),
    ] {
        let mut sized =
            SeriesConfig::scatter("s", "t", "a").size_by(column, [0.0, 1.0], [2.0, 12.0]);
        let kept = sized.clone();
        let result = renderer.fit_point_size_domain(&mut sized);
        match rule {
            Some(rule) => assert_rule(result, rule),
            None if column == "nowhere" => {
                assert!(matches!(result, Err(FiggyError::UnknownColumn { .. })))
            }
            None => assert!(matches!(result, Err(FiggyError::ColumnNotResident { .. }))),
        }
        assert_eq!(sized, kept, "{column}: unchanged");
    }
}

/// The scale value of a vector series.
fn scale_of(series: &SeriesConfig) -> f64 {
    match &series.render_type {
        DataRenderType::Vector { style, .. } => match style.scale {
            VectorScale::Pixels { px_per_unit } => px_per_unit,
            VectorScale::Data { factor } => factor,
        },
        other => panic!("not a vector field: {other:?}"),
    }
}

// The vector scale makes the longest vector the extents allow `target`
// long, for points, grids and 3D points; only the scale changes.
#[test]
fn fit_vector_scale_targets_the_longest_vector() {
    let renderer = fit_renderer();
    let flat = Config::default();
    // u = `u` (20..30), v = `a` (-1..1): the longest is hypot(30, 1).
    let mut points = SeriesConfig::vectors("v", "t", "a", "u", "a")
        .vector_scale(VectorScale::Pixels { px_per_unit: 1.0 });
    let before = points.clone();
    renderer.fit_vector_scale(&flat, &mut points, 24.0).unwrap();
    let longest = 30f64.hypot(1.0);
    assert!((scale_of(&points) - 24.0 / longest).abs() < 1e-12);
    let mut expected = before.clone();
    if let DataRenderType::Vector { style, .. } = &mut expected.render_type {
        style.scale = VectorScale::Pixels {
            px_per_unit: scale_of(&points),
        };
    }
    assert_eq!(points, expected, "only the scale changes");

    // A data scale takes data units.
    let mut data = SeriesConfig::vectors("v", "t", "a", "u", "a")
        .vector_scale(VectorScale::Data { factor: 1.0 });
    renderer.fit_vector_scale(&flat, &mut data, 2.0).unwrap();
    assert!((scale_of(&data) - 2.0 / longest).abs() < 1e-12);

    // A grid over two x positions (`pair`) and the eleven `t` values: u
    // from the union of its columns (`a`, `b`: -1..200), v from `t` and `u`
    // (0..30).
    let mut grid = SeriesConfig::vectors_grid(
        "g",
        "pair",
        "t",
        MatrixRef::columns_are_x(["a", "b"]),
        MatrixRef::columns_are_x(["t", "u"]),
    )
    .vector_scale(VectorScale::Pixels { px_per_unit: 1.0 });
    renderer.fit_vector_scale(&flat, &mut grid, 30.0).unwrap();
    assert!((scale_of(&grid) - 30.0 / 200f64.hypot(30.0)).abs() < 1e-12);

    // 3D points fit w too on a 3D chart, and ignore it on a flat one.
    let mut cube = Config::default();
    cube.set_view_3d(Some(renderer::View3d::default()));
    let spatial = SeriesConfig::vectors_3d("w", "t", "a", "z", "u", "a", "z")
        .vector_scale(VectorScale::Pixels { px_per_unit: 1.0 });
    let mut in_3d = spatial.clone();
    renderer.fit_vector_scale(&cube, &mut in_3d, 10.0).unwrap();
    let longest_3d = (30f64 * 30.0 + 1.0 + 10.0 * 10.0).sqrt();
    assert!((scale_of(&in_3d) - 10.0 / longest_3d).abs() < 1e-12);
    let mut on_flat = spatial.clone();
    renderer
        .fit_vector_scale(&flat, &mut on_flat, 10.0)
        .unwrap();
    assert!((scale_of(&on_flat) - 10.0 / longest).abs() < 1e-12);

    // Refusals leave the series as it was.
    let mut line = SeriesConfig::line("l", "t", "a");
    let kept = line.clone();
    assert_rule(
        renderer.fit_vector_scale(&flat, &mut line, 10.0),
        RuleKind::VectorFitNotVector,
    );
    assert_eq!(line, kept);
    for target in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut field = before.clone();
        assert_rule(
            renderer.fit_vector_scale(&flat, &mut field, target),
            RuleKind::VectorFitTarget,
        );
        assert_eq!(field, before);
    }
    for (u, v) in [("zero", "zero"), ("nan", "a")] {
        let mut field = SeriesConfig::vectors("v", "t", "a", u, v);
        let kept = field.clone();
        assert_rule(
            renderer.fit_vector_scale(&flat, &mut field, 10.0),
            RuleKind::VectorFitExtent,
        );
        assert_eq!(field, kept, "{u}, {v}: unchanged");
    }
    let mut streamed = SeriesConfig::vectors("v", "t", "a", "streamed", "a");
    assert!(matches!(
        renderer.fit_vector_scale(&flat, &mut streamed, 10.0),
        Err(FiggyError::ColumnNotResident { .. })
    ));
    let mut missing = SeriesConfig::vectors("v", "t", "a", "u", "nowhere");
    assert!(matches!(
        renderer.fit_vector_scale(&flat, &mut missing, 10.0),
        Err(FiggyError::UnknownColumn { .. })
    ));
}

// Column-backed series have no count limit (rule 10): a candidate with many
// series and overlay members fits them all.
#[test]
fn a_candidate_with_many_series_fits() {
    let renderer = fit_renderer();
    let ids: Vec<String> = (0..2000).map(|i| format!("s{i}")).collect();
    let series: Vec<SeriesConfig> = ids
        .iter()
        .map(|id| SeriesConfig::line(id.as_str(), "t", "b"))
        .collect();
    let members: Vec<&str> = ids[1000..].iter().map(String::as_str).collect();
    let chart =
        Chart::sized(320, 240).with_overlay(AxisOptions::overlay_y("B", Color::BLACK), &members);
    let fitted = renderer.fit_config(chart.config(), &series, 0.05).unwrap();
    assert_range(&fitted.left_y, 95.0, 205.0);
    assert_range(&fitted.overlays[0].y_axis, 95.0, 205.0);
}
