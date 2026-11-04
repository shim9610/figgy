#![doc = include_str!("../README.md")]

pub mod bounded_sources;
pub mod categorical;
/// Where the chart's chrome goes: the grid, axes, ticks and tick labels,
/// placed through a [`sink::Sink`] (#160). The renderer's machinery, not a
/// host API.
#[doc(hidden)]
pub mod chrome;
pub mod color;
pub mod colormap;
pub mod config;
pub mod coordinates;
pub use coordinates::Coordinates;
pub mod data;
pub mod data_config;
pub mod decoration;
pub mod default;
pub mod drag;
pub use decoration::{DecorationElement, DecorationId, DecorationLayout, DecorationOffset};
pub mod annotation;
/// How a document expands into the panels the renderer draws (overlays,
/// subplots, line offsets). Public because the renderer crate reads it; it
/// is the renderer's machinery, not a host API, and carries no
/// compatibility promise (#160).
#[doc(hidden)]
pub mod expand;
#[doc(hidden)]
pub use expand::{ExpandedPanel, Expansion};
pub mod layer;
pub use annotation::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationElement, AnnotationHandle,
    AnnotationKind, AnnotationLineStyle, AnnotationSpace, ArrowStyle, AxisMap, AxisSpace, BoxStyle,
    MeasuredText, PixelSpace, ShapeStyle,
};
pub use layer::{AnnotationLayer, CellRef, LayerRef, OverlayLayer, Subplot};
pub mod format;
pub mod layout;
pub mod legend;
pub mod line;
pub mod preset;
pub mod radial;
pub mod resize;
pub mod select;
pub mod sink;
pub mod text;
pub mod tick;
pub mod validate;
pub use validate::ConfigError;

// Public API re-exports.
pub use bounded_sources::{SeriesUpload, UploadValues};
pub use color::Color;
pub use colormap::{ColorMap, LUT_LEN};
pub use config::{
    AxisBreak, AxisBreakStyle, AxisBreaks, BarAlign, BreakMark, ColorBarOptions, Config,
    DataSelectionsConfig, MAX_AXIS_BREAK_GAP, MAX_AXIS_BREAKS, PickedDataRef,
};
pub use data::{Column, ColumnId};
pub use data_config::{
    BarGroups, BoxPlotRef, BoxPlotSeriesLabels, CategoryBarSeriesLabels, CategoryBarsRef,
    DataBarBinStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterPointStyleConfig, DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorBars,
    ErrorRef, FieldSeries, LineFill, LineInterpolation, LineOffsets, MAX_CONTOUR_LEVELS,
    PointColorMap, PointSizeMap, RadarRef, RadarSeriesLabels, ScatterShape, SeriesConfig,
    SeriesRef, SizeScale, StreamlineArrows, StreamlinesRef, VectorColorMap, VectorComponents,
    VectorPivot, VectorScale, VectorStyle, XySeries,
};
pub use drag::Draggable;
pub use preset::{AxisPreset, ColorCycle};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, ColorBarAxisElement,
    ColorBarElement, ColorBarLabelElement, ColorBarTitleElement, DataAreaElement, HitId, HitMap,
    LegendElement, OverlayAxisElement, OverlayAxisLabelElement, OverlayAxisTitleElement,
    OverlayLegendElement, Selectable, SelectionBox, SelectionGeometry, SelectionOutline,
    SubplotElement,
};
pub use sink::{DrawnPart, PlacedText, Sink, Slot};
pub use text::{MeasureText, TextAlign, TextExtents};

pub mod material;
pub use material::Material;

pub use radial::{
    RadialChart, RadialInteraction, RadialKind, RadialLabelFormat, RadialLabels, RadialOutline,
    RadialSlice, RadialSplit, RadialStyle, RadialTarget,
};
// The deprecated name stays importable from the crate root.
#[allow(deprecated)]
pub use radial::RadialMaterial;

#[allow(deprecated)]
pub use categorical::*;

pub mod boxplot;
#[allow(deprecated)]
pub use boxplot::*;
pub mod energy_levels;
pub mod parallel;
pub mod polar;
pub mod ternary;
pub mod view3d;
pub use energy_levels::{
    EnergyLevel, EnergyLevelSeries, EnergyLevelStyle, IonizationLimit, LabelPart, LevelLabelSide,
    LevelRole, LevelStrokeStyle, LevelView, Transition, TransitionArrowStyle, TransitionDirection,
};
pub use parallel::{ParallelAxis, ParallelOptions};
pub use polar::{
    AngleMap, AngleUnit, GridShape, MAX_POLAR_ARC_PIECES, PolarGeometry, PolarInterpolation,
    PolarOptions,
};
pub mod radar;
#[allow(deprecated)]
pub use radar::*;

pub mod bounded_title;
pub use bounded_title::BoundedTitleOptions;
