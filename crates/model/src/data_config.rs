//! Declarative series configuration types.
//!
//! Mapping layer between chart layout (`config::Config`) and raw data
//! (`data::DataCell`). Each `SeriesConfig` says which columns are X/Y/error
//! and which render type / style to use.

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Category};
use crate::data::ColumnId;
use crate::format::LabelFormat;
use crate::line::LineStylePreset;
use crate::material::Material;
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
    /// The bars of a categorical bar chart: one bar per category of the
    /// chart's categorical axis ([`AxisOptions::categories`]), in this
    /// series' colour and look.
    ///
    /// The bar direction ([`BarGroups::direction`] of `Config::bar_groups`)
    /// decides the column roles, like a histogram's orientation:
    /// `Vertical` reads `x_column` as the category positions (the host
    /// uploads `0, 1, ..., n - 1`) and `y_column` as the values, one per
    /// category with NaN for a missing bar; `Horizontal` reads `y_column`
    /// as the positions and `x_column` as the values. Both columns hold
    /// exactly one value per category.
    ///
    /// The `CategoryBars` series of one layer form its bar group: they
    /// stand side by side, stack or stack to 100 % as
    /// [`BarGroups::mode`] says, in series order. The renderer reads the
    /// other series' values from the pool for a stack; nothing per bar is
    /// kept on the CPU. A `PercentStacked` group whose value column has a
    /// negative value (the pool's minimum statistic below 0) is refused when
    /// it is drawn or picked, with `InvalidSeriesConfig` naming that series
    /// and the mode; `Stacked` and `Grouped` draw negative values.
    ///
    /// Value labels: `labels.text` holds the drawn strings ([`format_labels`]
    /// writes them; the renderer never formats a value), one per category,
    /// drawn at each bar's end as `labels.placement` (and a bar override's
    /// `labels`) says, moved by `labels.offsets`. An empty string draws
    /// nothing. Labels never displace each other: thinning the category
    /// axis is the only way to thin them.
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. Polar charts (`Config.polar`) reject category bars.
    CategoryBars {
        /// The fill colour of the series' bars, opaque.
        color: Color,
        /// The look of the bars: material, corner radius, outline, texture,
        /// gloss and the brightness of a picked bar.
        #[cfg_attr(feature = "serde", serde(default))]
        style: CategoryBarStyle,
        /// Where the value labels go, how values are written and the drawn
        /// text of each bar.
        #[cfg_attr(feature = "serde", serde(default))]
        labels: CategoryBarSeriesLabels,
        /// What a value label says.
        #[cfg_attr(feature = "serde", serde(default))]
        label_format: CategoryBarLabelFormat,
        /// Sparse per-bar overrides, keyed by category id and this series'
        /// id. At most one per bar; an override's `style` replaces the
        /// series' style whole.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        overrides: Option<Vec<CategoryBarOverride>>,
    },
    /// The boxes of a box plot: one box per category of the chart's
    /// categorical axis ([`AxisOptions::categories`]), in this series'
    /// colour and look, with its median, whiskers and caps, its mean and its
    /// outliers.
    ///
    /// `x_column` is the category index column (the host uploads
    /// `0, 1, ..., n - 1`) and `y_column` the id of the series' **record**
    /// of box statistics (the renderer's `BOX_PLOT_RECORD` schema: `median`,
    /// `q1`, `q3`, `whisker_low`, `whisker_high`, and optionally `mean`,
    /// `sample_count`, `ci_low`, `ci_high`), one row per category; a missing
    /// box is a row of NaN. The roles do not swap with the direction as a
    /// category bar's do: a record is not a value column of either axis, so
    /// the series names it in its value column slot and keeps its shape.
    /// `outliers` names a ragged column holding one group of outliers per
    /// category, when the series has any.
    ///
    /// The `BoxPlot` series of one layer form its box group: they stand side
    /// by side in series order, laid out by `Config::bar_groups`'
    /// direction, group width and gap ([`BarGroups`]); its mode must be
    /// grouped. Statistics the record does not order (whisker ≤ q1 ≤
    /// median ≤ q3 ≤ whisker, outliers outside the whiskers) are not
    /// checked: the drawing clamps. Nothing per box is kept on the CPU.
    ///
    /// Value labels: `labels.text` holds the drawn strings, which the host
    /// writes (the renderer has no per-box statistics to write them from;
    /// [`format_box_plot_labels`] helps), one per category, drawn past the
    /// top of each box (its high whisker, mean, notch and outliers) unless
    /// `labels.placement` (or a box override's `labels`) is `None`, moved by
    /// `labels.offsets`. An empty string draws nothing.
    ///
    /// The record and the outliers must be GPU resident; a streamed source
    /// is rejected with an error. Polar charts (`Config.polar`) reject box
    /// plots.
    BoxPlot {
        /// The fill colour of the series' boxes, opaque.
        color: Color,
        /// The look of the boxes and their parts.
        #[cfg_attr(feature = "serde", serde(default))]
        style: BoxPlotStyle,
        /// Which labels the boxes show, how a median is written and the
        /// drawn text of each box.
        #[cfg_attr(feature = "serde", serde(default))]
        labels: BoxPlotSeriesLabels,
        /// The ragged column of the outliers, one group per category.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        outliers: Option<ColumnId>,
        /// Sparse per-box overrides, keyed by category id and this series'
        /// id. At most one per box; an override's `style` replaces the
        /// series' style whole.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        overrides: Option<Vec<BoxPlotOverride>>,
    },
    /// The polygon of a radar chart: one vertex per category of the polar
    /// chart's categorical angle axis (`bottom_x` with
    /// [`AxisOptions::categories`]), joined into a closed outline, filled
    /// to the centre (the inner ring) and marked at every vertex, in this
    /// series' colour and look.
    ///
    /// `x_column` is the category index column (the host uploads
    /// `0, 1, ..., n - 1`) and `y_column` the value of each category on the
    /// radius axis (`left_y`), NaN for a missing value. Both columns hold
    /// exactly one value per category. The outline is the polar line path
    /// with its last segment drawn back to the first vertex; a NaN vertex
    /// breaks the outline and the fill on both of its sides, as a NaN
    /// breaks a line. The fill is the polar area path to the inner ring at
    /// [`RadarSeriesStyle::fill_color`], and the markers are the scatter
    /// path. Nothing per vertex is kept on the CPU.
    ///
    /// Value labels: `labels.text` holds the drawn strings
    /// ([`format_radar_labels`] writes them), one per category, drawn
    /// outside each vertex along its spoke when `labels.placement` is
    /// `Values` (or `Emphasized` and the series has a selected vertex or
    /// label), moved by `labels.offsets`. An empty string draws nothing.
    ///
    /// Only a polar chart (`Config.polar`) with a categorical angle axis
    /// draws radar series, in the precise draw style; the columns must be
    /// GPU resident, and a streamed source is rejected with an error.
    Radar {
        /// The outline and marker colour; the fill uses it at the style's
        /// fill opacity unless the style names a fill.
        color: Color,
        /// Outline width, fill and marker; unset fields take
        /// their defaults.
        #[cfg_attr(feature = "serde", serde(default))]
        style: RadarSeriesStyle,
        /// Which vertices carry value labels, how a value is written and
        /// the drawn text of each vertex.
        #[cfg_attr(feature = "serde", serde(default))]
        labels: RadarSeriesLabels,
    },
    /// Streamlines of a flow, electric or magnetic field: polylines the
    /// host integrated from its field, drawn by the line path, with
    /// arrowheads along each drawn line and an optional colour column.
    ///
    /// The renderer draws the paths it is given; integrating the field
    /// into paths is the host's job. `x_column` / `y_column` hold every
    /// streamline of the series one after another, separated by a row of
    /// NaN, which breaks a line as it does in any line series. The line
    /// is drawn with `line` (dash presets included), linearly
    /// interpolated and without a fill.
    ///
    /// With `arrows`, an arrowhead (the vector glyph's head) sits every
    /// `spacing_px` along the drawn path, measured over the whole series
    /// as one arc in pixels, pointing along the drawn path's tangent.
    /// Arrowheads are an appearance of the drawn line, like its dash
    /// phase: their positions follow the view, so a pan or a zoom moves
    /// them along the path and they keep their pixel spacing at every
    /// export scale. A NaN gap holds no arrowhead.
    ///
    /// With `color_map`, the line and its arrowheads take the colour of a
    /// host column (a speed, say), index-aligned with the coordinates,
    /// through `Config.colorbar`; the colourbar is the only authority on
    /// the range, scale, ramp and `nan_color`, as for a colour-mapped
    /// scatter, and a series with a colour map and no colourbar is
    /// rejected. The colour varies along each segment between its two
    /// samples' values. The renderer never computes a magnitude.
    ///
    /// Drawn in the precise draw style only. The columns must be GPU
    /// resident; a streamed source is rejected with an error. Polar charts
    /// (`Config.polar`) reject streamlines.
    Streamlines {
        /// The line the streamlines are drawn with. `interpolation` must
        /// be `Linear` and `fill` `None`.
        line: DataLineStyleConfig,
        /// Arrowheads along the drawn lines, or none.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        arrows: Option<StreamlineArrows>,
        /// Colour the lines and arrowheads by a host column through
        /// `Config.colorbar`.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        color_map: Option<PointColorMap>,
    },
}

/// The arrowheads of a [`DataRenderType::Streamlines`] series: the vector
/// glyph's head, every `spacing_px` along the drawn path. Logical pixels,
/// like `point_size`; an export at a larger scale scales them with it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StreamlineArrows {
    /// The arc length between two arrowheads along the drawn path. The
    /// first sits half a spacing from the start of the series.
    pub spacing_px: f32,
    /// Head length along the path; the head is centred on its position.
    pub head_length_px: f32,
    /// Full head width across the path.
    pub head_width_px: f32,
}

impl Default for StreamlineArrows {
    /// A head 8 px long and 7 px wide every 80 px.
    fn default() -> Self {
        Self {
            spacing_px: 80.0,
            head_length_px: 8.0,
            head_width_px: 7.0,
        }
    }
}

impl StreamlineArrows {
    /// The validation reason for a bad arrow size or spacing.
    pub const INVALID_REASON: &'static str = "streamline arrows need a finite spacing_px > 0, finite non-negative \
         head_length_px and head_width_px, and spacing_px >= head_length_px";

    /// Validate invariants owned by the arrows themselves: finite sizes, a
    /// positive spacing and no head longer than the spacing, so two heads
    /// never overlap along a straight path.
    pub fn validate(&self) -> Result<(), &'static str> {
        let finite_non_negative = |v: f32| v.is_finite() && v >= 0.0;
        if !(self.spacing_px.is_finite() && self.spacing_px > 0.0)
            || !finite_non_negative(self.head_length_px)
            || !finite_non_negative(self.head_width_px)
            || self.spacing_px < self.head_length_px
        {
            return Err(Self::INVALID_REASON);
        }
        Ok(())
    }
}

/// The fields of a [`DataRenderType::Streamlines`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct StreamlinesRef<'a> {
    pub line: &'a DataLineStyleConfig,
    pub arrows: Option<&'a StreamlineArrows>,
    pub color_map: Option<&'a PointColorMap>,
}

impl DataRenderType {
    /// The fields of a streamline series, or `None` for every other kind.
    pub fn streamlines(&self) -> Option<StreamlinesRef<'_>> {
        match self {
            DataRenderType::Streamlines {
                line,
                arrows,
                color_map,
            } => Some(StreamlinesRef {
                line,
                arrows: arrows.as_ref(),
                color_map: color_map.as_ref(),
            }),
            _ => None,
        }
    }
}

/// The rules one streamline series is held to on its own: a linearly
/// interpolated line without a fill, valid arrows, and named coordinate and
/// colour columns. The chart rules (no polar chart, a colourbar for a
/// colour map) are checked with the chart.
pub fn validate_streamline_series(series: &SeriesConfig) -> Result<(), &'static str> {
    let Some(streamlines) = series.render_type.streamlines() else {
        return Ok(());
    };
    if !streamlines.line.interpolation.is_linear() {
        return Err("streamlines are polylines: the line interpolation must be Linear");
    }
    if streamlines.line.fill.is_some() {
        return Err("streamlines have no fill");
    }
    if let Some(arrows) = streamlines.arrows {
        arrows.validate()?;
    }
    if series.x_column.is_empty() || series.y_column.is_empty() {
        return Err("a streamline series names its x_column and y_column");
    }
    if streamlines
        .color_map
        .is_some_and(|map| map.column.is_empty())
    {
        return Err("a streamline colour map names its column");
    }
    Ok(())
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

// ───── Category bars ─────

/// How the bars of one bar group share a category slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarMode {
    /// Side by side, one bar per series in series order.
    #[default]
    Grouped,
    /// On top of each other in series order; positive and negative values
    /// stack away from zero independently.
    Stacked,
    /// Stacked to the share of the category total, `0..=100` on the value
    /// axis. Values must not be negative: the renderer refuses a series
    /// whose value column's minimum is below 0 when it draws or picks the
    /// group (`InvalidSeriesConfig`, naming the series and the mode).
    PercentStacked,
}

/// The direction bars grow in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarDirection {
    /// Categories along x, values along y.
    #[default]
    Vertical,
    /// Categories along y, values along x.
    Horizontal,
}

/// Where a bar's value label goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarLabels {
    None,
    Inside,
    Outside,
    /// Outside grouped bars, inside stacked ones.
    #[default]
    Auto,
}

/// What a bar's value label says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CategoryBarLabelFormat {
    #[default]
    Value,
    /// The bar's share of its category total.
    Percent,
    ValuePercent,
    CategoryValue,
}

/// The look of a bar: [`DataRenderType::CategoryBars`]' `style`, and a
/// complete per-bar override.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct CategoryBarStyle {
    /// A bar is a flat body: [`Material::supported_on_flat_body`] decides.
    pub material: Material,
    /// Logical pixels. Only the exposed end of a stack is rounded.
    pub corner_radius: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
    pub texture_strength: f32,
    pub texture_scale: f32,
    pub gloss: f32,
    /// The brightness a picked bar gains. A highlight keeps the bar's data
    /// end; it never grows or lifts the bar.
    pub emphasis_brightness: f32,
}

impl Default for CategoryBarStyle {
    fn default() -> Self {
        Self {
            material: Material::Flat,
            corner_radius: 5.0,
            outline: false,
            outline_width: 1.0,
            outline_color: Color::from_rgb8(40, 55, 75),
            texture_strength: 0.2,
            texture_scale: 1.0,
            gloss: 0.45,
            emphasis_brightness: 0.08,
        }
    }
}

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

/// Every channel in `0..=1`, and opaque when `opaque` is set.
pub(crate) fn valid_bar_color(c: Color, opaque: bool) -> bool {
    [c.r, c.g, c.b, c.a]
        .into_iter()
        .all(|v| finite_in(v, 0.0, 1.0))
        && (!opaque || c.a == 1.0)
}

/// Whether `text` is one line of at most `max` characters.
pub(crate) fn single_line_within(text: &str, max: usize) -> bool {
    text.chars().count() <= max && !text.chars().any(char::is_control)
}

/// The farthest a value label may be dragged from where it is placed, in
/// logical pixels along either axis.
pub const MAX_VALUE_LABEL_OFFSET: f32 = 4096.0;

/// The rules of a series' value label offsets: empty, or one finite offset
/// per category of at most [`MAX_VALUE_LABEL_OFFSET`] along either axis.
fn validate_value_label_offsets(
    offsets: &[[f32; 2]],
    categories: usize,
) -> Result<(), &'static str> {
    if !offsets.is_empty() && offsets.len() != categories {
        return Err("value label offsets need one entry per category");
    }
    if offsets
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > MAX_VALUE_LABEL_OFFSET)
    {
        return Err("a value label offset is finite and at most 4096 px along either axis");
    }
    Ok(())
}

/// The element id of value label `index` of series `series_id`:
/// `"series:<series_id>:label:<index>"`. The id a pick reports for a label
/// hit and the one a host passes back to drag it.
pub fn value_label_id(series_id: &str, index: usize) -> String {
    format!("series:{series_id}:label:{index}")
}

/// The series id and the label index a [`value_label_id`] names, or `None`
/// for any other string. The index is read after the last `":label:"`, so a
/// series id may itself contain one.
pub fn parse_value_label_id(id: &str) -> Option<(&str, usize)> {
    let rest = id.strip_prefix("series:")?;
    let (series_id, index) = rest.rsplit_once(":label:")?;
    if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((series_id, index.parse().ok()?))
}

impl SeriesConfig {
    /// The value label offsets of a category bar, box plot or radar series
    /// (`labels.offsets`), or `None` for every other kind.
    pub fn value_label_offsets(&self) -> Option<&[[f32; 2]]> {
        match &self.render_type {
            DataRenderType::CategoryBars { labels, .. } => Some(&labels.offsets),
            DataRenderType::BoxPlot { labels, .. } => Some(&labels.offsets),
            DataRenderType::Radar { labels, .. } => Some(&labels.offsets),
            _ => None,
        }
    }

    /// The drawn strings of a category bar, box plot or radar series'
    /// value labels (`labels.text`), or `None` for every other kind.
    pub fn value_label_text(&self) -> Option<&[String]> {
        match &self.render_type {
            DataRenderType::CategoryBars { labels, .. } => Some(&labels.text),
            DataRenderType::BoxPlot { labels, .. } => Some(&labels.text),
            DataRenderType::Radar { labels, .. } => Some(&labels.text),
            _ => None,
        }
    }

    /// The drag offset of value label `index`, in logical pixels: its
    /// `labels.offsets` entry, or zero when none is stored.
    pub fn value_label_offset(&self, index: usize) -> [f32; 2] {
        self.value_label_offsets()
            .and_then(|offsets| offsets.get(index).copied())
            .unwrap_or([0.0; 2])
    }

    /// Move value label `index` of this series by `(dx, dy)` logical pixels
    /// (x right, y down): its `labels.offsets` entry grows by the delta, the
    /// list filled with zero offsets to `categories` entries on the first
    /// move. Nothing changes and the move is rejected for a series without
    /// value labels, an index past `categories`, a non-finite delta or an
    /// offset past [`MAX_VALUE_LABEL_OFFSET`]. The label's placement is the
    /// renderer's, from the pool, so there is no containment test beyond
    /// that bound.
    pub fn drag_value_label(
        &mut self,
        index: usize,
        categories: usize,
        dx: f32,
        dy: f32,
    ) -> crate::layout::NudgeResult {
        use crate::layout::{NudgeReject, NudgeResult};
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let offsets = match &mut self.render_type {
            DataRenderType::CategoryBars { labels, .. } => &mut labels.offsets,
            DataRenderType::BoxPlot { labels, .. } => &mut labels.offsets,
            DataRenderType::Radar { labels, .. } => &mut labels.offsets,
            _ => return reject,
        };
        if index >= categories || !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        let [x, y] = offsets.get(index).copied().unwrap_or([0.0; 2]);
        let moved = [x + dx, y + dy];
        if moved
            .iter()
            .any(|v| !v.is_finite() || v.abs() > MAX_VALUE_LABEL_OFFSET)
        {
            return reject;
        }
        if offsets.len() != categories {
            offsets.resize(categories, [0.0; 2]);
        }
        offsets[index] = moved;
        NudgeResult::Moved
    }
}

impl CategoryBarStyle {
    /// The rules a bar style is held to at every level (series, override):
    /// a material with a look on a flat body, a corner radius in `0..=32`,
    /// an outline width in `0..=8`, an outline colour with channels in
    /// `0..=1`, texture strength and gloss in `0..=1`, a texture scale in
    /// `0.1..=8` and an emphasis brightness in `0..=0.5`.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.material.supported_on_flat_body() {
            return Err("material not supported by bars");
        }
        if finite_in(self.corner_radius, 0.0, 32.0)
            && finite_in(self.outline_width, 0.0, 8.0)
            && valid_bar_color(self.outline_color, false)
            && finite_in(self.texture_strength, 0.0, 1.0)
            && finite_in(self.texture_scale, 0.1, 8.0)
            && finite_in(self.gloss, 0.0, 1.0)
            && finite_in(self.emphasis_brightness, 0.0, 0.5)
        {
            Ok(())
        } else {
            Err("invalid bar style")
        }
    }
}

/// One bar: the id of its category and the id of its series.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryBarTarget {
    pub category_id: String,
    pub series_id: String,
}

impl CategoryBarTarget {
    pub fn new(category: impl Into<String>, series: impl Into<String>) -> Self {
        Self {
            category_id: category.into(),
            series_id: series.into(),
        }
    }
}

/// A per-bar override. `None` fields keep the series' value; `style`
/// replaces the series' style whole.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CategoryBarOverride {
    pub target: CategoryBarTarget,
    pub color: Option<Color>,
    pub style: Option<CategoryBarStyle>,
    pub labels: Option<CategoryBarLabels>,
    pub label_format: Option<CategoryBarLabelFormat>,
}

impl CategoryBarOverride {
    pub fn new(target: CategoryBarTarget) -> Self {
        Self {
            target,
            color: None,
            style: None,
            labels: None,
            label_format: None,
        }
    }
}

/// The most characters one drawn value label may have.
pub const MAX_CATEGORY_BAR_LABEL_CHARS: usize = 160;

/// The value labels of a [`DataRenderType::CategoryBars`] series: where they
/// go, how a value is written, and the drawn text of every bar.
///
/// `text` is the string each bar shows, one per category in axis order (an
/// empty string draws nothing), or empty when the labels were not written
/// yet. The renderer never formats a value: [`format_labels`] writes `text`
/// at edit time from the values the host holds, `label_format`, `decimals`
/// and `value_suffix`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct CategoryBarSeriesLabels {
    pub placement: CategoryBarLabels,
    /// Fixed decimals of a written value, `0..=6`; trailing zeros are
    /// dropped.
    pub decimals: usize,
    /// Appended to a written value; one line of at most 24 characters.
    pub value_suffix: String,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub text: Vec<String>,
    /// Where each label was dragged to, in logical pixels from where it is
    /// placed (x right, y down), one per category in axis order, or empty
    /// when no label was dragged. The renderer adds the offset to the
    /// label's placement on every draw; a drag writes it
    /// ([`SeriesConfig::drag_value_label`]).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub offsets: Vec<[f32; 2]>,
}

impl Default for CategoryBarSeriesLabels {
    fn default() -> Self {
        Self {
            placement: CategoryBarLabels::Auto,
            decimals: 1,
            value_suffix: String::new(),
            text: Vec::new(),
            offsets: Vec::new(),
        }
    }
}

/// How the [`DataRenderType::CategoryBars`] series of one layer share their
/// categorical axis: `Config::bar_groups`. The property of the set of bar
/// series, not of one series. `None` there reads as the default (grouped,
/// vertical, 0.7 of the slot, 3 px apart). A [`CategoryBarMode::PercentStacked`]
/// group draws only non-negative values; a member series with a negative
/// value is refused when the group is drawn.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BarGroups {
    pub mode: CategoryBarMode,
    pub direction: CategoryBarDirection,
    /// Fraction of a category slot all bars of the slot cover, `0.1..=0.95`.
    pub group_width: f32,
    /// Logical pixels between grouped bars, `0..=24`.
    pub bar_gap: f32,
}

impl Default for BarGroups {
    fn default() -> Self {
        Self {
            mode: CategoryBarMode::Grouped,
            direction: CategoryBarDirection::Vertical,
            group_width: 0.7,
            bar_gap: 3.0,
        }
    }
}

impl BarGroups {
    pub fn validate(&self) -> Result<(), &'static str> {
        if finite_in(self.group_width, 0.1, 0.95) && finite_in(self.bar_gap, 0.0, 24.0) {
            Ok(())
        } else {
            Err("bar groups need a group width in 0.1..=0.95 and a bar gap in 0..=24 px")
        }
    }
}

/// The most series one [`LineOffsets`] group may hold. Each member is drawn
/// as an internal panel of its own (`Config::expand`), with the textures of
/// an overlay layer, so a document with more traces uses two charts.
pub const MAX_OFFSET_LINES: usize = 32;

/// Offset spectra (a ridgeline or waterfall of curves): line series drawn
/// shifted from each other so their traces separate instead of overlapping,
/// `Config::line_offsets`.
///
/// Member `k` of `series` (in stacking order, the first member unshifted) is
/// drawn shifted by `k · step_y` along y and `k · step_x` along x. The steps
/// are display parameters in data units, or decades on a logarithmic axis:
/// the host supplies the curves as they are and nothing is shifted on the
/// CPU. Hover, picking, selection details and `step_selection` report the
/// unshifted values; an auto-fit of the axes includes the shifts.
///
/// The renderer draws member `k` as an internal panel of its own, the base
/// panel (or, for a member an overlay layer scales, that overlay's panel)
/// with its X and Y ranges moved by the shift and every axis element hidden
/// (`Config::expand`), so lines, fills, markers, error bars, dashes and the
/// picked highlight follow without a transform of their own. Later members
/// draw over earlier ones, fill included, which gives the ridgeline
/// occlusion, and a pick answers with what is painted under the cursor: the
/// last member painted there first, else the nearest hit of any series.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct LineOffsets {
    /// The member series ids in stacking order: member `k` is shifted by
    /// `k` steps. Line kinds only (`Line`, `ScatterLine` and the
    /// `LineScatterErrorbar*` kinds), each id once, at most
    /// [`MAX_OFFSET_LINES`].
    pub series: Vec<SeriesRef>,
    /// The step along y between consecutive members: data units, or decades
    /// on a logarithmic axis.
    pub step_y: f64,
    /// The step along x for a skewed waterfall, in the same units as
    /// `step_y`. Omitted from JSON at `0.0`.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_zero_step"))]
    pub step_x: f64,
}

#[cfg(feature = "serde")]
fn is_zero_step(step: &f64) -> bool {
    *step == 0.0
}

impl Default for LineOffsets {
    /// No members and no steps.
    fn default() -> Self {
        Self {
            series: Vec::new(),
            step_y: 0.0,
            step_x: 0.0,
        }
    }
}

impl LineOffsets {
    /// The rules that do not depend on the series list: finite steps, each
    /// member id once and non-empty, at most [`MAX_OFFSET_LINES`] members.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.step_y.is_finite() || !self.step_x.is_finite() {
            return Err("line offset steps must be finite");
        }
        if self.series.len() > MAX_OFFSET_LINES {
            return Err("a line offset group holds at most 32 series");
        }
        for (index, id) in self.series.iter().enumerate() {
            if id.is_empty() || self.series[..index].contains(id) {
                return Err("a line offset member is a non-empty series id, listed once");
            }
        }
        Ok(())
    }

    /// The stacking position of `series_id`, `None` for a series that is not
    /// a member.
    pub fn member(&self, series_id: &str) -> Option<usize> {
        self.series.iter().position(|id| id == series_id)
    }

    /// The shift of member `k`: `[k · step_x, k · step_y]`.
    pub fn shift(&self, k: usize) -> [f64; 2] {
        let k = k as f64;
        [k * self.step_x, k * self.step_y]
    }
}

impl DataRenderType {
    /// Whether a series of this kind may be a [`LineOffsets`] member: the
    /// kinds that draw a line.
    pub fn takes_line_offset(&self) -> bool {
        match self {
            DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. } => true,
            DataRenderType::Scatter { .. }
            | DataRenderType::ScatterErrorbarX { .. }
            | DataRenderType::ScatterErrorbarY { .. }
            | DataRenderType::ScatterErrorbarXY { .. }
            | DataRenderType::Histogram { .. }
            | DataRenderType::Heatmap { .. }
            | DataRenderType::Contour { .. }
            | DataRenderType::HeatmapContour { .. }
            | DataRenderType::Vector { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::BoxPlot { .. }
            | DataRenderType::Radar { .. }
            // A field's streamlines share its axes; they are not a member.
            | DataRenderType::Streamlines { .. } => false,
        }
    }

    /// The fill of a line kind, `None` for a line without one and for every
    /// other kind.
    fn line_fill(&self) -> Option<&LineFill> {
        match self {
            DataRenderType::Line { line }
            | DataRenderType::ScatterLine { line, .. }
            | DataRenderType::LineScatterErrorbarX { line, .. }
            | DataRenderType::LineScatterErrorbarY { line, .. }
            | DataRenderType::LineScatterErrorbarXY { line, .. } => line.fill.as_ref(),
            _ => None,
        }
    }
}

impl crate::config::Config {
    /// The line offset group of the chart that draws `series_id` (the
    /// parent's for its base layer and overlays, a subplot's own for the
    /// series it draws) and the series' stacking position in it.
    fn offset_member(&self, series_id: &str) -> Option<(Option<usize>, usize)> {
        use crate::layer::LayerRef;
        let (cell, chart) = match self.layer_of_series(series_id) {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                (Some(index), &self.subplots.get(index)?.config)
            }
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => (None, self),
        };
        let k = chart.line_offsets.as_ref()?.member(series_id)?;
        Some((cell, k))
    }

    /// The display shift of `series_id` as `[dx, dy]`: `k · step` of its
    /// position `k` in the line offset group of the chart that draws it, in
    /// data units or decades on a logarithmic axis. `None` for a series that
    /// is not a member.
    pub fn line_offset_of(&self, series_id: &str) -> Option<[f64; 2]> {
        let (cell, k) = self.offset_member(series_id)?;
        let chart = match cell {
            Some(index) => &self.subplots[index].config,
            None => self,
        };
        Some(chart.line_offsets.as_ref()?.shift(k))
    }

    /// The line offset rules that do not depend on the series list, for the
    /// chart and every subplot: [`LineOffsets::validate`], no polar chart,
    /// and every member drawn by the chart that lists it (the parent's base
    /// layer or one of its overlays, or the subplot itself).
    pub fn validate_line_offset_layout(&self) -> Result<(), &'static str> {
        use crate::layer::LayerRef;
        let charts = std::iter::once((None, self)).chain(
            self.subplots
                .iter()
                .enumerate()
                .map(|(index, subplot)| (Some(index), &subplot.config)),
        );
        for (cell, chart) in charts {
            let Some(offsets) = &chart.line_offsets else {
                continue;
            };
            offsets.validate()?;
            if offsets.series.is_empty() {
                continue;
            }
            if chart.polar.is_some() {
                return Err("polar charts have no line offsets");
            }
            for id in &offsets.series {
                let drawn = match (cell, self.layer_of_series(id)) {
                    (None, LayerRef::Base | LayerRef::Overlay(_)) => true,
                    (Some(index), LayerRef::Subplot(at) | LayerRef::SubplotOverlay(at, _)) => {
                        at == index
                    }
                    _ => false,
                };
                if !drawn {
                    return Err("a line offset member must be drawn by the chart that lists it");
                }
            }
        }
        Ok(())
    }

    /// The line offset rules with the series list the chart draws:
    /// [`Self::validate_line_offset_layout`], every member a declared series
    /// of a line kind ([`DataRenderType::takes_line_offset`]), and no
    /// [`LineFill::ToSeries`] fill between two series of which only one is a
    /// member or that are different members: each member is drawn by a panel
    /// of its own, so such a fill would span two panels.
    pub fn validate_line_offsets(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        self.validate_line_offset_layout()?;
        let charts = std::iter::once(self).chain(self.subplots.iter().map(|s| &s.config));
        for chart in charts {
            let Some(offsets) = &chart.line_offsets else {
                continue;
            };
            for id in &offsets.series {
                let Some(member) = series.iter().find(|s| s.series_id == *id) else {
                    return Err("a line offset member names no series of the chart");
                };
                if !member.render_type.takes_line_offset() {
                    return Err(
                        "a line offset member must draw a line: Line, ScatterLine or a \
                         LineScatterErrorbar kind",
                    );
                }
            }
        }
        for item in series {
            if let Some(LineFill::ToSeries(target)) = item.render_type.line_fill()
                && self.offset_member(&item.series_id) != self.offset_member(target)
            {
                return Err("a ToSeries fill cannot join series with different line offsets");
            }
        }
        Ok(())
    }
}

/// The fields of a [`DataRenderType::CategoryBars`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct CategoryBarsRef<'a> {
    pub color: Color,
    pub style: &'a CategoryBarStyle,
    pub labels: &'a CategoryBarSeriesLabels,
    pub label_format: CategoryBarLabelFormat,
    pub overrides: &'a [CategoryBarOverride],
}

impl DataRenderType {
    /// The fields of a category bar series, or `None` for every other kind.
    pub fn category_bars(&self) -> Option<CategoryBarsRef<'_>> {
        match self {
            DataRenderType::CategoryBars {
                color,
                style,
                labels,
                label_format,
                overrides,
            } => Some(CategoryBarsRef {
                color: *color,
                style,
                labels,
                label_format: *label_format,
                overrides: overrides.as_deref().unwrap_or(&[]),
            }),
            _ => None,
        }
    }
}

impl SeriesConfig {
    /// The column of a category bar series that holds the category
    /// positions and the one that holds the values, by `direction`.
    pub fn category_bar_columns(&self, direction: CategoryBarDirection) -> (&str, &str) {
        match direction {
            CategoryBarDirection::Vertical => (&self.x_column, &self.y_column),
            CategoryBarDirection::Horizontal => (&self.y_column, &self.x_column),
        }
    }
}

/// The rules one category bar series is held to against the `categories`
/// of its axis: an opaque colour, a valid style, label decimals `0..=6`, a
/// one-line value suffix of at most 24 characters, `labels.text` empty or
/// one one-line string of at most [`MAX_CATEGORY_BAR_LABEL_CHARS`]
/// characters per category, and overrides that name one of the categories
/// and this series, at most one per bar, with an opaque colour and a valid
/// style.
pub fn validate_category_bar_series(
    series: &SeriesConfig,
    categories: &[Category],
) -> Result<(), &'static str> {
    let Some(bars) = series.render_type.category_bars() else {
        return Ok(());
    };
    if !valid_bar_color(bars.color, true) {
        return Err("a bar colour must be opaque with channels in 0..=1");
    }
    bars.style.validate()?;
    if bars.labels.decimals > 6 || !single_line_within(&bars.labels.value_suffix, 24) {
        return Err(
            "bar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        );
    }
    if !bars.labels.text.is_empty() && bars.labels.text.len() != categories.len() {
        return Err("bar label text needs one string per category");
    }
    validate_value_label_offsets(&bars.labels.offsets, categories.len())?;
    if bars
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_CATEGORY_BAR_LABEL_CHARS))
    {
        return Err("a bar label is one line of at most 160 characters");
    }
    for (index, o) in bars.overrides.iter().enumerate() {
        if o.target.series_id != series.series_id
            || !categories.iter().any(|c| c.id == o.target.category_id)
            || bars.overrides[..index].iter().any(|e| e.target == o.target)
        {
            return Err("a bar override names a category of the axis and its own series, once");
        }
        if o.color.is_some_and(|c| !valid_bar_color(c, true)) {
            return Err("a bar colour must be opaque with channels in 0..=1");
        }
        if let Some(style) = &o.style {
            style.validate()?;
        }
    }
    Ok(())
}

impl crate::config::Config {
    /// The bar group options of `layer`: the `bar_groups` of the chart that
    /// draws it (the parent's for `Base` and an overlay, the subplot's own
    /// for a subplot and its overlays), or the default.
    pub fn bar_groups_of(&self, layer: crate::layer::LayerRef) -> BarGroups {
        use crate::layer::LayerRef;
        let groups = match layer {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => self
                .subplots
                .get(index)
                .and_then(|subplot| subplot.config.bar_groups.as_ref()),
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => {
                self.bar_groups.as_ref()
            }
        };
        groups.cloned().unwrap_or_default()
    }

    /// The rules the category bar series of a chart are held to, with the
    /// series list it draws.
    ///
    /// Every `bar_groups` (the chart's and each subplot's) passes
    /// [`BarGroups::validate`]. The [`DataRenderType::CategoryBars`] series
    /// of one layer are its bar group: at most
    /// [`MAX_BAR_SERIES`](crate::categorical::MAX_BAR_SERIES) of them, each
    /// with its own value column, drawn against a categorical axis
    /// (`bottom_x` for vertical bars, `left_y` for horizontal ones) and a
    /// linear numeric value axis, with no cap on the number of bars (#107),
    /// and each series passes [`validate_category_bar_series`] against the
    /// axis' categories. Horizontal bars cannot sit on an overlay layer,
    /// whose own axis is a value axis on y, and a polar chart draws no
    /// category bars. Whether a column holds one value per category is
    /// data: the renderer checks it against the pool when it prepares.
    pub fn validate_category_bars(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        use crate::categorical::MAX_BAR_SERIES;
        use crate::layer::LayerRef;
        if let Some(groups) = &self.bar_groups {
            groups.validate()?;
        }
        for subplot in &self.subplots {
            if let Some(groups) = &subplot.config.bar_groups {
                groups.validate()?;
            }
        }
        let bars = || {
            series
                .iter()
                .filter(|s| s.render_type.category_bars().is_some())
        };
        if bars().next().is_none() {
            return Ok(());
        }
        if self.polar.is_some() {
            return Err("category bars are not drawn on polar axes");
        }
        for layer in self.panel_layers() {
            let members = || bars().filter(|s| self.layer_of_series(&s.series_id) == layer);
            let count = members().count();
            if count == 0 {
                continue;
            }
            if count > MAX_BAR_SERIES {
                return Err("a bar group holds at most 16 category bar series");
            }
            let groups = self.bar_groups_of(layer);
            let horizontal = groups.direction == CategoryBarDirection::Horizontal;
            let overlay = matches!(layer, LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..));
            if horizontal && overlay {
                return Err(
                    "horizontal category bars cannot sit on an overlay layer: its own axis is a value axis on y",
                );
            }
            // The axes as the layer draws them: a subplot with its links.
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            let category_axis = if horizontal {
                &chart.left_y
            } else {
                &chart.bottom_x
            };
            let value_axis = match layer {
                LayerRef::Overlay(index) | LayerRef::SubplotOverlay(_, index) => {
                    &chart.overlays[index].y_axis
                }
                _ if horizontal => &chart.bottom_x,
                _ => &chart.left_y,
            };
            let Some(categories) = category_axis.categories.as_deref() else {
                return Err(
                    "category bars need a categorical axis: bottom_x for vertical bars, left_y for horizontal ones",
                );
            };
            if value_axis.categories.is_some() || value_axis.scale == AxisScale::Logarithmic {
                return Err("category bars need a linear numeric value axis");
            }
            for (k, item) in members().enumerate() {
                validate_category_bar_series(item, categories)?;
                let (_, values) = item.category_bar_columns(groups.direction);
                if members()
                    .take(k)
                    .any(|earlier| earlier.category_bar_columns(groups.direction).1 == values)
                {
                    return Err("the category bar series of a group need distinct value columns");
                }
            }
        }
        Ok(())
    }
}

/// Write `labels.text` of every [`DataRenderType::CategoryBars`] series in
/// `series` from the values the host holds for it: the edit-time helper
/// the renderer relies on, since it never formats a value.
///
/// `value(k, i)` is the value of category `i` of the `k`-th category bar
/// series in `series` (`None` for a missing bar). `categories` are the
/// axis' categories, whose order the text follows. A bar's text follows its
/// override's `label_format`, else the series' one: the value with
/// `labels.decimals` fixed decimals (trailing zeros dropped) and
/// `labels.value_suffix`, its share of the category total, both, or the
/// category label with the value. The share is that of the bars of all the
/// category bar series in `series` at that category, and reads `—` where a
/// category has a negative value or no positive total. A missing bar's text
/// is empty. Other series are left alone.
pub fn format_labels(
    series: &mut [SeriesConfig],
    categories: &[Category],
    value: impl Fn(usize, usize) -> Option<f64>,
) {
    let members = series
        .iter()
        .filter(|s| s.render_type.category_bars().is_some())
        .count();
    let share = |k: usize, i: usize| -> Option<f64> {
        let v = value(k, i)?;
        // Normalize before summing so the shares stay finite even when the
        // raw total overflows.
        let mut max = 0.0f64;
        let mut negative = false;
        for m in 0..members {
            if let Some(x) = value(m, i) {
                max = max.max(x);
                negative |= x < 0.0;
            }
        }
        if negative || max <= 0.0 {
            return None;
        }
        let total: f64 = (0..members)
            .filter_map(|m| value(m, i))
            .map(|x| x / max)
            .sum();
        (total > 0.0).then(|| v / max / total * 100.0)
    };
    let mut k = 0;
    for item in series.iter_mut() {
        let series_id = item.series_id.clone();
        let DataRenderType::CategoryBars {
            labels,
            label_format,
            overrides,
            ..
        } = &mut item.render_type
        else {
            continue;
        };
        let number = crate::format::PlainNumberFormat::fixed(labels.decimals, true);
        labels.text.clear();
        for (i, category) in categories.iter().enumerate() {
            let Some(v) = value(k, i) else {
                labels.text.push(String::new());
                continue;
            };
            let format = overrides
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .find(|o| o.target.series_id == series_id && o.target.category_id == category.id)
                .and_then(|o| o.label_format)
                .unwrap_or(*label_format);
            let written = format!("{}{}", number.format(v), labels.value_suffix);
            let percent = share(k, i)
                .map(|p| format!("{}%", number.format(p)))
                .unwrap_or_else(|| "—".into());
            labels.text.push(match format {
                CategoryBarLabelFormat::Value => written,
                CategoryBarLabelFormat::Percent => percent,
                CategoryBarLabelFormat::ValuePercent => format!("{written} ({percent})"),
                CategoryBarLabelFormat::CategoryValue => {
                    format!("{}: {written}", category.label)
                }
            });
        }
        k += 1;
    }
}

// ───── Box plots ─────

/// One box: the id of its category and the id of its series.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPlotTarget {
    pub category_id: String,
    pub series_id: String,
}

impl BoxPlotTarget {
    pub fn new(category: impl Into<String>, series: impl Into<String>) -> Self {
        Self {
            category_id: category.into(),
            series_id: series.into(),
        }
    }
}

/// Which value labels a box shows: none, its median, or its median and its
/// sample count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxPlotLabels {
    #[default]
    None,
    Median,
    MedianAndCount,
}

/// The shape of a box's outlier markers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxOutlierShape {
    #[default]
    Circle,
    Square,
}

/// The look of a box: [`DataRenderType::BoxPlot`]'s `style`, and a complete
/// per-box override. Widths and sizes are logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BoxPlotStyle {
    /// A box is a flat body: [`Material::supported_on_flat_body`] decides.
    pub material: Material,
    /// Logical pixels, capped at 3 and at half the actual box extent.
    pub corner_radius: f32,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
    pub median_width: f32,
    pub median_color: Color,
    pub whisker_width: f32,
    pub whisker_color: Color,
    pub caps: bool,
    /// The cap's length as a share of the box thickness.
    pub cap_ratio: f32,
    /// Notch the box at the median's confidence interval (`ci_low`,
    /// `ci_high`); a box without one is drawn plain.
    pub notched: bool,
    /// The notch's depth as a share of the box thickness.
    pub notch_depth: f32,
    pub show_mean: bool,
    pub mean_size: f32,
    pub mean_color: Color,
    pub show_outliers: bool,
    pub outlier_size: f32,
    pub outlier_color: Color,
    pub outlier_shape: BoxOutlierShape,
    pub outlier_filled: bool,
    pub texture_strength: f32,
    pub texture_scale: f32,
    pub gloss: f32,
    /// The brightness a picked box gains.
    pub emphasis_brightness: f32,
}

impl Default for BoxPlotStyle {
    fn default() -> Self {
        let ink = Color::from_rgb8(32, 46, 60);
        Self {
            material: Material::Matte,
            corner_radius: 0.0,
            outline: true,
            outline_width: 1.2,
            outline_color: ink,
            median_width: 1.2,
            median_color: ink,
            whisker_width: 1.2,
            whisker_color: ink,
            caps: true,
            cap_ratio: 0.5,
            notched: false,
            notch_depth: 0.22,
            show_mean: false,
            mean_size: 7.0,
            mean_color: Color::from_rgb8(180, 45, 45),
            show_outliers: true,
            outlier_size: 7.0,
            outlier_color: ink,
            outlier_shape: BoxOutlierShape::Circle,
            outlier_filled: false,
            texture_strength: 0.2,
            texture_scale: 1.0,
            gloss: 0.45,
            emphasis_brightness: 0.08,
        }
    }
}

impl BoxPlotStyle {
    /// The rules a box style is held to at every level (series, override):
    /// a material with a look on a flat body, a corner radius in `0..=3`,
    /// outline, median and whisker widths in `0..=8`, a cap ratio in `0..=1`,
    /// a notch depth in `0..=0.45`, mean and outlier sizes in `1..=24`,
    /// texture strength and gloss in `0..=1`, a texture scale in `0.1..=8`,
    /// an emphasis brightness in `0..=0.5` and colours with channels in
    /// `0..=1`.
    pub fn validate(&self) -> Result<(), &'static str> {
        let Self {
            material,
            corner_radius,
            outline: _,
            outline_width,
            outline_color,
            median_width,
            median_color,
            whisker_width,
            whisker_color,
            caps: _,
            cap_ratio,
            notched: _,
            notch_depth,
            show_mean: _,
            mean_size,
            mean_color,
            show_outliers: _,
            outlier_size,
            outlier_color,
            outlier_shape: _,
            outlier_filled: _,
            texture_strength,
            texture_scale,
            gloss,
            emphasis_brightness,
        } = self;
        if !material.supported_on_flat_body() {
            return Err("material not supported by boxes");
        }
        if finite_in(*corner_radius, 0.0, 3.0)
            && finite_in(*outline_width, 0.0, 8.0)
            && finite_in(*median_width, 0.0, 8.0)
            && finite_in(*whisker_width, 0.0, 8.0)
            && finite_in(*cap_ratio, 0.0, 1.0)
            && finite_in(*notch_depth, 0.0, 0.45)
            && finite_in(*mean_size, 1.0, 24.0)
            && finite_in(*outlier_size, 1.0, 24.0)
            && finite_in(*texture_strength, 0.0, 1.0)
            && finite_in(*texture_scale, 0.1, 8.0)
            && finite_in(*gloss, 0.0, 1.0)
            && finite_in(*emphasis_brightness, 0.0, 0.5)
            && [
                *outline_color,
                *median_color,
                *whisker_color,
                *mean_color,
                *outlier_color,
            ]
            .into_iter()
            .all(|c| valid_bar_color(c, false))
        {
            Ok(())
        } else {
            Err("invalid boxplot style")
        }
    }
}

/// A per-box override. `None` fields keep the series' value; `style`
/// replaces the series' style whole.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPlotOverride {
    pub target: BoxPlotTarget,
    pub color: Option<Color>,
    pub style: Option<BoxPlotStyle>,
    pub labels: Option<BoxPlotLabels>,
}

impl BoxPlotOverride {
    pub fn new(target: BoxPlotTarget) -> Self {
        Self {
            target,
            color: None,
            style: None,
            labels: None,
        }
    }
}

/// The most characters one drawn box label may have.
pub const MAX_BOX_PLOT_LABEL_CHARS: usize = 160;

/// The value labels of a [`DataRenderType::BoxPlot`] series: which labels
/// its boxes show, how a value is written, and the drawn text of every box.
///
/// `text` is the string each box shows, one per category in axis order (an
/// empty string draws nothing), or empty when the labels were not written.
/// The median and the sample count are per-box data the configuration does
/// not hold, so the renderer never writes this text: a host that uploads
/// its own records writes it (for example with
/// [`format_box_plot_labels`] from the statistics it holds), and
/// `Config::from_boxplot` writes it from the document's summaries.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BoxPlotSeriesLabels {
    pub placement: BoxPlotLabels,
    /// Fixed decimals of a written median, `0..=6`.
    pub decimals: usize,
    /// Appended to a written median; one line of at most 24 characters.
    pub value_suffix: String,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub text: Vec<String>,
    /// Where each label was dragged to, in logical pixels from where it is
    /// placed (x right, y down), one per category in axis order, or empty
    /// when no label was dragged. The renderer adds the offset to the
    /// label's placement on every draw; a drag writes it
    /// ([`SeriesConfig::drag_value_label`]).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub offsets: Vec<[f32; 2]>,
}

impl Default for BoxPlotSeriesLabels {
    fn default() -> Self {
        Self {
            placement: BoxPlotLabels::None,
            decimals: 1,
            value_suffix: String::new(),
            text: Vec::new(),
            offsets: Vec::new(),
        }
    }
}

/// The fields of a [`DataRenderType::BoxPlot`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct BoxPlotRef<'a> {
    pub color: Color,
    pub style: &'a BoxPlotStyle,
    pub labels: &'a BoxPlotSeriesLabels,
    pub outliers: Option<&'a str>,
    pub overrides: &'a [BoxPlotOverride],
}

impl DataRenderType {
    /// The fields of a box plot series, or `None` for every other kind.
    pub fn box_plot(&self) -> Option<BoxPlotRef<'_>> {
        match self {
            DataRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
            } => Some(BoxPlotRef {
                color: *color,
                style,
                labels,
                outliers: outliers.as_deref(),
                overrides: overrides.as_deref().unwrap_or(&[]),
            }),
            _ => None,
        }
    }
}

impl SeriesConfig {
    /// The column of a box plot series that holds the category positions
    /// and the id of its record: `x_column` and `y_column`, whatever the
    /// direction.
    pub fn box_plot_columns(&self) -> (&str, &str) {
        (&self.x_column, &self.y_column)
    }
}

/// The rules one box plot series is held to against the `categories` of its
/// axis: an opaque colour, a valid style, label decimals `0..=6`, a one-line
/// value suffix of at most 24 characters, `labels.text` empty or one
/// one-line string of at most [`MAX_BOX_PLOT_LABEL_CHARS`] characters per
/// category, a record id, an outlier column id that is not empty when set,
/// and overrides that name one of the categories and this series, at most
/// one per box, with an opaque colour and a valid style.
pub fn validate_box_plot_series(
    series: &SeriesConfig,
    categories: &[Category],
) -> Result<(), &'static str> {
    let Some(boxes) = series.render_type.box_plot() else {
        return Ok(());
    };
    if !valid_bar_color(boxes.color, true) {
        return Err("a box colour must be opaque with channels in 0..=1");
    }
    boxes.style.validate()?;
    if boxes.labels.decimals > 6 || !single_line_within(&boxes.labels.value_suffix, 24) {
        return Err(
            "box labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        );
    }
    if !boxes.labels.text.is_empty() && boxes.labels.text.len() != categories.len() {
        return Err("box label text needs one string per category");
    }
    validate_value_label_offsets(&boxes.labels.offsets, categories.len())?;
    if boxes
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_BOX_PLOT_LABEL_CHARS))
    {
        return Err("a box label is one line of at most 160 characters");
    }
    let (positions, record) = series.box_plot_columns();
    if positions.is_empty() || record.is_empty() || boxes.outliers.is_some_and(str::is_empty) {
        return Err(
            "a box plot series names its category positions (x_column), its record (y_column) \
             and, when set, its outlier column",
        );
    }
    for (index, o) in boxes.overrides.iter().enumerate() {
        if o.target.series_id != series.series_id
            || !categories.iter().any(|c| c.id == o.target.category_id)
            || boxes.overrides[..index]
                .iter()
                .any(|e| e.target == o.target)
        {
            return Err("a box override names a category of the axis and its own series, once");
        }
        if o.color.is_some_and(|c| !valid_bar_color(c, true)) {
            return Err("a box colour must be opaque with channels in 0..=1");
        }
        if let Some(style) = &o.style {
            style.validate()?;
        }
    }
    Ok(())
}

impl crate::config::Config {
    /// The rules the box plot series of a chart are held to, with the series
    /// list it draws.
    ///
    /// The [`DataRenderType::BoxPlot`] series of one layer are its box
    /// group: they stand side by side in series order, laid out by the
    /// layer's `bar_groups` ([`crate::config::Config::bar_groups_of`]), whose mode must be
    /// [`CategoryBarMode::Grouped`] (boxes do not stack). At most
    /// [`MAX_BOX_SERIES`](crate::boxplot::MAX_BOX_SERIES) of them, drawn
    /// against a categorical axis (`bottom_x` for vertical boxes, `left_y`
    /// for horizontal ones) and a linear numeric value axis, with no cap on
    /// the number of boxes (#107); a layer draws either
    /// category bars or box plots. Each series passes
    /// [`validate_box_plot_series`] against the axis' categories.
    /// Horizontal boxes cannot sit on an overlay layer, and a polar chart
    /// draws no box plots. Whether the record holds one row per category is
    /// data: the renderer checks it against the pool when it prepares.
    pub fn validate_box_plots(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        use crate::boxplot::MAX_BOX_SERIES;
        use crate::layer::LayerRef;
        let boxes = || series.iter().filter(|s| s.render_type.box_plot().is_some());
        if boxes().next().is_none() {
            return Ok(());
        }
        if self.polar.is_some() {
            return Err("box plots are not drawn on polar axes");
        }
        for layer in self.panel_layers() {
            let members = || boxes().filter(|s| self.layer_of_series(&s.series_id) == layer);
            let count = members().count();
            if count == 0 {
                continue;
            }
            if count > MAX_BOX_SERIES {
                return Err("a box group holds at most 16 box plot series");
            }
            if series.iter().any(|s| {
                s.render_type.category_bars().is_some()
                    && self.layer_of_series(&s.series_id) == layer
            }) {
                return Err("a layer draws either category bars or box plots, not both");
            }
            let groups = self.bar_groups_of(layer);
            if groups.mode != CategoryBarMode::Grouped {
                return Err(
                    "box plots stand side by side: their bar_groups mode must be Grouped, not \
                     Stacked or PercentStacked",
                );
            }
            let horizontal = groups.direction == CategoryBarDirection::Horizontal;
            let overlay = matches!(layer, LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..));
            if horizontal && overlay {
                return Err(
                    "horizontal box plots cannot sit on an overlay layer: its own axis is a value axis on y",
                );
            }
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            let category_axis = if horizontal {
                &chart.left_y
            } else {
                &chart.bottom_x
            };
            let value_axis = match layer {
                LayerRef::Overlay(index) | LayerRef::SubplotOverlay(_, index) => {
                    &chart.overlays[index].y_axis
                }
                _ if horizontal => &chart.bottom_x,
                _ => &chart.left_y,
            };
            let Some(categories) = category_axis.categories.as_deref() else {
                return Err(
                    "box plots need a categorical axis: bottom_x for vertical boxes, left_y for horizontal ones",
                );
            };
            if value_axis.categories.is_some() || value_axis.scale == AxisScale::Logarithmic {
                return Err("box plots need a linear numeric value axis");
            }
            for item in members() {
                validate_box_plot_series(item, categories)?;
            }
        }
        Ok(())
    }
}

/// Write `labels.text` of every [`DataRenderType::BoxPlot`] series in
/// `series` from the statistics the host holds for it: the edit-time helper
/// a host that uploads records uses, since the renderer never formats a
/// value and the configuration holds no per-box statistics.
///
/// `value(k, i)` is the median and the sample count of category `i` of the
/// `k`-th box plot series in `series` (`None` for a missing box).
/// `categories` are the axis' categories, whose order the text follows. A
/// box's text follows its override's `labels`, else the series'
/// `labels.placement`: nothing, the median with `labels.decimals` fixed
/// decimals and `labels.value_suffix`, or that and `(n=<count>)`, `—` for an
/// unknown count. A missing box's text is empty. Other series are left
/// alone.
pub fn format_box_plot_labels(
    series: &mut [SeriesConfig],
    categories: &[Category],
    value: impl Fn(usize, usize) -> Option<(f64, Option<u64>)>,
) {
    let mut k = 0;
    for item in series.iter_mut() {
        let series_id = item.series_id.clone();
        let DataRenderType::BoxPlot {
            labels, overrides, ..
        } = &mut item.render_type
        else {
            continue;
        };
        let number = crate::format::PlainNumberFormat::fixed(labels.decimals, false);
        labels.text.clear();
        for (i, category) in categories.iter().enumerate() {
            let mode = overrides
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .find(|o| o.target.series_id == series_id && o.target.category_id == category.id)
                .and_then(|o| o.labels)
                .unwrap_or(labels.placement);
            let text = match (value(k, i), mode) {
                (None, _) | (_, BoxPlotLabels::None) => String::new(),
                (Some((median, _)), BoxPlotLabels::Median) => {
                    format!("{}{}", number.format(median), labels.value_suffix)
                }
                (Some((median, count)), BoxPlotLabels::MedianAndCount) => format!(
                    "{}{} (n={})",
                    number.format(median),
                    labels.value_suffix,
                    count.map_or_else(|| "—".into(), |n| n.to_string())
                ),
            };
            labels.text.push(text);
        }
        k += 1;
    }
}

/// The marker drawn at every vertex of a radar series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadarMarker {
    /// No marker.
    None,
    /// A filled circle (the default).
    #[default]
    Circle,
    /// A filled square.
    Square,
}

/// Which vertices of a radar series carry value labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RadarLabels {
    /// No data labels (the default).
    #[default]
    None,
    /// Every vertex of every series.
    Values,
    /// The vertices of the hovered or selected series only.
    Emphasized,
}

/// The look of a radar series. Every field is optional: an unset field
/// takes its default ([`Self::DEFAULT_LINE_WIDTH_PX`] and the other
/// `DEFAULT_*` constants), and a legacy [`RadarChart`](crate::radar::RadarChart)
/// series reads an unset field from its chart style
/// ([`Self::resolved_from`]).
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct RadarSeriesStyle {
    /// Outline width in logical pixels, `0..=12`; zero draws no outline.
    pub line_width_px: Option<f32>,
    /// The polygon fill. `None` fills with the series colour at
    /// `fill_opacity`; a transparent colour draws the outline only.
    pub fill: Option<Color>,
    /// The marker at every vertex.
    pub marker: Option<RadarMarker>,
    /// Marker diameter (circle) or side length (square) in logical pixels,
    /// `1..=24`.
    pub marker_size_px: Option<f32>,
    /// Alpha factor of the series colour the fill takes when `fill` is
    /// unset, `0..=1`.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub fill_opacity: Option<f32>,
}

impl RadarSeriesStyle {
    /// The outline width when `line_width_px` is unset.
    pub const DEFAULT_LINE_WIDTH_PX: f32 = 2.0;
    /// The fill opacity when `fill_opacity` is unset.
    pub const DEFAULT_FILL_OPACITY: f32 = 0.25;
    /// The marker when `marker` is unset.
    pub const DEFAULT_MARKER: RadarMarker = RadarMarker::Circle;
    /// The marker size when `marker_size_px` is unset.
    pub const DEFAULT_MARKER_SIZE_PX: f32 = 7.0;

    /// The outline width in logical pixels.
    pub fn line_width(&self) -> f32 {
        self.line_width_px.unwrap_or(Self::DEFAULT_LINE_WIDTH_PX)
    }

    /// The fill colour of a series in `color`: `fill`, or `color` with its
    /// alpha multiplied by the fill opacity.
    pub fn fill_color(&self, color: Color) -> Color {
        self.fill.unwrap_or(Color {
            a: color.a * self.fill_opacity.unwrap_or(Self::DEFAULT_FILL_OPACITY),
            ..color
        })
    }

    /// The vertex marker.
    pub fn marker(&self) -> RadarMarker {
        self.marker.unwrap_or(Self::DEFAULT_MARKER)
    }

    /// The marker diameter or side length in logical pixels.
    pub fn marker_size(&self) -> f32 {
        self.marker_size_px.unwrap_or(Self::DEFAULT_MARKER_SIZE_PX)
    }

    /// This style with every unset field read from a legacy chart style:
    /// what a legacy radar series draws with. The chart style's
    /// `emphasis_brightness` has no counterpart: the picked ring
    /// (`Config.picked_data`) is the highlight of a picked vertex, as on
    /// every polar line or marker, and no series is brightened.
    pub fn resolved_from(&self, chart: &crate::radar::RadarStyle) -> RadarSeriesStyle {
        RadarSeriesStyle {
            line_width_px: Some(self.line_width_px.unwrap_or(chart.line_width_px)),
            fill: self.fill,
            marker: Some(self.marker.unwrap_or(chart.marker)),
            marker_size_px: Some(self.marker_size_px.unwrap_or(chart.marker_size_px)),
            fill_opacity: Some(self.fill_opacity.unwrap_or(chart.fill_opacity)),
        }
    }

    /// The ranges a set field is held to: an outline width in `0..=12`, a
    /// fill colour with channels in `0..=1`, a marker size in `1..=24` and
    /// a fill opacity in `0..=1`.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.line_width_px.is_some_and(|v| !finite_in(v, 0.0, 12.0))
            || self.fill.is_some_and(|c| !valid_bar_color(c, false))
            || self
                .marker_size_px
                .is_some_and(|v| !finite_in(v, 1.0, 24.0))
            || self.fill_opacity.is_some_and(|v| !finite_in(v, 0.0, 1.0))
        {
            return Err("invalid radar series style");
        }
        Ok(())
    }
}

/// The value labels of a radar series: which vertices show one, how a
/// value is written, and the drawn text of each vertex.
///
/// `text` holds one string per category, written at edit time by
/// [`format_radar_labels`] (or by a document reader); the renderer draws
/// strings and never formats a value. It is empty until written.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct RadarSeriesLabels {
    pub placement: RadarLabels,
    /// Fixed decimals of a written value, `0..=6`.
    pub decimals: usize,
    /// Appended to a written value; one line of at most 24 characters.
    pub value_suffix: String,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub text: Vec<String>,
    /// Where each label was dragged to, in logical pixels from where it is
    /// placed (x right, y down), one per category in axis order, or empty
    /// when no label was dragged. The renderer adds the offset to the
    /// label's placement on every draw; a drag writes it
    /// ([`SeriesConfig::drag_value_label`]).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub offsets: Vec<[f32; 2]>,
}

impl Default for RadarSeriesLabels {
    fn default() -> Self {
        Self {
            placement: RadarLabels::None,
            decimals: 1,
            value_suffix: String::new(),
            text: Vec::new(),
            offsets: Vec::new(),
        }
    }
}

/// The most characters of one radar value label.
pub const MAX_RADAR_LABEL_CHARS: usize = 160;

/// The baseline a radar fill reaches: a value below every radius axis, which
/// the polar projection puts on the inner ring (the centre when
/// `inner_radius_fraction` is zero). Finite in both lanes of its `(hi, lo)`
/// pair, so the fill's sample check keeps it.
pub const RADAR_FILL_BASELINE: f64 = -(f32::MAX as f64);

/// The fields of a [`DataRenderType::Radar`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct RadarRef<'a> {
    pub color: Color,
    pub style: &'a RadarSeriesStyle,
    pub labels: &'a RadarSeriesLabels,
}

impl RadarRef<'_> {
    /// The polar line a radar series' outline and fill are drawn as: a
    /// solid, linearly interpolated line in the series colour at the
    /// outline width (transparent at width zero) whose fill reaches the
    /// inner ring ([`RADAR_FILL_BASELINE`]) in the fill colour, or no fill
    /// when that colour is transparent. The renderer draws it closed.
    pub fn line_style(&self) -> DataLineStyleConfig {
        let width = self.style.line_width();
        let fill = self.style.fill_color(self.color);
        DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: if width > 0.0 {
                self.color
            } else {
                Color {
                    a: 0.0,
                    ..self.color
                }
            },
            line_width: width,
            fill: (fill.a > 0.0).then_some(LineFill::ToBaseline(RADAR_FILL_BASELINE)),
            fill_color: Some(fill),
            interpolation: LineInterpolation::Linear,
        }
    }

    /// The markers a radar series' vertices are drawn with, `None` without
    /// a marker: filled circles of the marker size as diameter, or filled
    /// squares of the marker size as side length, in the series colour.
    pub fn scatter_style(&self) -> Option<DataScatterStyleConfig> {
        let half = self.style.marker_size() * 0.5;
        let (point_shape, point_size) = match self.style.marker() {
            RadarMarker::None => return None,
            RadarMarker::Circle => (ScatterShape::CircleFilled, half),
            // The shared marker sizes a square by the circle of equal area
            // (`visual_shape_radius`): its half side is `sqrt(pi) / 2` of
            // the radius.
            RadarMarker::Square => (ScatterShape::SquareFilled, half / SQUARE_HALF_SIDE),
        };
        Some(DataScatterStyleConfig {
            point_color: self.color,
            point_shape,
            point_size,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        })
    }
}

/// The half side of a filled square marker of radius 1 (`sqrt(pi) / 2`,
/// SHADER_COMMON.md §5.5).
const SQUARE_HALF_SIDE: f32 = 0.886_226_95;

impl DataRenderType {
    /// The fields of a radar series, or `None` for every other kind.
    pub fn radar(&self) -> Option<RadarRef<'_>> {
        match self {
            DataRenderType::Radar {
                color,
                style,
                labels,
            } => Some(RadarRef {
                color: *color,
                style,
                labels,
            }),
            _ => None,
        }
    }
}

/// The rules one radar series is held to against the `categories` of its
/// angle axis: a colour with channels in `0..=1`, a valid style, label
/// decimals `0..=6`, a one-line value suffix of at most 24 characters,
/// `labels.text` empty or one one-line string of at most
/// [`MAX_RADAR_LABEL_CHARS`] characters per category, and named category
/// index and value columns.
pub fn validate_radar_series(
    series: &SeriesConfig,
    categories: &[Category],
) -> Result<(), &'static str> {
    let Some(radar) = series.render_type.radar() else {
        return Ok(());
    };
    if !valid_bar_color(radar.color, false) {
        return Err("a radar colour needs channels in 0..=1");
    }
    radar.style.validate()?;
    if radar.labels.decimals > 6 || !single_line_within(&radar.labels.value_suffix, 24) {
        return Err(
            "radar labels need 0..=6 decimals and a one-line suffix of at most 24 characters",
        );
    }
    if !radar.labels.text.is_empty() && radar.labels.text.len() != categories.len() {
        return Err("radar label text needs one string per category");
    }
    validate_value_label_offsets(&radar.labels.offsets, categories.len())?;
    if radar
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_RADAR_LABEL_CHARS))
    {
        return Err("a radar label is one line of at most 160 characters");
    }
    if series.x_column.is_empty() || series.y_column.is_empty() {
        return Err(
            "a radar series names its category index column (x_column) and its value column \
             (y_column)",
        );
    }
    Ok(())
}

impl crate::config::Config {
    /// The rules the radar series of a chart are held to, with the series
    /// list it draws.
    ///
    /// A radar series is drawn on a polar chart (`Config.polar`, the
    /// parent's or a subplot's own) whose angle axis (`bottom_x`) is
    /// categorical, with chord segments
    /// ([`PolarInterpolation::Chord`](crate::polar::PolarInterpolation)):
    /// a polygon's edges are straight. At most
    /// [`MAX_RADAR_SERIES`](crate::radar::MAX_RADAR_SERIES) radar series
    /// share one chart, and each passes [`validate_radar_series`] against
    /// the angle axis' categories. Whether a column holds one value per
    /// category is data: the renderer checks it against the pool when it
    /// prepares.
    pub fn validate_radar(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        use crate::layer::LayerRef;
        use crate::radar::MAX_RADAR_SERIES;
        let radars = || series.iter().filter(|s| s.render_type.radar().is_some());
        if radars().next().is_none() {
            return Ok(());
        }
        for layer in self.panel_layers() {
            let members = || radars().filter(|s| self.layer_of_series(&s.series_id) == layer);
            let count = members().count();
            if count == 0 {
                continue;
            }
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            let Some(polar) = &chart.polar else {
                return Err("radar series are drawn on a polar chart (Config.polar)");
            };
            if count > MAX_RADAR_SERIES {
                return Err("a radar chart draws at most 16 radar series");
            }
            let Some(categories) = chart.bottom_x.categories.as_deref() else {
                return Err(
                    "radar series need a categorical angle axis: bottom_x.categories on the \
                     polar chart",
                );
            };
            if !polar.interpolation.is_chord() {
                return Err(
                    "radar polygons have straight edges: the polar interpolation must be Chord",
                );
            }
            for item in members() {
                validate_radar_series(item, categories)?;
            }
        }
        Ok(())
    }
}

/// Write `labels.text` of every [`DataRenderType::Radar`] series in
/// `series` from the values the host holds for it: the edit-time helper
/// the renderer relies on, since it never formats a value.
///
/// `value(k, i)` is the value of category `i` of the `k`-th radar series in
/// `series` (`None` for a missing value). `categories` are the angle axis'
/// categories, whose order the text follows. With `labels.placement`
/// [`RadarLabels::None`] every text is empty; otherwise a vertex's text is
/// its value with `labels.decimals` fixed decimals and
/// `labels.value_suffix`, and a missing or non-finite value's text is
/// empty. Other series are left alone.
pub fn format_radar_labels(
    series: &mut [SeriesConfig],
    categories: &[Category],
    value: impl Fn(usize, usize) -> Option<f64>,
) {
    let mut k = 0;
    for item in series.iter_mut() {
        let DataRenderType::Radar { labels, .. } = &mut item.render_type else {
            continue;
        };
        let number = crate::format::PlainNumberFormat::fixed(labels.decimals, false);
        labels.text.clear();
        for i in 0..categories.len() {
            let text = match (labels.placement, value(k, i)) {
                (RadarLabels::None, _) | (_, None) => String::new(),
                (_, Some(v)) if !v.is_finite() => String::new(),
                (_, Some(v)) => format!("{}{}", number.format(v), labels.value_suffix),
            };
            labels.text.push(text);
        }
        k += 1;
    }
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

impl DataRenderType {
    /// Whether a series of this kind draws one selectable point per source
    /// index, the selection [`crate::config::Config::step_selection`] steps
    /// a [`PickedDataRef::Point`](crate::config::PickedDataRef::Point)
    /// across: every kind with a line or markers, a radar series (its
    /// vertices), a streamline series (its vertices) and a point vector (its
    /// anchors, which pick and highlight like markers).
    fn steps_points(&self) -> bool {
        match self {
            DataRenderType::Scatter { .. }
            | DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::ScatterErrorbarX { .. }
            | DataRenderType::ScatterErrorbarY { .. }
            | DataRenderType::ScatterErrorbarXY { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. }
            | DataRenderType::Radar { .. }
            // Its vertices, NaN separators included, as a line's (D4).
            | DataRenderType::Streamlines { .. } => true,
            DataRenderType::Vector { components, .. } => {
                matches!(components, VectorComponents::Points { .. })
            }
            DataRenderType::Histogram { .. }
            | DataRenderType::Heatmap { .. }
            | DataRenderType::Contour { .. }
            | DataRenderType::HeatmapContour { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::BoxPlot { .. } => false,
        }
    }
}

impl crate::config::Config {
    /// Move the first typed data selection (`picked_data.refs[0]`) by
    /// source indices, over the series list `series` the chart draws, and
    /// return whether it moved. `column_len` gives the length of a
    /// registered column (`None` for a column that is not registered,
    /// which counts as empty). Indices clamp to the selected series; nothing
    /// wraps.
    ///
    /// - A point (of a series that draws points: lines, markers, radar
    ///   vertices, streamline vertices and point vectors) moves along its
    ///   series by `dx`, and `dy` moves to another point-drawing series, in
    ///   series order. A streamline's NaN separator rows count as points. A
    ///   point vector counts only the points where its `u` and `v` columns
    ///   have a value too.
    /// - A category bar moves along its categories by the delta of the
    ///   pointer axis its category axis runs along (`dx` for vertical bars,
    ///   `dy` for horizontal ones) and across the category bar series of its
    ///   layer by the other; a box moves the same way across the box plot
    ///   series of its layer (boxes, never their outliers).
    /// - A histogram bin moves along its bins by `dx` for vertical bars and
    ///   `dy` for horizontal ones, and across the histograms of the same
    ///   orientation by the other.
    /// - A matrix cell (a field's or a grid vector's, over its `u` grid)
    ///   moves by `dx` along x and `dy` along y.
    ///
    /// A contour level and a value label do not step. An error names a
    /// matrix cell selection whose series draws no matrix any more.
    pub fn step_selection(
        &mut self,
        series: &[SeriesConfig],
        dx: i32,
        dy: i32,
        column_len: impl Fn(&str) -> Option<usize>,
    ) -> Result<bool, &'static str> {
        let Some(next) = self.stepped_selection(series, dx, dy, &column_len)? else {
            return Ok(false);
        };
        let Some(current) = self.picked_data.as_mut().and_then(|s| s.refs.first_mut()) else {
            return Ok(false);
        };
        if *current == next {
            return Ok(false);
        }
        *current = next;
        Ok(true)
    }

    /// The selection [`Self::step_selection`] moves to, or `None` when it
    /// does not move.
    fn stepped_selection(
        &self,
        series: &[SeriesConfig],
        dx: i32,
        dy: i32,
        column_len: &dyn Fn(&str) -> Option<usize>,
    ) -> Result<Option<crate::config::PickedDataRef>, &'static str> {
        use crate::config::PickedDataRef;
        let Some(reference) = self.picked_data.as_ref().and_then(|s| s.refs.first()) else {
            return Ok(None);
        };
        let Some(position) = series
            .iter()
            .position(|s| s.series_id == reference.series_id())
        else {
            return Ok(None);
        };
        let len = |column: &str| column_len(column).unwrap_or(0);
        let shift = |index: usize, delta: i32, count: usize| {
            index
                .saturating_add_signed(delta as isize)
                .min(count.saturating_sub(1))
        };
        Ok(match reference {
            PickedDataRef::Point {
                series_id,
                point_index,
                ..
            } => {
                let points = || series.iter().filter(|s| s.render_type.steps_points());
                let Some(index) = points().position(|s| s.series_id == *series_id) else {
                    return Ok(None);
                };
                let Some(next) = points().nth(shift(index, dy, points().count())) else {
                    return Ok(None);
                };
                let mut count = len(&next.x_column).min(len(&next.y_column));
                // A point vector draws only where all four columns have a value.
                if let DataRenderType::Vector {
                    components: VectorComponents::Points { u, v },
                    ..
                } = &next.render_type
                {
                    count = count.min(len(u)).min(len(v));
                }
                Some(PickedDataRef::Point {
                    series_id: next.series_id.clone(),
                    source_id: next.source_id.clone(),
                    point_index: shift(*point_index, dx, count),
                })
            }
            PickedDataRef::HistogramBin {
                series_id,
                bin_index,
                ..
            } => {
                let selected = &series[position].render_type;
                let boxes = selected.box_plot().is_some();
                if selected.category_bars().is_some() || boxes {
                    let layer = self.layer_of_series(series_id);
                    let groups = self.bar_groups_of(layer);
                    let horizontal = groups.direction == CategoryBarDirection::Horizontal;
                    let members = || {
                        series.iter().filter(|s| {
                            (if boxes {
                                s.render_type.box_plot().is_some()
                            } else {
                                s.render_type.category_bars().is_some()
                            }) && self.layer_of_series(&s.series_id) == layer
                        })
                    };
                    let Some(index) = members().position(|s| s.series_id == *series_id) else {
                        return Ok(None);
                    };
                    let (bar_delta, series_delta) = if horizontal { (dy, dx) } else { (dx, dy) };
                    let Some(next) = members().nth(shift(index, series_delta, members().count()))
                    else {
                        return Ok(None);
                    };
                    // A box's record is not a registered column: its
                    // positions hold one value per category.
                    let count = if boxes {
                        len(next.box_plot_columns().0)
                    } else {
                        let (positions, values) = next.category_bar_columns(groups.direction);
                        len(positions).min(len(values))
                    };
                    if count == 0 {
                        return Ok(None);
                    }
                    Some(PickedDataRef::HistogramBin {
                        series_id: next.series_id.clone(),
                        source_id: next.source_id.clone(),
                        bin_index: shift(*bin_index, bar_delta, count),
                    })
                } else {
                    let DataRenderType::Histogram { bar } = selected else {
                        return Ok(None);
                    };
                    let horizontal = matches!(bar.orientation, BarOrientation::Horizontal);
                    let histograms = || {
                        series.iter().filter(|s| {
                            matches!(&s.render_type,
                                DataRenderType::Histogram { bar: other }
                                if other.orientation == bar.orientation)
                        })
                    };
                    let Some(index) = histograms().position(|s| s.series_id == *series_id) else {
                        return Ok(None);
                    };
                    let (bin_delta, series_delta) = if horizontal { (dy, dx) } else { (dx, dy) };
                    let Some(next) =
                        histograms().nth(shift(index, series_delta, histograms().count()))
                    else {
                        return Ok(None);
                    };
                    let (edges, values) = if horizontal {
                        (&next.y_column, &next.x_column)
                    } else {
                        (&next.x_column, &next.y_column)
                    };
                    let count = len(edges).saturating_sub(1).min(len(values));
                    if count == 0 {
                        return Ok(None);
                    }
                    Some(PickedDataRef::HistogramBin {
                        series_id: next.series_id.clone(),
                        source_id: next.source_id.clone(),
                        bin_index: shift(*bin_index, bin_delta, count),
                    })
                }
            }
            PickedDataRef::MatrixCell {
                source_id,
                series_id,
                x_index,
                y_index,
            } => {
                let selected = &series[position];
                // A field steps over its matrix; a grid vector over its `u`
                // grid, which every other grid of the series matches.
                let matrix = match &selected.render_type {
                    DataRenderType::Heatmap { matrix, .. }
                    | DataRenderType::Contour { matrix, .. }
                    | DataRenderType::HeatmapContour { matrix, .. } => matrix,
                    DataRenderType::Vector {
                        components: VectorComponents::Grid { u, .. },
                        ..
                    } => u,
                    _ => return Err("selected matrix no longer exists"),
                };
                let edges = usize::from(matches!(matrix.grid_layout, GridLayout::Edges));
                let depth = matrix
                    .columns
                    .iter()
                    .filter_map(|c| column_len(c))
                    .min()
                    .unwrap_or(0);
                let (nx, ny) = if matches!(matrix.orientation, MatrixOrientation::ColumnsAreY) {
                    (depth, matrix.columns.len())
                } else {
                    (matrix.columns.len(), depth)
                };
                Some(PickedDataRef::MatrixCell {
                    source_id: source_id.clone(),
                    series_id: series_id.clone(),
                    x_index: shift(
                        *x_index,
                        dx,
                        nx.min(len(&selected.x_column).saturating_sub(edges)),
                    ),
                    y_index: shift(
                        *y_index,
                        dy,
                        ny.min(len(&selected.y_column).saturating_sub(edges)),
                    ),
                })
            }
            PickedDataRef::ContourLevel { .. } | PickedDataRef::ValueLabel { .. } => None,
        })
    }
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

#[cfg(test)]
mod category_bar_tests {
    use super::*;
    use crate::categorical::MAX_BAR_SERIES;
    use crate::config::Config;
    use crate::layer::{OverlayLayer, Subplot};

    pub(crate) fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn bars(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("{id}.values"),
            render_type: DataRenderType::CategoryBars {
                color: Color::from_rgb8(40, 120, 200),
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels::default(),
                label_format: CategoryBarLabelFormat::Value,
                overrides: None,
            },
        }
    }

    pub(crate) fn chart(n: usize) -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn edit(series: &mut SeriesConfig, f: impl FnOnce(&mut CategoryBarStyle, &mut Color)) {
        let DataRenderType::CategoryBars { style, color, .. } = &mut series.render_type else {
            unreachable!()
        };
        f(style, color);
    }

    fn overrides(series: &mut SeriesConfig) -> &mut Vec<CategoryBarOverride> {
        let DataRenderType::CategoryBars { overrides, .. } = &mut series.render_type else {
            unreachable!()
        };
        overrides.get_or_insert_with(Vec::new)
    }

    fn labels(series: &mut SeriesConfig) -> &mut CategoryBarSeriesLabels {
        let DataRenderType::CategoryBars { labels, .. } = &mut series.render_type else {
            unreachable!()
        };
        labels
    }

    #[test]
    fn a_valid_group_passes_and_other_series_are_ignored() {
        let config = chart(3);
        assert_eq!(
            config.validate_category_bars(&[bars("a"), bars("b")]),
            Ok(())
        );
        assert_eq!(
            crate::default::default_config().validate_category_bars(&[]),
            Ok(())
        );
    }

    #[test]
    fn bars_need_a_categorical_axis_in_their_direction() {
        let config = crate::default::default_config();
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
    }

    #[test]
    fn group_limits_and_columns_are_checked() {
        let config = chart(4);
        let many: Vec<_> = (0..=MAX_BAR_SERIES)
            .map(|i| bars(&format!("s{i}")))
            .collect();
        assert!(config.validate_category_bars(&many).is_err());
        assert_eq!(
            config.validate_category_bars(&many[..MAX_BAR_SERIES]),
            Ok(())
        );
        // No bar cap (#107): 8 series over 65 categories (520 bars, past
        // the old 512) and 1 series over 600 categories are accepted.
        let eight: Vec<_> = (0..8).map(|i| bars(&format!("s{i}"))).collect();
        assert_eq!(chart(65).validate_category_bars(&eight), Ok(()));
        assert_eq!(chart(600).validate_category_bars(&eight[..1]), Ok(()));
        let mut shared = bars("b");
        shared.y_column = "a.values".into();
        assert!(
            chart(3)
                .validate_category_bars(&[bars("a"), shared])
                .is_err()
        );
    }

    #[test]
    fn the_value_axis_is_linear_and_numeric() {
        let mut config = chart(3);
        config.left_y.scale = AxisScale::Logarithmic;
        config.left_y.min = 1.0;
        config.left_y.max = 10.0;
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
        let mut config = chart(3);
        config.left_y.categories = Some(categories(2));
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
    }

    #[test]
    fn bar_groups_are_validated_on_the_chart_and_every_subplot() {
        for groups in [
            BarGroups {
                group_width: 0.05,
                ..Default::default()
            },
            BarGroups {
                group_width: 0.96,
                ..Default::default()
            },
            BarGroups {
                bar_gap: -1.0,
                ..Default::default()
            },
            BarGroups {
                bar_gap: f32::NAN,
                ..Default::default()
            },
            BarGroups {
                bar_gap: 24.5,
                ..Default::default()
            },
        ] {
            assert!(groups.validate().is_err(), "{groups:?}");
            let mut config = chart(3);
            config.bar_groups = Some(groups.clone());
            assert!(config.validate_category_bars(&[]).is_err());
            let mut config = chart(3);
            config.subplots.push(Subplot {
                config: chart(3),
                ..Default::default()
            });
            config.subplots[0].config.bar_groups = Some(groups);
            assert!(config.validate_category_bars(&[]).is_err());
        }
        assert_eq!(BarGroups::default().validate(), Ok(()));
    }

    #[test]
    fn overlays_and_subplots_hold_groups_of_their_own() {
        let mut config = chart(3);
        config.overlays.push(OverlayLayer {
            series: vec!["b".into()],
            ..Default::default()
        });
        assert_eq!(
            config.validate_category_bars(&[bars("a"), bars("b")]),
            Ok(())
        );
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
        assert!(
            config
                .validate_category_bars(&[bars("a"), bars("b")])
                .unwrap_err()
                .contains("overlay")
        );

        let mut config = crate::default::default_config();
        config.subplots.push(Subplot {
            config: chart(2),
            series: vec!["a".into()],
            ..Default::default()
        });
        assert_eq!(config.validate_category_bars(&[bars("a")]), Ok(()));
        // The parent has no categorical axis for a series of its own.
        assert!(
            config
                .validate_category_bars(&[bars("a"), bars("b")])
                .is_err()
        );
        assert_eq!(
            config.bar_groups_of(crate::layer::LayerRef::Subplot(0)),
            BarGroups::default()
        );
    }

    #[test]
    fn polar_charts_draw_no_category_bars() {
        let mut config = chart(3);
        config.polar = Some(Default::default());
        assert!(config.validate_category_bars(&[bars("a")]).is_err());
    }

    #[test]
    fn series_rules_hold_colours_styles_labels_and_overrides() {
        let cats = categories(2);
        assert_eq!(validate_category_bar_series(&bars("a"), &cats), Ok(()));
        let mut s = bars("a");
        edit(&mut s, |_, c| c.a = 0.5);
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        edit(&mut s, |style, _| style.material = Material::Toon);
        assert_eq!(
            validate_category_bar_series(&s, &cats),
            Err("material not supported by bars")
        );
        let edits: Vec<Box<dyn Fn(&mut CategoryBarStyle)>> = vec![
            Box::new(|s| s.corner_radius = 32.5),
            Box::new(|s| s.outline_width = -1.0),
            Box::new(|s| s.outline_color.a = 1.5),
            Box::new(|s| s.texture_strength = 1.5),
            Box::new(|s| s.texture_scale = 0.05),
            Box::new(|s| s.gloss = f32::NAN),
            Box::new(|s| s.emphasis_brightness = 0.6),
        ];
        for change in &edits {
            let mut s = bars("a");
            edit(&mut s, |style, _| change(style));
            assert!(validate_category_bar_series(&s, &cats).is_err());
            let mut s = bars("a");
            let mut style = CategoryBarStyle::default();
            change(&mut style);
            let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c0", "a"));
            o.style = Some(style);
            overrides(&mut s).push(o);
            assert!(validate_category_bar_series(&s, &cats).is_err());
        }
        let mut s = bars("a");
        labels(&mut s).decimals = 7;
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        labels(&mut s).value_suffix = "a\nb".into();
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        labels(&mut s).text = vec!["1".into()];
        assert!(validate_category_bar_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), "x".repeat(161)];
        assert!(validate_category_bar_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), String::new()];
        assert_eq!(validate_category_bar_series(&s, &cats), Ok(()));

        for target in [
            CategoryBarTarget::new("missing", "a"),
            CategoryBarTarget::new("c0", "other"),
        ] {
            let mut s = bars("a");
            overrides(&mut s).push(CategoryBarOverride::new(target));
            assert!(validate_category_bar_series(&s, &cats).is_err());
        }
        let mut s = bars("a");
        let target = CategoryBarTarget::new("c1", "a");
        overrides(&mut s).push(CategoryBarOverride::new(target.clone()));
        assert_eq!(validate_category_bar_series(&s, &cats), Ok(()));
        overrides(&mut s).push(CategoryBarOverride::new(target));
        assert!(validate_category_bar_series(&s, &cats).is_err());
        let mut s = bars("a");
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c1", "a"));
        o.color = Some(Color::new(1.0, 0.0, 0.0, 0.5));
        overrides(&mut s).push(o);
        assert!(validate_category_bar_series(&s, &cats).is_err());
    }

    #[test]
    fn value_label_offsets_are_validated_and_written_by_a_drag() {
        use crate::layout::{NudgeReject, NudgeResult};
        let cats = categories(3);
        let mut series = bars("a");
        assert_eq!(validate_category_bar_series(&series, &cats), Ok(()));
        assert_eq!(series.value_label_offset(1), [0.0, 0.0]);
        // The first drag fills one zero offset per category.
        assert_eq!(
            series.drag_value_label(1, cats.len(), 4.0, -2.5),
            NudgeResult::Moved
        );
        assert_eq!(
            labels(&mut series).offsets,
            [[0.0, 0.0], [4.0, -2.5], [0.0, 0.0]]
        );
        assert_eq!(
            series.drag_value_label(1, cats.len(), 1.0, 0.5),
            NudgeResult::Moved
        );
        assert_eq!(series.value_label_offset(1), [5.0, -2.0]);
        assert_eq!(validate_category_bar_series(&series, &cats), Ok(()));
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let before = series.clone();
        for (index, dx, dy) in [
            (3, 1.0, 1.0),
            (0, f32::NAN, 0.0),
            (0, 0.0, f32::INFINITY),
            (0, MAX_VALUE_LABEL_OFFSET + 1.0, 0.0),
        ] {
            assert_eq!(series.drag_value_label(index, cats.len(), dx, dy), reject);
            assert_eq!(series, before);
        }
        let mut line = bars("l");
        line.render_type = DataRenderType::Line {
            line: DataLineStyleConfig::default(),
        };
        assert_eq!(line.drag_value_label(0, 3, 1.0, 1.0), reject);
        assert_eq!(line.value_label_offsets(), None);

        for offsets in [
            vec![[0.0, 0.0]; 2],
            vec![[0.0, f32::NAN]; 3],
            vec![[MAX_VALUE_LABEL_OFFSET * 2.0, 0.0]; 3],
        ] {
            labels(&mut series).offsets = offsets;
            assert!(validate_category_bar_series(&series, &cats).is_err());
        }
        let mut boxes = box_plot_tests::boxes("b");
        let DataRenderType::BoxPlot { labels: l, .. } = &mut boxes.render_type else {
            unreachable!()
        };
        l.offsets = vec![[0.0, 0.0]; 2];
        assert!(validate_box_plot_series(&boxes, &cats).is_err());
        let mut radar = radar_tests::radar("r");
        let DataRenderType::Radar { labels: l, .. } = &mut radar.render_type else {
            unreachable!()
        };
        l.offsets = vec![[f32::NAN, 0.0]; 3];
        assert!(validate_radar_series(&radar, &cats).is_err());
    }

    #[test]
    fn a_value_label_id_names_its_series_and_category() {
        assert_eq!(value_label_id("a", 3), "series:a:label:3");
        assert_eq!(parse_value_label_id("series:a:label:3"), Some(("a", 3)));
        // A series id may hold the separator itself.
        let id = value_label_id("x:label:y", 12);
        assert_eq!(parse_value_label_id(&id), Some(("x:label:y", 12)));
        for other in [
            "axis_left",
            "series:a:label:",
            "series:a:label:-1",
            "series:a:label:1x",
            "a:label:1",
        ] {
            assert_eq!(parse_value_label_id(other), None, "{other}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn value_label_offsets_round_trip_and_are_omitted_when_empty() {
        let mut series = bars("a");
        let json = serde_json::to_value(&series).unwrap();
        let fields = json["render_type"]["CategoryBars"]["labels"]
            .as_object()
            .unwrap();
        assert!(!fields.contains_key("offsets"));
        labels(&mut series).offsets = vec![[1.5, -2.0], [0.0, 0.0], [3.0, 4.0]];
        let json = serde_json::to_string(&series).unwrap();
        assert!(json.contains(r#""offsets":[[1.5,-2.0],[0.0,0.0],[3.0,4.0]]"#));
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);
    }

    #[test]
    fn format_labels_writes_each_format_from_the_host_values() {
        let cats = categories(3);
        let values = [
            [Some(2.0), None, Some(1.0)],
            [Some(3.0), Some(0.0), Some(-1.0)],
        ];
        let mut series = vec![bars("a"), bars("b")];
        labels(&mut series[0]).value_suffix = " kg".into();
        let read = |k: usize, i: usize| values[k][i];
        format_labels(&mut series, &cats, read);
        assert_eq!(labels(&mut series[0]).text, ["2 kg", "", "1 kg"]);
        assert_eq!(labels(&mut series[1]).text, ["3", "0", "-1"]);

        let set = |s: &mut SeriesConfig, f: CategoryBarLabelFormat| {
            let DataRenderType::CategoryBars { label_format, .. } = &mut s.render_type else {
                unreachable!()
            };
            *label_format = f;
        };
        for s in &mut series {
            set(s, CategoryBarLabelFormat::Percent);
        }
        format_labels(&mut series, &cats, read);
        // A negative value leaves its category without shares.
        assert_eq!(labels(&mut series[0]).text, ["40%", "", "—"]);
        assert_eq!(labels(&mut series[1]).text, ["60%", "—", "—"]);
        for s in &mut series {
            set(s, CategoryBarLabelFormat::ValuePercent);
        }
        format_labels(&mut series, &cats, read);
        assert_eq!(labels(&mut series[0]).text[0], "2 kg (40%)");
        for s in &mut series {
            set(s, CategoryBarLabelFormat::CategoryValue);
        }
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c2", "b"));
        o.label_format = Some(CategoryBarLabelFormat::Value);
        overrides(&mut series[1]).push(o);
        labels(&mut series[1]).decimals = 2;
        format_labels(&mut series, &cats, |k, i| values[k][i].map(|v| v + 0.3));
        assert_eq!(labels(&mut series[0]).text[0], "C0: 2.3 kg");
        assert_eq!(labels(&mut series[1]).text, ["C0: 3.3", "C1: 0.3", "-0.7"]);
        // Every series that is not a category bar series is left alone.
        let mut line = bars("a");
        line.render_type = DataRenderType::Line {
            line: DataLineStyleConfig::default(),
        };
        let before = line.clone();
        format_labels(std::slice::from_mut(&mut line), &cats, read);
        assert_eq!(line, before);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn category_bars_round_trip_and_default_their_optional_fields() {
        let mut series = bars("a");
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("c0", "a"));
        o.color = Some(Color::WHITE);
        o.style = Some(CategoryBarStyle {
            material: Material::Paper,
            ..Default::default()
        });
        o.labels = Some(CategoryBarLabels::Inside);
        o.label_format = Some(CategoryBarLabelFormat::Percent);
        overrides(&mut series).push(o);
        labels(&mut series).text = vec!["1".into(), "2".into()];
        let json = serde_json::to_string(&series).unwrap();
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);

        let minimal = serde_json::json!({
            "series_id": "a",
            "label": null,
            "x_column": "positions",
            "y_column": "a.values",
            "render_type": {"CategoryBars": {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}}
        });
        let parsed: SeriesConfig = serde_json::from_value(minimal).unwrap();
        let bars = parsed.render_type.category_bars().unwrap();
        assert_eq!(bars.style, &CategoryBarStyle::default());
        assert_eq!(bars.labels, &CategoryBarSeriesLabels::default());
        assert!(bars.overrides.is_empty());
        let written = serde_json::to_value(&parsed).unwrap();
        let fields = &written["render_type"]["CategoryBars"];
        assert!(fields.get("overrides").is_none());
        assert!(fields["labels"].get("text").is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn bar_groups_and_cross_at_are_additive_on_the_wire() {
        let config = crate::default::default_config();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("bar_groups").is_none());
        assert!(json["bottom_x"].get("cross_at").is_none());
        let mut edited = config.clone();
        edited.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::PercentStacked,
            direction: CategoryBarDirection::Horizontal,
            group_width: 0.5,
            bar_gap: 2.0,
        });
        edited.bottom_x.cross_at = Some(0.0);
        let text = serde_json::to_string(&edited).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), edited);
        let groups: BarGroups = serde_json::from_str("{}").unwrap();
        assert_eq!(groups, BarGroups::default());
    }

    /// `log_ticks` is written only when it is not the default, so a
    /// document written before the field existed re-serializes unchanged,
    /// and an `Adaptive` axis round-trips (#42).
    #[cfg(feature = "serde")]
    #[test]
    fn log_ticks_are_additive_on_the_wire() {
        use crate::config::LogTickPolicy;
        let mut config = crate::default::default_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let json = serde_json::to_value(&config).unwrap();
        for axis in ["top_x", "bottom_x", "left_y", "right_y"] {
            assert!(json[axis].get("log_ticks").is_none(), "{axis}");
        }
        assert!(json["colorbar"]["axis"].get("log_ticks").is_none());
        let mut edited = config.clone();
        edited.left_y.log_ticks = LogTickPolicy::Adaptive;
        edited.colorbar.as_mut().unwrap().axis.log_ticks = LogTickPolicy::Adaptive;
        let json = serde_json::to_value(&edited).unwrap();
        assert_eq!(json["left_y"]["log_ticks"], "Adaptive");
        assert_eq!(json["colorbar"]["axis"]["log_ticks"], "Adaptive");
        assert!(json["bottom_x"].get("log_ticks").is_none());
        let text = serde_json::to_string(&edited).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), edited);
        // The written default parses too.
        let mut explicit = serde_json::to_value(&config).unwrap();
        explicit["left_y"]["log_ticks"] = "Decades".into();
        let parsed: Config = serde_json::from_value(explicit).unwrap();
        assert_eq!(parsed, config);
        assert_eq!(config.scaled(2.0).left_y.log_ticks, LogTickPolicy::Decades);
        assert_eq!(
            edited.scaled(2.0).left_y.log_ticks,
            LogTickPolicy::Adaptive,
            "a tick policy is not a pixel dimension"
        );
    }

    #[test]
    fn the_bar_gap_scales_and_the_rest_of_the_groups_stay() {
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            direction: CategoryBarDirection::Vertical,
            group_width: 0.6,
            bar_gap: 3.0,
        });
        config.bottom_x.cross_at = Some(1.5);
        let scaled = config.scaled(2.0);
        let groups = scaled.bar_groups.unwrap();
        assert_eq!((groups.bar_gap, groups.group_width), (6.0, 0.6));
        assert_eq!(groups.mode, CategoryBarMode::Stacked);
        assert_eq!(scaled.bottom_x.cross_at, Some(1.5));
    }

    #[test]
    fn the_columns_follow_the_direction() {
        let s = bars("a");
        assert_eq!(
            s.category_bar_columns(CategoryBarDirection::Vertical),
            ("positions", "a.values")
        );
        assert_eq!(
            s.category_bar_columns(CategoryBarDirection::Horizontal),
            ("a.values", "positions")
        );
    }
}

#[cfg(test)]
mod box_plot_tests {
    use super::*;
    use crate::boxplot::MAX_BOX_SERIES;
    use crate::config::Config;
    use crate::layer::{OverlayLayer, Subplot};

    fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn boxes(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("{id}.record"),
            render_type: DataRenderType::BoxPlot {
                color: Color::from_rgb8(40, 120, 200),
                style: BoxPlotStyle::default(),
                labels: BoxPlotSeriesLabels::default(),
                outliers: None,
                overrides: None,
            },
        }
    }

    fn chart(n: usize) -> Config {
        let mut config = crate::default::default_config();
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn render(series: &mut SeriesConfig) -> &mut DataRenderType {
        &mut series.render_type
    }

    fn style(series: &mut SeriesConfig) -> &mut BoxPlotStyle {
        let DataRenderType::BoxPlot { style, .. } = render(series) else {
            unreachable!()
        };
        style
    }

    fn overrides(series: &mut SeriesConfig) -> &mut Vec<BoxPlotOverride> {
        let DataRenderType::BoxPlot { overrides, .. } = render(series) else {
            unreachable!()
        };
        overrides.get_or_insert_with(Vec::new)
    }

    fn labels(series: &mut SeriesConfig) -> &mut BoxPlotSeriesLabels {
        let DataRenderType::BoxPlot { labels, .. } = render(series) else {
            unreachable!()
        };
        labels
    }

    #[test]
    fn a_valid_group_passes_and_other_series_are_ignored() {
        let config = chart(3);
        assert_eq!(config.validate_box_plots(&[boxes("a"), boxes("b")]), Ok(()));
        let mut plain = boxes("a");
        plain.render_type = DataRenderType::Line {
            line: DataLineStyleConfig::default(),
        };
        // No box plot series: nothing to hold, not even a categorical axis.
        assert_eq!(
            crate::default::default_config().validate_box_plots(&[plain]),
            Ok(())
        );
        assert_eq!(boxes("a").box_plot_columns(), ("positions", "a.record"));
    }

    #[test]
    fn box_groups_are_grouped_only() {
        let mut config = chart(3);
        for mode in [CategoryBarMode::Stacked, CategoryBarMode::PercentStacked] {
            config.bar_groups = Some(BarGroups {
                mode,
                ..BarGroups::default()
            });
            assert!(
                config
                    .validate_box_plots(&[boxes("a")])
                    .unwrap_err()
                    .contains("Grouped"),
                "{mode:?}"
            );
        }
        config.bar_groups = Some(BarGroups::default());
        assert_eq!(config.validate_box_plots(&[boxes("a")]), Ok(()));
    }

    #[test]
    fn boxes_need_the_categorical_axis_of_their_direction_and_a_linear_value_axis() {
        let mut config = chart(3);
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        assert!(config.validate_box_plots(&[boxes("a")]).is_err());
        config.left_y.categories = Some(categories(3));
        config.bottom_x.categories = None;
        assert_eq!(config.validate_box_plots(&[boxes("a")]), Ok(()));
        config.bottom_x.scale = AxisScale::Logarithmic;
        assert_eq!(
            config.validate_box_plots(&[boxes("a")]),
            Err("box plots need a linear numeric value axis")
        );
        let mut polar = chart(3);
        polar.polar = Some(Default::default());
        assert_eq!(
            polar.validate_box_plots(&[boxes("a")]),
            Err("box plots are not drawn on polar axes")
        );
    }

    #[test]
    fn group_size_is_bounded_and_box_count_is_not() {
        let config = chart(2);
        let many: Vec<_> = (0..=MAX_BOX_SERIES)
            .map(|i| boxes(&format!("s{i}")))
            .collect();
        assert!(config.validate_box_plots(&many).is_err());
        assert_eq!(config.validate_box_plots(&many[..MAX_BOX_SERIES]), Ok(()));
        // No box cap (#107): 8 series over 65 categories (520 boxes, past
        // the old 512) and 1 series over 600 categories are accepted.
        let eight: Vec<_> = (0..8).map(|i| boxes(&format!("s{i}"))).collect();
        assert_eq!(chart(65).validate_box_plots(&eight), Ok(()));
        assert_eq!(chart(600).validate_box_plots(&eight[..1]), Ok(()));
    }

    #[test]
    fn a_layer_draws_category_bars_or_box_plots() {
        let config = chart(3);
        let bars = SeriesConfig {
            render_type: DataRenderType::CategoryBars {
                color: Color::from_rgb8(1, 2, 3),
                style: CategoryBarStyle::default(),
                labels: CategoryBarSeriesLabels::default(),
                label_format: CategoryBarLabelFormat::Value,
                overrides: None,
            },
            ..boxes("bars")
        };
        assert_eq!(
            config.validate_box_plots(&[bars, boxes("a")]),
            Err("a layer draws either category bars or box plots, not both")
        );
    }

    #[test]
    fn horizontal_boxes_stay_off_overlays_and_subplots_have_their_own_groups() {
        let mut config = chart(3);
        config.overlays.push(OverlayLayer::default());
        config.overlays[0].series.push("over".into());
        assert_eq!(config.validate_box_plots(&[boxes("over")]), Ok(()));
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..BarGroups::default()
        });
        config.left_y.categories = Some(categories(3));
        assert!(
            config
                .validate_box_plots(&[boxes("over")])
                .unwrap_err()
                .contains("overlay")
        );
        let mut config = chart(3);
        let mut sub = chart(2);
        sub.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Stacked,
            ..BarGroups::default()
        });
        config.subplots.push(Subplot {
            config: sub,
            series: vec!["below".into()],
            link_x: None,
            link_y: None,
        });
        // The subplot's own groups decide: stacked there is rejected.
        assert!(config.validate_box_plots(&[boxes("a")]).is_ok());
        assert!(config.validate_box_plots(&[boxes("below")]).is_err());
    }

    #[test]
    fn styles_labels_ids_and_overrides_are_validated() {
        let config = chart(3);
        let cats = categories(3);
        let mut s = boxes("a");
        style(&mut s).material = crate::Material::Toon;
        assert_eq!(
            validate_box_plot_series(&s, &cats),
            Err("material not supported by boxes")
        );
        for edit in [
            |s: &mut BoxPlotStyle| s.corner_radius = 4.0,
            |s: &mut BoxPlotStyle| s.notch_depth = 0.5,
            |s: &mut BoxPlotStyle| s.mean_size = 0.0,
            |s: &mut BoxPlotStyle| s.whisker_width = -1.0,
            |s: &mut BoxPlotStyle| s.outlier_color.a = 2.0,
            |s: &mut BoxPlotStyle| s.gloss = f32::NAN,
        ] {
            let mut s = boxes("a");
            edit(style(&mut s));
            assert_eq!(
                validate_box_plot_series(&s, &cats),
                Err("invalid boxplot style")
            );
        }
        let mut s = boxes("a");
        let DataRenderType::BoxPlot { color, .. } = render(&mut s) else {
            unreachable!()
        };
        color.a = 0.5;
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        labels(&mut s).decimals = 7;
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        labels(&mut s).text = vec!["1".into(); 2];
        assert_eq!(
            validate_box_plot_series(&s, &cats),
            Err("box label text needs one string per category")
        );
        labels(&mut s).text = vec!["1".into(), "two\nlines".into(), "3".into()];
        assert!(validate_box_plot_series(&s, &cats).is_err());
        labels(&mut s).text = vec!["1".into(), "2".into(), String::new()];
        assert_eq!(validate_box_plot_series(&s, &cats), Ok(()));
        let mut s = boxes("a");
        s.y_column.clear();
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        let DataRenderType::BoxPlot { outliers, .. } = render(&mut s) else {
            unreachable!()
        };
        *outliers = Some(String::new());
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c1", "a")));
        assert_eq!(validate_box_plot_series(&s, &cats), Ok(()));
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c1", "a")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("zz", "a")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        overrides(&mut s).push(BoxPlotOverride::new(BoxPlotTarget::new("c0", "b")));
        assert!(validate_box_plot_series(&s, &cats).is_err());
        let mut s = boxes("a");
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c0", "a"));
        o.style = Some(BoxPlotStyle {
            cap_ratio: 2.0,
            ..BoxPlotStyle::default()
        });
        overrides(&mut s).push(o);
        assert!(validate_box_plot_series(&s, &cats).is_err());
        // The chart-level check runs every series' rules.
        assert!(config.validate_box_plots(&[s]).is_err());
    }

    #[test]
    fn format_box_plot_labels_writes_median_and_count_with_overrides() {
        let mut series = [boxes("a"), boxes("b")];
        let cats = categories(3);
        let values = |k: usize, i: usize| match (k, i) {
            (_, 1) => None,
            (0, i) => Some((1.5 + i as f64, Some(40))),
            (_, i) => Some((10.0 * i as f64, None)),
        };
        format_box_plot_labels(&mut series, &cats, values);
        assert_eq!(labels(&mut series[0]).text, ["", "", ""]);
        labels(&mut series[0]).placement = BoxPlotLabels::Median;
        labels(&mut series[0]).decimals = 2;
        labels(&mut series[0]).value_suffix = " ms".into();
        labels(&mut series[1]).placement = BoxPlotLabels::MedianAndCount;
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c2", "a"));
        o.labels = Some(BoxPlotLabels::MedianAndCount);
        overrides(&mut series[0]).push(o);
        format_box_plot_labels(&mut series, &cats, values);
        assert_eq!(
            labels(&mut series[0]).text,
            ["1.50 ms", "", "3.50 ms (n=40)"]
        );
        assert_eq!(labels(&mut series[1]).text, ["0.0 (n=—)", "", "20.0 (n=—)"]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn box_plot_series_round_trip_and_omit_their_defaults() {
        let plain = boxes("a");
        let json = serde_json::to_value(&plain).unwrap();
        let fields = json["render_type"]["BoxPlot"].as_object().unwrap();
        assert!(!fields.contains_key("outliers") && !fields.contains_key("overrides"));
        assert!(!fields["labels"].as_object().unwrap().contains_key("text"));
        let back: SeriesConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);
        let mut full = boxes("a");
        let DataRenderType::BoxPlot {
            outliers,
            labels: l,
            ..
        } = render(&mut full)
        else {
            unreachable!()
        };
        *outliers = Some("a.outliers".into());
        l.text = vec!["1".into(), "2".into(), "3".into()];
        style(&mut full).notched = true;
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c1", "a"));
        o.color = Some(Color::WHITE);
        o.labels = Some(BoxPlotLabels::Median);
        overrides(&mut full).push(o);
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), full);
        // A minimal document: style and labels default.
        let minimal: SeriesConfig = serde_json::from_str(
            r#"{"series_id":"m","label":null,"x_column":"p","y_column":"r",
                "render_type":{"BoxPlot":{"color":{"r":0,"g":0,"b":0,"a":1}}}}"#,
        )
        .unwrap();
        assert_eq!(
            minimal.render_type.box_plot().unwrap().style,
            &BoxPlotStyle::default()
        );
    }
}

#[cfg(test)]
mod radar_tests {
    use super::*;
    use crate::config::Config;
    use crate::layer::Subplot;
    use crate::polar::{PolarInterpolation, PolarOptions};
    use crate::radar::MAX_RADAR_SERIES;

    fn categories(n: usize) -> Vec<Category> {
        (0..n)
            .map(|i| Category::new(format!("c{i}"), format!("C{i}")))
            .collect()
    }

    pub(crate) fn radar(id: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("values.{id}"),
            render_type: DataRenderType::Radar {
                color: Color::from_rgb8(40, 120, 200),
                style: RadarSeriesStyle::default(),
                labels: RadarSeriesLabels::default(),
            },
        }
    }

    fn polar_config(n: usize) -> Config {
        let mut config = crate::default::default_config();
        config.polar = Some(PolarOptions::default());
        config.bottom_x.categories = Some(categories(n));
        config
    }

    fn style_mut(series: &mut SeriesConfig) -> &mut RadarSeriesStyle {
        match &mut series.render_type {
            DataRenderType::Radar { style, .. } => style,
            _ => unreachable!(),
        }
    }

    fn labels_mut(series: &mut SeriesConfig) -> &mut RadarSeriesLabels {
        match &mut series.render_type {
            DataRenderType::Radar { labels, .. } => labels,
            _ => unreachable!(),
        }
    }

    #[test]
    fn radar_series_need_a_polar_chart_with_a_categorical_angle_axis() {
        let config = polar_config(5);
        assert_eq!(config.validate_radar(&[radar("a"), radar("b")]), Ok(()));
        assert_eq!(
            crate::default::default_config().validate_radar(&[]),
            Ok(()),
            "a chart without radar series passes"
        );
        let mut rectangular = config.clone();
        rectangular.polar = None;
        assert!(
            rectangular
                .validate_radar(&[radar("a")])
                .unwrap_err()
                .contains("polar")
        );
        let mut numeric = config.clone();
        numeric.bottom_x.categories = None;
        assert!(
            numeric
                .validate_radar(&[radar("a")])
                .unwrap_err()
                .contains("categorical angle axis")
        );
        let mut arcs = config.clone();
        arcs.polar.as_mut().unwrap().interpolation = PolarInterpolation::Arc { pieces: 4 };
        assert!(
            arcs.validate_radar(&[radar("a")])
                .unwrap_err()
                .contains("Chord")
        );
        let mut sector = config.clone();
        sector.polar.as_mut().unwrap().sector = Some([0.0, 180.0]);
        sector.polar.as_mut().unwrap().inner_radius_fraction = 0.2;
        assert_eq!(sector.validate_radar(&[radar("a")]), Ok(()));
    }

    #[test]
    fn a_chart_draws_at_most_sixteen_radar_series() {
        let config = polar_config(4);
        let many: Vec<_> = (0..=MAX_RADAR_SERIES)
            .map(|i| radar(&format!("s{i}")))
            .collect();
        assert!(config.validate_radar(&many).is_err());
        assert_eq!(config.validate_radar(&many[..MAX_RADAR_SERIES]), Ok(()));
    }

    #[test]
    fn a_subplot_draws_radar_series_against_its_own_polar_axes() {
        let mut config = crate::default::default_config();
        let mut cell = polar_config(3);
        cell.chart_area = config.chart_area.clone();
        config.subplots.push(Subplot {
            config: cell,
            series: vec!["a".into()],
            ..Default::default()
        });
        assert_eq!(config.validate_radar(&[radar("a")]), Ok(()));
        assert!(
            config.validate_radar(&[radar("b")]).is_err(),
            "the parent chart is rectangular"
        );
    }

    #[test]
    fn style_and_labels_are_validated() {
        let config = polar_config(3);
        let check = |series: SeriesConfig| config.validate_radar(&[series]);
        for bad in [
            (|s: &mut RadarSeriesStyle| s.line_width_px = Some(13.0)) as fn(&mut _),
            |s| s.line_width_px = Some(f32::NAN),
            |s| s.fill = Some(Color::new(2.0, 0.0, 0.0, 1.0)),
            |s| s.marker_size_px = Some(0.5),
            |s| s.marker_size_px = Some(25.0),
            |s| s.fill_opacity = Some(1.5),
        ] {
            let mut series = radar("a");
            bad(style_mut(&mut series));
            assert!(check(series).is_err());
        }
        let mut series = radar("a");
        *style_mut(&mut series) = RadarSeriesStyle {
            line_width_px: Some(0.0),
            fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
            marker: Some(RadarMarker::Square),
            marker_size_px: Some(24.0),
            fill_opacity: Some(1.0),
        };
        assert_eq!(check(series), Ok(()));
        let mut series = radar("a");
        labels_mut(&mut series).decimals = 7;
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).value_suffix = "x".repeat(25);
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(); 2];
        assert!(
            check(series)
                .unwrap_err()
                .contains("one string per category"),
            "text follows the categories"
        );
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(), "a\nb".into(), "3".into()];
        assert!(check(series).is_err());
        let mut series = radar("a");
        labels_mut(&mut series).text = vec!["1".into(), "".into(), "3".into()];
        assert_eq!(check(series), Ok(()));
        let mut series = radar("a");
        series.render_type = DataRenderType::Radar {
            color: Color::new(0.0, 0.0, 1.5, 1.0),
            style: RadarSeriesStyle::default(),
            labels: RadarSeriesLabels::default(),
        };
        assert!(check(series).is_err());
        let mut series = radar("a");
        series.y_column.clear();
        assert!(check(series).is_err());
    }

    #[test]
    fn unset_style_fields_take_the_defaults() {
        let style = RadarSeriesStyle::default();
        let color = Color::from_rgba(0.2, 0.4, 0.6, 0.8);
        assert_eq!(style.line_width(), 2.0);
        assert_eq!(style.marker(), RadarMarker::Circle);
        assert_eq!(style.marker_size(), 7.0);
        assert!((style.fill_color(color).a - 0.2).abs() < 1e-6);
        let chart = crate::radar::RadarStyle {
            line_width_px: 3.0,
            fill_opacity: 0.5,
            marker: RadarMarker::Square,
            marker_size_px: 9.0,
            emphasis_brightness: 0.1,
            ..Default::default()
        };
        let resolved = style.resolved_from(&chart);
        assert_eq!(resolved.line_width_px, Some(3.0));
        assert_eq!(resolved.fill_opacity, Some(0.5));
        assert_eq!(resolved.marker, Some(RadarMarker::Square));
        assert_eq!(resolved.marker_size_px, Some(9.0));
        assert_eq!(resolved.fill, None);
        let own = RadarSeriesStyle {
            marker: Some(RadarMarker::None),
            ..Default::default()
        }
        .resolved_from(&chart);
        assert_eq!(own.marker, Some(RadarMarker::None));
    }

    #[test]
    fn a_radar_draws_as_a_filled_line_with_vertex_markers() {
        let series = radar("a");
        let r = series.render_type.radar().unwrap();
        let line = r.line_style();
        assert_eq!(line.line_style, LineStylePreset::Solid);
        assert_eq!(line.line_color, r.color);
        assert_eq!(line.line_width, 2.0);
        assert_eq!(line.fill, Some(LineFill::ToBaseline(RADAR_FILL_BASELINE)));
        assert_eq!(line.fill_color, Some(r.style.fill_color(r.color)));
        assert!(line.interpolation.is_linear());
        let (hi, lo) = (
            RADAR_FILL_BASELINE as f32,
            (RADAR_FILL_BASELINE - f64::from(RADAR_FILL_BASELINE as f32)) as f32,
        );
        assert!(hi.is_finite() && lo.is_finite());
        let scatter = r.scatter_style().unwrap();
        assert_eq!(scatter.point_shape, ScatterShape::CircleFilled);
        assert_eq!(scatter.point_size, 3.5, "a circle's diameter is the size");
        assert_eq!(scatter.point_color, r.color);
        let mut square = radar("a");
        style_mut(&mut square).marker = Some(RadarMarker::Square);
        let scatter = square.render_type.radar().unwrap().scatter_style().unwrap();
        assert_eq!(scatter.point_shape, ScatterShape::SquareFilled);
        assert!(
            (scatter.point_size * SQUARE_HALF_SIDE * 2.0 - 7.0).abs() < 1e-5,
            "a square's side is the size"
        );
        let mut bare = radar("a");
        *style_mut(&mut bare) = RadarSeriesStyle {
            line_width_px: Some(0.0),
            marker: Some(RadarMarker::None),
            fill_opacity: Some(0.0),
            ..Default::default()
        };
        let r = bare.render_type.radar().unwrap();
        assert_eq!(r.line_style().line_color.a, 0.0, "no outline at width 0");
        assert_eq!(r.line_style().fill, None, "no fill when it is transparent");
        assert!(r.scatter_style().is_none());
    }

    #[test]
    fn format_radar_labels_writes_one_string_per_category() {
        let cats = categories(3);
        let mut series = vec![radar("a"), radar("b")];
        let values = [[1.24, f64::NAN, 3.0], [0.0, 2.0, -1.5]];
        let value = |k: usize, i: usize| Some(values[k][i]);
        format_radar_labels(&mut series, &cats, value);
        for s in &series {
            assert_eq!(s.render_type.radar().unwrap().labels.text, ["", "", ""]);
        }
        for s in &mut series {
            let labels = labels_mut(s);
            labels.placement = RadarLabels::Emphasized;
            labels.decimals = 1;
            labels.value_suffix = " u".into();
        }
        format_radar_labels(&mut series, &cats, value);
        assert_eq!(
            series[0].render_type.radar().unwrap().labels.text,
            ["1.2 u", "", "3.0 u"]
        );
        assert_eq!(
            series[1].render_type.radar().unwrap().labels.text,
            ["0.0 u", "2.0 u", "-1.5 u"]
        );
        format_radar_labels(&mut series, &cats, |_, i| (i != 1).then_some(5.0));
        assert_eq!(
            series[0].render_type.radar().unwrap().labels.text,
            ["5.0 u", "", "5.0 u"]
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn radar_series_round_trip_and_omit_their_defaults() {
        let mut series = radar("a");
        let json = serde_json::to_value(&series.render_type).unwrap();
        assert_eq!(
            json["Radar"]["labels"],
            serde_json::json!({"placement": "None", "decimals": 1, "value_suffix": ""})
        );
        let style = &json["Radar"]["style"];
        assert!(style.get("fill_opacity").is_none());
        let back: DataRenderType = serde_json::from_value(json).unwrap();
        assert_eq!(back, series.render_type);
        let minimal: DataRenderType = serde_json::from_value(serde_json::json!({
            "Radar": {"color": {"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0}}
        }))
        .unwrap();
        assert_eq!(minimal.radar().unwrap().style, &RadarSeriesStyle::default());
        assert_eq!(
            minimal.radar().unwrap().labels,
            &RadarSeriesLabels::default()
        );
        *style_mut(&mut series) = RadarSeriesStyle {
            line_width_px: Some(3.0),
            fill: Some(Color::WHITE),
            marker: Some(RadarMarker::Square),
            marker_size_px: Some(9.0),
            fill_opacity: Some(0.4),
        };
        *labels_mut(&mut series) = RadarSeriesLabels {
            placement: RadarLabels::Values,
            decimals: 2,
            value_suffix: " pt".into(),
            text: vec!["1.00 pt".into(), "".into()],
            offsets: vec![[2.0, -1.5], [0.0, 0.0]],
        };
        let text = serde_json::to_string(&series).unwrap();
        let back: SeriesConfig = serde_json::from_str(&text).unwrap();
        assert_eq!(back, series);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
        // A legacy series style keeps its spelling: its four fields are
        // written as before and the two new ones are omitted while unset.
        let legacy = RadarSeriesStyle {
            marker: Some(RadarMarker::None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&legacy).unwrap(),
            r#"{"line_width_px":null,"fill":null,"marker":"None","marker_size_px":null}"#
        );
    }
}

#[cfg(test)]
mod step_selection_tests {
    use super::*;
    use crate::config::{Config, DataSelectionsConfig, PickedDataRef};
    use std::collections::HashMap;

    fn entry(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: Some(format!("{id}-source")),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type,
        }
    }

    fn line() -> DataRenderType {
        DataRenderType::Line {
            line: DataLineStyleConfig::default(),
        }
    }

    fn histogram(orientation: BarOrientation) -> DataRenderType {
        DataRenderType::Histogram {
            bar: DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 1.0,
                orientation,
                bar_style_overrides: None,
            },
        }
    }

    fn bars() -> DataRenderType {
        DataRenderType::CategoryBars {
            color: Color::BLACK,
            style: CategoryBarStyle::default(),
            labels: CategoryBarSeriesLabels::default(),
            label_format: CategoryBarLabelFormat::default(),
            overrides: None,
        }
    }

    fn boxes() -> DataRenderType {
        DataRenderType::BoxPlot {
            color: Color::BLACK,
            style: BoxPlotStyle::default(),
            labels: BoxPlotSeriesLabels::default(),
            outliers: None,
            overrides: None,
        }
    }

    fn matrix(columns: &[&str], orientation: MatrixOrientation, layout: GridLayout) -> MatrixRef {
        MatrixRef {
            columns: columns.iter().map(|c| (*c).into()).collect(),
            orientation,
            grid_layout: layout,
        }
    }

    fn vector(components: VectorComponents) -> DataRenderType {
        DataRenderType::Vector {
            components,
            style: VectorStyle {
                color: Color::BLACK,
                width_px: 1.0,
                scale: VectorScale::Pixels { px_per_unit: 1.0 },
                pivot: VectorPivot::default(),
                head_length_px: 4.0,
                head_width_px: 4.0,
                min_length_px: 0.0,
                color_map: None,
            },
        }
    }

    fn selected(reference: PickedDataRef) -> Config {
        let mut config = crate::default::default_config();
        config.picked_data = Some(DataSelectionsConfig {
            refs: vec![reference],
            ..Default::default()
        });
        config
    }

    fn point(series_id: &str, point_index: usize) -> PickedDataRef {
        PickedDataRef::Point {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            point_index,
        }
    }

    fn bin(series_id: &str, bin_index: usize) -> PickedDataRef {
        PickedDataRef::HistogramBin {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            bin_index,
        }
    }

    fn cell(series_id: &str, x_index: usize, y_index: usize) -> PickedDataRef {
        PickedDataRef::MatrixCell {
            source_id: Some(format!("{series_id}-source")),
            series_id: series_id.into(),
            x_index,
            y_index,
        }
    }

    /// Step `config` once and return the moved selection, `None` when it
    /// did not move.
    fn step(
        config: &mut Config,
        series: &[SeriesConfig],
        columns: &HashMap<&str, usize>,
        dx: i32,
        dy: i32,
    ) -> Option<PickedDataRef> {
        let moved = config
            .step_selection(series, dx, dy, |c| columns.get(c).copied())
            .unwrap();
        moved.then(|| config.picked_data.as_ref().unwrap().refs[0].clone())
    }

    #[test]
    fn a_point_steps_along_its_series_and_across_point_series_and_clamps() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("h", "e", "c", histogram(BarOrientation::Vertical)),
            entry("b", "x", "yb", line()),
            entry(
                "v",
                "x",
                "yb",
                vector(VectorComponents::Points {
                    u: "u".into(),
                    v: "vv".into(),
                }),
            ),
        ];
        let columns = HashMap::from([
            ("x", 10),
            ("ya", 10),
            ("yb", 8),
            ("e", 4),
            ("c", 3),
            ("u", 5),
            ("vv", 6),
        ]);
        let mut config = selected(point("a", 2));
        assert_eq!(
            step(&mut config, &series, &columns, 3, 0),
            Some(point("a", 5))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 100, 0),
            Some(point("a", 9))
        );
        // At the end of the series nothing moves.
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        // Across the point series, the histogram skipped, the index clamped
        // to the next series' shorter columns.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("b", 7))
        );
        // A point vector counts the points of all four columns.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("v", 4))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        assert_eq!(
            step(&mut config, &series, &columns, -100, -100),
            Some(point("a", 0))
        );
        assert_eq!(step(&mut config, &series, &columns, -1, -1), None);
    }

    #[test]
    fn a_histogram_bin_steps_by_its_orientation() {
        let series = [
            entry("v1", "e", "c", histogram(BarOrientation::Vertical)),
            entry("h1", "hc", "he", histogram(BarOrientation::Horizontal)),
            entry("v2", "e", "c2", histogram(BarOrientation::Vertical)),
        ];
        let columns = HashMap::from([("e", 6), ("c", 5), ("c2", 3), ("he", 4), ("hc", 3)]);
        let mut config = selected(bin("v1", 1));
        assert_eq!(
            step(&mut config, &series, &columns, 2, 0),
            Some(bin("v1", 3))
        );
        // `dy` moves across the vertical histograms only, clamping.
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(bin("v2", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        // A horizontal histogram's bins run along `dy`; its edges are y.
        let mut config = selected(bin("h1", 0));
        assert_eq!(
            step(&mut config, &series, &columns, 0, 5),
            Some(bin("h1", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        // Bins need edges and counts.
        let empty = HashMap::from([("e", 1), ("c", 5)]);
        let mut config = selected(bin("v1", 0));
        assert_eq!(step(&mut config, &series, &empty, 1, 0), None);
    }

    #[test]
    fn a_category_bar_or_box_steps_along_its_categories_and_across_its_layer() {
        let series = [
            entry("a", "pos", "va", bars()),
            entry("box", "pos", "rec", boxes()),
            entry("b", "pos", "vb", bars()),
            entry("c", "pos", "vc", bars()),
        ];
        let columns = HashMap::from([("pos", 4), ("va", 4), ("vb", 4), ("vc", 4)]);
        let mut config = selected(bin("a", 0));
        // Series `c` sits on an overlay, another bar group.
        config.overlays.push(crate::layer::OverlayLayer {
            y_axis: crate::default::default_axis_options_y(),
            series: vec!["c".into()],
            legend: None,
        });
        assert_eq!(
            step(&mut config, &series, &columns, 2, 0),
            Some(bin("a", 2))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(bin("b", 2))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
        // Horizontal bars run their categories along `dy`.
        config.bar_groups = Some(BarGroups {
            direction: CategoryBarDirection::Horizontal,
            ..Default::default()
        });
        config.overlays.clear();
        let horizontal = [
            entry("a", "va", "pos", bars()),
            entry("b", "vb", "pos", bars()),
        ];
        let mut config2 = config.clone();
        assert_eq!(
            step(&mut config2, &horizontal, &columns, 0, 1),
            Some(bin("b", 3))
        );
        assert_eq!(
            step(&mut config2, &horizontal, &columns, -1, -3),
            Some(bin("a", 0))
        );
        // A box steps over the box plot series, its count from its
        // positions (its record is not a registered column).
        let mut config = selected(bin("box", 0));
        assert_eq!(
            step(&mut config, &series, &columns, 9, 0),
            Some(bin("box", 3))
        );
        assert_eq!(step(&mut config, &series, &columns, 0, 1), None);
    }

    #[test]
    fn a_matrix_cell_steps_over_its_grid_and_a_vector_grid_over_u() {
        let field = DataRenderType::Heatmap {
            matrix: matrix(
                &["z0", "z1", "z2"],
                MatrixOrientation::ColumnsAreX,
                GridLayout::Centers,
            ),
            fill: FieldFillConfig {
                mode: FillMode::Continuous,
                shading: Shading::Flat,
                opacity: 1.0,
            },
        };
        let grid = vector(VectorComponents::Grid {
            u: matrix(
                &["u0", "u1"],
                MatrixOrientation::ColumnsAreY,
                GridLayout::Edges,
            ),
            v: matrix(
                &["v0", "v1"],
                MatrixOrientation::ColumnsAreY,
                GridLayout::Edges,
            ),
        });
        let series = [
            entry("f", "x", "y", field),
            entry("g", "gx", "gy", grid),
            entry("l", "x", "y", line()),
        ];
        let columns = HashMap::from([
            ("x", 3),
            ("y", 5),
            ("z0", 5),
            ("z1", 4),
            ("z2", 5),
            ("gx", 9),
            ("gy", 3),
            ("u0", 8),
            ("u1", 8),
        ]);
        let mut config = selected(cell("f", 0, 0));
        // Three columns along x, the shortest column (4) along y.
        assert_eq!(
            step(&mut config, &series, &columns, 5, 9),
            Some(cell("f", 2, 3))
        );
        // A grid vector: columns along y, edges cut one from each axis.
        let mut config = selected(cell("g", 0, 0));
        assert_eq!(
            step(&mut config, &series, &columns, 20, 20),
            Some(cell("g", 7, 1))
        );
        // A matrix selection on a series that draws no matrix is an error.
        let mut config = selected(cell("l", 0, 0));
        assert_eq!(
            config.step_selection(&series, 1, 0, |c| columns.get(c).copied()),
            Err("selected matrix no longer exists")
        );
    }

    #[test]
    fn no_selection_an_unknown_series_and_labels_do_not_step() {
        let series = [entry("a", "x", "y", line())];
        let columns = HashMap::from([("x", 3), ("y", 3)]);
        let mut config = crate::default::default_config();
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        let mut config = selected(point("gone", 0));
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        let label = PickedDataRef::ValueLabel {
            source_id: None,
            series_id: "a".into(),
            category_index: 0,
        };
        let mut config = selected(label.clone());
        assert_eq!(step(&mut config, &series, &columns, 1, 0), None);
        assert_eq!(config.picked_data.unwrap().refs, [label]);
        // An unregistered column counts as empty: the index clamps to 0.
        let mut config = selected(point("a", 2));
        assert_eq!(
            step(&mut config, &series, &HashMap::new(), 1, 0),
            Some(point("a", 0))
        );
    }

    /// D4: a streamline vertex steps like a line's, the NaN separator rows
    /// included, and across to the other point-drawing series.
    #[test]
    fn step_selection_walks_streamline_vertices() {
        let streamlines = DataRenderType::Streamlines {
            line: DataLineStyleConfig::default(),
            arrows: Some(StreamlineArrows::default()),
            color_map: Some(PointColorMap {
                column: "speed".into(),
            }),
        };
        let series = [
            entry("a", "x", "ya", line()),
            entry("flow", "sx", "sy", streamlines),
        ];
        // Two streamlines of three points with one NaN row between them.
        let columns = HashMap::from([("x", 3), ("ya", 3), ("sx", 7), ("sy", 7), ("speed", 7)]);
        let mut config = selected(point("flow", 2));
        // Onto the separator row and past it, then clamped at the end.
        assert_eq!(
            step(&mut config, &series, &columns, 1, 0),
            Some(point("flow", 3))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 1, 0),
            Some(point("flow", 4))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 9, 0),
            Some(point("flow", 6))
        );
        // Across to the line, clamped to its length.
        assert_eq!(
            step(&mut config, &series, &columns, 0, -1),
            Some(point("a", 2))
        );
        assert_eq!(
            step(&mut config, &series, &columns, 0, 1),
            Some(point("flow", 2))
        );
    }
    // A line offset is a display shift: stepping walks the source indices
    // and the series order exactly as without one.
    #[test]
    fn step_selection_ignores_line_offsets() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("b", "x", "yb", line()),
            entry("c", "x", "yb", line()),
        ];
        let columns = HashMap::from([("x", 10), ("ya", 10), ("yb", 8)]);
        let walk = |offsets: Option<LineOffsets>| {
            let mut config = selected(point("a", 2));
            config.line_offsets = offsets;
            let mut moves = Vec::new();
            for (dx, dy) in [(3, 0), (0, 1), (100, 0), (0, 1), (-1, -1), (0, -100)] {
                moves.push(step(&mut config, &series, &columns, dx, dy));
            }
            moves
        };
        let plain = walk(None);
        assert_eq!(plain[0], Some(point("a", 5)));
        assert_eq!(
            walk(Some(LineOffsets {
                series: vec!["c".into(), "a".into(), "b".into()],
                step_y: 2.5,
                step_x: 0.5,
            })),
            plain
        );
    }

    // The layout rules and the member rules of a line offset group.
    #[test]
    fn line_offsets_are_validated_with_and_without_the_series() {
        let series = [
            entry("a", "x", "ya", line()),
            entry("b", "x", "yb", line()),
            entry("h", "e", "c", histogram(BarOrientation::Vertical)),
        ];
        let group = |ids: &[&str]| LineOffsets {
            series: ids.iter().map(|id| (*id).into()).collect(),
            step_y: 1.0,
            step_x: 0.0,
        };
        let mut config = crate::default::default_config();
        config.line_offsets = Some(group(&["a", "b"]));
        assert_eq!(config.validate_line_offsets(&series), Ok(()));
        assert_eq!(config.line_offset_of("a"), Some([0.0, 0.0]));
        assert_eq!(config.line_offset_of("b"), Some([0.0, 1.0]));
        assert_eq!(config.line_offset_of("h"), None);
        let rejected = |offsets: LineOffsets, edit: fn(&mut Config)| {
            let mut config = crate::default::default_config();
            config.line_offsets = Some(offsets);
            edit(&mut config);
            config.validate_line_offsets(&series).is_err()
        };
        assert!(rejected(group(&["a", "a"]), |_| {}), "duplicate");
        assert!(rejected(group(&["a", ""]), |_| {}), "empty id");
        assert!(rejected(group(&["a", "zz"]), |_| {}), "unknown");
        assert!(rejected(group(&["h"]), |_| {}), "no line");
        let mut nan = group(&["a"]);
        nan.step_y = f64::INFINITY;
        assert!(rejected(nan, |_| {}), "non-finite");
        let many: Vec<String> = (0..=MAX_OFFSET_LINES).map(|i| format!("s{i}")).collect();
        assert!(
            LineOffsets {
                series: many,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            rejected(group(&["a"]), |c| {
                c.polar = Some(crate::polar::PolarOptions::default())
            }),
            "polar"
        );
        assert!(
            rejected(group(&["a"]), |c| c.subplots.push(crate::layer::Subplot {
                series: vec!["a".into()],
                ..Default::default()
            })),
            "drawn by a subplot"
        );
        // A fill between a member and another series spans two panels.
        let mut filled = series.clone();
        if let DataRenderType::Line { line } = &mut filled[1].render_type {
            line.fill = Some(LineFill::ToSeries("a".into()));
        }
        let mut config = crate::default::default_config();
        config.line_offsets = Some(group(&["a"]));
        assert!(config.validate_line_offsets(&filled).is_err());
        config.line_offsets = Some(group(&["a", "b"]));
        assert!(config.validate_line_offsets(&filled).is_err());
        config.line_offsets = None;
        assert_eq!(config.validate_line_offsets(&filled), Ok(()));
    }
}

#[cfg(test)]
mod streamline_tests {
    use super::*;

    fn streamlines(arrows: Option<StreamlineArrows>, color: Option<&str>) -> SeriesConfig {
        SeriesConfig {
            series_id: "flow".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Streamlines {
                line: DataLineStyleConfig {
                    line_width: 1.5,
                    ..DataLineStyleConfig::default()
                },
                arrows,
                color_map: color.map(|column| PointColorMap {
                    column: column.into(),
                }),
            },
        }
    }

    #[test]
    fn streamline_rules_hold_the_line_and_the_arrows() {
        assert_eq!(validate_streamline_series(&streamlines(None, None)), Ok(()));
        let full = streamlines(Some(StreamlineArrows::default()), Some("speed"));
        assert_eq!(validate_streamline_series(&full), Ok(()));
        let edit = |f: fn(&mut DataLineStyleConfig)| {
            let mut series = full.clone();
            let DataRenderType::Streamlines { line, .. } = &mut series.render_type else {
                unreachable!()
            };
            f(line);
            validate_streamline_series(&series)
        };
        assert!(edit(|line| line.interpolation = LineInterpolation::StepMid).is_err());
        assert!(edit(|line| line.fill = Some(LineFill::ToBaseline(0.0))).is_err());
        for arrows in [
            StreamlineArrows {
                spacing_px: 0.0,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                spacing_px: f32::NAN,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                head_length_px: -1.0,
                ..StreamlineArrows::default()
            },
            StreamlineArrows {
                head_width_px: f32::INFINITY,
                ..StreamlineArrows::default()
            },
            // A head longer than the spacing would overlap the next.
            StreamlineArrows {
                spacing_px: 6.0,
                head_length_px: 8.0,
                head_width_px: 7.0,
            },
        ] {
            assert_eq!(
                validate_streamline_series(&streamlines(Some(arrows), None)),
                Err(StreamlineArrows::INVALID_REASON),
                "{arrows:?}"
            );
        }
        assert!(validate_streamline_series(&streamlines(None, Some(""))).is_err());
        let mut unnamed = full.clone();
        unnamed.y_column.clear();
        assert!(validate_streamline_series(&unnamed).is_err());
        // Other kinds pass untouched.
        let mut line = full;
        line.render_type = DataRenderType::Line {
            line: DataLineStyleConfig::default(),
        };
        assert_eq!(validate_streamline_series(&line), Ok(()));
    }

    #[test]
    fn a_streamline_series_draws_its_line_in_the_line_colour() {
        let series = streamlines(Some(StreamlineArrows::default()), None);
        let flow = series.render_type.streamlines().unwrap();
        assert_eq!(flow.line.line_width, 1.5);
        assert_eq!(flow.arrows, Some(&StreamlineArrows::default()));
        assert_eq!(flow.color_map, None);
        let mut cycled = series.clone();
        crate::preset::ColorCycle::Vivid.apply_to_series(&mut cycled, 1);
        let DataRenderType::Streamlines { line, .. } = &cycled.render_type else {
            unreachable!()
        };
        assert_ne!(line.line_color, flow.line.line_color);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn streamlines_round_trip() {
        for series in [
            streamlines(None, None),
            streamlines(Some(StreamlineArrows::default()), None),
            streamlines(
                Some(StreamlineArrows {
                    spacing_px: 64.5,
                    head_length_px: 9.0,
                    head_width_px: 6.25,
                }),
                Some("speed"),
            ),
        ] {
            let json = serde_json::to_string(&series).unwrap();
            assert!(json.contains("\"Streamlines\""));
            assert_eq!(serde_json::from_str::<SeriesConfig>(&json).unwrap(), series);
            assert_eq!(
                serde_json::to_string(&serde_json::from_str::<SeriesConfig>(&json).unwrap())
                    .unwrap(),
                json
            );
        }
        // The optional fields are omitted when absent and default on read.
        let bare = serde_json::to_string(&streamlines(None, None)).unwrap();
        assert!(!bare.contains("arrows"));
        assert!(!bare.contains("color_map"));
        let minimal = r#"{"series_id":"flow","label":null,"x_column":"x","y_column":"y","render_type":{"Streamlines":{"line":{"line_style":"Solid","line_color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0},"line_width":1.0}}}}"#;
        let parsed: SeriesConfig = serde_json::from_str(minimal).unwrap();
        let flow = parsed.render_type.streamlines().unwrap();
        assert_eq!((flow.arrows, flow.color_map), (None, None));
        assert_eq!(serde_json::to_string(&parsed).unwrap(), minimal);
    }
}
