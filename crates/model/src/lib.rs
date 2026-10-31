#![doc = include_str!("../README.md")]

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
    PixelSpace,
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
pub mod text;
pub mod tick;

// Public API re-exports.
pub use color::Color;
pub use colormap::{ColorMap, LUT_LEN};
pub use config::{
    BarAlign, ColorBarOptions, Config, DataSelectionsConfig, PickedDataRef, PickedPointRef,
    PickedPointsConfig,
};
pub use data::{Column, ColumnId, DataCell};
pub use data_config::{
    DataBarBinStyleConfig, DataBarStyleOverride, DataErrorBarPointStyleConfig,
    DataErrorBarPointStyleOverride, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
    DataScatterPointStyleConfig, DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef,
    LineFill, LineInterpolation, MAX_CONTOUR_LEVELS, PointColorMap, PointSizeMap, ScatterShape,
    SeriesConfig, SeriesRef, SizeScale, VectorColorMap, VectorComponents, VectorPivot, VectorScale,
    VectorStyle,
};
pub use drag::Draggable;
pub use preset::{AxisPreset, ColorCycle};
pub use resize::{Resizable, ResizeHandle};
pub use select::{
    AxisElement, AxisLabelElement, AxisTitleElement, ChartTitleElement, ColorBarAxisElement,
    ColorBarElement, ColorBarLabelElement, ColorBarTitleElement, DataAreaElement, HitId, HitMap,
    LegendElement, OverlayAxisElement, OverlayAxisLabelElement, OverlayAxisTitleElement,
    OverlayLegendElement, Selectable, SelectionBox, SelectionOutline, SubplotElement,
};
pub use text::{MeasureText, TextAlign, TextExtents};

pub use radial::{
    RadialChart, RadialInteraction, RadialKind, RadialLabelFormat, RadialLabels, RadialMaterial,
    RadialOutline, RadialSlice, RadialSplit, RadialStyle, RadialTarget,
};

pub use categorical::*;

pub mod boxplot;
pub use boxplot::*;
pub mod polar;
pub use polar::{AngleUnit, MAX_POLAR_ARC_PIECES, PolarGeometry, PolarInterpolation, PolarOptions};
pub mod radar;
pub use radar::*;

pub mod bounded_title;
pub use bounded_title::BoundedTitleOptions;
