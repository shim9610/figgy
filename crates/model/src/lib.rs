//! figgy model layer — the chart option SSoT (`Config`, `SeriesConfig`), data
//! container definitions (`Column`, `DataCell`), and preset policies
//! (`default_config`, tick / label / layout rules).
//!
//! No GPU, raster, or windowing dependencies — and no render-side machinery:
//! dirty-flag tracking (`Chart`) and the GPU upload adapter (`ColumnSource`)
//! are renderer optimizations and live in the renderer crate, which depends
//! on this one and re-exports these modules under the same paths.

pub mod color;
pub mod config;
pub mod data;
pub mod data_config;
pub mod default;
pub mod drag;
pub mod format;
pub mod layout;
pub mod legend;
pub mod line;
pub mod preset;
pub mod resize;
pub mod select;
pub mod text;
pub mod tick;

// Public API re-exports.
pub use color::Color;
pub use config::{Config, PickedPointRef, PickedPointsConfig};
pub use data::{Column, ColumnId, DataCell};
pub use data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
    DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef, ScatterShape, SeriesConfig,
};
pub use drag::Draggable;
pub use preset::{AxisPreset, ColorCycle};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, DataAreaElement, HitId,
    HitMap, LegendElement, Selectable, SelectionBox,
};
pub use text::{MeasureText, TextExtents};
