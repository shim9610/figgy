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
use model::config::Config;
use model::data_config::{
    DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride, DataErrorBarStyleConfig,
    DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
    DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef, ScatterShape, SeriesConfig,
};
use model::default::default_config;
use model::line::LineStylePreset;
use model::text::{RichText, rich_segments_from_text};

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

fn config_json() -> String {
    serde_json::to_string_pretty(&canonical_config()).expect("serialize Config")
}

fn series_json() -> String {
    serde_json::to_string_pretty(&canonical_series()).expect("serialize SeriesConfig")
}

/// Extract the fenced ```json blocks from SCHEMA.md, in order.
fn schema_md_json_blocks() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/SCHEMA.md");
    let md = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path:?}: {e}"))
        .replace("\r\n", "\n");

    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut current = String::new();
    for line in md.lines() {
        if in_block {
            if line.trim_start() == "```" {
                blocks.push(current.trim_end().to_string());
                current = String::new();
                in_block = false;
            } else {
                current.push_str(line);
                current.push('\n');
            }
        } else if line.trim_start() == "```json" {
            in_block = true;
        }
    }
    blocks
}

#[test]
fn schema_doc_matches_serde_output() {
    let blocks = schema_md_json_blocks();
    assert!(
        blocks.len() >= 2,
        "SCHEMA.md must contain at least two ```json blocks (Config, SeriesConfig); found {}",
        blocks.len()
    );
    assert_eq!(
        blocks[0],
        config_json(),
        "SCHEMA.md Config block drifted from serde output — regenerate with the \
         print_schema test (see file header)."
    );
    assert_eq!(
        blocks[1],
        series_json(),
        "SCHEMA.md SeriesConfig block drifted from serde output — regenerate with \
         the print_schema test (see file header)."
    );
}

/// Utility — prints the canonical JSON for pasting into SCHEMA.md.
#[test]
#[ignore]
fn print_schema() {
    println!("===== Config =====\n{}", config_json());
    println!("===== SeriesConfig =====\n{}", series_json());
}
