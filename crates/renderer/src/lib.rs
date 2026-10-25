#![doc = include_str!("../README.md")]
#![doc = "\n## First chart: render a PNG\n\nThis is the complete native `line` example. With the `renderer`, `wgpu` and `pollster` dependencies above, copy it into `src/main.rs` and run `cargo run`. Rendering needs a working adapter; building the documentation does not.\n\n```rust,no_run\n"]
#![doc = include_str!("../examples/line.rs")]
#![doc = "\n```\n\nFor embedding, reuse your host's device/queue instead of the example's adapter setup. For editing, picking and exporting the current state, follow the workflow below.\n"]
#![doc = include_str!("../API.md")]

pub mod annotation;
pub mod axis_render;
#[cfg(target_arch = "wasm32")]
mod browser_compile;
pub mod chart;
pub mod data;
pub mod data_render;
mod decoration_render;
pub mod demo;
pub mod error;
mod gpu_compile;
pub mod gpu_contour;
mod gpu_data_pick;
pub mod gpu_errorbar;
pub mod gpu_memory;
mod gpu_pick;
pub mod init;
pub(crate) mod line_step;
pub mod pick;
pub mod radial;
pub mod raster;
mod render_warning;
pub mod renderer;
/// Compile-time WGSL module assembly (`wgsl_module!`) and the table of
/// assembled modules, for the renderer's own tests.
#[doc(hidden)]
pub mod shader_assembly;
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
pub use ::model::bounded_sources::{BoxColumn, BoxField};
pub use ::model::{SeriesUpload, UploadValues};
pub use ::model::{
    bounded_sources, boxplot, categorical, color, colormap, config, data_config, default, drag,
    expand, format, layout, legend, line, preset, resize, select, sink, text, tick,
};

// Public API re-exports.
pub use annotation::{AnnotationMap, BoundedAnnotationFrame, FrameBackground, LayerHit};
pub use chart::{Chart, FitExtent};
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
    DataScatterStyleConfig, ErrorRef, LineFill, LineInterpolation, MAX_CONTOUR_LEVELS,
    ScatterShape, SeriesConfig, SeriesRef,
};
pub use data_render::{
    AllocError, BOX_PLOT_RECORD, ColumnHandle, ColumnId, ColumnPool, DefragPolicy, GpuAllocCtx,
    GpuBudget, GrowthPolicy, MAX_RAGGED_VALUES, MAX_RECORD_FIELDS, RaggedId, RaggedRef, RaggedSlot,
    RaggedSource, RaggedStats, RecordId, RecordRef, RecordSchema, RecordSlot, RecordSource,
    RecordStats, StackExtent,
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
pub use pick::{PickedData, PickedPoint};
pub use preset::{AxisPreset, ColorCycle};
pub use render_warning::{
    AnnotationSkip, RenderWarning, SpacingRejection, SubplotAxis, WarningAxis,
};
pub use renderer::{
    AxisViewState, ChartDrawItem, ChartId, ChartRenderStamp, ChartStyle, ChartView,
    ChartViewLayerStamps, ChartViewState, FitCommitToken, GpuPickRequest, MAX_EXPORT_SCALE,
    MIN_EXPORT_SCALE, PipelinePreparation, PreparationFeature, PreparedFrame, PreparedPipelines,
    RasterImage, RegisteredChartDrawItem, RenderRevision, Renderer, RendererDevice,
    RendererLoadDemo, RendererVisualStamp, Series, SeriesDrawInfo, StreamingOperation,
    StreamingSelectionRequest, StreamingSelectionTicket, WebDerivedSnapshot, WebDerivedStamp,
    WindowedRenderer, clamp_export_scale, display_config_for_surface, dpi_to_scale,
    draw_style_supports, encode_png, fit_display_panel, place_document,
};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, ColorBarAxisElement,
    ColorBarElement, ColorBarLabelElement, ColorBarTitleElement, DataAreaElement, HitId, HitMap,
    LegendElement, OverlayAxisElement, OverlayAxisLabelElement, OverlayAxisTitleElement,
    OverlayLegendElement, Selectable, SelectionBox, SelectionGeometry, SelectionOutline,
    SubplotElement,
};
pub use sink::{DrawnPart, PlacedText, Sink, Slot};
pub use streaming_source::{
    AutoStreamRange, AutoStreamSourceVersion, AutoStreamingProgress, AutoStreamingRangeRequest,
    AutoStreamingRequest, LogicalColumn, RenderInterruptStatus, StreamBounds, StreamColumn,
    StreamEncoding, StreamRangeSourceBinding, StreamReplay, StreamSourceBinding, StreamStatistics,
    StreamingChartOptions, StreamingLimits, StreamingProgress, StreamingState, StreamingStatus,
    StreamingUsage, ViewResidencyStatus,
};
pub use text::{MeasureText, TextAlign};
pub use text_render::{CpuTextMeasure, FontPolicy};

/// The surface material shared by bars, boxes and sectors.
pub use model::Material;
pub use radial::{
    RadialChart, RadialError, RadialFrame, RadialInteraction, RadialKind, RadialLabelFormat,
    RadialLabels, RadialOutline, RadialPickTicket, RadialRenderer, RadialSlice, RadialSplit,
    RadialStyle, RadialTarget,
};
// The deprecated name stays importable from the crate root.
#[allow(deprecated)]
pub use radial::RadialMaterial;

pub use model::categorical::*;

pub use model::boxplot::*;
pub mod radar;
/// Polar coordinates for the Cartesian chart: `PolarOptions` for
/// `Config.polar`, the angle unit and segment interpolation it reads, and the
/// `PolarGeometry` circle the chart draws into.
pub mod polar {
    #![doc = include_str!("../POLAR.md")]
    pub use model::polar::*;
}
pub use model::polar::{
    AngleMap, AngleUnit, GridShape, PolarGeometry, PolarInterpolation, PolarOptions,
};
pub use model::radar::*;
pub use radar::{RadarError, RadarFrame, RadarPart, RadarPick, RadarRenderer};

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/support/gpu.rs"]
mod test_gpu;

pub use model::decoration::{DecorationId, DecorationLayout, DecorationOffset};
pub use model::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationElement, AnnotationHandle,
    AnnotationKind, AnnotationLayer, AnnotationLineStyle, AnnotationSpace, ArrowStyle, AxisMap,
    AxisSpace, BoxStyle, CellRef, ExpandedPanel, Expansion, LayerRef, OverlayLayer, PixelSpace,
    Subplot,
};
pub mod decoration;

mod bounded_title;
pub use model::BoundedTitleOptions;
