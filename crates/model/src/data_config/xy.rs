//! Line, scatter, error-bar and stem series ([`XySeries`]) and their styles.

use super::*;

/// Markers, a line and error bars over a series' x and y columns: the
/// composed form of what were nine render types (#160).
#[derive(Debug, Clone, PartialEq)]
pub struct XySeries {
    /// The markers, or none.
    pub scatter: Option<DataScatterStyleConfig>,
    /// The line through the points, or none.
    pub line: Option<DataLineStyleConfig>,
    /// The error bars, or none. They need markers.
    pub errors: Option<ErrorBars>,
    /// Stems from a baseline to each point (a stem or lollipop plot,
    /// #206), or none. They need markers.
    pub stem: Option<Stem>,
}

/// Stems from a baseline to each point: a stem or lollipop plot (#206).
///
/// A stem is drawn exactly where its marker's sample is valid, from the
/// baseline to the point along the value axis. It reads no column of its
/// own: the baseline is configuration.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Stem {
    /// Where the stems start, in data units of the value axis (y for
    /// `Vertical`, x for `Horizontal`). Finite. On a logarithmic value
    /// axis a baseline at or below zero is drawn at the axis minimum.
    pub baseline: f64,
    /// `Vertical` (the default, omitted when written): stems along y.
    /// `Horizontal`: stems along x. The column roles never swap.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::category_bar_direction",
            skip_serializing_if = "is_vertical_stem"
        )
    )]
    pub direction: CategoryBarDirection,
    /// The stroke colour.
    pub color: Color,
    /// The stroke width in logical pixels, finite and at least 0.
    pub width: f32,
    /// The dash (`Solid`, the default, omitted when written). Every stem's
    /// dash starts at its baseline.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::line_style_preset",
            skip_serializing_if = "is_solid_stem"
        )
    )]
    pub line_style: crate::line::LineStylePreset,
}

#[cfg(feature = "serde")]
pub(super) fn is_vertical_stem(direction: &CategoryBarDirection) -> bool {
    *direction == CategoryBarDirection::Vertical
}

#[cfg(feature = "serde")]
pub(super) fn is_solid_stem(style: &crate::line::LineStylePreset) -> bool {
    *style == crate::defaults::line_style_preset()
}

impl Stem {
    /// A stem's own rules: a finite baseline and a finite width of at
    /// least 0.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.baseline.is_finite() {
            return Err(crate::RuleKind::StemBaseline);
        }
        if !(self.width.is_finite() && self.width >= 0.0) {
            return Err(crate::RuleKind::StemWidth);
        }
        Ok(())
    }
}

/// Error bars along x, y or both, with one style.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBars {
    /// The x error, or none.
    pub x: Option<ErrorRef>,
    /// The y error, or none.
    pub y: Option<ErrorRef>,
    pub style: DataErrorBarStyleConfig,
}

impl XySeries {
    /// The combinations a series can draw: markers or a line (or both),
    /// error bars only with markers, along x, y or both, and stems only
    /// with markers.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if self.scatter.is_none() && self.line.is_none() {
            return Err(crate::RuleKind::XyNothingDrawn);
        }
        if let Some(errors) = &self.errors {
            if self.scatter.is_none() {
                return Err(crate::RuleKind::ErrorBarsWithoutMarkers);
            }
            if errors.x.is_none() && errors.y.is_none() {
                return Err(crate::RuleKind::ErrorBarsDirection);
            }
        }
        if let Some(stem) = &self.stem {
            if self.scatter.is_none() {
                return Err(crate::RuleKind::StemsWithoutMarkers);
            }
            stem.validate()?;
        }
        Ok(())
    }
}

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

/// How consecutive samples of a line series are connected.
///
/// A step mode draws every segment as axis-aligned pieces instead of one
/// straight segment. Dash phase, picking and the arc-length scan all follow the
/// stepped path, so a dashed step line keeps its pattern around every corner
/// and a pick on a riser still resolves to one of the two source indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineInterpolation {
    /// Straight segments between consecutive samples.
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
        serde(
            default = "crate::defaults::line_interpolation",
            skip_serializing_if = "LineInterpolation::is_linear"
        )
    )]
    pub interpolation: LineInterpolation,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SizeScale {
    /// The value is proportional to marker area (the radius grows with its
    /// square root), which is how bubble sizes are read.
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
    #[cfg_attr(feature = "serde", serde(default = "crate::defaults::size_scale"))]
    pub scale: SizeScale,
}

impl PointSizeMap {
    /// The validation reason for a non-finite or empty domain.
    pub const INVALID_DOMAIN_REASON: crate::RuleKind = crate::RuleKind::PointSizeMapDomain;
    /// The validation reason for a non-finite, negative or inverted pixel range.
    pub const INVALID_PIXELS_REASON: crate::RuleKind = crate::RuleKind::PointSizeMapPixels;

    /// Validate invariants owned by the map itself.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
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
    /// value is not finite (the point is hidden). The draw and pick shaders
    /// follow this formula (`size_map_radius`); the renderer's CPU twin, test
    /// support, checks them against it.
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

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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
