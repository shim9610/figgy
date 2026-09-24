pub mod axis_render;
pub mod chart;
pub mod data;
pub mod data_render;
pub mod demo;
pub mod error;
pub mod gpu_contour;
mod gpu_data_pick;
pub mod gpu_errorbar;
pub mod gpu_memory;
mod gpu_pick;
pub mod init;
pub mod pick;
pub mod raster;
pub mod renderer;
// Hand-drawn geometry helpers + the `DecoStroker` strategy — consumed by the
// deco layer (`axis_render`) when `Config::draw_style` selects a stylized
// mode (`DrawStyle::Sketch`, …).
mod sketch;
mod streaming;
pub mod streaming_source;
mod streaming_upload;
pub mod text_render;
mod time_axis;

// Model layer (chart option SSoT, data containers, preset policies) lives in
// the sibling `model` crate. Re-exported module-by-module so every path keeps
// its single-crate spelling: `renderer::config::…`, `renderer::layout::…`, etc.
// (`chart` and `data` are renderer modules: dirty-flag tracking and the
// `ColumnSource` upload adapter are render-side machinery.)
pub use ::model::{
    color, colormap, config, data_config, default, drag, format, layout, legend, line, preset,
    resize, select, text, tick,
};

// Public API re-exports.
pub use chart::{Chart, FitExtent, errorbar_extent};
pub use color::Color;
pub use colormap::{ColorMap, LUT_LEN};
pub use config::{
    BarAlign, ColorBarOptions, Config, DataSelectionsConfig, PickedDataRef, PickedPointRef,
    PickedPointsConfig,
};
pub use data::{
    Column, ColumnPairWriter, ColumnRangeWriteError, ColumnSource, ColumnUploadStats,
    HiLoColumnSource, StreamColumnSource,
};
pub use data_config::{
    DataBarBinStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, ErrorRef, MAX_CONTOUR_LEVELS, ScatterShape, SeriesConfig,
};
pub use data_render::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, DefragPolicy, GpuAllocCtx, GpuBudget,
    GrowthPolicy,
};
pub use drag::Draggable;
pub use error::{FiggyError, Result};
pub use gpu_data_pick::GpuDataPickTicket;
pub use gpu_errorbar::{
    GpuErrorbarError, GpuErrorbarExtent, GpuErrorbarExtentTicket, GpuFieldExtentMode,
    GpuSeriesExtent, GpuSeriesExtentColumnIds, GpuSeriesExtentMode, GpuSeriesExtentTicket,
    GpuSeriesFitMode,
};
pub use gpu_memory::{
    GpuMemoryUsage, GpuResourceKind, ResidentAdmission, ResidentAdmissionRequest,
    ResidentAdmissionStatus,
};
pub use gpu_pick::{GpuPickError, GpuPickTicket};
pub use init::{INIT_EVENT_SCHEMA_VERSION, InitEvent, InitPhase};
pub use pick::{PickedData, PickedPoint, PointColumnLookup, PointPickOptions, pick_nearest_point};
pub use preset::{AxisPreset, ColorCycle};
pub use renderer::{
    AxisViewState, ChartDrawItem, ChartId, ChartRenderStamp, ChartStyle, ChartView, ChartViewState,
    FitCommitToken, GpuPickRequest, MAX_EXPORT_SCALE, MIN_EXPORT_SCALE, PreparedFrame, RasterImage,
    RegisteredChartDrawItem, RenderRevision, Renderer, RendererDevice, RendererLoadDemo,
    RendererVisualStamp, Series, SeriesDrawInfo, WebDerivedSnapshot, WebDerivedStamp,
    WindowedRenderer, StreamingOperation, StreamingSelectionRequest, StreamingSelectionTicket, clamp_export_scale, display_config_for_surface, dpi_to_scale, encode_png,
    fit_display_panel,
};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, ColorBarAxisElement,
    ColorBarElement, ColorBarLabelElement, ColorBarTitleElement, DataAreaElement, HitId, HitMap,
    LegendElement, Selectable, SelectionBox,
};
pub use streaming_source::{
    AutoStreamRange, AutoStreamSourceVersion, AutoStreamingProgress, AutoStreamingRangeRequest,
    AutoStreamingRequest, LogicalColumn, RenderInterruptStatus, StreamBounds, StreamColumn,
    StreamEncoding, StreamRangeSourceBinding, StreamReplay, StreamSourceBinding, StreamStatistics,
    StreamingChartOptions, StreamingLimits, StreamingProgress, StreamingState, StreamingStatus,
    StreamingUsage, ViewResidencyStatus,
};
pub use text::MeasureText;
pub use text_render::{CpuTextMeasure, FontPolicy};
