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
use model::config::{ColorBarOptions, Config, PickedPointRef};
use model::data_config::{
    BarOrientation, ContourConfig, ContourLabelAnchor, ContourLabelConfig, DataBarBinStyleConfig,
    DataBarStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterPointStyleConfig, DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef,
    FieldFillConfig, FillMode, GridLayout, MAX_CONTOUR_LEVELS, MatrixOrientation, MatrixRef,
    ScatterShape, SeriesConfig, Shading,
};
use model::default::{default_colorbar_options, default_config};
use model::format::{
    FractionalSecondDigits, LabelFormat, TimestampLabelMode, TimestampTickPolicy, TimestampUnit,
    TimestampZone,
};
use model::line::LineStylePreset;
use model::text::{RichText, rich_segments_from_text};
use std::collections::BTreeMap;

const SCHEMA_BLOCK_NAMES: [&str; 6] = [
    "config",
    "series",
    "option-omissions",
    "timestamp-variants",
    "colorbar",
    "field-render-types",
];
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

/// The value of `Config.colorbar`, at its stock settings.
///
/// A separate block from `config` because the canonical `Config` has no z
/// dimension (`colorbar: None`, key absent), which is the shape every document
/// written before field series existed has. Hosts still need the object's full
/// form, and `axis` inside it is a complete `AxisOptions` — that is the point of
/// the design, not an accident of reuse.
fn canonical_colorbar() -> ColorBarOptions {
    default_colorbar_options()
}

/// One of each field / bar render type, with every optional key populated.
///
/// A struct rather than a map so the JSON key order is fixed by declaration
/// order and the block does not churn between runs.
#[derive(serde::Serialize)]
struct FieldRenderTypesFixture {
    histogram: DataRenderType,
    heatmap: DataRenderType,
    contour: DataRenderType,
    heatmap_contour: DataRenderType,
}

fn field_render_types_fixture() -> FieldRenderTypesFixture {
    let matrix = || MatrixRef {
        columns: vec!["z0".into(), "z1".into(), "z2".into()],
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Edges,
    };
    let fill = || FieldFillConfig {
        mode: FillMode::Continuous,
        shading: Shading::Interpolated,
        opacity: 1.0,
    };
    let contour = || ContourConfig {
        levels: vec![1.0, 2.0, 5.0],
        line: DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
        },
        per_level_color: Some(vec![
            Color::from_rgb8(230, 57, 70),
            Color::from_rgb8(29, 53, 87),
            Color::from_rgb8(42, 157, 143),
        ]),
        labels: Some(ContourLabelConfig {
            visible: true,
            font_size: 12.0,
            color: Color::BLACK,
            format: LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px: 140.0,
            anchors: vec![ContourLabelAnchor {
                level_index: 1,
                x: 0.5,
                y: 0.25,
                tx: 1.0,
                ty: 0.0,
            }],
            bg_color: Some(Color::from_rgb8(255, 255, 255)),
            bg_padding_px: 2.0,
        }),
    };
    FieldRenderTypesFixture {
        histogram: DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::from_rgb8(70, 130, 180),
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 0.85,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: Some(vec![DataBarStyleOverride {
                    index: 1,
                    style: DataBarBinStyleConfig {
                        fill_color: Some(Color::from_rgb8(230, 57, 70)),
                        border_color: Some(Color::from_rgb8(120, 20, 30)),
                        border_width: Some(2.0),
                        gap_px: None,
                        width_ratio: Some(0.6),
                    },
                }]),
            },
        },
        heatmap: DataRenderType::Heatmap {
            matrix: matrix(),
            fill: fill(),
        },
        contour: DataRenderType::Contour {
            matrix: matrix(),
            contour: contour(),
        },
        heatmap_contour: DataRenderType::HeatmapContour {
            matrix: matrix(),
            fill: fill(),
            contour: contour(),
        },
    }
}

fn colorbar_json() -> String {
    serde_json::to_string_pretty(&canonical_colorbar()).expect("serialize ColorBarOptions")
}

fn field_render_types_json() -> String {
    serde_json::to_string_pretty(&field_render_types_fixture())
        .expect("serialize field render types")
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
    assert_schema_block(&blocks, "colorbar", colorbar_json());
    assert_schema_block(&blocks, "field-render-types", field_render_types_json());
}

fn normalized_words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_contour_contract_marker(document: &str, scope: &str) {
    let prefix = format!("<!-- contour-contract: scope={scope} max-levels=");
    assert_eq!(
        document.matches(&prefix).count(),
        1,
        "{scope} must contain exactly one contour contract marker"
    );
    let marker = format!("{prefix}{MAX_CONTOUR_LEVELS} -->");
    assert!(
        document.contains(&marker),
        "{scope} contour limit drifted from MAX_CONTOUR_LEVELS"
    );
}

#[test]
fn contour_contract_docs_match_the_model_limit_and_failure_semantics() {
    let model_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &std::path::Path| {
        std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path:?}: {error}"))
    };
    let readme = read(&model_root.join("../../README.md"));
    let schema = read(&model_root.join("../web/SCHEMA.md"));
    let wasm = read(&model_root.join("../renderer/WASM.md"));

    for (document, scope) in [
        (readme.as_str(), "readme-en"),
        (readme.as_str(), "readme-ko"),
        (schema.as_str(), "schema"),
        (wasm.as_str(), "wasm"),
    ] {
        assert_contour_contract_marker(document, scope);
    }

    let readme = normalized_words(&readme);
    let schema = normalized_words(&schema);
    let wasm = normalized_words(&wasm);
    for required in [
        "Its accepted length is `0..=1024`; 1025 or more is an error, and no level is silently truncated.",
        "허용 길이는 `0..=1024`이며 1025개 이상은 오류이고 어떤 레벨도 조용히 잘라내지 않는다.",
        "Each fragment searches at most 32 blocks and runs coverage math only for reachable candidates",
        "if all 1024 levels actually cross one cell, all 1024 are composited in declaration order.",
        "Stroke distance is the quadratic crossing",
        "restricting the current cell's bilinear field to the current gradient-normal line.",
        "It is not a global shortest distance to the whole piecewise-bilinear contour.",
        "the per-level fallback may keep a closer candidate rather than omit a level.",
        "`spacing_px` must always be finite and greater than zero, including for hidden labels and explicit overrides.",
        "Automatic and explicit placement share a 1024-label capacity.",
        "Explicit anchors whose `level_index` is invalid are discarded",
        "only the first 1024 valid anchors are retained in input order.",
        "If that resolved list is empty, automatic placement runs; otherwise the resolved list overrides it.",
        "한 셀에 1024개가 실제로 모두 걸리면 선언 순서대로 1024개 전부를 합성한다.",
        "gradient-normal 직선으로 제한해서 얻는 이차방정식 교차근이며, 전체 piecewise-bilinear contour에 대한 전역 최단거리는 아니다.",
        "레벨별 fallback은 레벨을 누락시키지 않기 위해 더 가까운 후보를 남길 수 있다.",
        "`spacing_px`는 숨김 라벨과 명시 오버라이드에서도 항상 유한한 양수여야 한다.",
        "자동/명시 배치는 공통으로 1024개 용량을 쓴다.",
        "명시 앵커는 유효하지 않은 `level_index`를 버리고",
        "입력 순서에서 유효한 앞 1024개만 남긴다.",
        "resolved 목록이 비면 자동 배치하고, 하나라도 남으면 그 목록이 자동 배치를 대체한다.",
    ] {
        assert!(
            readme.contains(required),
            "README contour contract lost: {required}"
        );
    }
    assert!(
        schema.contains("허용 길이는 `0..=1024`이고 1025개 이상이면 `set_series`가 실패한다."),
        "SCHEMA contour limit contract drifted"
    );
    assert!(
        schema.contains("이전 config, series, GPU style은 그대로 유지된다."),
        "SCHEMA contour failure atomicity drifted"
    );
    for required in [
        "레벨별 fallback은 더 가까운 후보를 남길 수 있다.",
        "숨김 라벨과 명시 anchor에서도 유한한 양수여야 한다.",
        "automatic/explicit은 공통 1024개 용량을 쓴다.",
        "유효하지 않은 `level_index`를 제거한 뒤 입력 순서의 앞 1024개만 사용한다.",
        "resolved 목록이 비면 자동 배치하고, 하나라도 남으면 그 목록이 자동 배치를 대체한다.",
        "automatic에서만 clamp된 frame/export scale을 spacing에 곱해 유한한 양수인지 다시 검사한다.",
    ] {
        assert!(
            schema.contains(required),
            "SCHEMA contour anchor contract lost: {required}"
        );
    }
    assert!(
        wasm.contains("1025개 이상이면 JavaScript 예외를 반환하고 이전 config, series 선언, GPU style을 그대로 유지"),
        "WASM contour failure atomicity drifted"
    );
    assert!(
        wasm.contains("다음 frame도 이전 상태를 그린다."),
        "WASM contour next-frame contract drifted"
    );
    assert!(
        wasm.contains("Contour label도 automatic/explicit 공통으로 1024개까지 지원한다."),
        "WASM contour label capacity drifted"
    );
    assert!(
        wasm.contains("WASM 전용으로 더 작은 상한을 두거나 배열을 조용히 자르는 경로는 없다."),
        "WASM contour no-truncation contract drifted"
    );
}

fn synthetic_schema(block_names: &[&str]) -> String {
    block_names
        .iter()
        .map(|name| format!("<!-- schema-sync: name={name} -->\n```json\n{{}}\n```\n"))
        .collect()
}

#[test]
fn named_fence_parser_is_order_independent() {
    let mut reversed = SCHEMA_BLOCK_NAMES;
    reversed.reverse();
    let blocks = parse_schema_md_json_blocks(&synthetic_schema(&reversed))
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
    println!("===== ColorBarOptions =====\n{}", colorbar_json());
    println!(
        "===== Field render types =====\n{}",
        field_render_types_json()
    );
}
