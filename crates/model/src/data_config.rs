//! Declarative series configuration types.
//!
//! Mapping layer between chart layout (`config::Config`) and raw data
//! (`data::DataCell`). Each `SeriesConfig` says which columns are X/Y/error
//! and which render type / style to use.

use crate::color::Color;
use crate::data::ColumnId;
use crate::line::LineStylePreset;
use crate::text::RichText;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataConfig {
    pub data_id: String,
}

/// One series — which columns to draw, with which render type and style.
///
/// Columns are referenced by the id used when registering them with the
/// `ColumnPool`. `Renderer::paint` resolves them to byte ranges every frame
/// via `pool.handle_for(id)`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeriesConfig {
    pub series_id: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub source_id: Option<String>,
    pub label: Option<RichText>,
    /// X column id (must match the id passed to `add_column`).
    pub x_column: ColumnId,
    /// Y column id.
    pub y_column: ColumnId,
    pub render_type: DataRenderType,
}

/// Errorbar column reference: either a single column read as ±σ, or two
/// separate columns for the lower / upper bound.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ErrorRef {
    /// Column value is interpreted as ±σ (point ± column_value).
    Symmetric { column: ColumnId },
    /// Separate lower / upper columns (point − lower, point + upper).
    Asymmetric { lower: ColumnId, upper: ColumnId },
}

/// Series render type. Each variant maps to one independent draw path.
///
/// Combinations are kept explicit (instead of optional fields on a single
/// struct) so the renderer can pick its primitives with one `match`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataRenderType {
    Scatter {
        scatter: DataScatterStyleConfig,
    },
    Line {
        line: DataLineStyleConfig,
    },
    ScatterLine {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
    },
    ScatterErrorbarX {
        scatter: DataScatterStyleConfig,
        err_x: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
    ScatterErrorbarY {
        scatter: DataScatterStyleConfig,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
    ScatterErrorbarXY {
        scatter: DataScatterStyleConfig,
        err_x: ErrorRef,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
    LineScatterErrorbarX {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_x: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
    LineScatterErrorbarY {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
    LineScatterErrorbarXY {
        scatter: DataScatterStyleConfig,
        line: DataLineStyleConfig,
        err_x: ErrorRef,
        err_y: ErrorRef,
        err_style: DataErrorBarStyleConfig,
    },
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataLineStyleConfig {
    pub line_style: LineStylePreset,
    pub line_color: Color,
    pub line_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterStyleConfig {
    pub point_color: Color,
    pub point_shape: ScatterShape,
    pub point_size: f32,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_style_table: Option<Vec<DataScatterPointStyleConfig>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_style_index_column: Option<ColumnId>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_style_overrides: Option<Vec<DataScatterPointStyleOverride>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterPointStyleConfig {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_shape: Option<ScatterShape>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_size: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterPointStyleOverride {
    pub index: usize,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub style: DataScatterPointStyleConfig,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataErrorBarStyleConfig {
    pub error_bar_color: Color,
    pub error_bar_width: f32,
    pub error_bar_cap_size: f32,
    pub cap_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ScatterShape {
    Circle,
    Square,
    Triangle,
    Diamond,
    Cross,
    CircleFilled,
    SquareFilled,
    TriangleFilled,
    DiamondFilled,
    TriangleDown,
    TriangleLeft,
    TriangleRight,
    Plus,
    Pentagon,
    Hexagon,
    Octagon,
    Star,
    TriangleDownFilled,
    TriangleLeftFilled,
    TriangleRightFilled,
    PlusFilled,
    CrossFilled,
    PentagonFilled,
    HexagonFilled,
    OctagonFilled,
    StarFilled,
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::{
        DataRenderType, DataScatterPointStyleConfig, DataScatterPointStyleOverride,
        DataScatterStyleConfig, ScatterShape, SeriesConfig,
    };
    use crate::color::Color;

    fn base_series_json() -> serde_json::Value {
        serde_json::json!({
            "series_id": "s",
            "label": null,
            "x_column": "x",
            "y_column": "y",
            "render_type": {
                "Scatter": {
                    "scatter": {
                        "point_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
                        "point_shape": "Circle",
                        "point_size": 3.0
                    }
                }
            }
        })
    }

    #[test]
    fn old_series_json_without_additive_fields_still_parses() {
        let cfg: SeriesConfig =
            serde_json::from_value(base_series_json()).expect("old series shape parses");
        assert_eq!(cfg.source_id, None);
        let DataRenderType::Scatter { scatter } = &cfg.render_type else {
            panic!("expected scatter");
        };
        assert_eq!(scatter.point_style_table, None);
        assert_eq!(scatter.point_style_index_column, None);
        assert_eq!(scatter.point_style_overrides, None);

        let json = serde_json::to_value(cfg).expect("serialize");
        assert!(json.get("source_id").is_none());
        let scatter = &json["render_type"]["Scatter"]["scatter"];
        assert!(scatter.get("point_style_table").is_none());
        assert!(scatter.get("point_style_index_column").is_none());
        assert!(scatter.get("point_style_overrides").is_none());
    }

    #[test]
    fn point_style_mapping_round_trips_with_flattened_override() {
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 3.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: Some(Color::from_rgb8(255, 0, 0)),
                    point_shape: None,
                    point_size: Some(5.0),
                },
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::DiamondFilled),
                    point_size: None,
                },
            ]),
            point_style_index_column: Some("style_idx".into()),
            point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                index: 7,
                style: DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::StarFilled),
                    point_size: Some(9.0),
                },
            }]),
        };

        let json = serde_json::to_value(&scatter).expect("serialize scatter style");
        assert_eq!(json["point_style_index_column"], "style_idx");
        assert_eq!(json["point_style_overrides"][0]["index"], 7);
        assert_eq!(
            json["point_style_overrides"][0]["point_shape"],
            "StarFilled"
        );
        assert_eq!(json["point_style_overrides"][0]["point_size"], 9.0);
        assert!(json["point_style_overrides"][0].get("style").is_none());

        let back: DataScatterStyleConfig =
            serde_json::from_value(json).expect("parse scatter style");
        assert_eq!(back, scatter);
    }
}
