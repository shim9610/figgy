use super::*;
use crate::annotation::{Annotation, AnnotationAxis, AnnotationKind};
use crate::config::{Category, PickedDataRef, TickVisibility};
use crate::data_config::MatrixRef;
use crate::layer::{OverlayLayer, Subplot};
use crate::layout::{ChartArea, Rect};
use crate::legend::{append_legend_entry_rich, series_symbol_segments};

fn themes() -> [Theme; 5] {
    [
        Theme::light(),
        Theme::dark(),
        Theme::paper(),
        Theme::dashboard(),
        Theme::sketch(),
    ]
}

fn grid(prefix: &str) -> MatrixRef {
    MatrixRef::columns_are_x((0..3).map(|i| format!("{prefix}{i}")))
}

fn categorical() -> Config {
    let mut config = Config::default();
    config.bottom_x.categories = Some(
        (0..3)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect(),
    );
    config
}

fn valid_on(config: &Config, series: &[SeriesConfig]) {
    for item in series {
        if let Err(error) = config.validate_series(std::slice::from_ref(item)) {
            panic!("{:?}: {error:?}", item.render_type);
        }
    }
}

/// A chart with content in every field a theme keeps: ranges, titles,
/// categories, an annotation, an overlay, a subplot and a selection.
fn rich_chart() -> (Config, Vec<SeriesConfig>) {
    let mut config = Config::default();
    config.chart_area = ChartArea(Rect {
        x: 10,
        y: 20,
        width: 900,
        height: 500,
    });
    config.bottom_x.min = -3.0;
    config.bottom_x.max = 7.0;
    config.bottom_x.major_spacing = 2.5;
    config.left_y.scale = crate::config::AxisScale::Logarithmic;
    config.left_y.min = 0.1;
    config.left_y.max = 100.0;
    config.left_y.out_margin = 123.0;
    config.chart_title.text = RichText::plain("Title", Color::BLACK, 28.0, "");
    config.chart_title.offset_x = 14.0;
    config.bottom_x.title_option.text = RichText::plain("Time", Color::BLACK, 22.0, "");
    config.legend.visible = true;
    config.legend.offset_y = 9.0;
    config
        .annotations
        .items
        .push(Annotation::new(AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: 0.0,
            to: 1.0,
            fill: Color::new(1.0, 0.0, 0.0, 0.4),
        }));
    let mut overlay = OverlayLayer::default();
    overlay.series.push("b".into());
    overlay.y_axis.min = 5.0;
    overlay.y_axis.max = 6.0;
    config.overlays.push(overlay);
    for index in 0..2 {
        let mut subplot = Subplot::default();
        subplot.config.chart_area = ChartArea(Rect {
            x: 950,
            y: 20 + 260 * index,
            width: 300,
            height: 240,
        });
        subplot.series.push(format!("s{index}"));
        config.subplots.push(subplot);
    }
    config.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "a".into(),
            point_index: 3,
        }],
        ..crate::defaults::data_selections_config()
    });
    let series = vec![
        SeriesConfig::line("a", "x", "y").label("first"),
        SeriesConfig::scatter("b", "x", "y").label("second"),
        SeriesConfig::line("s0", "x", "y"),
        SeriesConfig::line("s1", "x", "y"),
    ];
    for item in &series {
        if let Some(label) = &item.label {
            let symbol = series_symbol_segments(item, None);
            append_legend_entry_rich(&mut config.legend.content, symbol, label.segments.clone());
        }
    }
    (config, series)
}

fn line_color(series: &SeriesConfig) -> Color {
    match &series.render_type {
        DataRenderType::Xy(xy) => xy
            .line
            .as_ref()
            .map(|line| line.line_color)
            .or(xy.scatter.as_ref().map(|scatter| scatter.point_color))
            .unwrap(),
        _ => panic!("an x-y series"),
    }
}

// The light theme is the documented default look: applying it to a default
// document changes nothing, at any subplot scope.
#[test]
fn light_on_a_default_document_changes_nothing() {
    for scope in [SubplotScope::None, SubplotScope::All] {
        let mut config = Config::default();
        Theme::light().apply_chrome(&mut config, scope);
        assert_eq!(config, Config::default());
    }
    let mut radial = crate::defaults::radial_chart();
    let before = radial.clone();
    Theme::light().apply_radial(&mut radial);
    assert_eq!(radial, before);
}

// Every theme writes the look and keeps the content: ranges, scales, the
// chart area, offsets, text, the annotation layer, the overlay and subplot
// structure, and the selection's refs.
#[test]
fn a_theme_keeps_the_content() {
    for theme in themes() {
        let (before, series_before) = rich_chart();
        let mut config = before.clone();
        let mut series = series_before.clone();
        theme.apply(&mut config, &mut series, SubplotScope::All);

        assert_eq!(config.chart_area, before.chart_area);
        for (axis, kept) in [
            (&config.bottom_x, &before.bottom_x),
            (&config.left_y, &before.left_y),
        ] {
            assert_eq!(
                (axis.scale.clone(), axis.min, axis.max, axis.major_spacing),
                (kept.scale.clone(), kept.min, kept.max, kept.major_spacing)
            );
            assert_eq!(axis.out_margin, kept.out_margin);
            assert_eq!(
                axis.title_option.text.segments,
                kept.title_option.text.segments
            );
        }
        assert_eq!(config.chart_title.offset_x, 14.0);
        assert_eq!(
            config.chart_title.text.segments,
            before.chart_title.text.segments
        );
        assert_eq!(config.legend.offset_y, 9.0);
        assert_eq!(config.annotations, before.annotations);
        assert_eq!(config.overlays.len(), 1);
        assert_eq!(config.overlays[0].series, before.overlays[0].series);
        assert_eq!(
            (config.overlays[0].y_axis.min, config.overlays[0].y_axis.max),
            (5.0, 6.0)
        );
        assert_eq!(config.subplots.len(), 2);
        for (subplot, kept) in config.subplots.iter().zip(&before.subplots) {
            assert_eq!(subplot.series, kept.series);
            assert_eq!(subplot.config.chart_area, kept.config.chart_area);
        }
        assert_eq!(
            config.picked_data.as_ref().unwrap().refs,
            before.picked_data.as_ref().unwrap().refs
        );
        for (item, kept) in series.iter().zip(&series_before) {
            assert_eq!(item.series_id, kept.series_id);
            assert_eq!(item.label, kept.label);
            let columns = |series: &SeriesConfig| {
                let mut names = Vec::new();
                series.visit_columns(&mut |name| names.push(name.to_owned()));
                names
            };
            assert_eq!(columns(item), columns(kept));
        }
        // The legend keeps its entry text.
        let text = |legend: &Legend| -> String {
            legend
                .content
                .segments
                .iter()
                .map(|segment| segment.text)
                .collect()
        };
        assert_eq!(text(&config.legend), text(&before.legend));
    }
}

// The look lands on the chart, its overlays and the chosen subplots, and on
// the series drawn there; the legend symbols follow the new series colours.
#[test]
fn a_theme_writes_the_look_where_it_applies() {
    let theme = Theme::dark();
    let (mut config, mut series) = rich_chart();
    let untouched = series[2].clone();
    theme.apply(&mut config, &mut series, SubplotScope::Only(&[1]));
    let chrome = &theme.chrome;
    assert_eq!(config.background, chrome.background);
    assert_eq!(config.draw_style, chrome.draw_style);
    assert_eq!(config.bottom_x.line_color, chrome.frame_ink);
    assert_eq!(config.bottom_x.label_style.color, chrome.label_ink);
    assert_eq!(config.bottom_x.title_option.text.color, chrome.ink);
    assert_eq!(config.chart_title.text.font_size, chrome.title_size);
    assert_eq!(config.grid.major_x_color, chrome.grid.major_x.color);
    assert_eq!(config.legend.bg_color, chrome.legend_fill);
    assert_eq!(config.legend.content.color, chrome.label_ink);
    // Overlays take the theme with their chart.
    assert_eq!(config.overlays[0].y_axis.line_color, chrome.frame_ink);
    // Subplot 1 is chosen, subplot 0 is not.
    assert_eq!(config.subplots[1].config.background, chrome.background);
    assert_eq!(
        config.subplots[1].config.bottom_x.line_color,
        chrome.frame_ink
    );
    assert_eq!(config.subplots[0].config.background, None);
    assert_eq!(config.subplots[0].config.bottom_x.line_color, Color::BLACK);
    assert_eq!(series[2], untouched, "a series of an unchosen subplot");
    assert_eq!(line_color(&series[3]), theme.series.cycle.color(3));
    // Series take the cycle by their place in the list.
    assert_eq!(line_color(&series[0]), theme.series.cycle.color(0));
    assert_eq!(line_color(&series[1]), theme.series.cycle.color(1));
    // The highlight look changes; the refs stay.
    let picked = config.picked_data.as_ref().unwrap();
    assert_eq!(picked.highlight_color, chrome.highlight.color);
    // Each legend symbol is the mark of its series as it now draws.
    let symbol_colors: Vec<_> = config
        .legend
        .content
        .segments
        .iter()
        .filter(|segment| segment.series_id.is_some())
        .filter_map(|segment| segment.color)
        .collect();
    assert!(symbol_colors.contains(&theme.series.cycle.color(0)));
    assert!(symbol_colors.contains(&theme.series.cycle.color(1)));
}

// The family parts write widths and sizes: lines, markers, stems and error
// bars on an x-y series.
#[test]
fn the_line_family_takes_widths_and_marker_sizes() {
    let theme = Theme::dashboard();
    let mut series = SeriesConfig::scatter_line("a", "x", "y").y_errors("e");
    if let DataRenderType::Xy(xy) = &mut series.render_type {
        xy.stem = Some(crate::data_config::Stem::default());
    }
    theme.style_series(&mut series, 0);
    let DataRenderType::Xy(xy) = &series.render_type else {
        panic!("an x-y series");
    };
    let lines = &theme.series.lines;
    assert_eq!(xy.line.as_ref().unwrap().line_width, lines.line_width);
    assert_eq!(xy.scatter.as_ref().unwrap().point_size, lines.marker_size);
    assert_eq!(xy.stem.as_ref().unwrap().width, lines.stem_width);
    let errors = &xy.errors.as_ref().unwrap().style;
    assert_eq!(errors.error_bar_width, lines.error_bar_width);
    assert_eq!(errors.cap_width, lines.error_cap_width);
}

// Every built-in theme leaves every family's series valid on its chart, and
// the chart valid too.
#[test]
fn every_theme_keeps_every_family_valid() {
    let ink = Color::BLACK;
    let mut polar = categorical();
    polar.set_polar(Some(crate::defaults::polar_options()));
    let mut ternary = Config::default();
    ternary.set_ternary(Some(crate::defaults::ternary_options()));
    let mut parallel = Config::default();
    let mut options = crate::defaults::parallel_options();
    options.axes = vec![
        crate::parallel::ParallelAxis::new(0.0, 1.0),
        crate::parallel::ParallelAxis::new(0.0, 1.0),
    ];
    parallel.set_parallel(Some(options));
    let cases: Vec<(Config, Vec<SeriesConfig>)> = vec![
        (
            Config::default(),
            vec![
                SeriesConfig::line("a", "x", "y"),
                SeriesConfig::scatter("b", "x", "y"),
                SeriesConfig::scatter_line("c", "x", "y").y_errors("e"),
                SeriesConfig::stem("d", "x", "y"),
                SeriesConfig::histogram("e", "edges", "counts"),
                SeriesConfig::heatmap("f", "x", "y", grid("z")),
                SeriesConfig::contours("g", "x", "y", grid("z"), vec![0.5]),
                SeriesConfig::hexbin("h", "x", "y", grid("z")),
                SeriesConfig::rgb_image("i", "x", "y", grid("r"), grid("g"), grid("b")),
                SeriesConfig::vectors("j", "x", "y", "u", "v"),
                SeriesConfig::streamlines("k", "x", "y"),
            ],
        ),
        (
            categorical(),
            vec![
                SeriesConfig::category_bars("a", "i", "v", ink),
                SeriesConfig::box_plot("b", "i", "records", ink),
                SeriesConfig::range_bars("c", "i", "start", "end", ink),
            ],
        ),
        (polar, vec![SeriesConfig::radar("a", "i", "v", ink)]),
        (ternary, vec![SeriesConfig::ternary("a", "p", "q", "r")]),
        (parallel, vec![SeriesConfig::parallel("a", ["p", "q"])]),
    ];
    for theme in themes() {
        for (config, series) in &cases {
            for item in series {
                let mut config = config.clone();
                let mut list = vec![item.clone()];
                theme.apply(&mut config, &mut list, SubplotScope::All);
                valid_on(&config, &list);
                assert!(config.validate().is_ok(), "{:?}", item.render_type);
            }
        }
    }
}

// A chart of energy levels takes the Grotrian frame and a Feynman diagram
// the hidden one, whatever the chrome's frame.
#[test]
fn a_family_with_its_own_frame_keeps_it() {
    let theme = Theme::dashboard();
    let mut config = Config::default();
    let mut series = [SeriesConfig::feynman_diagram(
        "f",
        crate::defaults::feynman_series(),
    )];
    theme.apply(&mut config, &mut series, SubplotScope::All);
    assert!(!config.bottom_x.line_visible);
    assert!(!config.bottom_x.label_style.label_visible);
    assert!(!config.grid.show_major_x || theme.chrome.grid.major_x.show);

    let mut config = Config::default();
    let mut series = [SeriesConfig::energy_diagram(
        "e",
        crate::defaults::energy_level_series(),
    )];
    theme.apply(&mut config, &mut series, SubplotScope::All);
    assert!(config.bottom_x.line_visible && config.top_x.line_visible);
    assert_eq!(config.bottom_x.tick, TickVisibility::Outside);
    let DataRenderType::EnergyLevels(levels) = &series[0].render_type else {
        panic!("an energy-level series");
    };
    assert_eq!(
        levels.style.transition.color,
        theme.series.energy_levels.transition_color
    );
}

// A chart without a selection gets the highlight look only when it differs
// from the documented one a first selection takes anyway.
#[test]
fn the_highlight_look_reaches_a_chart_without_a_selection() {
    let mut config = Config::default();
    Theme::light().apply_chrome(&mut config, SubplotScope::All);
    assert!(config.picked_data.is_none());
    let theme = Theme::paper();
    theme.apply_chrome(&mut config, SubplotScope::All);
    let picked = config.picked_data.as_ref().unwrap();
    assert!(picked.refs.is_empty());
    assert_eq!(picked.highlight_color, theme.chrome.highlight.color);
}

// Applying a theme twice is applying it once.
#[test]
fn applying_a_theme_again_changes_nothing() {
    for theme in themes() {
        let (mut config, mut series) = rich_chart();
        theme.apply(&mut config, &mut series, SubplotScope::All);
        let (once, once_series) = (config.clone(), series.clone());
        theme.apply(&mut config, &mut series, SubplotScope::All);
        assert_eq!(config, once);
        assert_eq!(series, once_series);
    }
}

// The radial chart takes its part: background, label look, material and
// the cycle on its top-level slices; values and labels stay.
#[test]
fn the_radial_chart_takes_its_part() {
    let theme = Theme::dark();
    let mut chart = crate::defaults::radial_chart();
    chart.slices = vec![
        crate::radial::RadialSlice::new("A", 1.0, Color::BLACK),
        crate::radial::RadialSlice::new("B", 2.0, Color::BLACK),
    ];
    theme.apply_radial(&mut chart);
    assert_eq!(chart.background, theme.series.radial.background);
    assert_eq!(chart.label_color, theme.series.radial.label_color);
    assert_eq!(chart.slices[1].color, theme.series.cycle.color(1));
    assert_eq!(
        (chart.slices[1].label.as_str(), chart.slices[1].value),
        ("B", 2.0)
    );
}

#[test]
fn the_built_in_themes_have_names() {
    for name in ["light", "dark", "paper", "dashboard", "sketch"] {
        assert!(Theme::named(name).is_some(), "{name}");
    }
    assert_eq!(Theme::named("dark"), Some(Theme::dark()));
    assert!(Theme::named("neon").is_none());
}

// An overlay's own axes take the theme's frame like the chart's data axes,
// so the dashboard draws no line or tick on an overlay's top or right axis
// either (#283).
#[test]
fn an_overlay_s_own_axes_take_the_theme_frame() {
    let theme = Theme::dashboard();
    let mut config = Config::default();
    config.overlays.push(OverlayLayer {
        series: vec!["b".into()],
        link_x: false,
        ..crate::defaults::overlay_layer()
    });
    let mut series = [
        SeriesConfig::line("a", "x", "y"),
        SeriesConfig::line("b", "u", "v"),
    ];
    theme.apply(&mut config, &mut series, SubplotScope::All);
    for axis in [
        &config.bottom_x,
        &config.overlays[0].x_axis,
        &config.overlays[0].y_axis,
    ] {
        assert!(!axis.line_visible);
        assert_eq!(axis.tick, TickVisibility::None);
        assert!(axis.label_style.label_visible);
    }
}
