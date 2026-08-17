//! Keeps `crates/web/SCHEMA.md` in sync with the actual serde output of the
//! SSoT types — the same discipline as the shader SHADER_COMMON.md check.
//!
//! Run with the `serde` feature:
//!     cargo test -p model --features serde --test schema_sync
//!
//! Regenerate the canonical JSON blocks after schema changes:
//!     cargo test -p model --features serde --test schema_sync print_schema -- --ignored --nocapture
#![cfg(feature = "serde")]

use model::color::Color;
use model::config::{Config, PickedPointRef};
use model::data_config::{
    DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride, DataErrorBarStyleConfig,
    DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
    DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef, ScatterShape, SeriesConfig,
};
use model::default::default_config;
use model::format::{
    FractionalSecondDigits, TimestampLabelMode, TimestampTickPolicy, TimestampUnit, TimestampZone,
};
use model::line::LineStylePreset;
use model::text::{RichText, rich_segments_from_text};
use std::collections::BTreeMap;

const SCHEMA_BLOCK_NAMES: [&str; 4] =
    ["config", "series", "option-omissions", "timestamp-variants"];
const SCHEMA_MARKER_PREFIX: &str = "<!-- schema-sync: name=";

fn canonical_config() -> Config {
    default_config()
}

/// One series using the richest render type, both `ErrorRef` forms, and a
/// populated label — every SeriesConfig field shape appears at least once.
fn canonical_series() -> Vec<SeriesConfig> {
    vec![SeriesConfig {
        series_id: "example".into(),
        source_id: Some("source-a".into()),
        label: Some(RichText {
            segments: rich_segments_from_text("V₀"),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
        }),
        x_column: "x".into(),
        y_column: "y".into(),
        render_type: DataRenderType::LineScatterErrorbarXY {
            scatter: DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: ScatterShape::CircleFilled,
                point_size: 4.0,
                point_style_table: Some(vec![
                    DataScatterPointStyleConfig {
                        point_color: Some(Color::from_rgb8(230, 57, 70)),
                        point_shape: Some(ScatterShape::CircleFilled),
                        point_size: Some(5.0),
                    },
                    DataScatterPointStyleConfig {
                        point_color: Some(Color::from_rgb8(29, 53, 87)),
                        point_shape: Some(ScatterShape::DiamondFilled),
                        point_size: None,
                    },
                ]),
                point_style_index_column: Some("style_index".into()),
                point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                    index: 3,
                    style: DataScatterPointStyleConfig {
                        point_color: None,
                        point_shape: Some(ScatterShape::StarFilled),
                        point_size: Some(7.0),
                    },
                }]),
            },
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 2.0,
            },
            err_x: ErrorRef::Asymmetric {
                lower: "ex_lo".into(),
                upper: "ex_hi".into(),
            },
            err_y: ErrorRef::Symmetric {
                column: "ey".into(),
            },
            err_style: DataErrorBarStyleConfig {
                error_bar_color: Color::BLACK,
                error_bar_width: 1.0,
                error_bar_cap_size: 3.0,
                cap_width: 1.0,
                error_bar_style_table: Some(vec![
                    DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::from_rgb8(217, 36, 36)),
                        error_bar_width: Some(2.0),
                        error_bar_cap_size: None,
                        cap_width: None,
                    },
                    DataErrorBarPointStyleConfig {
                        error_bar_color: None,
                        error_bar_width: None,
                        error_bar_cap_size: Some(6.0),
                        cap_width: Some(2.0),
                    },
                ]),
                error_bar_style_index_column: Some("err_style_index".into()),
                error_bar_style_overrides: Some(vec![DataErrorBarPointStyleOverride {
                    index: 2,
                    style: DataErrorBarPointStyleConfig {
                        error_bar_color: Some(Color::from_rgb8(29, 53, 87)),
                        error_bar_width: None,
                        error_bar_cap_size: None,
                        cap_width: Some(3.0),
                    },
                }]),
            },
        },
    }]
}

#[derive(serde::Serialize)]
struct OptionalKeyOmissionFixture {
    series: SeriesConfig,
    scatter_style: DataScatterStyleConfig,
    errorbar_style: DataErrorBarStyleConfig,
    scatter_point_style: DataScatterPointStyleConfig,
    errorbar_point_style: DataErrorBarPointStyleConfig,
    picked_point: PickedPointRef,
}

fn optional_key_omission_fixture() -> OptionalKeyOmissionFixture {
    OptionalKeyOmissionFixture {
        series: SeriesConfig {
            series_id: "no-source".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                },
            },
        },
        scatter_style: DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        },
        errorbar_style: DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 3.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        },
        scatter_point_style: DataScatterPointStyleConfig::default(),
        errorbar_point_style: DataErrorBarPointStyleConfig::default(),
        picked_point: PickedPointRef {
            source_id: None,
            series_id: "no-source".into(),
            point_index: 7,
        },
    }
}

#[derive(serde::Serialize)]
struct TimestampVariantsFixture {
    unit: [TimestampUnit; 4],
    timezone: [TimestampZone; 2],
    label: [TimestampLabelMode; 2],
    fractional: [FractionalSecondDigits; 2],
    tick_policy: [TimestampTickPolicy; 2],
}

fn timestamp_variants_fixture() -> TimestampVariantsFixture {
    TimestampVariantsFixture {
        unit: [
            TimestampUnit::Seconds,
            TimestampUnit::Milliseconds,
            TimestampUnit::Microseconds,
            TimestampUnit::Nanoseconds,
        ],
        timezone: [TimestampZone::Utc, TimestampZone::FixedOffsetMinutes(540)],
        label: [
            TimestampLabelMode::Auto,
            TimestampLabelMode::Pattern("%Y-%m-%d %H:%M:%S.%f".into()),
        ],
        fractional: [
            FractionalSecondDigits::Auto,
            FractionalSecondDigits::Fixed(3),
        ],
        tick_policy: [
            TimestampTickPolicy::AutoCalendar,
            TimestampTickPolicy::NumericSpacing,
        ],
    }
}

fn config_json() -> String {
    serde_json::to_string_pretty(&canonical_config()).expect("serialize Config")
}

fn series_json() -> String {
    serde_json::to_string_pretty(&canonical_series()).expect("serialize SeriesConfig")
}

fn option_omissions_json() -> String {
    serde_json::to_string_pretty(&optional_key_omission_fixture())
        .expect("serialize optional-key omission fixture")
}

fn timestamp_variants_json() -> String {
    serde_json::to_string_pretty(&timestamp_variants_fixture())
        .expect("serialize timestamp variants fixture")
}

fn parse_schema_md_json_blocks(md: &str) -> Result<BTreeMap<String, String>, String> {
    let normalized = md.replace("\r\n", "\n");
    let lines: Vec<_> = normalized.lines().collect();
    let mut blocks = BTreeMap::new();
    let mut line_index = 0;

    while line_index < lines.len() {
        let line = lines[line_index].trim();
        if let Some(marker) = line.strip_prefix(SCHEMA_MARKER_PREFIX) {
            let Some(name) = marker.strip_suffix(" -->") else {
                return Err(format!(
                    "unterminated schema-sync marker on line {}",
                    line_index + 1
                ));
            };
            if name.is_empty() {
                return Err(format!(
                    "schema-sync marker on line {} has an empty name",
                    line_index + 1
                ));
            }
            if blocks.contains_key(name) {
                return Err(format!("duplicate schema-sync marker name {name:?}"));
            }

            let fence_index = line_index + 1;
            if lines.get(fence_index).map(|line| line.trim()) != Some("```json") {
                return Err(format!(
                    "schema-sync marker {name:?} must be immediately followed by a ```json fence"
                ));
            }

            let mut block = String::new();
            let mut end_index = fence_index + 1;
            while end_index < lines.len() && lines[end_index].trim() != "```" {
                block.push_str(lines[end_index]);
                block.push('\n');
                end_index += 1;
            }
            if end_index == lines.len() {
                return Err(format!(
                    "unterminated ```json fence for schema-sync marker {name:?}"
                ));
            }

            blocks.insert(name.to_string(), block.trim_end().to_string());
            line_index = end_index + 1;
        } else {
            line_index += 1;
        }
    }

    for name in SCHEMA_BLOCK_NAMES {
        if !blocks.contains_key(name) {
            return Err(format!("missing schema-sync marker name {name:?}"));
        }
    }

    Ok(blocks)
}

fn schema_md_json_blocks() -> BTreeMap<String, String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/SCHEMA.md");
    let md = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    parse_schema_md_json_blocks(&md)
        .unwrap_or_else(|e| panic!("parse named JSON fences in {path:?}: {e}"))
}

fn assert_schema_block(blocks: &BTreeMap<String, String>, name: &str, expected: String) {
    assert_eq!(
        blocks.get(name),
        Some(&expected),
        "SCHEMA.md {name:?} block drifted from serde output -- regenerate with the \
         print_schema test (see file header)."
    );
}

#[test]
fn schema_doc_matches_serde_output() {
    let blocks = schema_md_json_blocks();
    assert_schema_block(&blocks, "config", config_json());
    assert_schema_block(&blocks, "series", series_json());
    assert_schema_block(&blocks, "option-omissions", option_omissions_json());
    assert_schema_block(&blocks, "timestamp-variants", timestamp_variants_json());
}

fn synthetic_schema(block_names: &[&str]) -> String {
    block_names
        .iter()
        .map(|name| format!("<!-- schema-sync: name={name} -->\n```json\n{{}}\n```\n"))
        .collect()
}

#[test]
fn named_fence_parser_is_order_independent() {
    let blocks = parse_schema_md_json_blocks(&synthetic_schema(&[
        "timestamp-variants",
        "option-omissions",
        "series",
        "config",
    ]))
    .expect("parse reordered named fences");

    for name in SCHEMA_BLOCK_NAMES {
        assert_eq!(blocks.get(name).map(String::as_str), Some("{}"));
    }
}

#[test]
fn named_fence_parser_rejects_duplicate_names() {
    let mut md = synthetic_schema(&SCHEMA_BLOCK_NAMES);
    md.push_str("<!-- schema-sync: name=config -->\n```json\n{}\n```\n");
    let err = parse_schema_md_json_blocks(&md).expect_err("reject duplicate marker name");
    assert!(err.contains("duplicate"), "{err}");
}

#[test]
fn named_fence_parser_rejects_missing_names() {
    let md = synthetic_schema(&SCHEMA_BLOCK_NAMES[..3]);
    let err = parse_schema_md_json_blocks(&md).expect_err("reject missing marker name");
    assert!(err.contains("missing"), "{err}");
}

#[test]
fn named_fence_parser_rejects_non_adjacent_fences() {
    let md = synthetic_schema(&SCHEMA_BLOCK_NAMES).replacen(
        "<!-- schema-sync: name=config -->\n```json",
        "<!-- schema-sync: name=config -->\n\n```json",
        1,
    );
    let err = parse_schema_md_json_blocks(&md).expect_err("reject non-adjacent fence");
    assert!(err.contains("immediately followed"), "{err}");
}

#[test]
fn named_fence_parser_rejects_unterminated_markers() {
    let md = synthetic_schema(&SCHEMA_BLOCK_NAMES).replacen(
        "<!-- schema-sync: name=config -->",
        "<!-- schema-sync: name=config",
        1,
    );
    let err = parse_schema_md_json_blocks(&md).expect_err("reject unterminated marker");
    assert!(err.contains("unterminated schema-sync marker"), "{err}");
}

#[test]
fn named_fence_parser_rejects_unterminated_fences() {
    let mut md = synthetic_schema(&SCHEMA_BLOCK_NAMES);
    md.truncate(md.rfind("```").expect("closing fence"));
    let err = parse_schema_md_json_blocks(&md).expect_err("reject unterminated fence");
    assert!(err.contains("unterminated ```json fence"), "{err}");
}

/// Utility — prints the canonical JSON for pasting into SCHEMA.md.
#[test]
#[ignore]
fn print_schema() {
    println!("===== Config =====\n{}", config_json());
    println!("===== SeriesConfig =====\n{}", series_json());
    println!(
        "===== Optional key omissions =====\n{}",
        option_omissions_json()
    );
    println!(
        "===== Timestamp variants =====\n{}",
        timestamp_variants_json()
    );
}
