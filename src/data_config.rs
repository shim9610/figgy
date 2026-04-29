//! Declarative series configuration types.
//!
//! Mapping layer between chart layout (`config::Config`) and raw data
//! (`data::DataCell`). Each `SeriesConfig` says which columns are X/Y/error
//! and which render type / style to use.

use crate::color::Color;
use crate::data_render::ColumnId;
use crate::line::LineStylePreset;
use crate::text::RichText;

#[derive(Debug, Clone, PartialEq)]
pub struct DataConfig {
    pub data_id: String,
}

/// One series — which columns to draw, with which render type and style.
///
/// Columns are referenced by the id used when registering them with the
/// `ColumnPool`. `Renderer::paint` resolves them to byte ranges every frame
/// via `pool.handle_for(id)`.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesConfig {
    pub series_id: String,
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
pub struct DataLineStyleConfig {
    pub line_style: LineStylePreset,
    pub line_color: Color,
    pub line_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataScatterStyleConfig {
    pub point_color: Color,
    pub point_shape: ScatterShape,
    pub point_size: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataErrorBarStyleConfig {
    pub error_bar_color: Color,
    pub error_bar_width: f32,
    pub error_bar_cap_size: f32,
    pub cap_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
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
}
