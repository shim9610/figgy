//! The theme matrix (#291, #295): every family and every custom look on
//! every playground scene of its kind.
//!
//! The families are the theme names; a family has a member for every kind.
//! Any other look is a custom theme, made for one kind. So for every scene
//! of the playground catalogue (`support/playground_scenes.rs`) the matrix
//! reads the kind of its base chart (`ChartKind::of`) and applies:
//! - every family by name with `Chart::apply_theme_to` over the whole
//!   document. A family gives every chart its own kind's member, so a scene
//!   whose subplots mix kinds takes it whole, each subplot in its kind's
//!   member;
//! - as custom themes with `Chart::apply_custom_theme_to`, the documented
//!   look of the kind (`Theme::default()` with its `kind`) and the gallery's
//!   custom looks of that kind (`examples/support/looks.rs`).
//!
//! The radial scenes take the families' radial members, the documented
//! radial look and the gallery's sunburst look. After a theme is applied:
//! - the document validates with its series;
//! - once saved, it serializes unchanged (`serde` feature);
//! - it exports at 1x and at 2x, and its image is not the image of a theme
//!   applied before it, so no frame is left stale (a theme that writes the
//!   same document as an earlier one draws the same image);
//! - the data is drawn: the 1x export differs from the export of the same
//!   themed chart without its series at [`MIN_DATA_PIXELS`] pixels or more
//!   inside the data areas, so no theme paints its series away.
//!
//! Two refusals are part of the contract and are checked to change nothing:
//! - a subplot of another kind refuses a custom theme over
//!   `SubplotScope::All` (`ThemeError::WrongKind` naming the subplot); a family
//!   never does. The theme is then applied with `SubplotScope::None`, which
//!   styles the base chart alone and must work;
//! - a custom theme whose draw style is not Precise refuses a series that
//!   style cannot draw (`ThemeError::UnsupportedDrawStyle`). A Precise theme,
//!   every family member among them, never does.
//!
//! The scenes keep the playground's document size. Every kind has a scene
//! (`every_kind_has_a_scene`), so every family member and every custom look
//! is drawn at least once. The colours of every family member, the
//! documented looks and the gallery's looks are held to a contrast floor
//! against the theme's background in a test of their own, without a GPU.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/playground_scenes.rs"]
mod scenes;

#[path = "../examples/support/looks.rs"]
mod looks;

use renderer::config::DrawStyle;
use renderer::theme::{ChartKind, RadialTheme, SubplotScope, Theme, ThemeError, ThemeFamily};
use renderer::{
    Chart, Color, Config, ExportOptions, LayerRef, RadialRenderer, RasterImage, Renderer,
    SeriesConfig,
};

use scenes::SceneKind;

/// Points of the large-data scene: enough to draw it, small enough for an
/// unoptimized build on a software GPU.
const BIG_DATA_POINTS: usize = 20_000;
/// A channel difference above this is a changed pixel.
const CHANGED: u8 = 24;
/// The fewest changed pixels inside the data areas that count as drawn
/// data: a hairline across a small plot is already more.
const MIN_DATA_PIXELS: usize = 300;
/// The logical canvas of the radial scenes.
const RADIAL_SIZE: (u32, u32) = (800, 600);
/// The share of a radial export the slices must cover at least.
const MIN_RADIAL_SHARE: f64 = 0.05;
/// WCAG contrast floors: text against the background, and a series colour
/// against it.
const MIN_TEXT_CONTRAST: f32 = 4.5;
const MIN_SERIES_CONTRAST: f32 = 1.5;

// The scenes, grouped so that no test runs too long. Together the groups
// are `SceneKind::ALL` (`the_groups_cover_every_scene`).
const XY_SCENES: &[SceneKind] = &[
    SceneKind::Lines,
    SceneKind::Scatter,
    SceneKind::ValueMaps,
    SceneKind::LogAxes,
    SceneKind::TwinAxes,
    SceneKind::Subplots,
    SceneKind::Annotations,
    SceneKind::CategoryAxis,
    SceneKind::BigData,
];
const XY_SCENES_2: &[SceneKind] = &[
    SceneKind::LongLabels,
    SceneKind::OffsetSpectra,
    SceneKind::BrokenAxes,
    SceneKind::Stem,
    SceneKind::Lollipop,
    SceneKind::ProbabilityPaper,
    SceneKind::AxisUnits,
    SceneKind::Monotonic,
    SceneKind::MonotonicClocks,
];
const BAR_SCENES: &[SceneKind] = &[
    SceneKind::Histogram,
    SceneKind::Categorical,
    SceneKind::BoxPlot,
    SceneKind::Waterfall,
    SceneKind::Candles,
    SceneKind::Gantt,
    SceneKind::Violins,
    SceneKind::Raincloud,
];
const FIELD_SCENES: &[SceneKind] = &[
    SceneKind::Field,
    SceneKind::VectorField,
    SceneKind::Streamlines,
    SceneKind::Hexbin,
    SceneKind::Image,
    SceneKind::TriMesh,
    SceneKind::Surface,
    SceneKind::Quiver3d,
    SceneKind::TriSurface,
];
const COORDINATE_SCENES: &[SceneKind] = &[
    SceneKind::Polar,
    SceneKind::Radar,
    SceneKind::Rose,
    SceneKind::PolarHeatmap,
    SceneKind::Ternary,
    SceneKind::Parallel,
];
const SCIENCE_SCENES: &[SceneKind] = &[
    SceneKind::EnergyLevels,
    SceneKind::EnergyLevelsAr,
    SceneKind::EnergyLevelsCu,
    SceneKind::EnergyLevelsU,
    SceneKind::EnergyLevelsArAll,
    SceneKind::Feynman,
];
const RADIAL_SCENES: &[SceneKind] = &[SceneKind::Radial, SceneKind::Sunburst];

const GROUPS: [&[SceneKind]; 7] = [
    XY_SCENES,
    XY_SCENES_2,
    BAR_SCENES,
    FIELD_SCENES,
    COORDINATE_SCENES,
    SCIENCE_SCENES,
    RADIAL_SCENES,
];

/// What one group of the matrix did.
#[derive(Default)]
struct Tally {
    /// Themes applied and drawn.
    applied: usize,
    /// Of those, family members.
    families: usize,
    /// Of those, families applied to a scene whose subplots are of another
    /// kind than its base chart.
    mixed: usize,
    /// Of those, the ones a subplot of another kind sent to
    /// `SubplotScope::None`.
    base_only: usize,
    /// Themes refused for their draw style.
    refused: usize,
}

fn renderer() -> Renderer {
    Renderer::headless_blocking().expect("headless renderer")
}

fn export(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    monotonic: &[scenes::MonotonicData],
    scale: f32,
) -> RasterImage {
    scenes::export_scene(r, chart, series, monotonic, |r, source| {
        r.export_rgba_blocking(source, ExportOptions::document(scale))
    })
    .unwrap_or_else(|error| panic!("export at {scale}x: {error}"))
}

/// The pixels of `a` and `b` that differ inside the data area of the base
/// chart or of a subplot.
fn changed_in_data_areas(config: &Config, a: &RasterImage, b: &RasterImage) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let areas = std::iter::once(config)
        .chain(config.subplots.iter().map(|subplot| &subplot.config))
        .map(|chart| chart.data_area().expect("a data area").0);
    let mut changed = 0;
    for area in areas {
        for y in area.y..(area.y + area.height).min(a.height) {
            for x in area.x..(area.x + area.width).min(a.width) {
                let at = ((y * a.width + x) * 4) as usize;
                if (0..4).any(|i| a.rgba[at + i].abs_diff(b.rgba[at + i]) > CHANGED) {
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// A themed document is plain values: it parses, and once saved it
/// serializes unchanged, as every document does. The JSON of the saved
/// document is compared with the JSON it reads back to.
#[cfg(feature = "serde")]
fn round_trip(at: &str, config: &Config, series: &[SeriesConfig]) {
    fn same(at: &str, what: &str, first: &str, second: &str) {
        if first != second {
            let from = first
                .bytes()
                .zip(second.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(first.len().min(second.len()));
            let cut = |text: &str| {
                let start = from.saturating_sub(80);
                String::from_utf8_lossy(&text.as_bytes()[start..(from + 80).min(text.len())])
                    .into_owned()
            };
            panic!(
                "{at}: the themed {what} does not round trip:\n{}\n{}",
                cut(first),
                cut(second)
            );
        }
    }
    // The first save: its JSON may print a host `f64` (a fitted range, a
    // wavelength) one digit shorter than the value it was read from.
    let saved = |json: String| -> (Config, String) {
        let back: Config =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("{at}: {error}"));
        let json = serde_json::to_string(&back).expect("it serialized once");
        (back, json)
    };
    let (_, first) = saved(serde_json::to_string(config).expect("a chart serializes"));
    let (_, second) = saved(first.clone());
    same(at, "chart", &first, &second);
    let saved = |json: String| -> String {
        let back: Vec<SeriesConfig> =
            serde_json::from_str(&json).unwrap_or_else(|error| panic!("{at}: {error}"));
        serde_json::to_string(&back).expect("it serialized once")
    };
    let first = saved(serde_json::to_string(series).expect("the series serialize"));
    same(at, "series", &first, &saved(first.clone()));
}

#[cfg(not(feature = "serde"))]
fn round_trip(_: &str, _: &Config, _: &[SeriesConfig]) {}

/// The gallery's custom looks (`examples/support/looks.rs`), by name.
fn gallery_looks() -> [(&'static str, Theme); 9] {
    [
        ("editorial", looks::editorial()),
        ("milkyway", looks::milkyway()),
        ("constellation", looks::constellation()),
        ("revenue", looks::revenue()),
        ("deep-space", looks::deep_space()),
        ("topographic", looks::topographic()),
        ("ocean", looks::ocean()),
        ("warm-terrain", looks::warm_terrain()),
        ("warm-diverging", looks::warm_diverging()),
    ]
}

/// The custom themes of `kind`: its documented look, then the gallery's
/// looks of that kind.
fn custom_themes(kind: ChartKind) -> Vec<(&'static str, Theme)> {
    let documented = Theme {
        kind,
        ..Theme::default()
    };
    std::iter::once(("documented", documented))
        .chain(
            gallery_looks()
                .into_iter()
                .filter(|(_, theme)| theme.kind == kind),
        )
        .collect()
}

/// One row of the matrix: every family and every custom theme of
/// `chart_kind` on the chart, which must be of that kind.
fn sweep_chart(
    r: &mut Renderer,
    what: &str,
    chart_kind: ChartKind,
    config: &Config,
    declared: &[SeriesConfig],
    monotonic: &[scenes::MonotonicData],
    tally: &mut Tally,
) {
    assert_eq!(
        ChartKind::of(config, declared, LayerRef::Base),
        Some(chart_kind),
        "{what}"
    );
    // The themed 1x exports so far: a theme that left a stale frame behind
    // would repeat the image of the one before it. A theme that writes the
    // same document as an earlier one draws the same image.
    let mut looks: Vec<(&str, Config, Vec<SeriesConfig>, RasterImage)> = Vec::new();
    let candidates = ThemeFamily::ALL
        .into_iter()
        .map(|family| (family.name(), Some(family), family.theme(chart_kind)))
        .chain(
            custom_themes(chart_kind)
                .into_iter()
                .map(|(name, theme)| (name, None, theme)),
        );
    for (name, family, theme) in candidates {
        let at = format!("{what}: {chart_kind} theme {name:?}");
        assert_eq!(theme.kind, chart_kind, "{at}");
        if family.is_some() {
            assert_eq!(theme.chrome.draw_style, DrawStyle::Precise, "{at}");
            assert_eq!(
                Chart::new(config.clone()).theme(name, declared),
                Ok(theme.clone()),
                "{at}: the member the chart takes"
            );
        }
        let apply = |chart: &mut Chart, series: &mut [SeriesConfig], scope| match family {
            Some(_) => chart.apply_theme_to(name, series, scope),
            None => chart.apply_custom_theme_to(&theme, series, scope),
        };
        let mut chart = Chart::new(config.clone());
        let mut series = declared.to_vec();

        let mut scope = SubplotScope::All;
        let mut result = apply(&mut chart, &mut series, scope);
        if let Err(ThemeError::WrongKind { layer, .. }) = &result {
            // A subplot of another kind than a custom theme: the theme
            // styles the base chart alone. A family has a member for every
            // kind.
            assert!(family.is_none(), "{at}: a family refused {layer:?}");
            assert!(matches!(layer, LayerRef::Subplot(_)), "{at}: {result:?}");
            assert_eq!(chart.config(), config, "{at}: a refusal changed the chart");
            assert_eq!(series, declared, "{at}: a refusal changed the series");
            scope = SubplotScope::None;
            result = apply(&mut chart, &mut series, scope);
            assert!(
                !matches!(result, Err(ThemeError::WrongKind { .. })),
                "{at}: the base chart refuses its own kind: {result:?}"
            );
        }
        match result {
            Ok(()) => {}
            Err(ThemeError::UnsupportedDrawStyle { series_id }) => {
                assert_ne!(
                    theme.chrome.draw_style,
                    DrawStyle::Precise,
                    "{at}: a Precise theme refused series {series_id:?}"
                );
                assert!(
                    declared.iter().any(|item| item.series_id == series_id),
                    "{at}: refused for an unknown series {series_id:?}"
                );
                assert_eq!(chart.config(), config, "{at}: a refusal changed the chart");
                assert_eq!(series, declared, "{at}: a refusal changed the series");
                tally.refused += 1;
                continue;
            }
            Err(error) => panic!("{at}: {error}"),
        }

        let themed = chart.config();
        assert_eq!(themed.background, theme.chrome.background, "{at}");
        assert_eq!(themed.draw_style, theme.chrome.draw_style, "{at}");
        // A family gives every subplot its own kind's member.
        let mut mixed = false;
        if let Some(family) = family {
            for index in 0..config.subplots.len() {
                let Some(kind) = ChartKind::of(config, declared, LayerRef::Subplot(index)) else {
                    continue;
                };
                mixed |= kind != chart_kind;
                let member = family.theme(kind);
                let cell = &themed.subplots[index].config;
                assert_eq!(cell.background, member.chrome.background, "{at}: [{index}]");
                assert_eq!(
                    cell.chart_title.text.color, member.chrome.ink,
                    "{at}: subplot {index} ({kind})"
                );
            }
        }
        assert_eq!(themed.validate_chart(), Ok(()), "{at}");
        assert_eq!(themed.validate_series(&series), Ok(()), "{at}");
        round_trip(&at, themed, &series);
        // Applying the theme again changes nothing more.
        let mut again = Chart::new(themed.clone());
        let mut series_again = series.clone();
        apply(&mut again, &mut series_again, scope)
            .unwrap_or_else(|error| panic!("{at}: again: {error}"));
        assert_eq!(again.config(), themed, "{at}: applied twice");
        assert_eq!(series_again, series, "{at}: applied twice");

        let drawn = export(r, &chart, &series, monotonic, 1.0);
        let doubled = export(r, &chart, &series, monotonic, 2.0);
        assert_eq!(
            (doubled.width, doubled.height),
            (drawn.width * 2, drawn.height * 2),
            "{at}"
        );
        assert!(
            doubled.rgba.chunks_exact(4).any(|px| px[3] != 0),
            "{at}: the 2x export is empty"
        );
        let chrome = export(r, &chart, &[], &[], 1.0);
        let data = changed_in_data_areas(themed, &drawn, &chrome);
        assert!(
            data >= MIN_DATA_PIXELS,
            "{at}: the series change {data} pixels of the data areas"
        );
        for (other, look_config, look_series, look) in &looks {
            if look_config == themed && *look_series == series {
                continue;
            }
            assert!(
                look.rgba != drawn.rgba,
                "{at}: drawn as the theme {other:?} before it"
            );
        }
        looks.push((name, themed.clone(), series, drawn));
        tally.applied += 1;
        if family.is_some() {
            tally.families += 1;
            if mixed {
                tally.mixed += 1;
            }
        }
        if scope == SubplotScope::None {
            tally.base_only += 1;
        }
    }
}

/// The matrix rows of `kinds`, on one renderer.
fn sweep_scenes(kinds: &[SceneKind]) -> Tally {
    let mut r = renderer();
    let mut tally = Tally::default();
    for &kind in kinds {
        let scene = scenes::build(&mut r, kind, BIG_DATA_POINTS)
            .unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        let chart_kind = ChartKind::of(&scene.config, &scene.series, LayerRef::Base)
            .unwrap_or_else(|| panic!("{kind:?}: the base chart draws no series"));
        sweep_chart(
            &mut r,
            &format!("{kind:?}"),
            chart_kind,
            &scene.config,
            &scene.series,
            &scene.monotonic,
            &mut tally,
        );
    }
    eprintln!(
        "theme matrix: {} scenes, {} themes applied ({} family members, {} of them over subplots \
         of other kinds; {} on the base chart alone), {} refused for their draw style",
        kinds.len(),
        tally.applied,
        tally.families,
        tally.mixed,
        tally.base_only,
        tally.refused
    );
    assert_eq!(
        tally.families,
        kinds.len() * ThemeFamily::ALL.len(),
        "every scene takes every family"
    );
    tally
}

#[test]
fn the_groups_cover_every_scene() {
    let grouped: Vec<SceneKind> = GROUPS
        .iter()
        .flat_map(|group| group.iter().copied())
        .collect();
    assert_eq!(grouped.len(), SceneKind::ALL.len());
    for kind in SceneKind::ALL {
        assert_eq!(
            grouped.iter().filter(|&&other| other == kind).count(),
            1,
            "{kind:?}"
        );
        assert_eq!(kind.is_radial(), RADIAL_SCENES.contains(&kind), "{kind:?}");
    }
}

// The Subplots scene has cells of other kinds than its base chart: the
// families theme it whole, each cell in its kind's member, and the line
// kind's own themes take the `SubplotScope::None` path. The Scatter and
// Lines scenes draw series the night-sky styles refuse.
#[test]
fn xy_scenes_take_their_themes() {
    let tally = sweep_scenes(XY_SCENES);
    assert!(tally.mixed > 0, "no family themed subplots of another kind");
    assert!(tally.base_only > 0, "no row took the base chart alone");
    assert!(tally.refused > 0, "no theme was refused for its draw style");
}

#[test]
fn more_xy_scenes_take_their_themes() {
    sweep_scenes(XY_SCENES_2);
}

#[test]
fn bar_scenes_take_their_themes() {
    sweep_scenes(BAR_SCENES);
}

#[test]
fn field_scenes_take_their_themes() {
    sweep_scenes(FIELD_SCENES);
}

#[test]
fn coordinate_scenes_take_their_themes() {
    sweep_scenes(COORDINATE_SCENES);
}

#[test]
fn science_scenes_take_their_themes() {
    sweep_scenes(SCIENCE_SCENES);
}

/// The radial themes of the matrix: the families' members, the documented
/// look and the gallery's sunburst look.
fn radial_themes() -> Vec<(&'static str, RadialTheme)> {
    ThemeFamily::ALL
        .into_iter()
        .map(|family| (family.name(), family.radial()))
        .chain([
            ("documented", RadialTheme::default()),
            ("sunburst", looks::sunburst()),
        ])
        .collect()
}

// The radial scenes take every radial theme, the families' members first:
// the chart validates, exports at 1x and 2x, and its slices cover a share
// of the canvas in colours that are not the background.
#[test]
fn radial_scenes_take_their_themes() {
    let mut r = RadialRenderer::headless_blocking().expect("headless radial renderer");
    for &kind in RADIAL_SCENES {
        let declared = scenes::radial::document(kind);
        for (name, theme) in radial_themes() {
            let at = format!("{kind:?}: radial theme {name:?}");
            let mut chart = declared.clone();
            if ThemeFamily::named(name).is_some() {
                chart
                    .apply_theme(name)
                    .unwrap_or_else(|error| panic!("{at}: {error}"));
            } else {
                theme.apply(&mut chart);
            }
            chart.format_labels();
            assert_eq!(chart.validate(), Ok(()), "{at}");
            assert_eq!(chart.background, theme.background, "{at}");
            #[cfg(feature = "serde")]
            {
                let json = serde_json::to_string(&chart).expect("a radial document");
                let back: renderer::RadialChart = serde_json::from_str(&json).expect("it parses");
                assert_eq!(back, chart, "{at}: the themed chart round trip");
            }
            // Values, labels and the structure stay.
            assert_eq!(chart.slices.len(), declared.slices.len(), "{at}");
            for (themed, slice) in chart.slices.iter().zip(&declared.slices) {
                assert_eq!(
                    (themed.value, &themed.label, themed.children.len()),
                    (slice.value, &slice.label, slice.children.len()),
                    "{at}"
                );
            }

            let bg = chart.background;
            let clear = [bg.r, bg.g, bg.b].map(|c| (c * 255.0).round() as u8);
            let mut size = None;
            for scale in [1.0f32, 2.0] {
                let image = r
                    .export_rgba_blocking(
                        &chart,
                        RADIAL_SIZE,
                        ExportOptions::document(scale).with_clear(bg),
                    )
                    .unwrap_or_else(|error| panic!("{at} at {scale}x: {error}"));
                r.end_frame();
                let pixels = (image.width * image.height) as usize;
                let drawn = image
                    .rgba
                    .chunks_exact(4)
                    .filter(|px| (0..3).any(|i| px[i].abs_diff(clear[i]) > CHANGED))
                    .count();
                assert!(
                    drawn as f64 >= pixels as f64 * MIN_RADIAL_SHARE,
                    "{at} at {scale}x: {drawn} of {pixels} pixels are off the background"
                );
                match size {
                    None => size = Some((image.width, image.height)),
                    Some((w, h)) => assert_eq!((image.width, image.height), (w * 2, h * 2)),
                }
            }
        }
    }
}

// Every kind has a playground scene, so every family member is
// drawn by the groups above. A kind that loses its scene, or a new kind
// without one, fails here: its themes need a scene to be drawn on.
#[test]
fn every_kind_has_a_scene() {
    let mut r = renderer();
    let mut seen = Vec::new();
    for kind in SceneKind::ALL {
        if kind.is_radial() {
            continue;
        }
        let scene =
            scenes::build(&mut r, kind, 1_000).unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        let chart_kind = ChartKind::of(&scene.config, &scene.series, LayerRef::Base)
            .unwrap_or_else(|| panic!("{kind:?}: the base chart draws no series"));
        if !seen.contains(&chart_kind) {
            seen.push(chart_kind);
        }
    }
    let missing: Vec<ChartKind> = ChartKind::ALL
        .into_iter()
        .filter(|kind| !seen.contains(kind))
        .collect();
    assert_eq!(missing, [], "the kinds no playground scene draws");
}

/// A colour channel in linear light (sRGB transfer function).
fn linear(channel: f32) -> f32 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an opaque sRGB colour.
fn luminance(color: Color) -> f32 {
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// WCAG contrast ratio of two opaque colours, 1 to 21.
fn contrast(a: Color, b: Color) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

// The colours of every family member, every documented look and every
// gallery look stand off their background. Text (`chrome.ink`,
// `chrome.label_ink`, a radial theme's `label_color`) reaches the WCAG AA
// ratio of 4.5 against the theme's background, white where the theme leaves
// it transparent. Every opaque palette colour reaches 1.5 against the fill
// it is drawn on (the plot background when the theme has one). A
// translucent palette colour is a faint role on purpose (mesh edges as
// white hairlines at 16 %) and is not held to the floor.
#[test]
fn theme_colours_stand_off_their_background() {
    let mut failures = Vec::new();
    let mut themes = 0;
    for kind in ChartKind::ALL {
        let members = ThemeFamily::ALL
            .into_iter()
            .map(|family| (family.name(), family.theme(kind)));
        for (name, theme) in members.chain(custom_themes(kind)) {
            themes += 1;
            let chrome = &theme.chrome;
            let background = chrome.background.unwrap_or(Color::WHITE);
            assert_eq!(background.a, 1.0, "{kind} {name}: a translucent background");
            for (role, ink) in [("ink", chrome.ink), ("label_ink", chrome.label_ink)] {
                let ratio = contrast(ink, background);
                if ink.a < 1.0 || ratio < MIN_TEXT_CONTRAST {
                    failures.push(format!("{kind} {name}: {role} {ink:?} at {ratio:.2}"));
                }
            }
            let plot = match chrome.plot_background {
                Some(fill) if fill.a == 1.0 => fill,
                _ => background,
            };
            for (index, &color) in theme.series.palette.iter().enumerate() {
                let ratio = contrast(color, plot);
                if color.a == 1.0 && ratio < MIN_SERIES_CONTRAST {
                    failures.push(format!(
                        "{kind} {name}: palette[{index}] {color:?} at {ratio:.2}"
                    ));
                }
            }
        }
    }
    for (name, theme) in radial_themes() {
        themes += 1;
        let ratio = contrast(theme.label_color, theme.background);
        if ratio < MIN_TEXT_CONTRAST {
            failures.push(format!(
                "radial {name}: label_color {:?} at {ratio:.2}",
                theme.label_color
            ));
        }
        for (index, &color) in theme.palette.iter().enumerate() {
            let ratio = contrast(color, theme.background);
            if color.a == 1.0 && ratio < MIN_SERIES_CONTRAST {
                failures.push(format!(
                    "radial {name}: palette[{index}] {color:?} at {ratio:.2}"
                ));
            }
        }
    }
    eprintln!("theme matrix: {themes} themes checked for contrast");
    assert!(failures.is_empty(), "{failures:#?}");
}
