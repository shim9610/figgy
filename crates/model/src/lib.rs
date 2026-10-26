#![doc = include_str!("../README.md")]

pub mod bounded_sources;
pub mod categorical;
pub mod color;
pub mod colormap;
pub mod config;
pub mod data;
pub mod data_config;
pub mod decoration;
pub mod default;
pub mod drag;
pub use decoration::{DecorationElement, DecorationId, DecorationLayout, DecorationOffset};
pub mod annotation;
pub mod expand;
pub use expand::{ExpandedPanel, Expansion};
pub mod layer;
pub use annotation::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationElement, AnnotationHandle,
    AnnotationKind, AnnotationLineStyle, AnnotationSpace, ArrowStyle, AxisMap, AxisSpace, BoxStyle,
    MeasuredText, PixelSpace,
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

// Public API re-exports.
pub use bounded_sources::{SeriesUpload, UploadValues};
pub use color::Color;
pub use colormap::{ColorMap, LUT_LEN};
pub use config::{
    BarAlign, ColorBarOptions, Config, DataSelectionsConfig, PickedDataRef, PickedPointRef,
    PickedPointsConfig,
};
pub use data::{Column, ColumnId, DataCell};
pub use data_config::{
    BarGroups, BoxPlotRef, BoxPlotSeriesLabels, CategoryBarSeriesLabels, CategoryBarsRef,
    DataBarBinStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterPointStyleConfig, DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef,
    LineFill, LineInterpolation, LineOffsets, MAX_CONTOUR_LEVELS, MAX_OFFSET_LINES, PointColorMap,
    PointSizeMap, RadarRef, RadarSeriesLabels, ScatterShape, SeriesConfig, SeriesRef, SizeScale,
    StreamlineArrows, StreamlinesRef, VectorColorMap, VectorComponents, VectorPivot, VectorScale,
    VectorStyle,
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
pub mod polar;
pub use polar::{
    AngleMap, AngleUnit, GridShape, MAX_POLAR_ARC_PIECES, PolarGeometry, PolarInterpolation,
    PolarOptions,
};
pub mod radar;
#[allow(deprecated)]
pub use radar::*;

pub mod bounded_title;
pub use bounded_title::BoundedTitleOptions;
