use super::*;
use crate::annotation::{Annotation, AnnotationAxis, AnnotationKind};
use crate::config::{AxisScale, Category, PickedDataRef, ProbabilityScale, TickVisibility};
use crate::data_config::MatrixRef;
use crate::layer::{OverlayLayer, Subplot};
use crate::layout::{ChartArea, Rect};
use crate::legend::{append_legend_entry_rich, series_symbol_segments};

/// The themes of `kind` a test sweeps: the documented default look, then
/// every family's member.
fn themes(kind: ChartKind) -> Vec<Theme> {
    std::iter::once(Theme::documented(kind))
        .chain(
            ThemeFamily::ALL
                .into_iter()
                .map(|family| family.theme(kind)),
        )
        .collect()
}

/// The radial themes a test sweeps: the documented default look, then
/// every family's radial member.
fn radial_themes() -> Vec<(&'static str, RadialTheme)> {
    std::iter::once(("default", RadialTheme::documented()))
        .chain(
            ThemeFamily::ALL
                .into_iter()
                .map(|family| (family.name(), family.radial())),
        )
        .collect()
}

/// `default`, the documented default look of `kind`, or a family's member.
fn named(kind: ChartKind, name: &str) -> Theme {
    match name {
        "default" => Theme::documented(kind),
        _ => ThemeFamily::named(name).expect("a family").theme(kind),
    }
}

/// A custom line theme drawn in `style`, which draws a line only with
/// markers or not at all: the documented look in another draw style.
fn line_in(style: crate::config::DrawStyle) -> Theme {
    let mut theme = Theme::documented(ChartKind::Line);
    theme.chrome.draw_style = style;
    theme
}

/// The palette colour of series `index`.
fn palette_color(theme: &Theme, index: usize) -> Color {
    let palette = &theme.series.palette;
    palette[index % palette.len()]
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

// Every kind's `default` is the documented default look: applying it to a
// default document changes nothing, at any subplot scope.
#[test]
fn default_on_a_default_document_changes_nothing() {
    for kind in ChartKind::ALL {
        assert_eq!(named(kind, "default"), Theme::documented(kind));
        for scope in [SubplotScope::None, SubplotScope::All] {
            let mut config = Config::default();
            named(kind, "default")
                .apply_chrome(&mut config, &[], scope)
                .unwrap();
            assert_eq!(config, Config::default(), "{kind}");
        }
    }
    let mut radial = crate::defaults::radial_chart();
    let before = radial.clone();
    RadialTheme::documented().apply(&mut radial);
    assert_eq!(radial, before);
}

// Every theme writes the look and keeps the content: ranges, scales, the
// chart area, offsets, text, the annotation layer, the overlay and subplot
// structure, and the selection's refs.
#[test]
fn a_theme_keeps_the_content() {
    for theme in themes(ChartKind::Line) {
        let (before, series_before) = rich_chart();
        let mut config = before.clone();
        let mut series = series_before.clone();
        theme
            .apply(&mut config, &mut series, SubplotScope::All)
            .unwrap();

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
    let theme = named(ChartKind::Line, "midnight");
    let (mut config, mut series) = rich_chart();
    let untouched = series[2].clone();
    theme
        .apply(&mut config, &mut series, SubplotScope::Only(&[1]))
        .unwrap();
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
    assert_eq!(line_color(&series[3]), palette_color(&theme, 3));
    // Series take the palette by their place in the list.
    assert_eq!(line_color(&series[0]), palette_color(&theme, 0));
    assert_eq!(line_color(&series[1]), palette_color(&theme, 1));
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
    assert!(symbol_colors.contains(&palette_color(&theme, 0)));
    assert!(symbol_colors.contains(&palette_color(&theme, 1)));
}

// A theme refuses a chart of another kind, and a subplot in scope of another
// kind, and changes nothing; a chart without series takes any theme.
#[test]
fn a_theme_refuses_a_chart_of_another_kind() {
    let theme = named(ChartKind::Scatter, "default");
    let (before, series_before) = rich_chart();
    let (mut config, mut series) = (before.clone(), series_before.clone());
    assert_eq!(
        theme.apply(&mut config, &mut series, SubplotScope::None),
        Err(ThemeError::WrongKind {
            theme: ChartKind::Scatter,
            chart: ChartKind::Line,
            layer: LayerRef::Base,
        })
    );
    assert_eq!((config, series), (before, series_before));

    // Only the stems' subplot is of another kind.
    let mut config = Config::default();
    config.subplots.push(Subplot {
        series: vec!["b".into()],
        ..Subplot::default()
    });
    let mut series = [
        SeriesConfig::line("a", "x", "y"),
        SeriesConfig::stem("b", "x", "y"),
    ];
    let line = named(ChartKind::Line, "default");
    assert!(
        line.apply(&mut config, &mut series, SubplotScope::None)
            .is_ok()
    );
    assert_eq!(
        line.apply(&mut config, &mut series, SubplotScope::All),
        Err(ThemeError::WrongKind {
            theme: ChartKind::Line,
            chart: ChartKind::Stem,
            layer: LayerRef::Subplot(0),
        })
    );

    let mut empty = Config::default();
    for kind in ChartKind::ALL {
        assert!(
            named(kind, "default")
                .apply_chrome(&mut empty, &[], SubplotScope::All)
                .is_ok()
        );
    }
}

// The family parts write widths and sizes: lines, markers, stems and error
// bars on an x-y series, and their opacities when the theme sets them.
#[test]
fn the_line_family_takes_widths_and_marker_sizes() {
    let mut theme = named(ChartKind::Line, "midnight");
    let lines = &mut theme.series.lines;
    lines.marker_opacity = Some(0.5);
    lines.stem_opacity = Some(0.75);
    lines.error_cap_size = 9.0;
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
    let scatter = xy.scatter.as_ref().unwrap();
    assert_eq!(scatter.point_size, lines.marker_size);
    assert_eq!(scatter.point_color.a, 0.5);
    let stem = xy.stem.as_ref().unwrap();
    assert_eq!(stem.width, lines.stem_width);
    assert_eq!(
        stem.color,
        Color {
            a: 0.75,
            ..palette_color(&theme, 0)
        }
    );
    let errors = &xy.errors.as_ref().unwrap().style;
    assert_eq!(errors.error_bar_width, lines.error_bar_width);
    assert_eq!(errors.cap_width, lines.error_cap_width);
    assert_eq!(errors.error_bar_cap_size, 9.0);
    // The line keeps its colour opaque.
    assert_eq!(line_color(&series), palette_color(&theme, 0));
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
    let mut probability = Config::default();
    probability.left_y.scale = AxisScale::Probability(ProbabilityScale::Normal);
    let cases: Vec<(Config, Vec<SeriesConfig>)> = cases
        .into_iter()
        .chain([(
            probability,
            vec![
                SeriesConfig::scatter("a", "x", "p"),
                SeriesConfig::line("b", "x", "p"),
            ],
        )])
        .collect();
    for (config, series) in &cases {
        for item in series {
            let list = vec![item.clone()];
            let kind = ChartKind::of(config, &list, LayerRef::Base).expect("a chart with a series");
            for theme in themes(kind) {
                let mut config = config.clone();
                let mut list = list.clone();
                theme
                    .apply(&mut config, &mut list, SubplotScope::All)
                    .unwrap();
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
    let mut theme = named(ChartKind::Feynman, "default");
    theme.chrome.axis_preset = AxisPreset::Minimal;
    let mut config = Config::default();
    let mut series = [SeriesConfig::feynman_diagram(
        "f",
        crate::defaults::feynman_series(),
    )];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    assert!(!config.bottom_x.line_visible);
    assert!(!config.bottom_x.label_style.label_visible);
    assert!(!config.grid.show_major_x || theme.chrome.grid.major_x.show);

    let mut theme = named(ChartKind::EnergyLevels, "default");
    theme.chrome.axis_preset = AxisPreset::Minimal;
    let mut config = Config::default();
    let mut series = [SeriesConfig::energy_diagram(
        "e",
        crate::defaults::energy_level_series(),
    )];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
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
    named(ChartKind::Line, "default")
        .apply_chrome(&mut config, &[], SubplotScope::All)
        .unwrap();
    assert!(config.picked_data.is_none());
    let mut theme = line_in(crate::config::DrawStyle::Milkyway(Default::default()));
    theme.chrome.highlight.color = Color::new(1.0, 0.84, 0.04, 1.0);
    theme
        .apply_chrome(&mut config, &[], SubplotScope::All)
        .unwrap();
    let picked = config.picked_data.as_ref().unwrap();
    assert!(picked.refs.is_empty());
    assert_eq!(picked.highlight_color, theme.chrome.highlight.color);
}

// Applying a theme twice is applying it once, opacities included.
#[test]
fn applying_a_theme_again_changes_nothing() {
    let mut probability = Config::default();
    probability.left_y.scale = AxisScale::Probability(ProbabilityScale::Normal);
    let cases = [
        rich_chart(),
        (
            Config::default(),
            vec![
                SeriesConfig::scatter("a", "x", "y"),
                SeriesConfig::scatter("b", "x", "y"),
            ],
        ),
        (
            Config::default(),
            vec![SeriesConfig::scatter_line("a", "x", "y").y_errors("e")],
        ),
        (
            Config::default(),
            vec![
                SeriesConfig::line("a", "x", "y"),
                SeriesConfig::stem("b", "x", "y"),
            ],
        ),
        (
            probability,
            vec![
                SeriesConfig::scatter("a", "x", "p"),
                SeriesConfig::line("b", "x", "p"),
            ],
        ),
    ];
    for (config, series) in cases {
        let kind = ChartKind::of(&config, &series, LayerRef::Base).unwrap();
        for theme in themes(kind) {
            let (mut config, mut series) = (config.clone(), series.clone());
            theme
                .apply(&mut config, &mut series, SubplotScope::All)
                .unwrap();
            let (once, once_series) = (config.clone(), series.clone());
            theme
                .apply(&mut config, &mut series, SubplotScope::All)
                .unwrap();
            assert_eq!(config, once, "{kind}");
            assert_eq!(series, once_series, "{kind}");
        }
    }
}

// The radial chart takes its theme: background, label look, material and
// the palette on its top-level slices; values and labels stay.
#[test]
fn the_radial_chart_takes_its_part() {
    let mut theme = RadialTheme::documented();
    theme.palette = vec![
        Color::new(1.0, 0.0, 0.0, 1.0),
        Color::new(0.0, 0.0, 1.0, 1.0),
    ];
    theme.background = Color::new(0.1, 0.1, 0.1, 1.0);
    let mut chart = crate::defaults::radial_chart();
    chart.slices = vec![
        crate::radial::RadialSlice::new("A", 1.0, Color::BLACK),
        crate::radial::RadialSlice::new("B", 2.0, Color::BLACK),
    ];
    theme.apply(&mut chart);
    assert_eq!(chart.background, theme.background);
    assert_eq!(chart.label_color, theme.label_color);
    assert_eq!(chart.slices[1].color, theme.palette[1]);
    assert_eq!(
        (chart.slices[1].label.as_str(), chart.slices[1].value),
        ("B", 2.0)
    );
}

// The family names are the only theme names: each parses to its family,
// and any other name is refused with the family names.
#[test]
fn the_family_names_are_the_theme_names() {
    for family in ThemeFamily::ALL {
        assert_eq!(ThemeFamily::parse(family.name()), Ok(family));
    }
    for name in ["default", "neon", "editorial", "document", ""] {
        assert_eq!(
            ThemeFamily::parse(name),
            Err(ThemeError::UnknownName {
                name: name.into(),
                names: ThemeFamily::NAMES,
            })
        );
    }
}

// An overlay's own axes take the theme's frame like the chart's data axes,
// so a scene theme draws no line or tick on an overlay's top or right axis
// either (#283).
#[test]
fn an_overlay_s_own_axes_take_the_theme_frame() {
    let theme = named(ChartKind::Line, "midnight");
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
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
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

// The bar part writes a histogram's border and gap and a category or range
// bar's material, corners and texture.
#[test]
fn the_bar_family_takes_borders_corners_and_materials() {
    let theme = named(ChartKind::Histogram, "gallery");
    let mut series = SeriesConfig::histogram("a", "edges", "counts");
    theme.style_series(&mut series, 0);
    let DataRenderType::Histogram { bar } = &series.render_type else {
        panic!("a histogram");
    };
    let bars = &theme.series.bars;
    assert_eq!(bar.fill_color, palette_color(&theme, 0));
    assert_eq!(bar.border_color, bars.border_color);
    assert_eq!(bar.border_width, bars.border_width);
    assert_eq!(bar.gap_px, 2.0);

    let theme = named(ChartKind::CategoryBars, "cool-slate");
    let mut series = SeriesConfig::category_bars("a", "i", "v", Color::BLACK);
    theme.style_series(&mut series, 1);
    let DataRenderType::CategoryBars { color, style, .. } = &series.render_type else {
        panic!("category bars");
    };
    assert_eq!(*color, palette_color(&theme, 1));
    assert_eq!(style.material, crate::material::Material::Matte);
    assert_eq!(style.corner_radius, theme.series.bars.corner_radius);
    assert_eq!(style.texture_strength, 0.0);
}

// The box part writes the finish, which parts are drawn and the raw points'
// opacity; a violin half with a fill of its own takes a palette colour, the
// high one of two the next, and a half without one keeps following its box.
#[test]
fn the_box_family_takes_its_parts_and_colours_own_violin_fills() {
    use crate::data_config::{BoxPointsStyle, ViolinStyle};
    let half = |fill: Option<Color>| ViolinHalf {
        curve: "curve".into(),
        density: "density".into(),
        density_scale: 1.0,
        style: ViolinStyle {
            fill,
            ..ViolinStyle::default()
        },
    };
    let split = |low: Option<Color>, high: Option<Color>| {
        SeriesConfig::box_plot("a", "i", "records", Color::BLACK)
            .violin(Violin {
                body: ViolinBody::Split {
                    low: Some(half(low)),
                    high: Some(half(high)),
                },
                box_width: 0.1,
            })
            .style_box_plot(|parts| {
                *parts.points = Some(BoxPoints {
                    positions: "cat".into(),
                    values: "value".into(),
                    style: BoxPointsStyle::default(),
                });
            })
    };
    let fills = |series: &SeriesConfig| {
        let DataRenderType::BoxPlot {
            violin: Some(violin),
            ..
        } = &series.render_type
        else {
            panic!("a violin");
        };
        let ViolinBody::Split {
            low: Some(low),
            high: Some(high),
        } = &violin.body
        else {
            panic!("a split violin");
        };
        (low.style.clone(), high.style.clone())
    };

    let theme = named(ChartKind::Distribution, "warm-paper");
    let mut series = split(None, None);
    theme.style_series(&mut series, 0);
    let (low, high) = fills(&series);
    assert_eq!((low.fill, high.fill), (None, None));
    assert!(!low.outline && !high.outline);
    let DataRenderType::BoxPlot { style, points, .. } = &series.render_type else {
        panic!("a box plot");
    };
    let boxes = &theme.series.boxes;
    assert!(!style.caps);
    assert_eq!(style.median_width, 2.0);
    assert_eq!(style.whisker_color, boxes.ink);
    let points = &points.as_ref().unwrap().style;
    assert_eq!(
        points.color,
        Color {
            a: 0.55,
            ..boxes.ink
        }
    );
    assert_eq!(points.size, boxes.point_size);

    let theme = named(ChartKind::Distribution, "cool-slate");
    let mut series = split(Some(Color::BLACK), Some(Color::BLACK));
    theme.style_series(&mut series, 0);
    let (low, high) = fills(&series);
    assert_eq!(low.fill, Some(palette_color(&theme, 0)));
    assert_eq!(high.fill, Some(palette_color(&theme, 1)));
    assert!(low.outline && high.outline);
    assert_eq!(low.outline_color, theme.series.boxes.ink);

    // A finish varied on the part reaches the boxes.
    let mut theme = named(ChartKind::Distribution, "gallery");
    theme.series.boxes.gloss = 0.6;
    theme.series.boxes.outline = false;
    let mut series = SeriesConfig::box_plot("a", "i", "records", Color::BLACK);
    theme.style_series(&mut series, 0);
    let DataRenderType::BoxPlot { style, .. } = &series.render_type else {
        panic!("a box plot");
    };
    assert_eq!(style.gloss, 0.6);
    assert!(!style.outline);
}

// The range-bar part recolours the colouring a series has (direction or
// state colours, never which one), writes the unfinished part's opacity,
// the whiskers and the connector; a theme without those colours keeps the
// series' own.
#[test]
fn the_range_bar_family_recolours_directions_and_states() {
    use crate::data_config::{
        RangeConnector, RangeProgress, RangeStateStyle, RangeTotal, RangeWhiskers,
    };
    let ink = Color::BLACK;
    let directed = || {
        SeriesConfig::range_bars("a", "i", "start", "end", ink).style_range_bars(|parts| {
            *parts.color_by = RangeColor::Direction {
                up: ink,
                down: ink,
                total: Some(RangeTotal {
                    column: "total".into(),
                    color: ink,
                }),
            };
            *parts.connector = Some(RangeConnector {
                color: ink,
                width: 1.0,
            });
            *parts.whiskers = Some(RangeWhiskers {
                low: "low".into(),
                high: "high".into(),
                width: 1.0,
                color: None,
            });
        })
    };
    let theme = named(ChartKind::RangeBars, "warm-paper");
    let range = &theme.series.range_bars;
    let mut series = directed();
    theme.style_series(&mut series, 0);
    let DataRenderType::RangeBars {
        style,
        color_by,
        connector,
        whiskers,
        ..
    } = &series.render_type
    else {
        panic!("range bars");
    };
    let RangeColor::Direction { up, down, total } = color_by else {
        panic!("the colouring stays by direction");
    };
    assert_eq!(Some(*up), range.up);
    assert_eq!(Some(*down), range.down);
    assert_eq!(total.as_ref().map(|total| total.color), range.total);
    assert_eq!(total.as_ref().unwrap().column, "total");
    assert_eq!(style.corner_radius, 3.0);
    let connector = connector.as_ref().unwrap();
    assert_eq!(connector.color, range.connector_color);
    assert_eq!(connector.width, 1.5);
    assert_eq!(whiskers.as_ref().unwrap().width, range.whisker_width);

    // The default theme names no direction colour.
    let mut series = directed();
    named(ChartKind::RangeBars, "default").style_series(&mut series, 0);
    let DataRenderType::RangeBars { color_by, .. } = &series.render_type else {
        panic!("range bars");
    };
    assert!(matches!(
        color_by,
        RangeColor::Direction { up, down, .. } if *up == ink && *down == ink
    ));

    // More states than the theme has colours wrap around.
    let theme = named(ChartKind::RangeBars, "cool-slate");
    let states = &theme.series.range_bars.states;
    let mut series =
        SeriesConfig::range_bars("a", "i", "start", "end", ink).style_range_bars(|parts| {
            *parts.color_by = RangeColor::State {
                column: "state".into(),
                styles: vec![RangeStateStyle { color: ink }; states.len() + 1],
            };
            *parts.progress = Some(RangeProgress {
                column: "progress".into(),
                remaining_opacity: 1.0,
            });
        });
    theme.style_series(&mut series, 0);
    let DataRenderType::RangeBars {
        color_by: RangeColor::State { styles, .. },
        progress,
        ..
    } = &series.render_type
    else {
        panic!("range bars by state");
    };
    for (k, style) in styles.iter().enumerate() {
        assert_eq!(style.color, states[k % states.len()]);
    }
    assert_eq!(progress.as_ref().unwrap().remaining_opacity, 0.35);
}

// The document base: a transparent background, an open frame without tick
// marks in the text colour, and the value grid only.
#[test]
fn the_document_themes_draw_an_open_frame_without_ticks() {
    for kind in [ChartKind::CategoryBars, ChartKind::Distribution] {
        let theme = named(kind, "gallery");
        let mut config = categorical();
        theme
            .apply(&mut config, &mut [], SubplotScope::All)
            .unwrap();
        assert_eq!(config.background, None);
        assert!(config.bottom_x.line_visible && config.left_y.line_visible);
        assert!(!config.top_x.line_visible && !config.right_y.line_visible);
        // The ticks are hidden, not shortened: the labels keep their place.
        for axis in [&config.bottom_x, &config.left_y] {
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert_eq!(
            config.left_y.major_tick_length,
            Config::default().left_y.major_tick_length
        );
        assert_eq!(config.left_y.line_color, theme.chrome.ink);
        assert!(config.grid.show_major_y && !config.grid.show_major_x);
    }
}

fn field_contour(series: &SeriesConfig) -> &crate::data_config::ContourConfig {
    match &series.render_type {
        DataRenderType::Field(FieldSeries {
            contour: Some(contour),
            ..
        }) => contour,
        _ => panic!("a field with contours"),
    }
}

fn field_surface(series: &SeriesConfig) -> Option<&crate::view3d::SurfaceStyle> {
    match &series.render_type {
        DataRenderType::Field(field) => field.surface.as_ref(),
        _ => panic!("a field"),
    }
}

// The field part writes the fill's opacity and the contour lines' colour,
// width and label look; the levels, the labels' spacing and their presence
// stay. Without a contour colour the lines take the palette's.
#[test]
fn the_field_family_takes_fills_contours_and_labels() {
    use crate::data_config::ContourLabelConfig;
    let labelled = || {
        SeriesConfig::contours("a", "x", "y", grid("z"), vec![0.25, 0.5])
            .filled()
            .contour_labels(ContourLabelConfig {
                spacing_px: 140.0,
                ..Default::default()
            })
    };
    let theme = named(ChartKind::Field, "gallery");
    let fields = &theme.series.fields;
    let mut series = labelled();
    theme.style_series(&mut series, 0);
    let contour = field_contour(&series);
    assert_eq!(contour.levels, vec![0.25, 0.5]);
    assert_eq!(Some(contour.line.line_color), fields.contour_color);
    assert_eq!(contour.line.line_width, 1.3);
    assert_eq!(contour.per_level_color, None);
    let labels = contour.labels.as_ref().unwrap();
    assert_eq!(labels.color, Color::WHITE);
    assert_eq!(labels.font_size, 13.0);
    assert_eq!(labels.bg_padding_px, 3.0);
    assert_eq!(labels.spacing_px, 140.0);

    // No contour colour: the palette's, at the field part's width.
    let theme = named(ChartKind::TriMesh, "midnight");
    assert_eq!(theme.series.fields.contour_color, None);
    let mut series = labelled();
    theme.style_series(&mut series, 0);
    let contour = field_contour(&series);
    assert_eq!(contour.line.line_color, palette_color(&theme, 0));
    assert_eq!(contour.line.line_width, theme.series.fields.contour_width);
    assert_ne!(contour.line.line_width, theme.series.lines.line_width);

    // The fill's opacity, where the theme sets one.
    let opacity = |series: &SeriesConfig| match &series.render_type {
        DataRenderType::Field(FieldSeries {
            fill: Some(fill), ..
        }) => fill.opacity,
        _ => panic!("a filled field"),
    };
    let mut series = SeriesConfig::heatmap("a", "x", "y", grid("z")).opacity(0.6);
    named(ChartKind::VectorField, "midnight").style_series(&mut series, 0);
    assert_eq!(opacity(&series), 0.6);
    named(ChartKind::VectorField, "gallery").style_series(&mut series, 0);
    assert_eq!(opacity(&series), 0.35);
}

// A theme with a contour shade colours every level from the chart's
// colourbar, which carries the theme's map: the map's colour at the level,
// darkened. A chart without a colourbar, and a series styled alone, keep
// their per-level colours.
#[test]
fn a_contour_shade_colours_each_level_from_the_colourbar() {
    let theme = named(ChartKind::Surface3d, "warm-paper");
    let shade = theme.series.fields.contour_shade.unwrap();
    let levels = vec![1.0, 2.0, 3.0];
    let contours = || SeriesConfig::contours("a", "x", "y", grid("z"), levels.clone()).filled();
    let mut config = Config::default();
    config.set_view_3d(Some(crate::defaults::view3d()));
    let mut bar = crate::defaults::color_bar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 4.0;
    config.colorbar = Some(bar);

    let mut series = [contours()];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    let bar = config.colorbar.as_ref().unwrap();
    assert_eq!(bar.colormap, theme.series.fields.colormap);
    let colors = field_contour(&series[0]).per_level_color.clone().unwrap();
    assert_eq!(colors.len(), levels.len());
    for (level, color) in levels.iter().zip(&colors) {
        let map = bar.color_for_z(*level);
        assert_eq!(
            *color,
            Color::new(map.r * shade, map.g * shade, map.b * shade, map.a)
        );
    }
    assert_ne!(colors[0], colors[2]);
    valid_on(&config, &series);

    // Applied again, the same colours.
    let before = series.clone();
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    assert_eq!(series, before);

    let mut alone = contours();
    theme.style_series(&mut alone, 0);
    assert_eq!(field_contour(&alone).per_level_color, None);

    let mut config = Config::default();
    config.set_view_3d(Some(crate::defaults::view3d()));
    let mut series = [contours()];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    assert_eq!(field_contour(&series[0]).per_level_color, None);
}

// A 3D theme decides the wireframe of every filled field, which its chart
// draws as a surface; a field without a fill draws none and gets no surface
// look. A theme of another kind leaves a wireframe as the series has it.
#[test]
fn a_3d_theme_decides_the_surface_and_its_wireframe() {
    use crate::material::Material;
    use crate::view3d::{SurfaceStyle, SurfaceWireframe};
    let filled = || SeriesConfig::heatmap("a", "x", "y", grid("z"));
    let wired = || {
        filled().style_surface(|surface| {
            surface.wireframe = Some(SurfaceWireframe {
                color: Color::BLACK,
                width: 3.0,
            })
        })
    };
    let mut config = Config::default();
    config.set_view_3d(Some(crate::defaults::view3d()));

    let theme = named(ChartKind::Surface3d, "warm-paper");
    let surfaces = &theme.series.surfaces;
    let mut series = filled();
    theme.style_series(&mut series, 0);
    let surface = field_surface(&series).expect("a surface look");
    assert_eq!(surface.material, Material::Matte);
    assert_eq!(
        surface.wireframe,
        Some(SurfaceWireframe {
            color: surfaces.wireframe_color,
            width: surfaces.wireframe_width,
        })
    );
    valid_on(&config, std::slice::from_ref(&series));

    let mut lines = SeriesConfig::contours("a", "x", "y", grid("z"), vec![0.5]);
    theme.style_series(&mut lines, 0);
    assert_eq!(field_surface(&lines), None);
    valid_on(&config, std::slice::from_ref(&lines));

    // A 3D theme without a wireframe removes one, and writes no surface
    // look where the documented one is drawn anyway.
    let theme = named(ChartKind::Surface3d, "midnight");
    assert!(!theme.series.surfaces.wireframe);
    let mut series = wired();
    theme.style_series(&mut series, 0);
    assert_eq!(
        field_surface(&series),
        Some(&SurfaceStyle {
            wireframe: None,
            ..crate::defaults::surface_style()
        })
    );
    let mut series = filled();
    theme.style_series(&mut series, 0);
    assert_eq!(field_surface(&series), None);

    // A flat chart's theme keeps the wireframe and recolours it.
    let theme = named(ChartKind::Field, "gallery");
    let mut series = wired();
    theme.style_series(&mut series, 0);
    let wireframe = field_surface(&series).unwrap().wireframe.clone().unwrap();
    assert_eq!(wireframe.width, theme.series.surfaces.wireframe_width);
    let mut series = filled();
    theme.style_series(&mut series, 0);
    assert_eq!(field_surface(&series), None);
}

// The vector part writes the arrows' width and head, and the palette their
// colour; the scale, the pivot and the colour map stay.
#[test]
fn the_vector_family_takes_widths_and_heads() {
    use crate::data_config::{VectorPivot, VectorScale};
    let theme = named(ChartKind::VectorField, "cool-slate");
    let mut series = SeriesConfig::vectors("a", "x", "y", "u", "v")
        .pivot(VectorPivot::Middle)
        .vector_scale(VectorScale::Pixels { px_per_unit: 16.0 });
    theme.style_series(&mut series, 0);
    let DataRenderType::Vector { style, .. } = &series.render_type else {
        panic!("vectors");
    };
    assert_eq!(style.color, palette_color(&theme, 0));
    assert_eq!(
        (style.width_px, style.head_length_px, style.head_width_px),
        (1.8, 7.0, 6.0)
    );
    assert_eq!(style.pivot, VectorPivot::Middle);
    assert_eq!(style.scale, VectorScale::Pixels { px_per_unit: 16.0 });
}

// The radar part writes what it names and keeps the rest: the default
// theme names nothing, the document theme the outline, the markers and the
// fill.
#[test]
fn the_radar_part_keeps_what_it_does_not_name() {
    let own = || {
        let mut series = SeriesConfig::radar("a", "i", "v", Color::BLACK);
        if let DataRenderType::Radar { style, .. } = &mut series.render_type {
            style.line_width_px = Some(1.5);
            style.fill_opacity = Some(0.1);
        }
        series
    };
    let mut series = own();
    let before = series.clone();
    let mut default = named(ChartKind::Radar, "default");
    default.series.palette.clear();
    default.style_series(&mut series, 0);
    assert_eq!(series, before);

    let theme = named(ChartKind::Radar, "gallery");
    let mut series = own();
    theme.style_series(&mut series, 3);
    let DataRenderType::Radar { color, style, .. } = &series.render_type else {
        panic!("a radar series");
    };
    assert_eq!(*color, palette_color(&theme, 3));
    assert_eq!(style.line_width_px, Some(2.0));
    assert_eq!(style.marker_size_px, Some(7.0));
    assert_eq!(style.fill_opacity, Some(0.25));

    // The radar document's frame: no axis lines, rings and spokes from the
    // grid, the ticks' documented length.
    let mut polar = categorical();
    polar.set_polar(Some(crate::defaults::polar_options()));
    let documented = polar.left_y.major_tick_length;
    theme.apply(&mut polar, &mut [], SubplotScope::All).unwrap();
    assert!(!polar.bottom_x.line_visible && !polar.left_y.line_visible);
    assert_eq!(polar.left_y.major_tick_length, documented);
    assert!(polar.grid.show_major_x && polar.grid.show_major_y);
    assert_ne!(polar.grid.major_x_color, polar.grid.major_y_color);
}

// The ternary part colours a region series' fill by its index and outlines
// it; a series without a region takes the palette and the line part. On a
// ternary chart `right_y`, the third component, is framed as the labelled
// sides are, and the legend text takes its own ink.
#[test]
fn the_ternary_part_colours_regions_and_frames_the_third_axis() {
    use crate::ternary::TernaryRegionStyle;
    let theme = named(ChartKind::Ternary, "warm-paper");
    let ternary = &theme.series.ternary;
    let (outline, outline_width) = ternary.region_outline.unwrap();
    let region = || {
        SeriesConfig::of_kind(
            "a",
            "p",
            "q",
            DataRenderType::Ternary {
                c_column: "r".into(),
                scatter: None,
                line: Some(Default::default()),
                region: Some(TernaryRegionStyle::default()),
            },
        )
    };
    let mut series = region();
    theme.style_series(&mut series, 13);
    let DataRenderType::Ternary { line, region, .. } = &series.render_type else {
        panic!("a ternary series");
    };
    let region = region.as_ref().unwrap();
    assert_eq!(region.color, ternary.regions[13 % ternary.regions.len()]);
    assert_eq!(region.opacity, 0.75);
    let line = line.as_ref().unwrap();
    assert_eq!((line.line_color, line.line_width), (outline, outline_width));

    let mut series = SeriesConfig::ternary("a", "p", "q", "r").with_line(1.0);
    theme.style_series(&mut series, 1);
    let DataRenderType::Ternary { line, scatter, .. } = &series.render_type else {
        panic!("a ternary series");
    };
    let line = line.as_ref().unwrap();
    assert_eq!(line.line_color, palette_color(&theme, 1));
    assert_eq!(line.line_width, theme.series.lines.line_width);
    assert_eq!(
        scatter.as_ref().unwrap().point_color,
        palette_color(&theme, 1)
    );

    let mut config = Config::default();
    config.set_ternary(Some(crate::defaults::ternary_options()));
    config.legend.visible = true;
    theme
        .apply(&mut config, &mut [], SubplotScope::All)
        .unwrap();
    for axis in [&config.bottom_x, &config.left_y, &config.right_y] {
        assert!(axis.line_visible);
        assert_eq!(axis.tick, TickVisibility::Outside);
        assert!(axis.label_style.label_visible);
        assert_eq!(axis.label_style.color, theme.chrome.label_ink);
    }
    assert!(!config.top_x.line_visible);
    assert_eq!(Some(config.legend.content.color), theme.chrome.legend_ink);
    assert_ne!(config.legend.content.color, theme.chrome.label_ink);

    // The same on a 3D chart, whose height is read on `right_y`; a flat
    // chart keeps `right_y` an unlabelled side.
    let scene = named(ChartKind::Surface3d, "midnight");
    let mut config = Config::default();
    config.set_view_3d(Some(crate::defaults::view3d()));
    scene
        .apply(&mut config, &mut [], SubplotScope::All)
        .unwrap();
    assert!(config.right_y.label_style.label_visible);
    assert!(!config.right_y.line_visible);
    let mut config = Config::default();
    scene
        .apply(&mut config, &mut [], SubplotScope::All)
        .unwrap();
    assert!(!config.right_y.label_style.label_visible);
}

// A parallel chart reads its values on the parallel axes, drawn in the
// left axis' style, and names them in the category axis' labels: an open
// frame draws the axes with their ticks and no line under the names, which
// take the parallel part's size.
#[test]
fn a_parallel_chart_frames_its_axes_and_sizes_their_names() {
    let theme = named(ChartKind::Parallel, "midnight");
    let mut config = categorical();
    let mut options = crate::defaults::parallel_options();
    options.axes = vec![crate::parallel::ParallelAxis::new(0.0, 1.0); 3];
    config.set_parallel(Some(options));
    let mut series = [SeriesConfig::parallel("a", ["p", "q", "r"])];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    assert!(config.left_y.line_visible);
    assert_eq!(config.left_y.tick, TickVisibility::Outside);
    assert_eq!(config.left_y.line_color, theme.chrome.frame_ink);
    assert_eq!(config.left_y.label_style.font_size, 16.0);
    assert!(!config.bottom_x.line_visible);
    assert_eq!(config.bottom_x.tick, TickVisibility::None);
    assert!(config.bottom_x.label_style.label_visible);
    assert_eq!(config.bottom_x.label_style.font_size, 20.0);
    let DataRenderType::Parallel { line, .. } = &series[0].render_type else {
        panic!("a parallel series");
    };
    assert_eq!(line.line_color, palette_color(&theme, 0));
    assert_eq!(line.line_width, 1.2);
    valid_on(&config, &series);

    // The documented look changes nothing on a default parallel chart.
    let mut config = categorical();
    let mut options = crate::defaults::parallel_options();
    options.axes = vec![crate::parallel::ParallelAxis::new(0.0, 1.0); 3];
    config.set_parallel(Some(options));
    let before = config.clone();
    named(ChartKind::Parallel, "default")
        .apply(&mut config, &mut [], SubplotScope::All)
        .unwrap();
    assert_eq!(config, before);
}

// Every field-family theme draws a colourbar in its map and look.
#[test]
fn a_colourbar_takes_the_field_part_s_map() {
    for (kind, name) in [
        (ChartKind::Field, "warm-paper"),
        (ChartKind::Field, "midnight"),
        (ChartKind::VectorField, "cool-slate"),
        (ChartKind::TriMesh, "midnight"),
    ] {
        let theme = named(kind, name);
        let mut config = Config::default();
        config.colorbar = Some(crate::defaults::color_bar_options());
        theme
            .apply(&mut config, &mut [], SubplotScope::All)
            .unwrap();
        let bar = config.colorbar.as_ref().unwrap();
        assert_eq!(bar.colormap, theme.series.fields.colormap, "{kind} {name}");
        assert_ne!(bar.colormap, crate::defaults::color_bar_options().colormap);
        assert_eq!(bar.thickness_px, theme.chrome.colorbar.thickness);
        assert_eq!(bar.axis.label_style.color, theme.chrome.label_ink);
        assert_eq!(bar.axis.title_option.text.color, theme.chrome.ink);
    }
}

fn override_colors(series: &SeriesConfig) -> Vec<(String, Option<Color>)> {
    match &series.render_type {
        DataRenderType::BoxPlot { overrides, .. } => (overrides.iter().flatten())
            .map(|o| (o.target.category_id.clone(), o.color))
            .collect(),
        DataRenderType::CategoryBars { overrides, .. } => (overrides.iter().flatten())
            .map(|o| (o.target.category_id.clone(), o.color))
            .collect(),
        _ => panic!("boxes or category bars"),
    }
}

// A theme that colours by category gives the one box plot or category-bar
// series of a chart a palette colour per category, in reading order: left
// to right, and top to bottom for horizontal bars (a category axis that is
// not inverted lists its bottom row first). Two series keep one colour
// each, and a theme without the flag writes no per-category colour.
#[test]
fn a_single_series_takes_one_palette_colour_per_category() {
    use crate::data_config::{BarGroups, CategoryBarDirection};
    type Make = fn() -> SeriesConfig;
    let boxes: Make = || SeriesConfig::box_plot("s", "pos", "rec", Color::BLACK);
    let bars: Make = || SeriesConfig::category_bars("s", "pos", "v", Color::BLACK);
    for (kind, name, make) in [
        (ChartKind::Distribution, "warm-paper", boxes),
        (ChartKind::Distribution, "gallery", boxes),
        (ChartKind::CategoryBars, "gallery", bars),
    ] {
        let theme = named(kind, name);
        let palette = |k: usize| Some(palette_color(&theme, k));
        let mut config = categorical();
        let mut series = [make()];
        theme
            .apply(&mut config, &mut series, SubplotScope::All)
            .unwrap();
        assert_eq!(
            override_colors(&series[0]),
            [
                ("c0".to_string(), palette(0)),
                ("c1".to_string(), palette(1)),
                ("c2".to_string(), palette(2)),
            ],
            "{kind} {name}"
        );
        valid_on(&config, &series);
        // Applied again, the colours are rewritten in place.
        let once = series.clone();
        theme
            .apply(&mut config, &mut series, SubplotScope::All)
            .unwrap();
        assert_eq!(series, once);

        // Horizontal: the axis' first category is the bottom row, unless
        // the axis is inverted.
        for inverted in [false, true] {
            let mut config = Config::default();
            config.left_y.categories = categorical().bottom_x.categories;
            config.left_y.inverted = inverted;
            config.bar_groups = Some(BarGroups {
                direction: CategoryBarDirection::Horizontal,
                ..crate::defaults::bar_groups()
            });
            let mut series = [make()];
            theme
                .apply(&mut config, &mut series, SubplotScope::All)
                .unwrap();
            let top_down: Vec<_> = if inverted {
                vec![0, 1, 2]
            } else {
                vec![2, 1, 0]
            };
            let colors: Vec<_> = (override_colors(&series[0]).into_iter())
                .map(|(_, color)| color)
                .collect();
            let expected: Vec<_> = top_down.into_iter().map(palette).collect();
            assert_eq!(colors, expected, "{kind} {name} inverted {inverted}");
        }

        // Two series: one colour each.
        let mut config = categorical();
        let mut series = [make(), make()];
        series[1].series_id = "t".into();
        theme
            .apply(&mut config, &mut series, SubplotScope::All)
            .unwrap();
        assert!(override_colors(&series[0]).is_empty(), "{kind} {name}");
        assert!(override_colors(&series[1]).is_empty(), "{kind} {name}");

        // Without the flag nothing is written.
        let mut plain = theme.clone();
        plain.series.boxes.color_by_category = false;
        plain.series.bars.color_by_category = false;
        let mut series = [make()];
        plain
            .apply(&mut categorical(), &mut series, SubplotScope::All)
            .unwrap();
        assert!(override_colors(&series[0]).is_empty(), "{kind} {name}");
    }
}

// The Grotrian theme writes the texts a diagram owns (level, transition,
// limit and band labels, the wavelength labels' look, the column labels
// and group headers on the column axis) and fills the continuum the series
// wrote; the axis breaks stay as the host sized them.
#[test]
fn the_grotrian_theme_writes_the_diagram_s_texts_and_continuum() {
    use crate::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark, PieceSizing};
    use crate::config::{CategoryGroup, CategoryGroups};
    use crate::energy_levels::{
        EnergyLevel, IonizationLimit, LevelBand, LevelRole, Transition, WavelengthLabels,
    };
    let theme = named(ChartKind::EnergyLevels, "gallery");
    let energy = theme.series.energy_levels;
    let text = |s: &str| RichText::plain(s, Color::BLACK, 9.0, "");
    let level = |id: &str, energy: f64| EnergyLevel {
        id: id.into(),
        column: "S".into(),
        configuration: String::new(),
        term: String::new(),
        j: String::new(),
        g: None,
        prefix: String::new(),
        energy,
        suffix: String::new(),
        uncertainty: None,
        reference: String::new(),
        role: LevelRole::Level,
        label: Some(text("level")),
        label_offset: [0.0; 2],
    };
    let mut levels = crate::defaults::energy_level_series();
    levels.levels = vec![level("low", 0.0), level("mid", 3.0), level("high", 4.0)];
    levels.transitions = vec![Transition {
        id: "line".into(),
        lower: "low".into(),
        upper: "mid".into(),
        label: Some(text("line")),
        label_offset: [0.0; 2],
        value: None,
        x_offset: 0.0,
        wavelength: Some(500.0),
        wavelength_uncertainty: None,
        intensity: String::new(),
        aki: None,
        accuracy: String::new(),
        line_type: String::new(),
        tp_reference: String::new(),
        line_reference: String::new(),
    }];
    levels.limits = vec![IonizationLimit {
        id: "ion".into(),
        energy: 5.0,
        label: Some(text("limit")),
        label_offset: [0.0; 2],
        uncertainty: None,
        reference: String::new(),
    }];
    levels.bands = vec![LevelBand {
        id: "band".into(),
        column: "S".into(),
        first: "mid".into(),
        last: "high".into(),
        label: Some(text("band")),
        label_offset: [0.0; 2],
    }];
    assert_eq!(levels.validate(), Ok(()));
    levels.style.wavelength_labels = Some(WavelengthLabels {
        text: text(""),
        ..crate::defaults::wavelength_labels()
    });
    let mut config = Config::default();
    let column = levels.levels[0].column.clone();
    let mut category = Category::new(column.clone(), "");
    category.rich_label = Some(text("column"));
    config.bottom_x.categories = Some(vec![category, Category::new("plain", "Plain")]);
    config.bottom_x.category_groups = Some(CategoryGroups {
        groups: vec![CategoryGroup {
            label: text("group"),
            first: column.clone(),
            last: column,
            bracket: true,
        }],
        offset: [0.0; 2],
    });
    let breaks = AxisBreaks {
        ranges: vec![AxisBreak::new(1.0, 2.0)],
        style: AxisBreakStyle::Continuous,
        gap_px: 12.0,
        mark: BreakMark::default(),
        frame_each_panel: true,
        sizing: PieceSizing::Weights(vec![9.0, 1.0]),
    };
    config.left_y.breaks = Some(breaks.clone());
    let grey = Color::new(0.5, 0.5, 0.5, 0.2);
    levels
        .write_continuum(&mut config.annotations, "e", 100.0, grey)
        .unwrap();
    // A span that is not the series' continuum keeps its fill.
    config.annotations.items.push(Annotation {
        id: "host".into(),
        visible: true,
        kind: AnnotationKind::Span {
            axis: AnnotationAxis::Y,
            from: 0.0,
            to: 1.0,
            fill: grey,
        },
    });
    let mut series = [SeriesConfig::energy_diagram("e", levels)];
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();

    let look = |text: &RichText| {
        Some(TextLook {
            color: text.color,
            size: text.font_size,
        })
    };
    let levels = series[0].energy_levels().unwrap();
    assert_eq!(levels.style.level.color, palette_color(&theme, 0));
    assert_eq!(levels.style.level.width, energy.level_width);
    for level in &levels.levels {
        assert_eq!(look(level.label.as_ref().unwrap()), energy.level_labels);
    }
    for line in &levels.transitions {
        assert_eq!(look(line.label.as_ref().unwrap()), energy.transition_labels);
    }
    for limit in &levels.limits {
        assert_eq!(look(limit.label.as_ref().unwrap()), energy.limit_labels);
    }
    for band in &levels.bands {
        assert_eq!(look(band.label.as_ref().unwrap()), energy.band_labels);
    }
    let wavelengths = levels.style.wavelength_labels.as_ref().unwrap();
    assert_eq!(look(&wavelengths.text), energy.transition_labels);
    let columns = config.bottom_x.categories.as_ref().unwrap();
    assert_eq!(
        look(columns[0].rich_label.as_ref().unwrap()),
        energy.column_labels
    );
    assert_eq!(columns[1].rich_label, None);
    let groups = config.bottom_x.category_groups.as_ref().unwrap();
    assert_eq!(look(&groups.groups[0].label), energy.group_headers);
    assert_eq!(groups.groups[0].label.segments, text("group").segments);
    assert_eq!(config.left_y.breaks, Some(breaks));
    let fills: Vec<Color> = (config.annotations.items.iter())
        .filter_map(|item| match item.kind {
            AnnotationKind::Span { fill, .. } => Some(fill),
            _ => None,
        })
        .collect();
    let (host, continuum) = fills.split_last().unwrap();
    assert!(!continuum.is_empty());
    assert!(
        continuum
            .iter()
            .all(|fill| Some(*fill) == energy.continuum_fill)
    );
    assert_eq!(*host, grey);
    assert!(!config.grid.show_major_x && !config.grid.show_major_y);
    valid_on(&config, &series);
}

// The parchment theme colours each propagator by its particle, leaves the
// fermions in the series colour, and writes the strokes and the label ink.
#[test]
fn the_parchment_theme_colours_lines_by_particle() {
    use crate::feynman::{FeynmanSeries, FeynmanVertex, MomentumArrow, Propagator, VertexRole};
    let theme = named(ChartKind::Feynman, "warm-paper");
    let feynman = theme.series.feynman;
    let text = |s: &str| RichText::plain(s, Color::BLACK, 9.0, "");
    let line = |id: &str, particle| Propagator {
        label: Some(text(id)),
        ..Propagator::new(id, "a", "b", particle)
    };
    let mut diagram = FeynmanSeries {
        vertices: vec![
            FeynmanVertex {
                label: Some(text("a")),
                ..FeynmanVertex::new("a", VertexRole::Incoming)
            },
            FeynmanVertex::new("b", VertexRole::Outgoing),
        ],
        propagators: vec![
            line("fermion", ParticleLine::Fermion),
            line("photon", ParticleLine::Photon),
            line("gluon", ParticleLine::Gluon),
            line("boson", ParticleLine::ChargedBoson),
            line("scalar", ParticleLine::Scalar),
        ],
        ..crate::defaults::feynman_series()
    };
    diagram.propagators[1].momentum = Some(MomentumArrow {
        label: text("k"),
        side: crate::defaults::momentum_side(),
        reversed: false,
        label_offset: [0.0; 2],
    });
    let mut series = [SeriesConfig::feynman_diagram("f", diagram)];
    let mut config = Config::default();
    theme
        .apply(&mut config, &mut series, SubplotScope::All)
        .unwrap();
    let DataRenderType::Feynman(diagram) = &series[0].render_type else {
        panic!("a Feynman series");
    };
    let colors: Vec<_> = diagram.propagators.iter().map(|line| line.color).collect();
    assert_eq!(
        colors,
        [
            None,
            feynman.photon,
            feynman.gluon,
            feynman.boson,
            feynman.scalar
        ]
    );
    assert!(colors[1..].iter().all(Option::is_some));
    let style = &diagram.style;
    assert_eq!(style.color, palette_color(&theme, 0));
    assert_eq!(
        (style.width, style.wave, style.coil),
        (feynman.width, feynman.wave, feynman.coil)
    );
    assert_eq!(
        (style.head_length, style.head_width, style.dot_radius),
        (feynman.head_length, feynman.head_width, feynman.dot_radius)
    );
    let look = |text: &RichText| {
        Some(TextLook {
            color: text.color,
            size: text.font_size,
        })
    };
    assert_eq!(
        look(diagram.vertices[0].label.as_ref().unwrap()),
        feynman.vertex_labels
    );
    for line in &diagram.propagators {
        assert_eq!(look(line.label.as_ref().unwrap()), feynman.line_labels);
    }
    let momentum = diagram.propagators[1].momentum.as_ref().unwrap();
    assert_eq!(look(&momentum.label), feynman.line_labels);
    assert_eq!(config.background, theme.chrome.background);
    assert!(!config.bottom_x.line_visible && !config.grid.show_major_y);
    valid_on(&config, &series);
}

// Every radial theme writes the whole look into a plain chart: the view,
// the slice shape, the material and the outline; a slice's own style, the
// kind and the values stay. The sunburst theme steps each ring paler than
// the one inside it, and each sibling paler than the one before.
#[test]
fn the_radial_themes_write_the_view_shape_and_ring_tints() {
    use crate::radial::{RadialKind, RadialSlice};
    let slice = |label: &str| RadialSlice::new(label, 1.0, Color::BLACK);
    for (name, theme) in radial_themes() {
        let mut chart = crate::defaults::radial_chart().with_title("Title");
        chart.kind = RadialKind::Donut { inner_radius: 0.4 };
        chart.slices = vec![slice("A"), slice("B")];
        let own = crate::radial::RadialStyle::three_dimensional();
        chart.slices[1].style = Some(own.clone());
        theme.apply(&mut chart);
        let style = &chart.style;
        assert_eq!(
            (style.material, style.gloss, style.roughness, style.shadow),
            (theme.material, theme.gloss, theme.roughness, theme.shadow),
            "{name}"
        );
        assert_eq!(
            (
                style.tilt_degrees,
                style.depth,
                style.bevel,
                style.gap_degrees
            ),
            (
                theme.tilt_degrees,
                theme.depth,
                theme.bevel,
                theme.gap_degrees
            ),
            "{name}"
        );
        assert_eq!(
            (style.inner_corner, style.outer_corner, style.outline),
            (theme.inner_corner, theme.outer_corner, theme.outline),
            "{name}"
        );
        assert_eq!(
            (style.texture_strength, style.texture_scale),
            (theme.texture_strength, theme.texture_scale),
            "{name}"
        );
        let title = &chart.chart_title.as_ref().unwrap().text;
        assert_eq!(
            (title.color, title.font_size),
            (theme.label_color, theme.title_size)
        );
        assert_eq!(chart.slices[1].style, Some(own));
        assert_eq!(chart.kind, RadialKind::Donut { inner_radius: 0.4 });
        assert_eq!(chart.validate(), Ok(()), "{name}");
        if name != "default" {
            assert_eq!(chart.slices[0].color, theme.palette[0], "{name}");
            assert_ne!(theme, RadialTheme::documented(), "{name}");
        }
    }
    // The default look on a titled default chart changes nothing.
    let mut chart = crate::defaults::radial_chart().with_title("Title");
    let before = chart.clone();
    RadialTheme::documented().apply(&mut chart);
    assert_eq!(chart, before);

    let theme = ThemeFamily::CoolSlate.radial();
    let mut chart = crate::defaults::radial_chart();
    chart.slices = vec![RadialSlice {
        children: vec![
            RadialSlice {
                children: vec![slice("leaf")],
                ..slice("first")
            },
            slice("second"),
        ],
        ..slice("root")
    }];
    theme.apply(&mut chart);
    // On a white background a paler colour has a larger sum.
    let light = |color: Color| color.r + color.g + color.b;
    let root = &chart.slices[0];
    let (first, second) = (&root.children[0], &root.children[1]);
    assert_eq!(root.color, theme.palette[0]);
    assert!(light(first.color) > light(root.color));
    assert!(light(second.color) > light(first.color));
    assert!(light(first.children[0].color) > light(first.color));
    assert!(light(second.color) < 3.0);
    // A theme without a tint keeps the children's colours.
    let mut chart = crate::defaults::radial_chart();
    chart.slices = vec![RadialSlice {
        children: vec![slice("child")],
        ..slice("root")
    }];
    ThemeFamily::Gallery.radial().apply(&mut chart);
    assert_eq!(chart.slices[0].children[0].color, Color::BLACK);
    // A sunburst draws flat, so it keeps its top view under a tilted theme
    // and stays a valid document under every theme.
    for (name, theme) in radial_themes() {
        let mut chart = crate::defaults::radial_chart();
        chart.slices = vec![RadialSlice {
            children: vec![slice("child")],
            ..slice("root")
        }];
        theme.apply(&mut chart);
        assert_eq!(
            (chart.style.tilt_degrees, chart.style.depth),
            (0.0, 0.0),
            "{name}"
        );
        assert_eq!(chart.style.material, theme.material, "{name}");
        assert_eq!(chart.validate(), Ok(()), "{name}");
    }
}

// --- Theme families (#295) ---

/// The draw styles a test renderer draws: Precise draws everything, the
/// rest nothing, so a theme in another style is refused for any series.
fn precise_only(style: &crate::config::DrawStyle, _: &DataRenderType) -> bool {
    *style == crate::config::DrawStyle::Precise
}

/// A dashboard whose charts are of four kinds: a line chart, then a
/// scatter, a heatmap and a histogram subplot, the last with a colourbar,
/// and a subplot without series yet.
fn dashboard() -> (Config, Vec<SeriesConfig>) {
    let mut config = Config::default();
    config.chart_title.text = RichText::plain("Dashboard", Color::BLACK, 28.0, "");
    let ids = ["s0", "s1", "s2", ""];
    for (index, id) in ids.into_iter().enumerate() {
        let mut subplot = Subplot::default();
        subplot.config.chart_area = ChartArea(Rect {
            x: 950,
            y: 20 + 200 * index as u32,
            width: 300,
            height: 180,
        });
        if !id.is_empty() {
            subplot.series.push(id.into());
        }
        config.subplots.push(subplot);
    }
    config.subplots[2].config.colorbar = Some(crate::defaults::color_bar_options());
    let series = vec![
        SeriesConfig::line("a", "x", "y").label("first"),
        SeriesConfig::scatter("s0", "x", "y"),
        SeriesConfig::heatmap("s1", "x", "y", grid("z")),
        SeriesConfig::histogram("s2", "edges", "counts"),
    ];
    (config, series)
}

// Every family has a member for every kind and for the radial chart: a
// Precise theme of that kind, which the family's name looks up too, and
// each family's member differs from the others'.
#[test]
fn every_family_has_a_member_for_every_kind() {
    for (place, family) in ThemeFamily::ALL.into_iter().enumerate() {
        assert_eq!(ThemeFamily::NAMES[place], family.name());
        assert_eq!(ThemeFamily::named(family.name()), Some(family));
        for kind in ChartKind::ALL {
            let member = family.theme(kind);
            assert_eq!(member.kind, kind, "{family} {kind}");
            assert_eq!(
                member.chrome.draw_style,
                crate::config::DrawStyle::Precise,
                "{family} {kind}"
            );
            assert!(!member.series.palette.is_empty(), "{family} {kind}");
            assert_eq!(named(kind, family.name()), member, "{family} {kind}");
            for other in ThemeFamily::ALL
                .into_iter()
                .filter(|&other| other != family)
            {
                assert_ne!(other.theme(kind), member, "{family} {other} {kind}");
            }
        }
        assert!(!family.radial().palette.is_empty(), "{family}");
    }
    assert_eq!(ThemeFamily::named("neon"), None);
    // A family's chrome is one look: every member of a family with a
    // background of its own paints it, the gallery's leave the paper
    // white.
    for kind in ChartKind::ALL {
        assert_eq!(
            ThemeFamily::Gallery.theme(kind).chrome.background,
            None,
            "{kind}"
        );
        for family in [
            ThemeFamily::CoolSlate,
            ThemeFamily::Midnight,
            ThemeFamily::WarmPaper,
        ] {
            assert!(
                family.theme(kind).chrome.background.is_some(),
                "{family} {kind}"
            );
        }
    }
}

// On a document of one kind, a family name writes exactly what its member
// writes through `Theme::apply`.
#[test]
fn a_family_name_applies_its_member_to_every_kind() {
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
    let mut probability = Config::default();
    probability.left_y.scale = AxisScale::Probability(ProbabilityScale::Normal);
    let cases: Vec<(Config, Vec<SeriesConfig>)> = vec![
        rich_chart(),
        (
            Config::default(),
            vec![SeriesConfig::scatter("a", "x", "y")],
        ),
        (
            Config::default(),
            vec![SeriesConfig::scatter_line("a", "x", "y").y_errors("e")],
        ),
        (Config::default(), vec![SeriesConfig::stem("a", "x", "y")]),
        (
            Config::default(),
            vec![SeriesConfig::histogram("a", "edges", "counts")],
        ),
        (
            categorical(),
            vec![SeriesConfig::category_bars("a", "i", "v", ink)],
        ),
        (
            categorical(),
            vec![SeriesConfig::box_plot("a", "i", "records", ink)],
        ),
        (
            categorical(),
            vec![SeriesConfig::range_bars("a", "i", "start", "end", ink)],
        ),
        (
            Config::default(),
            vec![SeriesConfig::heatmap("a", "x", "y", grid("z"))],
        ),
        (
            Config::default(),
            vec![SeriesConfig::vectors("a", "x", "y", "u", "v")],
        ),
        (polar.clone(), vec![SeriesConfig::line("a", "x", "y")]),
        (polar, vec![SeriesConfig::radar("a", "i", "v", ink)]),
        (ternary, vec![SeriesConfig::ternary("a", "p", "q", "r")]),
        (parallel, vec![SeriesConfig::parallel("a", ["p", "q"])]),
        (
            probability,
            vec![
                SeriesConfig::scatter("a", "x", "p"),
                SeriesConfig::line("b", "x", "p"),
            ],
        ),
    ];
    let mut seen = Vec::new();
    for (config, series) in cases {
        let kind = ChartKind::of(&config, &series, LayerRef::Base).unwrap();
        seen.push(kind);
        for family in ThemeFamily::ALL {
            let (mut by_name, mut named_series) = (config.clone(), series.clone());
            Theme::apply_named(
                family.name(),
                &mut by_name,
                &mut named_series,
                SubplotScope::All,
            )
            .unwrap_or_else(|error| panic!("{family} {kind}: {error}"));
            let (mut by_member, mut member_series) = (config.clone(), series.clone());
            family
                .theme(kind)
                .apply(&mut by_member, &mut member_series, SubplotScope::All)
                .unwrap();
            assert_eq!(by_name, by_member, "{family} {kind}");
            assert_eq!(named_series, member_series, "{family} {kind}");
            valid_on(&by_name, &named_series);
        }
    }
    assert!(seen.len() >= 14, "{seen:?}");
}

// A family themes a dashboard whose subplots mix kinds in one call: the
// base chart and every subplot take their own kind's member, a subplot
// without series the line member, and every series its chart's member.
#[test]
fn a_family_themes_subplots_of_mixed_kinds_each_in_its_kind_s_member() {
    for family in ThemeFamily::ALL {
        let (mut config, mut series) = dashboard();
        Theme::apply_named(family.name(), &mut config, &mut series, SubplotScope::All).unwrap();
        let kinds = [
            ChartKind::Scatter,
            ChartKind::Field,
            ChartKind::Histogram,
            ChartKind::Line,
        ];
        let line = family.theme(ChartKind::Line);
        assert_eq!(config.background, line.chrome.background, "{family}");
        assert_eq!(config.chart_title.text.color, line.chrome.ink, "{family}");
        assert_eq!(
            config.chart_title.text.font_size, line.chrome.title_size,
            "{family}"
        );
        for (index, kind) in kinds.into_iter().enumerate() {
            let member = family.theme(kind);
            let chart = &config.subplots[index].config;
            let at = format!("{family}: subplot {index} ({kind})");
            assert_eq!(chart.background, member.chrome.background, "{at}");
            assert_eq!(chart.chart_title.text.color, member.chrome.ink, "{at}");
            assert_eq!(
                chart.grid.show_major_x, member.chrome.grid.major_x.show,
                "{at}"
            );
            assert_eq!(
                chart.grid.show_major_y, member.chrome.grid.major_y.show,
                "{at}"
            );
            assert_eq!(
                chart.bottom_x.label_style.font_size, member.chrome.tick_label_size,
                "{at}"
            );
        }
        // Each series in its chart's member, coloured as the series of its
        // index in the document.
        let scatter = family.theme(ChartKind::Scatter);
        let DataRenderType::Xy(xy) = &series[1].render_type else {
            panic!("an x-y series");
        };
        let marker = xy.scatter.as_ref().unwrap();
        assert_eq!(
            marker.point_size, scatter.series.lines.marker_size,
            "{family}"
        );
        let histogram = family.theme(ChartKind::Histogram);
        let DataRenderType::Histogram { bar } = &series[3].render_type else {
            panic!("a histogram");
        };
        assert_eq!(
            bar.border_color, histogram.series.bars.border_color,
            "{family}"
        );
        assert_eq!(
            bar.border_width, histogram.series.bars.border_width,
            "{family}"
        );
        assert_eq!(
            line_color(&series[0]),
            line.series.palette[0],
            "{family}: the base chart's line"
        );
        valid_on(&config, &series);
    }
}

// Each subplot's colourbar takes its own member's map: the dashboard's
// histogram subplot holds a colourbar, which takes the histogram member's
// map, while the base chart takes the line member's look.
#[test]
fn a_family_writes_each_chart_s_colourbar_from_its_member() {
    for family in ThemeFamily::ALL {
        let (mut config, mut series) = dashboard();
        Theme::apply_named(family.name(), &mut config, &mut series, SubplotScope::All).unwrap();
        let bar = config.subplots[2].config.colorbar.as_ref().unwrap();
        let member = family.theme(ChartKind::Histogram);
        assert_eq!(bar.colormap, member.series.fields.colormap, "{family}");
        assert_eq!(bar.border_color, member.chrome.colorbar.border, "{family}");
    }
}

// The subplot scope chooses the charts as it does for one theme: the base
// chart always, the subplots it names, and nothing else.
#[test]
fn a_family_keeps_the_subplots_out_of_scope() {
    let (declared, declared_series) = dashboard();
    let (mut config, mut series) = (declared.clone(), declared_series.clone());
    Theme::apply_named(
        "midnight",
        &mut config,
        &mut series,
        SubplotScope::Only(&[2]),
    )
    .unwrap();
    let midnight = ThemeFamily::Midnight;
    assert_eq!(
        config.background,
        midnight.theme(ChartKind::Line).chrome.background
    );
    assert_eq!(config.subplots[0], declared.subplots[0]);
    assert_eq!(config.subplots[1], declared.subplots[1]);
    assert_eq!(config.subplots[3], declared.subplots[3]);
    assert_eq!(
        config.subplots[2].config.background,
        midnight.theme(ChartKind::Histogram).chrome.background
    );
    assert_eq!(series[1], declared_series[1]);
    assert_eq!(series[2], declared_series[2]);
    assert_ne!(series[3], declared_series[3]);
}

// A name that is not a family's is refused with the family names, by a
// document of any kind and by an empty one, and changes nothing; the
// names of the former per-kind catalogue are not theme names.
#[test]
fn an_unknown_name_lists_the_family_names() {
    let (declared, declared_series) = dashboard();
    for name in ["neon", "default", "editorial", "document", "ceramic"] {
        let (mut config, mut series) = (declared.clone(), declared_series.clone());
        let error =
            Theme::apply_named(name, &mut config, &mut series, SubplotScope::All).unwrap_err();
        assert_eq!(
            error,
            ThemeError::UnknownName {
                name: name.into(),
                names: ThemeFamily::NAMES,
            }
        );
        assert!(
            error
                .to_string()
                .ends_with(": use one of gallery, cool-slate, midnight, warm-paper"),
            "{error}"
        );
        assert_eq!((&config, &series), (&declared, &declared_series));
        assert_eq!(
            Theme::for_chart(name, &declared, &declared_series, LayerRef::Base),
            Err(error)
        );
    }
    let mut empty = Config::default();
    assert!(matches!(
        Theme::apply_named("neon", &mut empty, &mut [], SubplotScope::All),
        Err(ThemeError::UnknownName { .. })
    ));
    assert_eq!(empty, Config::default());
}

// A chart that draws no series yet takes a family's line member.
#[test]
fn a_chart_without_series_takes_the_line_member() {
    for family in ThemeFamily::ALL {
        let mut config = Config::default();
        Theme::apply_named(family.name(), &mut config, &mut [], SubplotScope::All).unwrap();
        let mut expected = Config::default();
        family
            .theme(ChartKind::Line)
            .apply(&mut expected, &mut [], SubplotScope::All)
            .unwrap();
        assert_eq!(config, expected, "{family}");
    }
}

// A theme whose draw style cannot draw a series in scope is refused before
// A custom theme whose draw style cannot draw a series on a chart it
// themes is refused before any chart is written; a family's members all
// draw in the Precise style, which draws every series.
#[test]
fn a_draw_style_refusal_changes_nothing() {
    let (declared, declared_series) = dashboard();
    let (mut config, mut series) = (declared.clone(), declared_series.clone());
    let milkyway = line_in(crate::config::DrawStyle::Milkyway(Default::default()));
    assert_eq!(
        milkyway.apply_to_its_kind(&mut config, &mut series, precise_only),
        Err(ThemeError::UnsupportedDrawStyle {
            series_id: "a".into()
        })
    );
    assert_eq!((&config, &series), (&declared, &declared_series));
    for family in ThemeFamily::ALL {
        for kind in ChartKind::ALL {
            assert_eq!(
                family.theme(kind).chrome.draw_style,
                crate::config::DrawStyle::Precise,
                "{family} {kind}"
            );
        }
    }
}

// Applying a family twice is applying it once.
#[test]
fn applying_a_family_again_changes_nothing() {
    for family in ThemeFamily::ALL {
        let (mut config, mut series) = dashboard();
        for _ in 0..2 {
            Theme::apply_named(family.name(), &mut config, &mut series, SubplotScope::All).unwrap();
        }
        let (mut once, mut once_series) = dashboard();
        Theme::apply_named(
            family.name(),
            &mut once,
            &mut once_series,
            SubplotScope::All,
        )
        .unwrap();
        assert_eq!(config, once, "{family}");
        assert_eq!(series, once_series, "{family}");
    }
}

// A document themed by a family is plain values: it serializes, parses
// back to itself and serializes to the same JSON.
#[cfg(feature = "serde")]
#[test]
fn a_family_themed_document_round_trips_through_json() {
    for family in ThemeFamily::ALL {
        let (mut config, mut series) = dashboard();
        Theme::apply_named(family.name(), &mut config, &mut series, SubplotScope::All).unwrap();
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config, "{family}");
        assert_eq!(serde_json::to_string(&back).unwrap(), json, "{family}");
        let json = serde_json::to_string(&series).unwrap();
        let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, series, "{family}");
        assert_eq!(serde_json::to_string(&back).unwrap(), json, "{family}");
    }
}

// The theme a name resolves to on one chart is the one `apply_named`
// writes there: a value to edit, refused as the name is (#295).
#[test]
fn a_name_resolves_to_its_chart_s_theme() {
    let (config, series) = dashboard();
    let at = |layer| Theme::for_chart("midnight", &config, &series, layer);
    assert_eq!(
        at(LayerRef::Base),
        Ok(ThemeFamily::Midnight.theme(ChartKind::Line))
    );
    assert_eq!(
        at(LayerRef::Subplot(1)),
        Ok(ThemeFamily::Midnight.theme(ChartKind::Field))
    );
    // The empty subplot takes the line member.
    assert_eq!(
        at(LayerRef::Subplot(3)),
        Ok(ThemeFamily::Midnight.theme(ChartKind::Line))
    );
    assert!(matches!(
        Theme::for_chart("neon", &config, &series, LayerRef::Base),
        Err(ThemeError::UnknownName { .. })
    ));
    // A custom theme applied to a chart of another kind.
    let error = named(ChartKind::Scatter, "default")
        .apply(&mut config.clone(), &mut series.clone(), SubplotScope::None)
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.starts_with("a scatter theme does not apply to the line chart at Base"),
        "{message}"
    );
    assert!(
        message.contains("apply_custom_theme skips the others"),
        "{message}"
    );
}

// A radial chart takes a family's radial member by name; any other name
// is refused with the family names and changes nothing (#295).
#[test]
fn a_radial_chart_takes_a_family_by_name() {
    let mut chart = crate::defaults::radial_chart();
    chart.slices = vec![
        crate::radial::RadialSlice::new("A", 1.0, Color::BLACK),
        crate::radial::RadialSlice::new("B", 2.0, Color::BLACK),
    ];
    let before = chart.clone();
    for name in ["neon", "ceramic", "default"] {
        assert_eq!(
            chart.apply_theme(name),
            Err(ThemeError::UnknownName {
                name: name.into(),
                names: ThemeFamily::NAMES,
            })
        );
        assert_eq!(chart, before);
    }
    for family in ThemeFamily::ALL {
        let mut themed = before.clone();
        themed.apply_theme(family.name()).unwrap();
        let mut expected = before.clone();
        family.radial().apply(&mut expected);
        assert_eq!(themed, expected, "{family}");
    }
}

// Every error says what to do next: a wrong kind names the families and
// the scope, a draw style the Precise style, an unknown name the family
// names, and a subplot index the indices there are.
#[test]
fn theme_errors_say_what_to_do_next() {
    let wrong = ThemeError::WrongKind {
        theme: ChartKind::Line,
        chart: ChartKind::Scatter,
        layer: LayerRef::Subplot(0),
    }
    .to_string();
    assert!(
        wrong.starts_with("a line theme does not apply to the scatter chart at Subplot(0)"),
        "{wrong}"
    );
    assert!(
        wrong.contains("gallery, cool-slate, midnight, warm-paper"),
        "{wrong}"
    );
    assert!(wrong.contains("subplot scope"), "{wrong}");
    let style = ThemeError::UnsupportedDrawStyle {
        series_id: "a".into(),
    }
    .to_string();
    assert!(style.contains("Precise"), "{style}");
    let unknown = ThemeError::UnknownName {
        name: "neon".into(),
        names: ThemeFamily::NAMES,
    }
    .to_string();
    assert_eq!(
        unknown,
        "no theme family \"neon\": use one of gallery, cool-slate, midnight, warm-paper"
    );
    // A subplot index error is a type of its own, which a theme error
    // wraps and names as its source.
    assert_eq!(SubplotIndexError::check(1, 2), Ok(()));
    assert_eq!(
        SubplotIndexError::check(2, 2),
        Err(SubplotIndexError { index: 2, count: 2 })
    );
    let wrapped = ThemeError::from(SubplotIndexError { index: 3, count: 2 });
    assert_eq!(
        std::error::Error::source(&wrapped).map(ToString::to_string),
        Some(SubplotIndexError { index: 3, count: 2 }.to_string())
    );
    let index = |count| ThemeError::SubplotIndex(SubplotIndexError { index: 3, count }).to_string();
    assert_eq!(
        index(2),
        "subplot index out of range: no subplot 3, the chart has 2 subplots (indices 0 to 1)"
    );
    assert!(index(0).ends_with("the chart has no subplots"));
    assert!(index(1).ends_with("the chart has one subplot (index 0)"));
}

// A scope that names a subplot the chart does not have is refused with the
// index, by a name and by a theme, and changes nothing.
#[test]
fn a_scope_past_the_subplots_is_refused() {
    let (declared, declared_series) = dashboard();
    let (mut config, mut series) = (declared.clone(), declared_series.clone());
    let refused = Err(ThemeError::SubplotIndex(SubplotIndexError {
        index: 9,
        count: 4,
    }));
    assert_eq!(
        Theme::apply_named(
            "gallery",
            &mut config,
            &mut series,
            SubplotScope::Only(&[1, 9])
        ),
        refused
    );
    let line = ThemeFamily::Gallery.theme(ChartKind::Line);
    assert_eq!(
        line.apply(&mut config, &mut series, SubplotScope::Only(&[9])),
        refused
    );
    assert_eq!(
        line.apply_chrome(&mut config, &series, SubplotScope::Only(&[9])),
        refused
    );
    assert_eq!((&config, &series), (&declared, &declared_series));
}

// A theme applied to the charts of its kind themes the base chart and the
// subplots of that kind or without series, leaves the others alone, and is
// refused only when no chart takes it.
#[test]
fn a_theme_applies_to_the_charts_of_its_kind() {
    let (declared, declared_series) = dashboard();
    // The scatter member: only subplot 0 is a scatter chart.
    let scatter = ThemeFamily::Midnight.theme(ChartKind::Scatter);
    let (mut config, mut series) = (declared.clone(), declared_series.clone());
    scatter
        .apply_to_its_kind(&mut config, &mut series, precise_only)
        .unwrap();
    assert_eq!(config.chart_title, declared.chart_title, "the line base");
    assert_eq!(config.background, declared.background);
    assert_eq!(
        config.subplots[0].config.background,
        scatter.chrome.background
    );
    // The empty subplot takes any theme.
    assert_eq!(
        config.subplots[3].config.background,
        scatter.chrome.background
    );
    assert_eq!(config.subplots[1], declared.subplots[1]);
    assert_eq!(config.subplots[2], declared.subplots[2]);
    assert_ne!(series[1], declared_series[1]);
    assert_eq!(series[0], declared_series[0]);
    assert_eq!(series[2], declared_series[2]);
    assert_eq!(series[3], declared_series[3]);

    // No chart of its kind: refused, naming the base chart.
    let polar = ThemeFamily::Midnight.theme(ChartKind::Polar);
    let mut config = declared.clone();
    config.subplots.pop();
    let mut series = declared_series.clone();
    let before = (config.clone(), series.clone());
    assert_eq!(
        polar.apply_to_its_kind(&mut config, &mut series, precise_only),
        Err(ThemeError::WrongKind {
            theme: ChartKind::Polar,
            chart: ChartKind::Line,
            layer: LayerRef::Base,
        })
    );
    assert_eq!((config, series), before);

    // The draw style is checked on the charts it themes only.
    let mut milkyway = line_in(crate::config::DrawStyle::Milkyway(Default::default()));
    milkyway.kind = ChartKind::Histogram;
    let (mut config, mut series) = (declared.clone(), declared_series.clone());
    assert_eq!(
        milkyway.apply_to_its_kind(&mut config, &mut series, precise_only),
        Err(ThemeError::UnsupportedDrawStyle {
            series_id: "s2".into()
        })
    );
    assert_eq!((&config, &series), (&declared, &declared_series));
}

// A theme and its parts serialize behind the `serde` feature, for a host
// that edits a theme as JSON (the browser's `theme(name)`): every member
// and the documented looks read back equal. A kind serializes as its name.
#[cfg(feature = "serde")]
#[test]
fn a_theme_round_trips_through_json() {
    for kind in ChartKind::ALL {
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            format!("\"{}\"", kind.name())
        );
        for theme in themes(kind) {
            let json = serde_json::to_string(&theme).unwrap();
            let back: Theme = serde_json::from_str(&json).unwrap();
            assert_eq!(back, theme, "{kind}");
        }
    }
    for (name, theme) in radial_themes() {
        let json = serde_json::to_string(&theme).unwrap();
        let back: RadialTheme = serde_json::from_str(&json).unwrap();
        assert_eq!(back, theme, "radial {name}");
    }
}

// A theme object saved before a field was added still parses: every field
// a theme and its parts miss reads the documented default look's (the
// rule the document format follows, CLAUDE.md).
#[cfg(feature = "serde")]
#[test]
fn a_theme_missing_fields_parses() {
    let member = ThemeFamily::Midnight.theme(ChartKind::Scatter);
    let mut json = serde_json::to_value(&member).unwrap();
    let object = json.as_object_mut().unwrap();
    object["chrome"]
        .as_object_mut()
        .unwrap()
        .remove("highlight");
    object["chrome"]["grid"]
        .as_object_mut()
        .unwrap()
        .remove("minor_y");
    object["series"].as_object_mut().unwrap().remove("feynman");
    object["series"]["lines"]
        .as_object_mut()
        .unwrap()
        .remove("marker_size");
    let back: Theme = serde_json::from_value(json).unwrap();
    let documented = Theme::default();
    assert_eq!(back.kind, ChartKind::Scatter);
    assert_eq!(back.chrome.background, member.chrome.background);
    assert_eq!(back.chrome.highlight, documented.chrome.highlight);
    assert_eq!(back.chrome.grid.minor_y, documented.chrome.grid.minor_y);
    assert_eq!(back.series.feynman, documented.series.feynman);
    assert_eq!(
        back.series.lines.marker_size,
        documented.series.lines.marker_size
    );
    assert_eq!(back.series.palette, member.series.palette);
    // An empty object is the documented line look; so is an empty radial
    // object the documented radial look.
    assert_eq!(
        serde_json::from_str::<Theme>("{}").unwrap(),
        Theme::default()
    );
    assert_eq!(
        serde_json::from_str::<RadialTheme>("{}").unwrap(),
        RadialTheme::documented()
    );
}
