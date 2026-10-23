//! Declarative series configuration types.
//!
//! Mapping layer between chart layout (`config::Config`) and raw data
//! (`data::DataCell`). Each `SeriesConfig` says which columns are X/Y/error
//! and which render type / style to use.

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale};
use crate::data::ColumnId;
use crate::format::LabelFormat;
use crate::line::LineStylePreset;
use crate::text::RichText;
use crate::tick::TickError;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataConfig {
    pub data_id: String,
}

/// One series — which columns to draw, with which render type and style.
///
/// Columns are referenced by the id used when registering them with the
/// `ColumnPool`. `Renderer::prepare` resolves them to allocation-stamped GPU
/// handles; the resulting `PreparedFrame` owns those handles for immutable
/// paint recording.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeriesConfig {
    pub series_id: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub source_id: Option<String>,
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Bars from pre-binned data. Binning is the host's job: the renderer is
    /// handed `(edges, counts)` and draws them.
    ///
    /// `bar.orientation` alone decides which column is which — `Vertical` reads
    /// `x_column` as edges and `y_column` as counts, `Horizontal` the reverse.
    /// The lengths are not consulted to guess the roles.
    Histogram {
        bar: DataBarStyleConfig,
    },
    /// A filled field and nothing else. `x_column` / `y_column` are the grid
    /// coordinates; `matrix` names the columns that make up z.
    Heatmap {
        matrix: MatrixRef,
        fill: FieldFillConfig,
    },
    /// Contour lines only — the field is not filled.
    Contour {
        matrix: MatrixRef,
        contour: ContourConfig,
    },
    /// Filled field with contour lines drawn over it.
    HeatmapContour {
        matrix: MatrixRef,
        fill: FieldFillConfig,
        contour: ContourConfig,
    },
    /// A vector field (quiver plot): one arrow per sample, with the direction
    /// and length given by the host's `u` / `v` components.
    ///
    /// The renderer draws the vectors it is given. It does not derive them
    /// from a scalar field and it does not reduce them to a scalar: colouring
    /// by magnitude takes a host-supplied value column or grid through
    /// [`VectorStyle::color_map`].
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. Polar charts (`Config.polar`) reject vector series.
    Vector {
        components: VectorComponents,
        style: VectorStyle,
    },
}

/// Where a vector series' components come from.
///
/// The anchors are the series' `x_column` / `y_column` in both forms. The grid
/// form shares a heatmap's coordinate columns, orientation and layout, so
/// arrows over a field are one more series naming the same columns.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorComponents {
    /// `u` and `v` are index-aligned with the anchor columns. The draw count
    /// is the shortest of the four columns, or of five with a
    /// [`VectorColorMap::Column`]; nothing is length-inferred.
    Points {
        /// The x component column.
        u: ColumnId,
        /// The y component column.
        v: ColumnId,
    },
    /// `u` and `v` are grids over the coordinate columns, both with the same
    /// orientation and layout. With `Centers` the arrow sits on the
    /// coordinate; with `Edges` it sits at the cell centre, where a heatmap
    /// cell's value sits.
    Grid {
        /// The x component grid.
        u: MatrixRef,
        /// The y component grid.
        v: MatrixRef,
    },
}

impl VectorComponents {
    /// The grid declarations of the grid form.
    pub fn grid(&self) -> Option<(&MatrixRef, &MatrixRef)> {
        match self {
            Self::Grid { u, v } => Some((u, v)),
            Self::Points { .. } => None,
        }
    }
}

/// How an arrow's length is scaled.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorScale {
    /// The tip is `origin + factor * vector` in data units. Meaningful when
    /// both axes share a unit; the arrows zoom with the data.
    Data {
        /// Data units of tip offset per unit of vector magnitude.
        factor: f64,
    },
    /// The screen length is `px_per_unit * |vector|` logical pixels along the
    /// data-space direction projected through the live transform, so arrows
    /// keep their length under zoom.
    Pixels {
        /// Logical pixels of arrow length per unit of vector magnitude.
        px_per_unit: f64,
    },
}

impl Default for VectorScale {
    fn default() -> Self {
        Self::Pixels { px_per_unit: 1.0 }
    }
}

/// Which point of the arrow sits on the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorPivot {
    /// The tail sits on the anchor (the default).
    #[default]
    Tail,
    /// The arrow is centred on the anchor.
    Middle,
    /// The tip sits on the anchor.
    Tip,
}

/// A value mapped to arrow colour through `Config.colorbar`, in the same form
/// as the components: a column for [`VectorComponents::Points`], a grid for
/// [`VectorComponents::Grid`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorColorMap {
    /// A value column index-aligned with the anchors.
    Column(ColumnId),
    /// A value grid over the coordinate columns.
    Grid(MatrixRef),
}

/// Arrow appearance of a vector series. Pixel sizes are logical pixels, like
/// `point_size`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectorStyle {
    /// The arrow colour when `color_map` is `None`.
    pub color: Color,
    /// Shaft thickness.
    pub width_px: f32,
    /// How the vector's magnitude becomes a length on screen.
    pub scale: VectorScale,
    /// Which point of the arrow sits on the anchor.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pivot: VectorPivot,
    /// Head length along the shaft. An arrow shorter than its head scales the
    /// head down with it.
    pub head_length_px: f32,
    /// Full head width across the shaft.
    pub head_width_px: f32,
    /// Arrows shorter than this on screen are not drawn.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "VectorStyle::is_zero")
    )]
    pub min_length_px: f32,
    /// Colour each arrow by a value through `Config.colorbar`; the colourbar
    /// is the only authority on the range, scale, ramp and `nan_color`, as for
    /// a heatmap. A series with a colour map and no colourbar is rejected.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub color_map: Option<VectorColorMap>,
}

impl VectorStyle {
    /// The validation reason for a non-finite or negative pixel size.
    pub const INVALID_PIXELS_REASON: &'static str = "vector width_px, head_length_px, head_width_px and min_length_px must be finite and non-negative";
    /// The validation reason for a non-finite or zero scale.
    pub const INVALID_SCALE_REASON: &'static str = "vector scale must be finite and non-zero";

    #[cfg(feature = "serde")]
    fn is_zero(value: &f32) -> bool {
        *value == 0.0
    }

    /// Validate invariants owned by the style itself.
    pub fn validate(&self) -> Result<(), &'static str> {
        let pixels = [
            self.width_px,
            self.head_length_px,
            self.head_width_px,
            self.min_length_px,
        ];
        if pixels.iter().any(|px| !px.is_finite() || *px < 0.0) {
            return Err(Self::INVALID_PIXELS_REASON);
        }
        let scale = match self.scale {
            VectorScale::Data { factor } => factor,
            VectorScale::Pixels { px_per_unit } => px_per_unit,
        };
        if !scale.is_finite() || scale == 0.0 {
            return Err(Self::INVALID_SCALE_REASON);
        }
        Ok(())
    }

    /// Set the scale so the longest vector the bounds allow,
    /// `(max |u|, max |v|)` over `u_bounds` × `v_bounds`, draws `target` long:
    /// logical pixels for [`VectorScale::Pixels`], data units for
    /// [`VectorScale::Data`]. No arrow is longer than `target`; the longest
    /// drawn arrow is shorter when the largest `|u|` and `|v|` come from
    /// different samples. The bounds are the component columns' statistics,
    /// which the host already has.
    ///
    /// A display helper, like `PointSizeMap::fit_domain`: it writes only the
    /// scale value. A non-finite or zero extent, or a `target` that is not
    /// finite and positive, leaves the style unchanged and returns `false`.
    pub fn fit_scale(&mut self, u_bounds: (f64, f64), v_bounds: (f64, f64), target: f64) -> bool {
        let reach = |(min, max): (f64, f64)| {
            if !min.is_finite() || !max.is_finite() {
                return f64::NAN;
            }
            min.abs().max(max.abs())
        };
        let longest = reach(u_bounds).hypot(reach(v_bounds));
        if !longest.is_finite() || longest <= 0.0 || !target.is_finite() || target <= 0.0 {
            return false;
        }
        match &mut self.scale {
            VectorScale::Data { factor } => *factor = target / longest,
            VectorScale::Pixels { px_per_unit } => *px_per_unit = target / longest,
        }
        true
    }
}

/// Identifier of another series in the same chart: its `series_id`. Plain
/// `String`, like [`ColumnId`] — resolution against the chart's series list is
/// the renderer's job.
pub type SeriesRef = String;

/// What a line series fills: the region between the line and a baseline or
/// between the line and another series' line.
///
/// The fill is a property of the line. It picks as part of the line (a hit
/// inside it names the filled segment's nearer sample, and it loses every tie
/// to a stroke, marker or error bar), selecting the line selects its fill,
/// and it is drawn before every series body, in series order, so another
/// series' fill can never cover this series' line. Fills composite over each other in series order; there is no
/// automatic sorting.
///
/// A fill assumes an x-monotonic series. A series whose x runs back on itself
/// produces trapezoids that overlap and blend twice; the renderer documents
/// and tests that outcome rather than masking it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineFill {
    /// Fill between the line and a constant y value in data units. `0.0` is
    /// the usual area chart. On a logarithmic y axis a baseline at or below
    /// zero has no position and is clamped to the axis minimum instead of
    /// masking the whole area — an explicit exception to the rule that
    /// nonpositive samples on a logarithmic axis are omitted.
    ToBaseline(f64),
    /// Fill between this line and another series' line, index-aligned: value
    /// `i` of this series pairs with value `i` of the referenced series, the
    /// way an errorbar reads its extra columns. Both series must share the
    /// same x column; a different x column is an explicit error, because
    /// resampling would be renderer-derived data. Both series' columns must
    /// be GPU resident; streamed sources are rejected.
    ToSeries(SeriesRef),
}

impl Default for LineFill {
    /// `ToBaseline(0.0)`.
    fn default() -> Self {
        Self::ToBaseline(0.0)
    }
}

/// How consecutive samples of a line series are connected.
///
/// A step mode draws every segment as axis-aligned pieces instead of one
/// straight segment. Dash phase, picking and the arc-length scan all follow the
/// stepped path, so a dashed step line keeps its pattern around every corner
/// and a pick on a riser still resolves to one of the two source indices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineInterpolation {
    /// Straight segments between consecutive samples.
    #[default]
    Linear,
    /// The value changes at the *start* of each segment: a vertical riser at
    /// `x[i]` up to `y[i + 1]`, then a horizontal run to `x[i + 1]`.
    StepPre,
    /// The value is held until the *end* of each segment: a horizontal run at
    /// `y[i]` to `x[i + 1]`, then a vertical riser to `y[i + 1]`.
    StepPost,
    /// The value changes halfway: a horizontal run at `y[i]` to the midpoint
    /// of `x[i]` and `x[i + 1]` (in data units), a riser there, then a
    /// horizontal run at `y[i + 1]`.
    StepMid,
}

impl LineInterpolation {
    /// `true` for [`Self::Linear`]. The serde default predicate: the field is
    /// omitted from JSON at its default so existing documents round-trip
    /// byte-identically.
    pub fn is_linear(&self) -> bool {
        matches!(self, Self::Linear)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataLineStyleConfig {
    pub line_style: LineStylePreset,
    pub line_color: Color,
    pub line_width: f32,
    /// Area under the line or between two lines. `None` draws the line alone.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub fill: Option<LineFill>,
    /// Fill colour. `None` derives it from `line_color`: the same hue with
    /// the alpha multiplied by [`Self::DEFAULT_FILL_ALPHA`]; see
    /// [`Self::effective_fill_color`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub fill_color: Option<Color>,
    /// How consecutive samples are connected. `Linear` by default and omitted
    /// from JSON at that default.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "LineInterpolation::is_linear")
    )]
    pub interpolation: LineInterpolation,
}

impl Default for DataLineStyleConfig {
    /// A solid, opaque black, 1 px line with no fill and linear interpolation.
    fn default() -> Self {
        Self {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: crate::data_config::LineInterpolation::Linear,
        }
    }
}

impl DataLineStyleConfig {
    /// Alpha multiplier applied to `line_color` when `fill_color` is `None`.
    pub const DEFAULT_FILL_ALPHA: f32 = 0.25;

    /// The colour the fill is painted with: `fill_color`, or `line_color` with
    /// its alpha multiplied by [`Self::DEFAULT_FILL_ALPHA`]. Meaningful only
    /// when `fill` is `Some`; the legend band and the renderer both read it.
    pub fn effective_fill_color(&self) -> Color {
        self.fill_color.unwrap_or(Color {
            a: self.line_color.a * Self::DEFAULT_FILL_ALPHA,
            ..self.line_color
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterStyleConfig {
    pub point_color: Color,
    pub point_shape: ScatterShape,
    pub point_size: f32,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    /// Per-slot colour, shape and size overrides, selected per point by
    /// `point_style_index_column`.
    pub point_style_table: Option<Vec<DataScatterPointStyleConfig>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    /// A column of slot indices into `point_style_table`, index-aligned
    /// with the series' x/y columns.
    pub point_style_index_column: Option<ColumnId>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    /// Sparse per-index overrides that win over the table and the maps.
    pub point_style_overrides: Option<Vec<DataScatterPointStyleOverride>>,
    /// Color each point by a value column through `Config.colorbar`.
    ///
    /// The colorbar is the only authority on the range, scale, ramp and
    /// `nan_color`, exactly as for heatmaps; a series with this map and no
    /// colorbar is rejected rather than given an invented range. The value
    /// column must be GPU resident; a streamed source is rejected.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_color_map: Option<PointColorMap>,
    /// Size each point by a value column (a bubble chart). The value column
    /// must be GPU resident; a streamed source is rejected.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_size_map: Option<PointSizeMap>,
}

/// A value column mapped to point color through the chart's colorbar.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PointColorMap {
    /// Index-aligned with the series' x/y columns; never length-inferred.
    pub column: ColumnId,
}

/// How a size value becomes a marker radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SizeScale {
    /// The value is proportional to marker area (the radius grows with its
    /// square root), which is how bubble sizes are read.
    #[default]
    Area,
    /// The value is proportional to the radius.
    Radius,
}

/// A value column mapped to the marker radius.
///
/// `domain_min..domain_max` maps onto `min_px..max_px` (logical pixels, the
/// unit of `point_size`); values outside the domain clamp to the ends. The
/// domain is explicit: the renderer never infers it from the column, so
/// appending a point never resizes the others. [`Self::fit_domain`] fills it
/// from statistics the host already has.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PointSizeMap {
    /// Index-aligned with the series' x/y columns; never length-inferred.
    pub column: ColumnId,
    /// The value mapped to `min_px`; smaller values clamp to it.
    pub domain_min: f64,
    /// The value mapped to `max_px`; larger values clamp to it.
    pub domain_max: f64,
    /// The marker radius at `domain_min`, in logical pixels.
    pub min_px: f32,
    /// The marker radius at `domain_max`, in logical pixels.
    pub max_px: f32,
    /// Whether the value is proportional to marker area or radius.
    #[cfg_attr(feature = "serde", serde(default))]
    pub scale: SizeScale,
}

impl PointSizeMap {
    /// The validation reason for a non-finite or empty domain.
    pub const INVALID_DOMAIN_REASON: &'static str =
        "point size map domain must be finite with domain_min < domain_max";
    /// The validation reason for a non-finite, negative or inverted pixel range.
    pub const INVALID_PIXELS_REASON: &'static str =
        "point size map min_px and max_px must be finite, non-negative and min_px <= max_px";

    /// Validate invariants owned by the map itself.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.domain_min.is_finite()
            || !self.domain_max.is_finite()
            || self.domain_min >= self.domain_max
        {
            return Err(Self::INVALID_DOMAIN_REASON);
        }
        if !self.min_px.is_finite()
            || !self.max_px.is_finite()
            || self.min_px < 0.0
            || self.min_px > self.max_px
        {
            return Err(Self::INVALID_PIXELS_REASON);
        }
        Ok(())
    }

    /// Set the domain to `min..max`, typically a column's statistics.
    ///
    /// A display helper, like `AxisOptions::auto_ticks`: it only writes the
    /// two bounds. A non-finite or empty range leaves the map unchanged and
    /// returns `false`.
    pub fn fit_domain(&mut self, min: f64, max: f64) -> bool {
        if !min.is_finite() || !max.is_finite() || min >= max {
            return false;
        }
        self.domain_min = min;
        self.domain_max = max;
        true
    }

    /// The marker radius in logical pixels for `value`, or `None` when the
    /// value is not finite (the point is hidden). The shader and CPU picking
    /// both follow this formula.
    pub fn radius_px(&self, value: f64) -> Option<f32> {
        if !value.is_finite() {
            return None;
        }
        let span = self.domain_max - self.domain_min;
        let t = ((value - self.domain_min) / span).clamp(0.0, 1.0) as f32;
        let (lo, hi) = (self.min_px, self.max_px);
        Some(match self.scale {
            SizeScale::Radius => lo + (hi - lo) * t,
            SizeScale::Area => (lo * lo + (hi * hi - lo * lo) * t).sqrt(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterPointStyleConfig {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_shape: Option<ScatterShape>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub point_size: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataScatterPointStyleOverride {
    pub index: usize,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub style: DataScatterPointStyleConfig,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataErrorBarPointStyleConfig {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_width: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_cap_size: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub cap_width: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataErrorBarPointStyleOverride {
    pub index: usize,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub style: DataErrorBarPointStyleConfig,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataErrorBarStyleConfig {
    pub error_bar_color: Color,
    pub error_bar_width: f32,
    pub error_bar_cap_size: f32,
    pub cap_width: f32,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_style_table: Option<Vec<DataErrorBarPointStyleConfig>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_style_index_column: Option<ColumnId>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub error_bar_style_overrides: Option<Vec<DataErrorBarPointStyleOverride>>,
}

/// A 2-D grid declared as a bundle of column ids — and nothing more.
///
/// There is no separate matrix container, no renderer-side registry, and no new
/// data trait. The grid *is* those columns in the pool; this says which ones and
/// how to read them. Keeping the declaration inline in the render type is what
/// lets `Config` + `series` fully define the picture: an id pointing at
/// something the renderer holds privately would not.
///
/// z statistics come from the constituent columns' own cached `min` / `max` /
/// `min_positive`, combined. No GPU round trip, no second extent engine.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MatrixRef {
    /// The grid's constituent columns, in grid order.
    pub columns: Vec<ColumnId>,
    /// Whether each constituent column is a slice along x or along y.
    pub orientation: MatrixOrientation,
    /// Whether the coordinate columns hold cell boundaries (n + 1 values) or
    /// cell centres (n). Never inferred from the lengths.
    pub grid_layout: GridLayout,
}

/// Which axis a constituent column runs along.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MatrixOrientation {
    /// Each column is one x position, holding that position's y values.
    ColumnsAreX,
    /// Each column is one y position, holding that position's x values.
    ColumnsAreY,
}

/// Whether coordinate columns give cell edges or cell centres.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridLayout {
    /// `n + 1` coordinates bounding `n` cells.
    Edges,
    /// `n` coordinates at the middle of `n` cells.
    Centers,
}

impl MatrixRef {
    /// Cells along each axis, given the coordinate column lengths.
    ///
    /// Returns `(cols, rows)` in constituent-column order: `cols` is how many
    /// constituent columns are usable, `rows` how deep into each of them to
    /// read. A declaration that does not line up with the data is **not an
    /// error** — the smallest common extent is drawn and the caller reports the
    /// truncation. Refusing to draw would turn a data-shape surprise into a
    /// blank chart.
    pub fn effective_extent(
        &self,
        x_len: usize,
        y_len: usize,
        shortest_column_len: usize,
    ) -> (usize, usize) {
        let (along, across) = self.coordinate_cells(x_len, y_len);
        (
            self.columns.len().min(along),
            shortest_column_len.min(across),
        )
    }

    /// Cells the **coordinate columns alone** can bound, in constituent-column
    /// order: `(along, across)`.
    ///
    /// The half of [`Self::effective_extent`] that does not depend on the data,
    /// split out because a caller reporting truncation has to know what the
    /// coordinates offered before the grid columns capped it.
    pub fn coordinate_cells(&self, x_len: usize, y_len: usize) -> (usize, usize) {
        let (along, across) = match self.orientation {
            MatrixOrientation::ColumnsAreX => (x_len, y_len),
            MatrixOrientation::ColumnsAreY => (y_len, x_len),
        };
        let cells = |coordinates: usize| match self.grid_layout {
            GridLayout::Edges => coordinates.saturating_sub(1),
            GridLayout::Centers => coordinates,
        };
        (cells(along), cells(across))
    }
}

/// How a field's cells are painted.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldFillConfig {
    pub mode: FillMode,
    pub shading: Shading,
    /// Multiplied into the colormap's alpha — the way to put contour lines over
    /// a muted field without editing the ramp.
    pub opacity: f32,
}

/// Continuous ramp, or quantized into the contour levels' bands.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FillMode {
    Continuous,
    /// One flat colour per interval between contour levels. With no levels this
    /// is a single band, i.e. a flat field.
    Bands,
}

/// Whether a cell is one colour or interpolated across its corners.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shading {
    /// One colour per cell, from that cell's own z.
    Flat,
    /// Bilinear across the cell's corner values.
    Interpolated,
}

/// Maximum number of contour levels accepted by one series.
///
/// Explicit level lists and levels produced by [`ContourConfig::nice_levels`]
/// share this limit. Renderers reject longer explicit lists rather than
/// truncating them.
pub const MAX_CONTOUR_LEVELS: usize = 1024;

fn push_contour_level(levels: &mut Vec<f64>, value: f64) -> Result<(), TickError> {
    if levels.len() == MAX_CONTOUR_LEVELS {
        return Err(TickError::InvalidRange);
    }
    levels.push(value);
    Ok(())
}

/// Contour lines over a field.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourConfig {
    /// The levels to draw, in data units, with at most
    /// [`MAX_CONTOUR_LEVELS`] entries. An explicit list only — there is no
    /// "auto" variant, because a level set inferred at draw time is a value the
    /// config does not contain. A helper computes levels and writes them here.
    pub levels: Vec<f64>,
    pub line: DataLineStyleConfig,
    /// Per-level colours, indexed by level. `None` means **every level is
    /// `line.line_color`** — colours are not derived from the colormap here. A
    /// helper that wants colormap colours computes them and writes them in.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub per_level_color: Option<Vec<Color>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub labels: Option<ContourLabelConfig>,
}

impl ContourConfig {
    /// About `target_count` round levels inside a colourbar's z range.
    ///
    /// There is no `Auto` level variant, and this is why there does not need to
    /// be one: the levels a chart draws are always the ones in
    /// [`Self::levels`], and a caller who wants them chosen for it calls this and
    /// stores the answer. What is drawn is then still exactly what the config
    /// says.
    ///
    /// Levels are **strictly inside** the range. One sitting exactly on
    /// `axis.min` or `axis.max` coincides with the field's outer boundary or has
    /// no visible coverage at
    /// all, and it would also make the banded fill's `n + 1` bands come out
    /// wrong by one.
    ///
    /// The spacing is the axis machinery's own: `compute_nice_ticks` picks it,
    /// and the walk is the axes' [`crate::tick::LinearGrid`], anchored to
    /// absolute multiples of it, so the levels are round numbers even when the
    /// range ends are not. A logarithmic colourbar gets every step-th decade
    /// from [`crate::tick::LogDecades`], with no separate rule here.
    pub fn nice_levels(axis: &AxisOptions, target_count: usize) -> Result<Vec<f64>, TickError> {
        // `compute_nice_ticks` widens min/max to nice bounds; a level outside the
        // colourbar's range has no colour, so only its spacing is used.
        let plan = AxisOptions::compute_nice_ticks(
            axis.scale.clone(),
            axis.min,
            axis.max,
            target_count.max(1),
        )?;
        let mut out = Vec::new();
        // Half a step of slack keeps a level that lands on the boundary through
        // floating-point noise from being kept as an interior one.
        let inside = |v: f64| v > axis.min && v < axis.max;
        match axis.scale {
            AxisScale::Linear => {
                let step = plan.major_spacing;
                if !(step.is_finite() && step > 0.0) {
                    return Err(TickError::InvalidRange);
                }
                // No interval cap: `MAX_CONTOUR_LEVELS` bounds the walk.
                let Some(grid) = crate::tick::LinearGrid::new(axis.min, axis.max, step, usize::MAX)
                else {
                    // No multiple of the step lies inside, or the step is too
                    // small to move the range ends at `f64` precision; then
                    // the range holds more levels than the limit when it
                    // holds more steps.
                    let steps = axis.max / step - axis.min / step;
                    return if steps > MAX_CONTOUR_LEVELS as f64 {
                        Err(TickError::InvalidRange)
                    } else {
                        Ok(out)
                    };
                };
                for (_, value) in grid.iter() {
                    if inside(value) {
                        push_contour_level(&mut out, value)?;
                    }
                }
            }
            AxisScale::Logarithmic => {
                let step = plan.major_spacing.max(1.0);
                let start = axis.min.log10().ceil();
                let end = axis.max.log10().floor();
                if !(step.is_finite()
                    && step <= i32::MAX as f64
                    && start.is_finite()
                    && start >= i32::MIN as f64
                    && start <= i32::MAX as f64
                    && end.is_finite()
                    && end >= i32::MIN as f64
                    && end <= i32::MAX as f64)
                {
                    return Err(TickError::InvalidRange);
                }
                let decades = crate::tick::LogDecades::new(start as i32, end as i32, step as i32)
                    .ok_or(TickError::InvalidRange)?;
                for (_, value) in decades {
                    if inside(value) {
                        push_contour_level(&mut out, value)?;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Replace [`Self::levels`] with [`Self::nice_levels`].
    ///
    /// [`Self::per_level_color`] is left alone: a colour list that no longer
    /// lines up with the new levels falls back to [`Self::line`]'s colour per
    /// level rather than being silently thrown away. Call
    /// [`Self::set_colormap_colors`] after this to re-derive it.
    pub fn set_nice_levels(
        &mut self,
        axis: &AxisOptions,
        target_count: usize,
    ) -> Result<(), TickError> {
        self.levels = Self::nice_levels(axis, target_count)?;
        Ok(())
    }

    /// Colour every level by where it sits on the colourbar's ramp.
    ///
    /// The helper the `per_level_color` doc means by "a helper that wants
    /// colormap colours computes them and writes them in" — the field's fill and
    /// the lines over it then agree about what a value looks like, because both
    /// go through `ColorBarOptions::color_for_z`. A level the ramp cannot place
    /// takes `nan_color`, the same as a cell with that value.
    pub fn set_colormap_colors(&mut self, bar: &crate::config::ColorBarOptions) {
        self.per_level_color = Some(
            self.levels
                .iter()
                .map(|level| bar.color_for_z(*level))
                .collect(),
        );
    }
}

/// Inline level labels on contour lines.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourLabelConfig {
    pub visible: bool,
    pub font_size: f32,
    /// Text colour, independent of the contour stroke and any per-level ramp
    /// colours. Labels are annotations; changing a line palette must not make
    /// their typography unreadable.
    #[cfg_attr(feature = "serde", serde(default = "default_contour_label_color"))]
    pub color: Color,
    pub format: LabelFormat,
    pub significant_digits: u8,
    /// Target minimum screen distance between labels selected by the normal
    /// automatic sweep. Must be finite and greater than zero even when labels
    /// are hidden or explicit anchors override automatic placement.
    ///
    /// Read by the anchor pass, which seeds a lattice at this pitch over the data
    /// area, projects each seed onto its level's isoline, and keeps the ones that
    /// stay this far apart. A per-level fallback may keep a closer candidate
    /// rather than omit that level. Labels are not arc-length-even: the seed
    /// lattice is even, the curve is not. Ignored when the resolved override is
    /// non-empty.
    #[cfg_attr(
        feature = "serde",
        serde(deserialize_with = "deserialize_contour_label_spacing_px")
    )]
    pub spacing_px: f32,
    /// Where the labels go — an **override**.
    ///
    /// Empty is the normal case: the GPU projects a seed lattice onto each
    /// level's isoline and selects a separated subset. For an explicit list, the
    /// renderer removes anchors whose `level_index` is outside `levels`, keeps at
    /// most 1024 in input order, and uses that resolved list as the override. If
    /// resolution leaves it empty, automatic placement remains active. Automatic
    /// and explicit placement share the same 1024-label draw capacity. Either way
    /// the same anchor record reaches the same draw.
    pub anchors: Vec<ContourLabelAnchor>,
    /// Fill painted behind the text, so a label over a line stays readable.
    ///
    /// `None` draws the text alone. This is a property of the *label*, not of the
    /// chart — which is why it can live here at all: the renderer composites the
    /// chrome over the data and does not know what colour is behind it, but the
    /// label knows what it wants behind *itself*.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bg_color: Option<Color>,
    /// Space around the text. This pads `bg_color` when one is present and is
    /// also the contour-line gap around a transparent label.
    #[cfg_attr(feature = "serde", serde(default))]
    pub bg_padding_px: f32,
}

#[cfg(feature = "serde")]
fn default_contour_label_color() -> Color {
    Color::BLACK
}

impl ContourLabelConfig {
    pub const INVALID_SPACING_REASON: &'static str =
        "contour label spacing_px must be finite and greater than zero";

    /// Validate invariants owned by the label configuration itself.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.spacing_px.is_finite() || self.spacing_px <= 0.0 {
            return Err(Self::INVALID_SPACING_REASON);
        }
        Ok(())
    }
}

#[cfg(feature = "serde")]
fn deserialize_contour_label_spacing_px<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let spacing_px = <f32 as serde::Deserialize>::deserialize(deserializer)?;
    if !spacing_px.is_finite() || spacing_px <= 0.0 {
        return Err(serde::de::Error::custom(
            ContourLabelConfig::INVALID_SPACING_REASON,
        ));
    }
    Ok(spacing_px)
}

/// One label placement, in data space.
///
/// Data coordinates and a data-space tangent, not screen values: the draw
/// re-projects them, so the same record works whether the GPU projected it from
/// the implicit field or a host wrote it by hand. The screen angle is this tangent
/// projected through the current transform — exact on a linear axis, the local
/// direction on a logarithmic one.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourLabelAnchor {
    /// Index into [`ContourConfig::levels`].
    pub level_index: u32,
    pub x: f64,
    pub y: f64,
    /// Tangent direction in data space; the label's screen angle is this
    /// projected through the current transform.
    pub tx: f64,
    pub ty: f64,
}

/// Bar appearance for [`DataRenderType::Histogram`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarStyleConfig {
    pub fill_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    /// The value bars grow from — usually 0. On a logarithmic count axis 0 has
    /// no position, and the base is clamped to the axis minimum instead.
    pub baseline: f64,
    /// Screen-space gap between neighbouring bars. The renderer reduces the
    /// gap when necessary so a positive-width bin keeps a visible footprint;
    /// Subpixel bins use a GPU pixel-column maximum envelope instead: gap
    /// and positive width ratios do not cut holes in that envelope.
    pub gap_px: f32,
    /// Fraction of each bin occupied by its bar, centred on the bin. Values are
    /// clamped to `0..=1` by the renderer. `1` uses the whole bin before
    /// applying [`Self::gap_px`], while `0.8` leaves 10% empty on each side.
    #[cfg_attr(feature = "serde", serde(default = "default_bar_width_ratio"))]
    pub width_ratio: f32,
    /// Which column is edges and which is counts. This alone decides; the
    /// `edges = counts + 1` length relation is not used to guess.
    pub orientation: BarOrientation,
    /// Sparse, declaration-order overrides keyed by rendered bin index. A
    /// later record for the same index layers over an earlier partial record.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bar_style_overrides: Option<Vec<DataBarStyleOverride>>,
}

#[cfg(feature = "serde")]
fn default_bar_width_ratio() -> f32 {
    1.0
}

/// Partial appearance override for one histogram bin.
///
/// `None` inherits the series-level value. A zero `border_width` removes the
/// outline for just that bin.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarBinStyleConfig {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub fill_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub border_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub border_width: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub gap_px: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub width_ratio: Option<f32>,
}

/// One sparse histogram-bin style override.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarStyleOverride {
    pub index: usize,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub style: DataBarBinStyleConfig,
}

/// Bar direction, and with it the roles of `x_column` / `y_column`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BarOrientation {
    /// Bars rise along y: `x_column` is edges, `y_column` is counts.
    Vertical,
    /// Bars extend along x: `y_column` is edges, `x_column` is counts.
    Horizontal,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    TriangleDown,
    TriangleLeft,
    TriangleRight,
    Plus,
    Pentagon,
    Hexagon,
    Octagon,
    Star,
    TriangleDownFilled,
    TriangleLeftFilled,
    TriangleRightFilled,
    PlusFilled,
    CrossFilled,
    PentagonFilled,
    HexagonFilled,
    OctagonFilled,
    StarFilled,
}

#[cfg(test)]
mod vector_style_tests {
    use super::*;

    fn style(scale: VectorScale) -> VectorStyle {
        VectorStyle {
            color: Color::BLACK,
            width_px: 1.5,
            scale,
            pivot: VectorPivot::Tail,
            head_length_px: 8.0,
            head_width_px: 6.0,
            min_length_px: 0.0,
            color_map: None,
        }
    }

    #[test]
    fn validation_rejects_bad_pixels_and_scales() {
        assert_eq!(style(VectorScale::default()).validate(), Ok(()));
        let mut bad = style(VectorScale::default());
        bad.width_px = -1.0;
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_PIXELS_REASON));
        let mut bad = style(VectorScale::default());
        bad.head_width_px = f32::NAN;
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_PIXELS_REASON));
        let bad = style(VectorScale::Data { factor: 0.0 });
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_SCALE_REASON));
        let bad = style(VectorScale::Pixels {
            px_per_unit: f64::INFINITY,
        });
        assert_eq!(bad.validate(), Err(VectorStyle::INVALID_SCALE_REASON));
        // A negative data factor flips every arrow; it is a choice, not an error.
        assert_eq!(style(VectorScale::Data { factor: -2.0 }).validate(), Ok(()));
    }

    #[test]
    fn fit_scale_targets_the_longest_vector_in_either_mode() {
        let mut pixels = style(VectorScale::Pixels { px_per_unit: 1.0 });
        // The longest possible vector is (|u| = 3, |v| = 4): length 5.
        assert!(pixels.fit_scale((-3.0, 1.0), (0.0, 4.0), 40.0));
        assert_eq!(pixels.scale, VectorScale::Pixels { px_per_unit: 8.0 });
        let mut data = style(VectorScale::Data { factor: 1.0 });
        assert!(data.fit_scale((-3.0, 1.0), (0.0, 4.0), 0.5));
        assert_eq!(data.scale, VectorScale::Data { factor: 0.1 });
    }

    #[test]
    fn fit_scale_leaves_the_style_alone_for_empty_or_invalid_bounds() {
        let mut style = style(VectorScale::Pixels { px_per_unit: 3.0 });
        assert!(!style.fit_scale((0.0, 0.0), (0.0, 0.0), 40.0));
        assert!(!style.fit_scale((f64::NAN, 1.0), (0.0, 1.0), 40.0));
        assert!(!style.fit_scale((-1.0, 1.0), (-1.0, 1.0), 0.0));
        assert_eq!(style.scale, VectorScale::Pixels { px_per_unit: 3.0 });
    }
}

#[cfg(test)]
mod point_size_map_tests {
    use super::*;

    fn map(scale: SizeScale) -> PointSizeMap {
        PointSizeMap {
            column: "s".into(),
            domain_min: 0.0,
            domain_max: 4.0,
            min_px: 2.0,
            max_px: 10.0,
            scale,
        }
    }

    #[test]
    fn radius_scaling_clamps_to_the_pixel_range() {
        let radius = map(SizeScale::Radius);
        assert_eq!(radius.radius_px(0.0), Some(2.0));
        assert_eq!(radius.radius_px(2.0), Some(6.0));
        assert_eq!(radius.radius_px(4.0), Some(10.0));
        assert_eq!(radius.radius_px(-5.0), Some(2.0));
        assert_eq!(radius.radius_px(99.0), Some(10.0));
        assert_eq!(radius.radius_px(f64::NAN), None);
        assert_eq!(radius.radius_px(f64::INFINITY), None);
    }

    #[test]
    fn area_scaling_is_linear_in_area() {
        let area = map(SizeScale::Area);
        let lo = 2.0f32 * 2.0;
        let hi = 10.0f32 * 10.0;
        let mid = area.radius_px(2.0).unwrap();
        assert!((mid * mid - (lo + hi) / 2.0).abs() < 1e-4);
        assert_eq!(area.radius_px(0.0), Some(2.0));
        assert_eq!(area.radius_px(4.0), Some(10.0));
    }

    #[test]
    fn validation_rejects_bad_domains_and_pixel_ranges() {
        assert_eq!(map(SizeScale::Area).validate(), Ok(()));
        let mut bad = map(SizeScale::Area);
        bad.domain_max = bad.domain_min;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_DOMAIN_REASON));
        let mut bad = map(SizeScale::Area);
        bad.domain_min = f64::NAN;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_DOMAIN_REASON));
        let mut bad = map(SizeScale::Area);
        bad.min_px = 11.0;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
        let mut bad = map(SizeScale::Area);
        bad.min_px = -1.0;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
        let mut bad = map(SizeScale::Area);
        bad.max_px = f32::INFINITY;
        assert_eq!(bad.validate(), Err(PointSizeMap::INVALID_PIXELS_REASON));
    }

    #[test]
    fn fit_domain_writes_only_valid_ranges() {
        let mut m = map(SizeScale::Area);
        assert!(m.fit_domain(-3.0, 7.0));
        assert_eq!((m.domain_min, m.domain_max), (-3.0, 7.0));
        assert!(!m.fit_domain(5.0, 5.0));
        assert!(!m.fit_domain(f64::NAN, 1.0));
        assert_eq!((m.domain_min, m.domain_max), (-3.0, 7.0));
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    fn matrix(
        orientation: MatrixOrientation,
        grid_layout: GridLayout,
        columns: usize,
    ) -> MatrixRef {
        MatrixRef {
            columns: (0..columns).map(|i| format!("z{i}")).collect(),
            orientation,
            grid_layout,
        }
    }

    // The declared shape, matching the data exactly: nothing is trimmed.
    #[test]
    fn a_well_formed_grid_uses_everything_it_declares() {
        let centers = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        assert_eq!(centers.effective_extent(10, 5, 5), (10, 5));

        // Edges bound cells, so the coordinate columns are one longer.
        let edges = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Edges, 10);
        assert_eq!(edges.effective_extent(11, 6, 5), (10, 5));
    }

    // A mismatch is drawn to the smallest common extent and reported. It
    // never errors, and it never draws past the data.
    #[test]
    fn a_mismatched_grid_shrinks_to_the_smallest_common_extent() {
        let m = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        // Fewer x coordinates than constituent columns.
        assert_eq!(m.effective_extent(4, 5, 5), (4, 5));
        // A short constituent column limits the depth.
        assert_eq!(m.effective_extent(10, 5, 2), (10, 2));
        // Fewer y coordinates than the columns are deep.
        assert_eq!(m.effective_extent(10, 3, 5), (10, 3));
        // Nothing at all: zero cells, still not an error.
        assert_eq!(m.effective_extent(0, 0, 0), (0, 0));
    }

    // `Edges` with a single coordinate bounds no cell. Saturating, not wrapping:
    // `0 - 1` as a usize would ask for 18 quintillion cells.
    #[test]
    fn one_edge_bounds_no_cell() {
        let m = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Edges, 10);
        assert_eq!(m.effective_extent(1, 1, 5), (0, 0));
        assert_eq!(m.effective_extent(0, 0, 5), (0, 0));
    }

    // Orientation swaps which coordinate column bounds which direction. It is
    // the declaration that decides — never the lengths.
    #[test]
    fn orientation_decides_which_coordinate_bounds_the_columns() {
        let columns_are_x = matrix(MatrixOrientation::ColumnsAreX, GridLayout::Centers, 10);
        let columns_are_y = matrix(MatrixOrientation::ColumnsAreY, GridLayout::Centers, 10);
        // x = 4, y = 20: one reads 4 usable columns, the other 10.
        assert_eq!(columns_are_x.effective_extent(4, 20, 20), (4, 20));
        assert_eq!(columns_are_y.effective_extent(4, 20, 20), (10, 4));
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::{
        DataBarStyleConfig, DataErrorBarPointStyleConfig, DataErrorBarPointStyleOverride,
        DataErrorBarStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterPointStyleOverride, DataScatterStyleConfig, GridLayout, MatrixOrientation,
        MatrixRef, PointColorMap, ScatterShape, SeriesConfig, SizeScale, VectorColorMap,
        VectorComponents, VectorPivot, VectorScale, VectorStyle,
    };
    use crate::color::Color;

    fn base_series_json() -> serde_json::Value {
        serde_json::json!({
            "series_id": "s",
            "label": null,
            "x_column": "x",
            "y_column": "y",
            "render_type": {
                "Scatter": {
                    "scatter": {
                        "point_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
                        "point_shape": "Circle",
                        "point_size": 3.0
                    }
                }
            }
        })
    }

    #[test]
    fn old_series_json_without_additive_fields_still_parses() {
        let cfg: SeriesConfig =
            serde_json::from_value(base_series_json()).expect("old series shape parses");
        assert_eq!(cfg.source_id, None);
        let DataRenderType::Scatter { scatter } = &cfg.render_type else {
            panic!("expected scatter");
        };
        assert_eq!(scatter.point_style_table, None);
        assert_eq!(scatter.point_style_index_column, None);
        assert_eq!(scatter.point_style_overrides, None);
        assert_eq!(scatter.point_color_map, None);
        assert_eq!(scatter.point_size_map, None);

        let json = serde_json::to_value(cfg).expect("serialize");
        assert!(json.get("source_id").is_none());
        let scatter = &json["render_type"]["Scatter"]["scatter"];
        assert!(scatter.get("point_style_table").is_none());
        assert!(scatter.get("point_style_index_column").is_none());
        assert!(scatter.get("point_style_overrides").is_none());
        assert!(scatter.get("point_color_map").is_none());
        assert!(scatter.get("point_size_map").is_none());
        assert_eq!(json, base_series_json());
    }

    #[test]
    fn value_maps_round_trip_and_default_the_size_scale() {
        let mut json = base_series_json();
        let scatter = &mut json["render_type"]["Scatter"]["scatter"];
        scatter["point_color_map"] = serde_json::json!({ "column": "c" });
        scatter["point_size_map"] = serde_json::json!({
            "column": "s",
            "domain_min": 0.0,
            "domain_max": 10.0,
            "min_px": 2.0,
            "max_px": 12.0
        });
        let cfg: SeriesConfig = serde_json::from_value(json).expect("maps parse");
        let DataRenderType::Scatter { scatter } = &cfg.render_type else {
            panic!("expected scatter");
        };
        assert_eq!(
            scatter.point_color_map,
            Some(PointColorMap { column: "c".into() })
        );
        let size = scatter.point_size_map.as_ref().expect("size map");
        assert_eq!(size.scale, SizeScale::Area);

        let back: SeriesConfig =
            serde_json::from_value(serde_json::to_value(&cfg).expect("serialize"))
                .expect("round trip");
        assert_eq!(back, cfg);
    }

    #[test]
    fn point_style_mapping_round_trips_with_flattened_override() {
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 3.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: Some(Color::from_rgb8(255, 0, 0)),
                    point_shape: None,
                    point_size: Some(5.0),
                },
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::DiamondFilled),
                    point_size: None,
                },
            ]),
            point_style_index_column: Some("style_idx".into()),
            point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                index: 7,
                style: DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::StarFilled),
                    point_size: Some(9.0),
                },
            }]),
            point_color_map: None,
            point_size_map: None,
        };

        let json = serde_json::to_value(&scatter).expect("serialize scatter style");
        assert_eq!(json["point_style_index_column"], "style_idx");
        assert_eq!(json["point_style_overrides"][0]["index"], 7);
        assert_eq!(
            json["point_style_overrides"][0]["point_shape"],
            "StarFilled"
        );
        assert_eq!(json["point_style_overrides"][0]["point_size"], 9.0);
        assert!(json["point_style_overrides"][0].get("style").is_none());

        let back: DataScatterStyleConfig =
            serde_json::from_value(json).expect("parse scatter style");
        assert_eq!(back, scatter);
    }

    #[test]
    fn errorbar_style_mapping_round_trips_with_flattened_override() {
        let err = DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 3.0,
            cap_width: 1.0,
            error_bar_style_table: Some(vec![
                DataErrorBarPointStyleConfig {
                    error_bar_color: Some(Color::from_rgb8(255, 0, 0)),
                    error_bar_width: Some(2.0),
                    error_bar_cap_size: None,
                    cap_width: None,
                },
                DataErrorBarPointStyleConfig {
                    error_bar_color: None,
                    error_bar_width: None,
                    error_bar_cap_size: Some(8.0),
                    cap_width: Some(3.0),
                },
            ]),
            error_bar_style_index_column: Some("err_style_idx".into()),
            error_bar_style_overrides: Some(vec![DataErrorBarPointStyleOverride {
                index: 4,
                style: DataErrorBarPointStyleConfig {
                    error_bar_color: None,
                    error_bar_width: Some(4.0),
                    error_bar_cap_size: None,
                    cap_width: Some(2.0),
                },
            }]),
        };

        let json = serde_json::to_value(&err).expect("serialize errorbar style");
        assert_eq!(json["error_bar_style_index_column"], "err_style_idx");
        assert_eq!(json["error_bar_style_overrides"][0]["index"], 4);
        assert_eq!(json["error_bar_style_overrides"][0]["error_bar_width"], 4.0);
        assert_eq!(json["error_bar_style_overrides"][0]["cap_width"], 2.0);
        assert!(json["error_bar_style_overrides"][0].get("style").is_none());

        let back: DataErrorBarStyleConfig =
            serde_json::from_value(json).expect("parse errorbar style");
        assert_eq!(back, err);
    }

    // The line fields added for area fill and step lines are all defaulted and
    // omitted at their defaults, so a document written before them parses and
    // serializes byte-identically.
    #[test]
    fn legacy_line_style_json_round_trips_without_fill_or_interpolation_keys() {
        use super::{DataLineStyleConfig, LineInterpolation};

        let legacy = serde_json::json!({
            "line_style": "Dash",
            "line_color": { "r": 0.5, "g": 0.25, "b": 0.75, "a": 1.0 },
            "line_width": 2.0
        });
        let parsed: DataLineStyleConfig =
            serde_json::from_value(legacy.clone()).expect("parse legacy line style");
        assert_eq!(parsed.fill, None);
        assert_eq!(parsed.fill_color, None);
        assert_eq!(parsed.interpolation, LineInterpolation::Linear);
        assert_eq!(
            serde_json::to_value(&parsed).expect("serialize legacy line style"),
            legacy,
            "defaults must not add keys to a legacy document"
        );
    }

    #[test]
    fn line_fill_and_interpolation_round_trip_and_keep_their_spelling() {
        use super::{DataLineStyleConfig, LineFill, LineInterpolation};

        let line = DataLineStyleConfig {
            line_style: crate::line::LineStylePreset::Solid,
            line_color: Color::from_rgb8(10, 20, 30),
            line_width: 1.5,
            fill: Some(LineFill::ToSeries("lower".into())),
            fill_color: Some(Color::from_rgba(0.5, 0.5, 0.5, 0.25)),
            interpolation: LineInterpolation::StepMid,
        };
        let json = serde_json::to_value(&line).expect("serialize line style");
        assert_eq!(json["fill"], serde_json::json!({ "ToSeries": "lower" }));
        assert_eq!(json["interpolation"], "StepMid");
        assert_eq!(json["fill_color"]["a"], 0.25);
        let back: DataLineStyleConfig = serde_json::from_value(json).expect("parse line style");
        assert_eq!(back, line);

        let baseline = DataLineStyleConfig {
            fill: Some(LineFill::ToBaseline(-1.0)),
            ..DataLineStyleConfig::default()
        };
        let json = serde_json::to_value(&baseline).expect("serialize baseline fill");
        assert_eq!(json["fill"], serde_json::json!({ "ToBaseline": -1.0 }));
        assert!(json.get("fill_color").is_none());
        assert!(json.get("interpolation").is_none());
        let back: DataLineStyleConfig = serde_json::from_value(json).expect("parse baseline fill");
        assert_eq!(back, baseline);

        for (mode, spelling) in [
            (LineInterpolation::StepPre, "StepPre"),
            (LineInterpolation::StepPost, "StepPost"),
            (LineInterpolation::StepMid, "StepMid"),
        ] {
            assert_eq!(
                serde_json::to_value(mode).expect("serialize interpolation"),
                serde_json::json!(spelling)
            );
        }
    }

    #[test]
    fn legacy_histogram_style_defaults_to_full_width_without_overrides() {
        let legacy = serde_json::json!({
            "fill_color": { "r": 0.2, "g": 0.3, "b": 0.4, "a": 1.0 },
            "border_color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 },
            "border_width": 1.0,
            "baseline": 0.0,
            "gap_px": 2.0,
            "orientation": "Vertical"
        });
        let parsed: DataBarStyleConfig =
            serde_json::from_value(legacy).expect("parse legacy histogram style");
        assert_eq!(parsed.width_ratio, 1.0);
        assert_eq!(parsed.bar_style_overrides, None);
    }

    #[test]
    fn vector_render_types_round_trip_and_default_the_optional_fields() {
        let style = VectorStyle {
            color: Color::BLACK,
            width_px: 1.5,
            scale: VectorScale::Pixels { px_per_unit: 20.0 },
            pivot: VectorPivot::Middle,
            head_length_px: 8.0,
            head_width_px: 6.0,
            min_length_px: 2.0,
            color_map: Some(VectorColorMap::Column("speed".into())),
        };
        let points = SeriesConfig {
            series_id: "flow".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                },
                style: style.clone(),
            },
        };
        let json = serde_json::to_string(&points).unwrap();
        assert!(json.contains("\"Vector\""));
        assert!(json.contains("\"Points\""));
        assert!(json.contains("\"Pixels\""));
        assert!(json.contains("\"Middle\""));
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), points);

        let grid = |columns: &[&str]| MatrixRef {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        };
        let gridded = SeriesConfig {
            render_type: DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: grid(&["u0", "u1"]),
                    v: grid(&["v0", "v1"]),
                },
                style: VectorStyle {
                    scale: VectorScale::Data { factor: 0.5 },
                    color_map: Some(VectorColorMap::Grid(grid(&["m0", "m1"]))),
                    ..style
                },
            },
            ..points
        };
        let json = serde_json::to_string(&gridded).unwrap();
        assert!(json.contains("\"Grid\""));
        assert!(json.contains("\"Data\""));
        assert_eq!(
            serde_json::from_str::<SeriesConfig>(&json).unwrap(),
            gridded
        );

        // The optional fields default: the pivot is the tail, the minimum
        // length is zero and omitted, and there is no colour map.
        let minimal = r#"{"series_id":"q","source_id":null,"label":null,"x_column":"x","y_column":"y","render_type":{"Vector":{"components":{"Points":{"u":"u","v":"v"}},"style":{"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0},"width_px":1.0,"scale":{"Pixels":{"px_per_unit":1.0}},"head_length_px":6.0,"head_width_px":4.0}}}}"#;
        let parsed: SeriesConfig = serde_json::from_str(minimal).unwrap();
        let DataRenderType::Vector { style, .. } = &parsed.render_type else {
            panic!("expected a vector series");
        };
        assert_eq!(style.pivot, VectorPivot::Tail);
        assert_eq!(style.min_length_px, 0.0);
        assert_eq!(style.color_map, None);
        let again = serde_json::to_string(&parsed).unwrap();
        assert!(!again.contains("min_length_px"));
        assert!(!again.contains("color_map"));
        assert!(again.contains("\"pivot\":\"Tail\""));
    }

    #[test]
    fn field_render_types_round_trip() {
        use super::{
            ContourConfig, ContourLabelAnchor, ContourLabelConfig, DataBarBinStyleConfig,
            DataBarStyleConfig, DataBarStyleOverride, DataLineStyleConfig, FieldFillConfig,
            FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading,
        };
        use crate::format::LabelFormat;
        use crate::line::LineStylePreset;

        let matrix = MatrixRef {
            columns: vec!["z0".into(), "z1".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Edges,
        };
        let fill = FieldFillConfig {
            mode: FillMode::Bands,
            shading: Shading::Interpolated,
            opacity: 0.8,
        };
        let contour = ContourConfig {
            levels: vec![1.0, 2.0, 4.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.5,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: Some(vec![Color::from_rgb8(255, 0, 0)]),
            labels: Some(ContourLabelConfig {
                visible: true,
                font_size: 11.0,
                color: Color::BLACK,
                format: LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px: 120.0,
                anchors: vec![ContourLabelAnchor {
                    level_index: 1,
                    x: 0.25,
                    y: 0.5,
                    tx: 1.0,
                    ty: 0.0,
                }],
                bg_color: None,
                bg_padding_px: 0.0,
            }),
        };
        let cases = [
            DataRenderType::Histogram {
                bar: DataBarStyleConfig {
                    fill_color: Color::from_rgb8(70, 130, 180),
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 0.8,
                    orientation: super::BarOrientation::Vertical,
                    bar_style_overrides: Some(vec![DataBarStyleOverride {
                        index: 2,
                        style: DataBarBinStyleConfig {
                            fill_color: Some(Color::from_rgb8(255, 0, 0)),
                            border_color: None,
                            border_width: Some(3.0),
                            gap_px: None,
                            width_ratio: Some(0.5),
                        },
                    }]),
                },
            },
            DataRenderType::Heatmap {
                matrix: matrix.clone(),
                fill: fill.clone(),
            },
            DataRenderType::Contour {
                matrix: matrix.clone(),
                contour: contour.clone(),
            },
            DataRenderType::HeatmapContour {
                matrix,
                fill,
                contour,
            },
        ];
        for case in cases {
            let json = serde_json::to_value(&case).expect("serialize render type");
            let back: DataRenderType = serde_json::from_value(json).expect("parse render type");
            assert_eq!(back, case);
        }
    }

    // `per_level_color: None` means "every level uses `line.line_color`". It has
    // to survive as an absent key, not as an empty list — an empty list is a
    // table with no entries, which is a different statement.
    #[test]
    fn contour_optional_keys_are_omitted_when_absent() {
        use super::{ContourConfig, DataLineStyleConfig, FillMode};
        use crate::line::LineStylePreset;

        let contour = ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        };
        let json = serde_json::to_value(&contour).expect("serialize contour");
        assert!(json.get("per_level_color").is_none());
        assert!(json.get("labels").is_none());
        let back: ContourConfig = serde_json::from_value(json).expect("parse contour");
        assert_eq!(back, contour);
        assert_eq!(back.per_level_color, None);

        // And the discriminants stay spelled the way hosts write them.
        assert_eq!(
            serde_json::to_value(FillMode::Continuous).expect("serialize fill mode"),
            serde_json::json!("Continuous")
        );
    }

    #[test]
    fn contour_label_spacing_serde_requires_a_finite_positive_value() {
        use super::ContourLabelConfig;

        let json = |spacing_px: f32| {
            serde_json::json!({
                "visible": false,
                "font_size": 12.0,
                "format": "Decimal",
                "significant_digits": 3,
                "spacing_px": spacing_px,
                "anchors": []
            })
        };
        let parsed = serde_json::from_value::<ContourLabelConfig>(json(0.5)).unwrap();
        assert_eq!(
            parsed.color,
            Color::BLACK,
            "older JSON without the independent label colour defaults to black"
        );
        for spacing_px in [0.0, -1.0] {
            let error = serde_json::from_value::<ContourLabelConfig>(json(spacing_px))
                .expect_err("invalid spacing must fail during deserialization");
            assert!(error.to_string().contains("spacing_px must be finite"));
        }
    }
}

#[cfg(test)]
mod line_style_tests {
    use super::*;

    #[test]
    fn line_fill_defaults_to_the_zero_baseline_and_the_line_colour_at_quarter_alpha() {
        assert_eq!(LineFill::default(), LineFill::ToBaseline(0.0));
        let line = DataLineStyleConfig {
            line_color: Color::from_rgba(0.2, 0.4, 0.6, 0.8),
            ..DataLineStyleConfig::default()
        };
        assert_eq!(
            line.effective_fill_color(),
            Color::from_rgba(0.2, 0.4, 0.6, 0.8 * DataLineStyleConfig::DEFAULT_FILL_ALPHA)
        );
        let explicit = DataLineStyleConfig {
            fill_color: Some(Color::WHITE),
            ..line
        };
        assert_eq!(explicit.effective_fill_color(), Color::WHITE);
    }

    #[test]
    fn the_default_interpolation_is_linear() {
        assert!(LineInterpolation::default().is_linear());
    }
}

#[cfg(test)]
mod contour_level_tests {
    use super::*;
    use crate::config::AxisScale;
    use crate::default::{default_axis_options_colorbar, default_colorbar_options};

    fn axis(min: f64, max: f64, scale: AxisScale) -> AxisOptions {
        let mut axis = default_axis_options_colorbar();
        axis.min = min;
        axis.max = max;
        axis.scale = scale;
        axis
    }

    fn contour() -> ContourConfig {
        ContourConfig {
            levels: Vec::new(),
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        }
    }

    #[test]
    fn contour_label_spacing_rust_validation_applies_to_all_placement_states() {
        for spacing_px in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let labels = ContourLabelConfig {
                visible: false,
                font_size: 12.0,
                color: Color::BLACK,
                format: LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px,
                anchors: vec![ContourLabelAnchor {
                    level_index: 0,
                    x: 0.0,
                    y: 0.0,
                    tx: 1.0,
                    ty: 0.0,
                }],
                bg_color: None,
                bg_padding_px: 0.0,
            };
            assert_eq!(
                labels.validate(),
                Err(ContourLabelConfig::INVALID_SPACING_REASON)
            );
        }
    }

    /// Asking for more levels gets more, and they stay inside. That is the whole
    /// "how dense" control: the count in, the level list out.
    #[test]
    fn asking_for_more_levels_gets_more() {
        let sparse = ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), 3).unwrap();
        let dense = ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), 11).unwrap();
        assert!(
            dense.len() > sparse.len(),
            "sparse {sparse:?} vs dense {dense:?}"
        );
        for level in sparse.iter().chain(dense.iter()) {
            assert!(*level > 0.0 && *level < 1.0, "{level} is not inside");
        }
    }

    #[test]
    fn nice_levels_accepts_exactly_the_linear_limit() {
        let levels =
            ContourConfig::nice_levels(&axis(0.0, 1025.0, AxisScale::Linear), 2001).unwrap();

        assert_eq!(levels.len(), MAX_CONTOUR_LEVELS);
        assert_eq!(levels.first(), Some(&1.0));
        assert_eq!(levels.last(), Some(&1024.0));
        assert!(levels.windows(2).all(|pair| pair[1] - pair[0] == 1.0));
    }

    #[test]
    fn a_tiny_linear_spacing_is_bounded_by_the_level_limit() {
        assert_eq!(
            ContourConfig::nice_levels(&axis(0.0, 1.0, AxisScale::Linear), usize::MAX),
            Err(TickError::InvalidRange)
        );
    }

    #[test]
    fn logarithmic_levels_are_not_silently_truncated() {
        let levels = ContourConfig::nice_levels(
            &axis(f64::from_bits(1), f64::MAX, AxisScale::Logarithmic),
            MAX_CONTOUR_LEVELS,
        )
        .unwrap();

        assert_eq!(levels.len(), 632);
        assert_eq!(levels.first(), Some(&1e-323));
        assert_eq!(levels.last(), Some(&1e308));
        assert!(levels.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// A range whose ends are not round still gets round levels: the walk anchors
    /// to absolute multiples of the spacing, not to `axis.min`.
    #[test]
    fn an_awkward_range_still_gets_round_levels() {
        let levels = ContourConfig::nice_levels(&axis(0.137, 9.42, AxisScale::Linear), 5).unwrap();
        assert!(!levels.is_empty());
        for level in &levels {
            assert!(
                (level / 2.0).fract().abs() < 1e-9,
                "{level} is not a multiple of the spacing"
            );
        }
    }

    /// A logarithmic colourbar gets decades, from the same axis machinery — no
    /// second rule lives here.
    #[test]
    fn a_logarithmic_range_gets_decades() {
        let levels =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 5).unwrap();
        assert_eq!(levels, vec![10.0, 100.0, 1000.0]);
    }

    #[test]
    fn target_count_zero_keeps_its_existing_one_tick_correction() {
        let corrected =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 0).unwrap();
        let explicit_one =
            ContourConfig::nice_levels(&axis(1.0, 10_000.0, AxisScale::Logarithmic), 1).unwrap();
        assert_eq!(corrected, explicit_one);
    }

    /// A degenerate range has no levels to pick, and says so rather than
    /// inventing one.
    #[test]
    fn a_degenerate_range_is_refused() {
        assert!(ContourConfig::nice_levels(&axis(1.0, 1.0, AxisScale::Linear), 5).is_err());
        assert!(ContourConfig::nice_levels(&axis(-1.0, 10.0, AxisScale::Logarithmic), 5).is_err());
    }

    /// `set_nice_levels` writes the answer into the config — what is drawn stays
    /// exactly what the config holds. Existing per-level colours are left for the
    /// draw's own fallback rather than discarded. The levels are round numbers on
    /// the spacing's own absolute grid, and they sit **inside** the range — a level
    /// on the boundary coincides with the field's edge and would leave the banded
    /// fill one band short.
    #[test]
    fn set_nice_levels_writes_the_levels_and_keeps_the_colours() {
        let mut cfg = contour();
        cfg.per_level_color = Some(vec![Color::BLACK]);
        cfg.set_nice_levels(&axis(0.0, 100.0, AxisScale::Linear), 5)
            .unwrap();
        assert_eq!(cfg.levels, vec![20.0, 40.0, 60.0, 80.0]);
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    #[test]
    fn set_nice_levels_preserves_the_config_when_the_limit_is_exceeded() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);

        assert_eq!(
            cfg.set_nice_levels(&axis(0.0, 1026.0, AxisScale::Linear), 2001),
            Err(TickError::InvalidRange)
        );
        assert_eq!(cfg.levels, vec![42.0]);
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    #[test]
    fn set_nice_levels_preserves_all_state_on_invalid_bounds() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);
        let before = cfg.clone();

        assert_eq!(
            cfg.set_nice_levels(&axis(1.0, f64::INFINITY, AxisScale::Logarithmic), 5),
            Err(TickError::InvalidRange)
        );
        assert_eq!(cfg, before);
    }

    #[test]
    fn set_nice_levels_replaces_levels_with_a_successful_empty_result() {
        let mut cfg = contour();
        cfg.levels = vec![42.0];
        cfg.per_level_color = Some(vec![Color::BLACK]);

        cfg.set_nice_levels(&axis(2.0, 9.0, AxisScale::Logarithmic), 5)
            .unwrap();
        assert!(cfg.levels.is_empty());
        assert_eq!(cfg.per_level_color, Some(vec![Color::BLACK]));
    }

    /// Colouring the levels from the ramp goes through the same `color_for_z` the
    /// field's cells do, so a line and the fill it bounds agree about the value.
    #[test]
    fn colormap_colours_come_from_color_for_z() {
        let mut bar = default_colorbar_options();
        bar.axis.min = 0.0;
        bar.axis.max = 100.0;
        let mut cfg = contour();
        cfg.set_nice_levels(&bar.axis, 5).unwrap();
        cfg.set_colormap_colors(&bar);
        let colors = cfg.per_level_color.as_ref().expect("colours written");
        assert_eq!(colors.len(), cfg.levels.len());
        for (level, color) in cfg.levels.iter().zip(colors) {
            assert_eq!(*color, bar.color_for_z(*level));
        }
    }

    /// Ranges the previous index walk got wrong, pinned for the shared grid:
    /// indices past `i64` (it saturated into an empty list) now get the
    /// level-limit error, and a step that moves neither range end at `f64`
    /// precision (it repeated the few representable values) gets no levels.
    #[test]
    fn nice_levels_refuse_what_the_grid_cannot_walk() {
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e-9, 3e-9, AxisScale::Linear), usize::MAX),
            Err(TickError::InvalidRange)
        );
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e16, 1e16 + 4.0, AxisScale::Linear), 5),
            Ok(Vec::new())
        );
        assert_eq!(
            ContourConfig::nice_levels(&axis(1e16, 1e16 + 4.0, AxisScale::Linear), 2001),
            Err(TickError::InvalidRange)
        );
    }
}
