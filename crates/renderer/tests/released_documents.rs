//! Documents written by the published figgy-model releases still parse with
//! this source, and a document that uses no newer field serializes back to the
//! exact bytes the release wrote. `tests/fixtures/released/README.md` says
//! which releases wrote the fixtures and how. No GPU is needed.
#![cfg(feature = "serde")]

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
                assert_same_document($file, fixture, &written);
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
