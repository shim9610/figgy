//! Candidate fits and style domain fits (#312): `Renderer::fit_config`,
//! `Renderer::fit_y_axes_config`, the scoped `Renderer::fit_config_scoped`
//! (#334), `Renderer::fit_point_size_domain` and
//! `Renderer::fit_vector_scale`, all from the resident columns' cached
//! statistics, with no chart installed or changed.
#![cfg(not(target_arch = "wasm32"))]

use renderer::config::{AxisOptions, Config};
use renderer::data_config::{MatrixRef, VectorScale};
use renderer::{
    CellRef, Chart, Color, DataRenderType, FIT_RANGES_PADDING, FiggyError, FitAxes, FitAxis,
    FitAxisRef, FitScope, LayerRef, LinkedFitData, Renderer, RuleKind, SeriesConfig, VectorSeries,
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
    let (config, mut series) = layered();
    let before = config.clone();
    let fitted = renderer
        .fit_config(&config, &series, FIT_RANGES_PADDING)
        .unwrap();
    assert_eq!(config, before, "the candidate is not changed");

    let mut chart = Chart::new(config.clone());
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
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

    // A registered chart is untouched by a candidate fit (registered as
    // given, without its one-time fit).
    let mut renderer = renderer;
    let id = renderer
        .register_chart_as_given(config.clone(), series.clone())
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
        DataRenderType::Vector(VectorSeries { style, .. }) => match style.scale {
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
    if let DataRenderType::Vector(VectorSeries { style, .. }) = &mut expected.render_type {
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

fn style_of(series: &SeriesConfig) -> &renderer::data_config::VectorStyle {
    match &series.render_type {
        DataRenderType::Vector(VectorSeries { style, .. }) => style,
        other => panic!("not a vector field: {other:?}"),
    }
}

// An auto-scaled vector series (the constructors' default, #357) fits its
// longest vector to the spacing between its arrows over the data area, and
// the fit consumes its `auto_scale` flag; an explicit scale, a data scale,
// zero vectors and other series stay. A second fit changes nothing, the fit
// refuses streamed and unknown columns before any write, and it counts
// every column of a grid.
#[test]
fn fit_vector_scales_sizes_auto_scaled_arrows_to_their_spacing() {
    let renderer = fit_renderer();
    let chart = Chart::sized(800, 600);
    let config = chart.config();
    let area = config.data_area().unwrap().0;
    let (width, height) = (f64::from(area.width), f64::from(area.height));
    let mut series = vec![
        SeriesConfig::vectors("auto", "t", "a", "u", "a"),
        SeriesConfig::vectors("fixed", "t", "a", "u", "a")
            .vector_scale(VectorScale::Pixels { px_per_unit: 3.0 }),
        SeriesConfig::vectors("data", "t", "a", "u", "a")
            .style_vectors(|style| style.scale = VectorScale::Data { factor: 1.0 }),
        SeriesConfig::vectors("zero", "t", "a", "zero", "zero"),
        SeriesConfig::line("l", "t", "a"),
    ];
    assert!(style_of(&series[0]).auto_scale, "on by default");
    assert!(!style_of(&series[1]).auto_scale, "an explicit scale");
    let before = series.clone();
    renderer.fit_vector_scales(config, &mut series).unwrap();
    let style = style_of(&series[0]);
    assert!(!style.auto_scale, "the first fit consumes the flag");
    let target = style.auto_scale_target_px(11, width, height).unwrap();
    assert!((scale_of(&series[0]) - target / 30f64.hypot(1.0)).abs() < 1e-12);
    assert_eq!(series[1..], before[1..], "only the auto-scaled field moves");
    let once = series.clone();
    renderer.fit_vector_scales(config, &mut series).unwrap();
    assert_eq!(series, once, "the stored scale stays");
    // Zero vectors had nothing to fit: they keep the flag.
    assert!(style_of(&series[3]).auto_scale);

    // A grid's arrows are every cell of its `u` columns.
    let mut grid = vec![SeriesConfig::vectors_grid(
        "g",
        "pair",
        "t",
        MatrixRef::columns_are_x(["a", "b"]),
        MatrixRef::columns_are_x(["t", "u"]),
    )];
    renderer.fit_vector_scales(config, &mut grid).unwrap();
    let target = style_of(&grid[0])
        .auto_scale_target_px(22, width, height)
        .unwrap();
    assert!((scale_of(&grid[0]) - target / 200f64.hypot(30.0)).abs() < 1e-12);

    // Refusals change no series, not even the ones before the refused.
    for bad in ["streamed", "nowhere"] {
        let mut series = vec![
            SeriesConfig::vectors("auto", "t", "a", "u", "a"),
            SeriesConfig::vectors("bad", "t", "a", bad, "a"),
        ];
        let kept = series.clone();
        assert!(matches!(
            renderer.fit_vector_scales(config, &mut series),
            Err(FiggyError::ColumnNotResident { .. } | FiggyError::UnknownColumn { .. })
        ));
        assert_eq!(series, kept, "{bad}");
    }
}

// The installed whole-chart fit (`commit_auto_fit_all`, the WASM
// `auto_fit_all`) also sizes the chart's auto-scaled arrows, writing the
// scale into the chart's series and consuming the flag: the SSOT says how
// long they draw (#357), and a second fit keeps the stored scale.
#[test]
fn the_installed_auto_fit_sizes_auto_scaled_arrows() {
    let mut renderer = fit_renderer();
    renderer.ensure_errorbar_extent_engine_blocking().unwrap();
    let series = vec![
        SeriesConfig::vectors("auto", "t", "a", "u", "a"),
        SeriesConfig::vectors("fixed", "t", "a", "u", "a")
            .vector_scale(VectorScale::Pixels { px_per_unit: 3.0 }),
    ];
    let id = renderer
        .register_chart_as_given(Chart::sized(800, 600).config().clone(), series.clone())
        .unwrap();
    let fit = |renderer: &mut Renderer| {
        let ticket = renderer.begin_auto_fit_all(id, 0.05).unwrap();
        let resolved = ticket.resolve_blocking().unwrap();
        renderer.commit_auto_fit_all(resolved).unwrap();
    };
    fit(&mut renderer);
    let mut expected = series;
    renderer
        .fit_vector_scales(renderer.chart_config(id).unwrap(), &mut expected)
        .unwrap();
    let installed = renderer.chart_series(id).unwrap().to_vec();
    assert_eq!(installed, expected);
    assert!(scale_of(&installed[0]) > 0.5, "{:?}", installed[0]);
    assert!(!style_of(&installed[0]).auto_scale, "consumed");
    assert_eq!(scale_of(&installed[1]), 3.0);
    fit(&mut renderer);
    assert_eq!(renderer.chart_series(id).unwrap(), &installed[..]);

    // The registered chart draws what its SSOT says: its PNG export at 1x
    // and 2x is the export of its config and series held by the caller.
    let held = Chart::new(renderer.chart_config(id).unwrap().clone());
    for scale in [1.0, 2.0] {
        let options = renderer::ExportOptions::panel(scale).with_clear(Color::WHITE);
        let registered = renderer
            .export_rgba_blocking(renderer::ExportSource::Registered(id), options)
            .unwrap();
        let own = renderer
            .export_rgba_blocking(renderer::ExportSource::chart(&held, &installed), options)
            .unwrap();
        assert_eq!(registered.rgba, own.rgba, "{scale}x");
    }
}

// One rule for every renderer-fitted value (#357): registering a new chart
// (`LayoutState::Fitted`) fits its ranges, layout and vector scales once, by
// the code `fit_ranges` runs, and stores them; nothing refits them on its
// own. A `Fixed` document and `register_chart_as_given` are registered as
// given, and a fit that cannot run refuses the registration.
#[test]
fn registration_fits_a_new_chart_once() {
    let mut renderer = fit_renderer();
    let series = vec![
        SeriesConfig::line("l", "t", "b"),
        SeriesConfig::vectors("v", "t", "a", "u", "a"),
    ];
    let config = Chart::sized(800, 600).config().clone();
    assert!(config.layout_state.is_fitted());
    let id = renderer
        .register_chart(config.clone(), series.clone())
        .unwrap();

    // What the host's own fit gives: the same ranges, layout and scale.
    let mut held = Chart::new(config.clone());
    let mut expected = series.clone();
    renderer.fit_ranges(&mut held, &mut expected).unwrap();
    assert_eq!(renderer.chart_config(id).unwrap(), held.config());
    assert_eq!(renderer.chart_series(id).unwrap(), &expected[..]);
    assert_range(&held.config().bottom_x, -0.5, 10.5);
    assert_range(&held.config().left_y, -11.05, 210.05);
    assert!(!style_of(&expected[1]).auto_scale, "the flag is consumed");
    assert!(scale_of(&expected[1]) > 0.0);

    // Stored, not refitted: a config edit through the registry keeps the
    // fitted ranges and scale, and the export draws them.
    let mut edited = renderer.chart_config(id).unwrap().clone();
    edited.chart_title.text.font_size += 2.0;
    renderer.set_chart_config(id, edited).unwrap();
    assert_range(&renderer.chart_config(id).unwrap().left_y, -11.05, 210.05);
    assert_eq!(renderer.chart_series(id).unwrap(), &expected[..]);
    let stored = Chart::as_given(renderer.chart_config(id).unwrap().clone());
    for scale in [1.0, 2.0] {
        let options = renderer::ExportOptions::panel(scale).with_clear(Color::WHITE);
        let registered = renderer
            .export_rgba_blocking(renderer::ExportSource::Registered(id), options)
            .unwrap();
        let own = renderer
            .export_rgba_blocking(renderer::ExportSource::chart(&stored, &expected), options)
            .unwrap();
        assert_eq!(registered.rgba, own.rgba, "{scale}x");
    }

    // The opt-outs: a `Fixed` document and `register_chart_as_given` keep
    // the SSOT exactly as given, placeholder ranges and flag included.
    let mut fixed = config.clone();
    fixed.layout_state = renderer::LayoutState::Fixed;
    let as_doc = renderer
        .register_chart(fixed.clone(), series.clone())
        .unwrap();
    assert_eq!(renderer.chart_config(as_doc).unwrap(), &fixed);
    assert_eq!(renderer.chart_series(as_doc).unwrap(), &series[..]);
    let given = renderer
        .register_chart_as_given(config.clone(), series.clone())
        .unwrap();
    assert_eq!(renderer.chart_config(given).unwrap(), &config);
    assert_eq!(renderer.chart_series(given).unwrap(), &series[..]);
    assert_eq!(Chart::as_given(config.clone()).config(), &config);

    // A new chart over a streamed source cannot be fitted (rule 8): the
    // registration is refused and nothing is registered; as given, it is.
    let streamed = vec![SeriesConfig::line("s", "t", "streamed")];
    let charts = renderer.chart_order().len();
    assert!(matches!(
        renderer.register_chart(config.clone(), streamed.clone()),
        Err(FiggyError::ColumnNotResident { id }) if id == "streamed"
    ));
    assert_eq!(renderer.chart_order().len(), charts);
    renderer.register_chart_as_given(config, streamed).unwrap();
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

/// A parent drawing `a` (x `t` 0..10, y `a` -1..1) with an overlay `o` that
/// follows its X axis and has a Y axis of its own (x `t`, y `b` 100..200),
/// and a subplot drawing `s` (x `u` 20..30, y `z` 5..10) whose X axis is
/// linked to the parent's. Every axis starts at a hand-set range, the links
/// in step.
fn linked() -> (Config, Vec<SeriesConfig>) {
    let series = vec![
        SeriesConfig::line("a", "t", "a"),
        SeriesConfig::line("o", "t", "b"),
        SeriesConfig::line("s", "u", "z"),
    ];
    let inner = Chart::placed(320, 0, 320, 240).config().clone();
    let mut config = Chart::sized(320, 240)
        .with_overlay(AxisOptions::overlay_y("B", Color::BLACK), &["o"])
        .with_subplot(renderer::Subplot::new(inner, &["s"]).with_link_x(CellRef::Parent))
        .config()
        .clone();
    let set = |axis: &mut AxisOptions, min: f64, max: f64| {
        axis.min = min;
        axis.max = max;
    };
    set(&mut config.bottom_x, 2.0, 3.0);
    set(&mut config.top_x, 2.0, 3.0);
    set(&mut config.left_y, 4.0, 5.0);
    set(&mut config.right_y, 4.0, 5.0);
    set(&mut config.overlays[0].y_axis, 600.0, 700.0);
    set(&mut config.subplots[0].config.left_y, 8.0, 9.0);
    set(&mut config.subplots[0].config.right_y, 8.0, 9.0);
    config.sync_subplot_links(None);
    (config, series)
}

fn scope(layer: LayerRef, axes: FitAxes) -> FitScope<'static> {
    FitScope {
        layer,
        axes,
        linked: LinkedFitData::Group,
        keep: &[],
    }
}

const X: FitAxes = FitAxes {
    x: true,
    y: false,
    z: false,
};
const Y: FitAxes = FitAxes {
    x: false,
    y: true,
    z: false,
};

/// Every Y-side axis of `fitted` is `given`'s.
#[track_caller]
fn assert_y_axes_kept(fitted: &Config, given: &Config) {
    assert_eq!(fitted.left_y, given.left_y);
    assert_eq!(fitted.right_y, given.right_y);
    assert_eq!(fitted.overlays[0].y_axis, given.overlays[0].y_axis);
    let (cell, kept) = (&fitted.subplots[0].config, &given.subplots[0].config);
    assert_eq!(cell.left_y, kept.left_y);
    assert_eq!(cell.right_y, kept.right_y);
}

// An X-only fit moves the parent's X axis and its linked subplot with it,
// and leaves every Y axis as given. The group reads every series drawn
// against the link group; the target reads only its own panel's, and the
// group still shares the range.
#[test]
fn fit_config_scoped_moves_only_the_selected_axes() {
    let renderer = fit_renderer();
    let (config, series) = linked();
    let before = config.clone();

    let group = renderer
        .fit_config_scoped(&config, &series, 0.05, &scope(LayerRef::Base, X))
        .unwrap();
    assert_eq!(config, before, "the candidate is not changed");
    // `a` and `o` span 0..10 and the linked subplot's `s` 20..30.
    assert_range(&group.bottom_x, -1.5, 31.5);
    assert_range(&group.top_x, -1.5, 31.5);
    assert_range(&group.subplots[0].config.bottom_x, -1.5, 31.5);
    assert_y_axes_kept(&group, &config);

    let target = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.05,
            &FitScope {
                linked: LinkedFitData::Target,
                ..scope(LayerRef::Base, X)
            },
        )
        .unwrap();
    assert_range(&target.bottom_x, -0.5, 10.5);
    assert_range(&target.subplots[0].config.bottom_x, -0.5, 10.5);
    assert_y_axes_kept(&target, &config);

    // From the subplot, the target is the subplot's panel: `s` alone sets
    // the group's range.
    let from_cell = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.0,
            &FitScope {
                linked: LinkedFitData::Target,
                ..scope(LayerRef::Subplot(0), X)
            },
        )
        .unwrap();
    assert_range(&from_cell.bottom_x, 20.0, 30.0);
    assert_range(&from_cell.subplots[0].config.bottom_x, 20.0, 30.0);
    assert_y_axes_kept(&from_cell, &config);

    // A Y-only fit of the overlay moves its own Y axis alone.
    let overlay = renderer
        .fit_config_scoped(&config, &series, 0.05, &scope(LayerRef::Overlay(0), Y))
        .unwrap();
    assert_range(&overlay.overlays[0].y_axis, 95.0, 205.0);
    let mut expected = config.clone();
    expected.overlays[0].y_axis = overlay.overlays[0].y_axis.clone();
    assert_eq!(overlay, expected);

    // No axis selected: nothing moves.
    let none = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.05,
            &scope(LayerRef::Base, FitAxes::default()),
        )
        .unwrap();
    assert_eq!(none, config);

    // Every axis of a chart without links or overlays is `fit_config`'s.
    let plain = Chart::sized(320, 240).config().clone();
    let only = [SeriesConfig::line("a", "t", "a")];
    assert_eq!(
        renderer
            .fit_config_scoped(&plain, &only, 0.05, &scope(LayerRef::Base, FitAxes::ALL))
            .unwrap(),
        renderer.fit_config(&plain, &only, 0.05).unwrap()
    );
}

// A kept axis holds its range and every axis linked to it: the subplot's X
// is the parent's, so keeping the parent X holds the subplot's fit too.
#[test]
fn fit_config_scoped_keeps_the_kept_link_groups() {
    let renderer = fit_renderer();
    let (config, series) = linked();
    let subplot_x = scope(LayerRef::Subplot(0), X);
    let moved = renderer
        .fit_config_scoped(&config, &series, 0.05, &subplot_x)
        .unwrap();
    assert_range(&moved.subplots[0].config.bottom_x, -1.5, 31.5);
    assert_range(&moved.bottom_x, -1.5, 31.5);

    let keep = [FitAxisRef {
        layer: LayerRef::Base,
        axis: FitAxis::X,
    }];
    let held = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.05,
            &FitScope {
                keep: &keep,
                ..subplot_x
            },
        )
        .unwrap();
    assert_eq!(held, config);

    // Keeping the overlay's Y while fitting the overlay leaves it, and
    // keeping the subplot's own Y does not hold the parent's.
    let keep = [FitAxisRef {
        layer: LayerRef::Overlay(0),
        axis: FitAxis::Y,
    }];
    let held = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.05,
            &FitScope {
                keep: &keep,
                ..scope(LayerRef::Overlay(0), Y)
            },
        )
        .unwrap();
    assert_eq!(held, config);
    let keep = [FitAxisRef {
        layer: LayerRef::Subplot(0),
        axis: FitAxis::Y,
    }];
    let parent_y = renderer
        .fit_config_scoped(
            &config,
            &series,
            0.05,
            &FitScope {
                keep: &keep,
                ..scope(LayerRef::Base, Y)
            },
        )
        .unwrap();
    assert_range(&parent_y.left_y, -1.1, 1.1);
    assert_eq!(parent_y.subplots[0], config.subplots[0]);
}

// On a 3D chart a Y-only fit leaves the z axis (`right_y`) as given, and a
// z-only fit moves it alone.
#[test]
fn fit_config_scoped_separates_y_and_z_in_3d() {
    let renderer = fit_renderer();
    let mut cube = Chart::sized(320, 240);
    cube.config_mut()
        .set_view_3d(Some(renderer::View3d::default()));
    let mut config = cube.config().clone();
    config.right_y.min = 40.0;
    config.right_y.max = 50.0;
    let points = [SeriesConfig::scatter("p", "t", "a").z("z")];
    let y = renderer
        .fit_config_scoped(&config, &points, 0.05, &scope(LayerRef::Base, Y))
        .unwrap();
    assert_range(&y.left_y, -1.1, 1.1);
    assert_eq!(y.right_y, config.right_y);
    assert_eq!(y.bottom_x, config.bottom_x);
    let z = FitAxes {
        z: true,
        ..FitAxes::default()
    };
    let z = renderer
        .fit_config_scoped(&config, &points, 0.05, &scope(LayerRef::Base, z))
        .unwrap();
    assert_range(&z.right_y, 4.75, 10.25);
    assert_eq!(z.left_y, config.left_y);
    assert_eq!(z.bottom_x, config.bottom_x);
}

// Every refusal is an error, the candidate is not changed and an installed
// chart is untouched.
#[test]
fn fit_config_scoped_refuses_what_it_cannot_fit() {
    let mut renderer = fit_renderer();
    let (config, series) = linked();
    let before = config.clone();
    for layer in [
        LayerRef::Overlay(1),
        LayerRef::Subplot(1),
        LayerRef::SubplotOverlay(0, 0),
        LayerRef::SubplotOverlay(3, 0),
    ] {
        let error = renderer
            .fit_config_scoped(&config, &series, 0.05, &scope(layer, X))
            .unwrap_err();
        assert!(
            matches!(
                error,
                FiggyError::InvalidConfig {
                    field: "fit_scope",
                    kind: RuleKind::FitScopeLayer
                }
            ),
            "{layer:?}: {error}"
        );
        let keep = [FitAxisRef {
            layer,
            axis: FitAxis::Y,
        }];
        assert_rule(
            renderer.fit_config_scoped(
                &config,
                &series,
                0.05,
                &FitScope {
                    keep: &keep,
                    ..scope(LayerRef::Base, X)
                },
            ),
            RuleKind::FitScopeLayer,
        );
    }
    for padding in [-0.1, f64::NAN] {
        assert_rule(
            renderer.fit_config_scoped(&config, &series, padding, &scope(LayerRef::Base, X)),
            RuleKind::FitPadding,
        );
    }
    let mut ghost = config.clone();
    ghost.overlays[0].series.push("ghost".into());
    assert_rule(
        renderer.fit_config_scoped(&ghost, &series, 0.05, &scope(LayerRef::Base, X)),
        RuleKind::LayerSeriesUnknown,
    );
    let streamed = [SeriesConfig::line("a", "t", "streamed")];
    assert!(matches!(
        renderer.fit_config_scoped(&Config::default(), &streamed, 0.05, &scope(LayerRef::Base, Y)),
        Err(FiggyError::ColumnNotResident { id }) if id == "streamed"
    ));
    let monotonic = [SeriesConfig::monotonic_line("m", "daq", 0)];
    assert_rule(
        renderer.fit_config_scoped(
            &Config::default(),
            &monotonic,
            0.05,
            &scope(LayerRef::Base, X),
        ),
        RuleKind::MonotonicMappingEdit,
    );
    assert_eq!(config, before);

    // A registered chart is untouched by a scoped candidate fit. The
    // registration fits the layout of the config it is given (#357), so
    // the chart is compared with itself before the candidate fit.
    let id = renderer
        .register_chart_as_given(config.clone(), series.clone())
        .unwrap();
    let registered = renderer.chart_config(id).unwrap().clone();
    renderer
        .fit_config_scoped(&config, &series, 0.05, &scope(LayerRef::Base, FitAxes::ALL))
        .unwrap();
    assert_eq!(renderer.chart_config(id).unwrap(), &registered);
}

/// The speed-band shares of #417's wind rose: sixteen directions from N
/// clockwise, four bands each, as fractions of all hours.
const WIND_ROSE: [[f64; 4]; 16] = [
    [0.009636, 0.015251, 0.007903, 0.002426],
    [0.010121, 0.014627, 0.006932, 0.003674],
    [0.009983, 0.013241, 0.008666, 0.005338],
    [0.009775, 0.015598, 0.011092, 0.009497],
    [0.011023, 0.016846, 0.011924, 0.010260],
    [0.010121, 0.018718, 0.012478, 0.008042],
    [0.011924, 0.017192, 0.009636, 0.004367],
    [0.011231, 0.015737, 0.007140, 0.003536],
    [0.010884, 0.018094, 0.009151, 0.006447],
    [0.012894, 0.022114, 0.013934, 0.013033],
    [0.011854, 0.024749, 0.018302, 0.015945],
    [0.013449, 0.031612, 0.023847, 0.015598],
    [0.011646, 0.035563, 0.030364, 0.024957],
    [0.014627, 0.033899, 0.030225, 0.037712],
    [0.012548, 0.031612, 0.032721, 0.046863],
    [0.011854, 0.020867, 0.015945, 0.012756],
];

/// #417's stacked wind rose on a headless renderer: a polar chart of
/// sixteen categories, four stacked bands, the radius labels in
/// `LabelFormat::Percent`.
fn wind_rose() -> (Renderer, Config, Vec<SeriesConfig>) {
    use renderer::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
    let mut renderer = Renderer::headless_blocking().expect("headless renderer");
    let directions: Vec<f64> = (0..WIND_ROSE.len()).map(|i| i as f64).collect();
    renderer.add_column("directions", &directions).unwrap();
    let series: Vec<SeriesConfig> = (0..4)
        .map(|band| {
            let id = format!("band{band}");
            let values: Vec<f64> = WIND_ROSE.iter().map(|row| row[band]).collect();
            renderer.add_column(&id, &values).unwrap();
            SeriesConfig::category_bars(id.as_str(), "directions", id.as_str())
        })
        .collect();
    let mut config = renderer::default::default_config();
    config.bottom_x.categories = Some(
        (0..WIND_ROSE.len())
            .map(|i| renderer::config::Category::new(format!("row-{i}"), format!("D{i}")))
            .collect(),
    );
    config.set_polar(Some(renderer::polar::PolarOptions {
        clockwise: true,
        zero_angle_degrees: 90.0,
        ..renderer::polar::PolarOptions::default()
    }));
    config.bar_groups = Some(BarGroups {
        direction: CategoryBarDirection::Vertical,
        mode: CategoryBarMode::Stacked,
        group_width: 0.75,
        bar_gap: 0.0,
    });
    config.left_y.label_style.format = renderer::format::LabelFormat::Percent;
    (renderer, config, series)
}

// #417: an explicit `Percent` on the linear radius axis survives the
// candidate fit and `fit_ranges`, while the range and the spacing move; an
// axis on `Auto` stays `Auto`.
#[test]
fn a_range_fit_keeps_the_radius_axis_percent_format() {
    use renderer::format::{AutoNumberFormat, LabelFormat};
    let (renderer, config, mut series) = wind_rose();
    let before = config.left_y.clone();
    let fitted = renderer
        .fit_config(&config, &series, FIT_RANGES_PADDING)
        .unwrap();
    assert_eq!(fitted.left_y.label_style.format, LabelFormat::Percent);
    assert_eq!(
        fitted.left_y.label_style.significant_digits,
        before.label_style.significant_digits
    );
    // The stack bound of the four bands, padded.
    let top: f64 = (0..4)
        .map(|band| WIND_ROSE.iter().map(|row| row[band]).fold(0.0, f64::max))
        .sum();
    assert!(fitted.left_y.max > top && fitted.left_y.max < top * 1.2);
    assert_ne!(fitted.left_y.major_spacing, before.major_spacing);
    assert!(fitted.left_y.major_spacing > 0.0 && fitted.left_y.major_spacing < top);

    let mut chart = Chart::new(config.clone());
    renderer.fit_ranges(&mut chart, &mut series).unwrap();
    // One fit path for the axes; `fit_ranges` also chooses the radius
    // labels' ray (#417), which a candidate fit leaves to its installer.
    assert_eq!(fitted.left_y, chart.config().left_y, "one fit path");
    assert_eq!(
        chart.config().left_y.label_style.format,
        LabelFormat::Percent
    );

    // The same chart on `Auto` keeps `Auto`.
    let mut auto = config;
    auto.left_y.label_style.format = LabelFormat::Auto(AutoNumberFormat::DEFAULT);
    let fitted = renderer
        .fit_config(&auto, &series, FIT_RANGES_PADDING)
        .unwrap();
    assert_eq!(
        fitted.left_y.label_style.format,
        LabelFormat::Auto(AutoNumberFormat::DEFAULT)
    );
}
