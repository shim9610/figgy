pub mod axis_render;
pub mod chart;
pub mod data;
pub mod data_render;
pub mod demo;
pub mod error;
pub mod pick;
pub mod raster;
pub mod renderer;
// Hand-drawn geometry helpers + the `DecoStroker` strategy — consumed by the
// deco layer (`axis_render`) when `Config::draw_style` selects a stylized
// mode (`DrawStyle::Sketch`, …).
mod sketch;
pub mod text_render;

// Model layer (chart option SSoT, data containers, preset policies) lives in
// the sibling `model` crate. Re-exported module-by-module so every path keeps
// its single-crate spelling: `renderer::config::…`, `renderer::layout::…`, etc.
// (`chart` and `data` are renderer modules: dirty-flag tracking and the
// `ColumnSource` upload adapter are render-side machinery.)
pub use ::model::{
    color, config, data_config, default, drag, format, layout, legend, line, preset, resize,
    select, text, tick,
};

// Public API re-exports.
pub use chart::{Chart, FitExtent, errorbar_extent};
pub use color::Color;
pub use config::Config;
pub use data::{Column, ColumnSource};
pub use data_config::{
    DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride, DataErrorBarStyleConfig,
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, ErrorRef, ScatterShape,
    SeriesConfig,
};
pub use data_render::{AllocError, ColumnHandle, ColumnId, ColumnPool, DefragPolicy};
pub use drag::Draggable;
pub use error::{FiggyError, Result};
pub use pick::{PickedPoint, PointColumnLookup, PointPickOptions, pick_nearest_point};
pub use preset::{AxisPreset, ColorCycle};
pub use renderer::{
    ChartDrawItem, ChartStyle, ChartView, MAX_EXPORT_SCALE, MIN_EXPORT_SCALE, PreparedFrame,
    RasterImage, Renderer, RendererDevice, Series, WindowedRenderer, clamp_export_scale,
    dpi_to_scale, encode_png,
};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, DataAreaElement, HitId,
    HitMap, LegendElement, Selectable, SelectionBox,
};
pub use text::MeasureText;
pub use text_render::{CpuTextMeasure, FontPolicy};
