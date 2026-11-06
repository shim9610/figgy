#![doc = include_str!("../README.md")]
#![doc = "\n## First chart: render a PNG\n\nThis is the complete native `line` example. With the `renderer` and `wgpu` dependencies above, copy it into `src/main.rs` and run `cargo run`. Rendering needs a working adapter; building the documentation does not.\n\n```rust,no_run\n"]
#![doc = include_str!("../examples/line.rs")]
#![doc = "\n```\n\nFor embedding, reuse your host's device/queue instead of the example's adapter setup. For editing, picking and exporting the current state, follow the workflow below.\n"]
#![doc = include_str!("../API.md")]

pub mod annotation;
pub mod axis_render;
#[cfg(target_arch = "wasm32")]
mod browser_compile;
pub mod chart;
pub mod data;
/// The GPU drawing internals: pipelines, uniforms, the column pool's
/// machinery and the per-primitive draws the renderer issues. Public for the
/// renderer's own tests and hosts that study it; nothing here is a stable
/// API (#160). Device and surface setup is [`gpu`]; the column pool types a
/// host names are re-exported at the crate root.
#[doc(hidden)]
pub mod data_render;
mod decoration_render;
/// Sample data for the examples and the browser demo. Not part of the API
/// (#160).
#[doc(hidden)]
pub mod demo;
pub mod error;
mod gpu_compile;

/// GPU device and surface setup: the instance, adapter and device a
/// renderer is built on ([`RendererDevice::new`] takes the device and queue),
/// and the window surface helpers. A host that already owns a device skips
/// all of it.
pub mod gpu {
    pub use crate::data_render::{
        RenderOutcome, configure_surface, create_instance, create_surface_for_window,
        reconfigure_surface, render_clear, request_adapter_async,
        request_adapter_for_surface_async, request_device_async, try_configure_surface,
    };
    #[cfg(not(target_arch = "wasm32"))]
    pub use crate::data_render::{
        request_adapter_blocking, request_adapter_for_surface_blocking, request_device_blocking,
        request_device_with_blocking,
    };
}
/// Contour label placement constants, for the renderer's own tests.
#[doc(hidden)]
pub mod gpu_contour;
mod gpu_data_pick;
pub mod gpu_extent;
pub mod gpu_memory;
mod gpu_pick;
#[cfg(not(target_arch = "wasm32"))]
mod gpu_poller;
mod gpu_readback;
pub mod init;
pub(crate) mod line_step;
pub mod pick;
pub mod radial;
/// The CPU canvas the decoration raster draws on, for the renderer's own
/// tests.
#[doc(hidden)]
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
mod vector_export;
/// Test support: the parts the vector export writes (`cpu-twin` only).
#[cfg(feature = "cpu-twin")]
#[doc(hidden)]
pub use vector_export::{WrittenPart, written_document_parts};

// Model layer (chart option SSoT, data containers, preset policies) lives in
// the sibling `model` crate. Re-exported module-by-module so every path keeps
// its single-crate spelling: `renderer::config::…`, `renderer::layout::…`, etc.
// (`chart` and `data` are renderer modules: dirty-flag tracking and the
// `ColumnSource` upload adapter are render-side machinery.)
pub use ::model::bounded_sources::{BoxColumn, BoxField};
/// The chart chrome's placement, from the model (#160): where every grid
/// line, axis, tick and tick label goes. The renderer's machinery, public
/// for its own tests.
#[doc(hidden)]
pub use ::model::chrome;
pub use ::model::{ConfigError, Coordinates, SeriesUpload, UploadValues};
pub use ::model::{
    bounded_sources, boxplot, categorical, color, colormap, config, coordinates, data_config,
    default, drag, format, layout, legend, line, preset, resize, select, sink, text, tick,
    validate,
};

// Public API re-exports.
pub use annotation::{AnnotationMap, BoundedAnnotationFrame, LayerHit};
pub use chart::{Chart, FitExtent};
pub use color::Color;
pub use colormap::{ColorMap, LUT_LEN};
pub use config::{BarAlign, ColorBarOptions, Config, DataSelectionsConfig, PickedDataRef};
pub use data::{
    Column, ColumnPairWriter, ColumnRangeWriteError, ColumnSource, HiLoColumnSource,
    StreamColumnSource,
};
pub use data_config::{
    DataBarBinStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterStyleConfig, ErrorBars, ErrorRef, FieldSeries, LineFill, LineInterpolation,
    MAX_CONTOUR_LEVELS, RadarRef, RadarSeriesLabels, ScatterShape, SeriesConfig, SeriesRef,
    StreamlineArrows, XySeries,
};
pub use data_render::{
    AllocError, BOX_PLOT_RECORD, ColumnHandle, ColumnId, ColumnPool, DefragPolicy, GpuAllocCtx,
    GrowthPolicy, MAX_RECORD_FIELDS, RaggedId, RaggedRef, RaggedSlot, RaggedSource, RaggedStats,
    RecordId, RecordRef, RecordSchema, RecordSlot, RecordSource, RecordStats, TriangleId,
    TriangleRef, TriangleSlot, TriangleSource, TriangleStats,
};
pub use drag::Draggable;
pub use error::{FiggyError, Result};
pub use gpu_data_pick::GpuDataPickTicket;
pub use gpu_extent::{
    GpuErrorbarExtent, GpuErrorbarExtentTicket, GpuExtentError, GpuFieldExtentMode,
    GpuSeriesExtent, GpuSeriesExtentColumnIds, GpuSeriesExtentMode, GpuSeriesExtentTicket,
    GpuSeriesFitMode,
};
pub use gpu_memory::{
    GpuMemoryUsage, GpuResourceKind, ResidentAdmission, ResidentAdmissionRequest,
    ResidentAdmissionStatus,
};
pub use gpu_pick::{GpuPickError, GpuPickTicket};
pub use gpu_readback::TryResolve;
pub use init::{INIT_EVENT_SCHEMA_VERSION, InitEvent, InitPhase};
pub use pick::{PickedData, PickedPoint};
pub use preset::{AxisPreset, ColorCycle};
pub use render_warning::{
    AnnotationSkip, RenderWarning, SpacingRejection, SubplotAxis, WarningAxis,
};
pub use renderer::{
    AxisViewState, ChartColumnsReplacement, ChartDrawItem, ChartId, ChartRenderStamp, ChartStyle,
    ChartView, ChartViewLayerStamps, ChartViewState, ExportOptions, ExportScope, ExportSource,
    FitCommitToken, GpuPickRequest, MIN_EXPORT_SCALE, PipelinePreparation, PreparationFeature,
    PreparedFrame, PreparedPipelines, RasterImage, RegisteredChartDrawItem, RenderRevision,
    Renderer, RendererDevice, RendererVisualStamp, Series, SeriesDrawInfo, StreamingOperation,
    StreamingSelectionRequest, StreamingSelectionTicket, WindowedRenderer, clamp_export_scale,
    display_config_for_surface, dpi_to_scale, draw_style_supports, encode_png, fit_display_panel,
    place_document,
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

#[allow(deprecated)]
pub use model::categorical::*;

#[allow(deprecated)]
pub use model::boxplot::*;
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
/// Ternary coordinates for the Cartesian chart: `TernaryOptions` for
/// `Config.ternary` and the `TernaryGeometry` triangle the chart draws into.
pub mod ternary {
    #![doc = include_str!("../TERNARY.md")]
    pub use model::ternary::*;
}
pub use model::ternary::{TernaryGeometry, TernaryOptions, TernaryRegionStyle};
/// 3D charts: `View3d` for `Config.view_3d`, the camera a heatmap is drawn
/// through as a surface, and `SurfaceStyle`, the surface's look.
pub mod view3d {
    #![doc = include_str!("../SURFACE.md")]
    pub use model::view3d::*;
}
pub use model::view3d::{SurfaceStyle, View3d};
/// Parallel coordinates for the Cartesian chart (#141): `ParallelOptions`
/// for `Config.parallel`, one `ParallelAxis` range per category of the
/// bottom axis.
pub mod parallel {
    pub use model::parallel::*;
}
pub use model::parallel::{ParallelAxis, ParallelOptions};
/// Energy-level (Grotrian) diagrams (#203): the DataLess
/// [`DataRenderType::EnergyLevels`]
/// series' records, look and placement.
pub mod energy_levels {
    pub use model::energy_levels::*;
}
pub use model::energy_levels::{
    EnergyLevel, EnergyLevelSeries, EnergyLevelStyle, IonizationLimit, LabelPart, LevelRole,
    LevelView, Transition, TransitionDirection,
};
/// Feynman diagrams (#234): a DataLess series kind, re-exported from the
/// model. See [`DataRenderType::Feynman`].
pub mod feynman {
    pub use model::feynman::*;
}
pub use model::feynman::{
    FeynmanDrag, FeynmanLabelPart, FeynmanLayout, FeynmanSeries, FeynmanStyle, FeynmanVertex,
    MomentumArrow, MomentumSide, ParticleLine, Propagator, TimeDirection, VertexMarker, VertexRole,
    Wiggle,
};
/// Radar charts: radar series on a polar chart with a categorical angle
/// axis, and the legacy `RadarChart` document `Config::from_radar` reads.
pub mod radar {
    #![doc = include_str!("../RADAR.md")]
    #[allow(deprecated)]
    pub use model::radar::*;
}
#[allow(deprecated)]
pub use model::radar::*;

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/support/gpu.rs"]
mod test_gpu;

pub use model::decoration::{DecorationId, DecorationLayout, DecorationOffset};
pub use model::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationElement, AnnotationHandle,
    AnnotationKind, AnnotationLayer, AnnotationLineStyle, AnnotationSpace, ArrowStyle, AxisMap,
    AxisSpace, BoxStyle, CellRef, ExpandedPanel, Expansion, LayerRef, OverlayLayer, PixelSpace,
    ShapeStyle, Subplot,
};
pub mod decoration;

pub use model::BoundedTitleOptions;
