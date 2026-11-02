//! Documents written by the published figgy-model releases still parse with
//! this source, and a document that uses no newer field serializes back to the
//! exact bytes the release wrote, except where 0.19 moved a fact to one owner
//! (#160): such a document is written in the new form, which [`migrate`]
//! spells out and which then round trips byte for byte.
//! `tests/fixtures/released/README.md` says which releases wrote the fixtures
//! and how. No GPU is needed.
#![cfg(feature = "serde")]
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]

use renderer::config::DrawStyle;
use renderer::{BoxPlotChart, CategoricalChart, Config, RadarChart, RadialChart, SeriesConfig};

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/released");

/// One test per fixture: parse with this source's type, serialize again, and
/// require the fixture's bytes back.
macro_rules! released_documents {
    ($($test:ident: $ty:ty = $file:literal;)*) => {
        const FIXTURES: &[&str] = &[$($file),*];

        $(
            #[test]
            fn $test() {
                let fixture = include_str!(concat!("fixtures/released/", $file));
                let parsed: $ty = serde_json::from_str(fixture).unwrap_or_else(|err| {
                    panic!("{}: a released document no longer parses: {err}", $file)
                });
                let written = serde_json::to_string(&parsed).expect("serialize");
                let mut expected: serde_json::Value =
                    serde_json::from_str(fixture).expect("fixture is JSON");
                if !migrate($file, &mut expected) {
                    assert_same_document($file, fixture, &written);
                    return;
                }
                let new: serde_json::Value =
                    serde_json::from_str(&written).expect("output is JSON");
                if let Some(difference) = first_difference("$", &expected, &new) {
                    panic!("{}: not the documented 0.19 form: {difference}", $file);
                }
                let again: $ty = serde_json::from_str(&written).expect("the new form parses");
                let rewritten = serde_json::to_string(&again).expect("serialize");
                assert_same_document($file, &written, &rewritten);
            }
        )*
    };
}

released_documents! {
    model_0_12_1_radial_default: RadialChart = "model-0.12.1-radial-default.json";
    model_0_12_1_radial_edited: RadialChart = "model-0.12.1-radial-edited.json";
    model_0_12_1_categorical_default: CategoricalChart = "model-0.12.1-categorical-default.json";
    model_0_12_1_categorical_edited: CategoricalChart = "model-0.12.1-categorical-edited.json";
    model_0_12_1_boxplot_default: BoxPlotChart = "model-0.12.1-boxplot-default.json";
    model_0_12_1_boxplot_edited: BoxPlotChart = "model-0.12.1-boxplot-edited.json";
    model_0_12_1_radar_default: RadarChart = "model-0.12.1-radar-default.json";
    model_0_12_1_radar_edited: RadarChart = "model-0.12.1-radar-edited.json";

    model_0_11_0_config_default: Config = "model-0.11.0-config-default.json";
    model_0_11_0_config_series: Config = "model-0.11.0-config-series.json";
    model_0_11_0_config_edited: Config = "model-0.11.0-config-edited.json";
    model_0_11_0_draw_styles: Vec<DrawStyle> = "model-0.11.0-draw-styles.json";
    model_0_11_0_series_line: Vec<SeriesConfig> = "model-0.11.0-series-line.json";
    model_0_11_0_series_scatter: Vec<SeriesConfig> = "model-0.11.0-series-scatter.json";
    model_0_11_0_series_errorbar: Vec<SeriesConfig> = "model-0.11.0-series-errorbar.json";
    model_0_11_0_series_histogram: Vec<SeriesConfig> = "model-0.11.0-series-histogram.json";
    model_0_11_0_series_field: Vec<SeriesConfig> = "model-0.11.0-series-field.json";
    model_0_11_0_radial_default: RadialChart = "model-0.11.0-radial-default.json";
    model_0_11_0_radial_edited: RadialChart = "model-0.11.0-radial-edited.json";
    model_0_11_0_categorical_default: CategoricalChart = "model-0.11.0-categorical-default.json";
    model_0_11_0_categorical_edited: CategoricalChart = "model-0.11.0-categorical-edited.json";
    model_0_11_0_boxplot_default: BoxPlotChart = "model-0.11.0-boxplot-default.json";
    model_0_11_0_boxplot_edited: BoxPlotChart = "model-0.11.0-boxplot-edited.json";

    model_0_10_0_config_default: Config = "model-0.10.0-config-default.json";
    model_0_10_0_config_series: Config = "model-0.10.0-config-series.json";
    model_0_10_0_config_edited: Config = "model-0.10.0-config-edited.json";
    model_0_10_0_draw_styles: Vec<DrawStyle> = "model-0.10.0-draw-styles.json";
    model_0_10_0_series_line: Vec<SeriesConfig> = "model-0.10.0-series-line.json";
    model_0_10_0_series_scatter: Vec<SeriesConfig> = "model-0.10.0-series-scatter.json";
    model_0_10_0_series_errorbar: Vec<SeriesConfig> = "model-0.10.0-series-errorbar.json";
    model_0_10_0_series_histogram: Vec<SeriesConfig> = "model-0.10.0-series-histogram.json";
    model_0_10_0_series_field: Vec<SeriesConfig> = "model-0.10.0-series-field.json";
    model_0_10_0_radial_default: RadialChart = "model-0.10.0-radial-default.json";
    model_0_10_0_radial_edited: RadialChart = "model-0.10.0-radial-edited.json";
    model_0_10_0_categorical_default: CategoricalChart = "model-0.10.0-categorical-default.json";
    model_0_10_0_categorical_edited: CategoricalChart = "model-0.10.0-categorical-edited.json";
    model_0_10_0_boxplot_default: BoxPlotChart = "model-0.10.0-boxplot-default.json";
    model_0_10_0_boxplot_edited: BoxPlotChart = "model-0.10.0-boxplot-edited.json";
}

/// Rewrite a released document into the form 0.19 writes, the way the
/// CHANGELOG's migration notes describe it. `false` when nothing changed.
fn migrate(file: &str, document: &mut serde_json::Value) -> bool {
    let Some(object) = document.as_object_mut() else {
        return false;
    };
    if file.contains("-radial-") {
        return migrate_radial_title(object);
    }
    migrate_picked_points(object)
}

/// A radial chart's plain `title` becomes its `chart_title` (#160): drawn in
/// the document's font at 1.5 × `font_size` and in `label_color` when the
/// document has no `chart_title`, dropped beside one, and removed when it is
/// empty.
fn migrate_radial_title(chart: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    use serde_json::json;
    let Some(title) = chart.remove("title") else {
        return false;
    };
    let title = title.as_str().expect("a plain title").to_owned();
    if chart.get("chart_title").is_none_or(|t| t.is_null()) && !title.is_empty() {
        let color: renderer::Color =
            serde_json::from_value(chart["label_color"].clone()).expect("label colour");
        let plain = renderer::config::ChartTitleOptions::from_legacy_plain(
            &title,
            chart["font_family"].as_str().expect("font family"),
            chart["font_size"].as_f64().expect("font size") as f32,
            color,
        );
        // Through the JSON text, as the chart writes its `f32` fields.
        let text: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&plain.text).expect("serialize"))
                .expect("JSON");
        chart.insert(
            "chart_title".into(),
            json!({"visible": plain.visible, "text": text}),
        );
    }
    true
}

/// `picked_points` joins `picked_data` (#160): its refs become `point` refs
/// after the typed ones, and its ring look becomes the highlight's when the
/// document has no `picked_data`. Hidden points are dropped beside a typed
/// selection.
fn migrate_picked_points(config: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    use serde_json::{Value, json};
    let Some(legacy) = config.remove("picked_points") else {
        return false;
    };
    let points = legacy["refs"].as_array().cloned().unwrap_or_default();
    let points = points.into_iter().map(|mut point| {
        point
            .as_object_mut()
            .expect("a ref is an object")
            .insert("kind".into(), json!("point"));
        point
    });
    match config.get_mut("picked_data") {
        Some(Value::Object(typed)) => {
            if legacy["visible"] != json!(false) {
                let refs = typed.entry("refs").or_insert_with(|| json!([]));
                refs.as_array_mut().expect("refs").extend(points);
            }
        }
        _ => {
            let default =
                serde_json::to_value(renderer::DataSelectionsConfig::default()).expect("serialize");
            let field = |key: &str, default_key: &str| {
                legacy
                    .get(key)
                    .cloned()
                    .unwrap_or(default[default_key].clone())
            };
            config.insert(
                "picked_data".into(),
                json!({
                    "visible": field("visible", "visible"),
                    "refs": points.collect::<Vec<_>>(),
                    "highlight_color": field("ring_color", "highlight_color"),
                    "outline_width_px": field("ring_width_px", "outline_width_px"),
                    "point_radius_extra_px": field("radius_extra_px", "point_radius_extra_px"),
                    "contour_width_extra_px": default["contour_width_extra_px"],
                }),
            );
        }
    }
    true
}

/// Every released categorical document opens as a Cartesian chart: the
/// reader's config and series validate as category bars, survive a serde
/// round trip, and its uploads cover every series.
#[test]
fn released_categorical_documents_convert_to_category_bars() {
    for file in FIXTURES.iter().filter(|f| f.contains("categorical")) {
        let fixture = std::fs::read_to_string(format!("{FIXTURE_DIR}/{file}")).unwrap();
        let doc: CategoricalChart = serde_json::from_str(&fixture).unwrap();
        let (config, series, uploads) = Config::from_categorical(&doc);
        assert_eq!(series.len(), doc.series.len(), "{file}");
        assert_eq!(uploads.len(), doc.series.len() + 1, "{file}");
        if !doc.categories.is_empty() && !doc.series.is_empty() {
            assert_eq!(config.validate_category_bars(&series), Ok(()), "{file}");
        }
        let config_json = serde_json::to_string(&config).unwrap();
        let again: Config = serde_json::from_str(&config_json).unwrap();
        assert_eq!(again, config, "{file}: the config round trips");
        let series_json = serde_json::to_string(&series).unwrap();
        let again: Vec<SeriesConfig> = serde_json::from_str(&series_json).unwrap();
        assert_eq!(again, series, "{file}: the series round trip");
    }
}

/// Every released box plot document opens as a Cartesian chart: the
/// reader's config and series validate as box plots, survive a serde round
/// trip, and its uploads cover every series' record, its outliers when it
/// has any, and the positions.
#[test]
fn released_box_plot_documents_convert_to_box_plot_series() {
    for file in FIXTURES.iter().filter(|f| f.contains("boxplot")) {
        let fixture = std::fs::read_to_string(format!("{FIXTURE_DIR}/{file}")).unwrap();
        let doc: BoxPlotChart = serde_json::from_str(&fixture).unwrap();
        let (config, series, uploads) = Config::from_boxplot(&doc);
        assert_eq!(series.len(), doc.series.len(), "{file}");
        let with_outliers = doc
            .series
            .iter()
            .filter(|s| s.values.iter().flatten().any(|v| !v.outliers.is_empty()))
            .count();
        assert_eq!(
            uploads.len(),
            1 + doc.series.len() + with_outliers,
            "{file}"
        );
        if !doc.categories.is_empty() && !doc.series.is_empty() {
            assert_eq!(config.validate_box_plots(&series), Ok(()), "{file}");
        }
        let config_json = serde_json::to_string(&config).unwrap();
        let again: Config = serde_json::from_str(&config_json).unwrap();
        assert_eq!(again, config, "{file}: the config round trips");
        let series_json = serde_json::to_string(&series).unwrap();
        let again: Vec<SeriesConfig> = serde_json::from_str(&series_json).unwrap();
        assert_eq!(again, series, "{file}: the series round trip");
    }
}

/// Every released radar document opens as a polar chart: the reader's
/// config and series validate as radar series on a categorical angle axis,
/// survive a serde round trip, and its uploads are the category indices and
/// one value column per series.
#[test]
fn released_radar_documents_convert_to_radar_series() {
    for file in FIXTURES.iter().filter(|f| f.contains("radar")) {
        let fixture = std::fs::read_to_string(format!("{FIXTURE_DIR}/{file}")).unwrap();
        let doc: RadarChart = serde_json::from_str(&fixture).unwrap();
        let (config, series, uploads) = Config::from_radar(&doc);
        assert_eq!(series.len(), doc.series.len(), "{file}");
        assert_eq!(uploads.len(), doc.series.len() + 1, "{file}");
        assert!(config.polar().is_some(), "{file}");
        assert_eq!(
            config.bottom_x.category_count(),
            Some(doc.categories.len()),
            "{file}"
        );
        if !doc.categories.is_empty() && !doc.series.is_empty() {
            assert_eq!(config.validate_polar(), Ok(()), "{file}");
            assert_eq!(config.validate_radar(&series), Ok(()), "{file}");
        }
        for (s, upload) in doc.series.iter().zip(&uploads[1..]) {
            assert_eq!(upload.values.len(), s.values.len(), "{file}");
        }
        let config_json = serde_json::to_string(&config).unwrap();
        let again: Config = serde_json::from_str(&config_json).unwrap();
        assert_eq!(again, config, "{file}: the config round trips");
        let series_json = serde_json::to_string(&series).unwrap();
        let again: Vec<SeriesConfig> = serde_json::from_str(&series_json).unwrap();
        assert_eq!(again, series, "{file}: the series round trip");
    }
}

/// A fixture added to the directory without a test above would be silently
/// unchecked.
#[test]
fn every_released_fixture_has_a_test() {
    let mut on_disk: Vec<String> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixture directory")
        .map(|entry| entry.expect("directory entry").file_name())
        .map(|name| name.into_string().expect("UTF-8 file name"))
        .filter(|name| name.ends_with(".json"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = FIXTURES.iter().map(|name| name.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed);
}

/// Byte equality, with a failure message that names the first differing JSON
/// path and both values instead of two long one-line documents.
fn assert_same_document(file: &str, fixture: &str, written: &str) {
    if fixture == written {
        return;
    }
    let old: serde_json::Value = serde_json::from_str(fixture).expect("fixture is JSON");
    let new: serde_json::Value = serde_json::from_str(written).expect("output is JSON");
    let difference = first_difference("$", &old, &new).unwrap_or_else(|| {
        let at = fixture
            .bytes()
            .zip(written.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(fixture.len().min(written.len()));
        format!(
            "same JSON value, different bytes from offset {at}: fixture {:?}, written {:?}",
            around(fixture, at),
            around(written, at),
        )
    });
    panic!("{file}: re-serialized document differs from the released bytes: {difference}");
}

/// About 40 bytes either side of `at`, cut on character boundaries.
fn around(text: &str, at: usize) -> &str {
    let start = text.floor_char_boundary(at.saturating_sub(40));
    let end = text.ceil_char_boundary((at + 40).min(text.len()));
    &text[start..end]
}

fn first_difference(
    path: &str,
    old: &serde_json::Value,
    new: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|key| {
                let at = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => first_difference(&at, x, y),
                    (Some(x), None) => Some(format!("{at}: fixture {x}, written <absent>")),
                    (None, Some(y)) => Some(format!("{at}: fixture <absent>, written {y}")),
                    (None, None) => None,
                }
            })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(i, (x, y))| first_difference(&format!("{path}[{i}]"), x, y)),
        _ if old == new => None,
        _ => Some(format!("{path}: fixture {old}, written {new}")),
    }
}
