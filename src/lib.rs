pub mod axis_render;
pub mod chart;
pub mod color;
pub mod config;
pub mod error;
pub mod format;
pub mod layout;
pub mod line;
pub mod renderer;
pub mod text_render;
pub mod default;
pub mod demo;
pub mod text;
pub mod tick;
pub mod data;
pub mod data_config;
pub mod data_render;

// Public API re-exports.
pub use chart::Chart;
pub use color::Color;
pub use config::Config;
pub use data::{Column, ColumnSource};
pub use data_render::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, DefragPolicy,
};
pub use error::{FiggyError, Result};
pub use data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
    ErrorRef, ScatterShape, SeriesConfig,
};
pub use renderer::{
    clamp_export_scale, dpi_to_scale, encode_png, ChartDrawItem, ChartStyle, ChartView,
    RasterImage, Renderer, Series, WindowedRenderer,
    MAX_EXPORT_SCALE, MIN_EXPORT_SCALE,
};