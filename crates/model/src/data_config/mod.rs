//! Declarative series configuration types.
//!
//! Mapping layer between chart layout (`config::Config`) and raw data
//! (`data::Column`). Each `SeriesConfig` says which columns are X/Y/error
//! and which render type / style to use.

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Category};
use crate::data::ColumnId;
use crate::format::LabelFormat;
use crate::line::LineStylePreset;
use crate::material::Material;
use crate::text::RichText;
use crate::tick::TickError;
use crate::view3d::SurfaceStyle;

mod box_plot;
mod category_bars;
mod field;
mod histogram;
mod line_offsets;
mod monotonic_line;
mod radar;
mod range_bars;
mod selection;
#[cfg(test)]
mod tests;
mod vector;
mod wire;
mod xy;

pub use box_plot::*;
pub use category_bars::*;
pub use field::*;
pub use histogram::*;
pub use line_offsets::*;
pub use monotonic_line::*;
pub use radar::*;
pub use range_bars::*;
pub use vector::*;
#[cfg(feature = "serde")]
pub(crate) use wire::*;
pub use xy::*;
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
    /// Z column id: the height of a `Scatter`, `Line` or `ScatterLine`
    /// series, the anchor height of a point vector, or the vertex height of
    /// a triangle mesh (#210) on a 3D chart (`Config.view_3d`), read on
    /// `right_y`. Kept and unused on a flat chart; `None` (omitted from
    /// JSON) is every other series.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub z_column: Option<ColumnId>,
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

/// Series render type: what a series draws from its columns.
///
/// Points, a line and error bars over the `x_column` / `y_column` pair are
/// one composed kind, [`DataRenderType::Xy`]; a filled field, its contours
/// and its 3D surface over a matrix are another, [`DataRenderType::Field`]
/// (#160). The other kinds each name their own fields. Serialized documents
/// keep the earlier one-variant-per-combination form (`{"ScatterLine":
/// {...}}`), so they parse and re-serialize byte for byte.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Deserialize),
    serde(from = "WireRenderType")
)]
pub enum DataRenderType {
    /// Markers, a line and error bars over the series' x and y columns, in
    /// any combination [`XySeries::validate`] accepts.
    Xy(XySeries),
    /// A field over a matrix: its fill, its contours, or both, and its look
    /// as a surface on a 3D chart ([`FieldSeries::validate`]).
    Field(FieldSeries),

    /// Bars from pre-binned data. Binning is the host's job: the renderer is
    /// handed `(edges, counts)` and draws them.
    ///
    /// `bar.orientation` alone decides which column is which — `Vertical` reads
    /// `x_column` as edges and `y_column` as counts, `Horizontal` the reverse.
    /// The lengths are not consulted to guess the roles.
    Histogram { bar: DataBarStyleConfig },
    /// Three matrices drawn as a colour image (#139): `red`, `green` and
    /// `blue` on one lattice (the series' coordinate columns; the grids
    /// share their column count, orientation and layout), each channel's
    /// value mapped linearly from `range` to 0..1 and clamped. No colourbar
    /// is read: the colours are the data. `Flat` paints each cell its
    /// colour, `Interpolated` blends each channel between sample points; a
    /// cell with a non-finite value in any channel is unpainted. Drawn flat
    /// only; see [`Config::validate_rgb_image_series`](crate::config::Config::validate_rgb_image_series).
    RgbImage {
        red: MatrixRef,
        green: MatrixRef,
        blue: MatrixRef,
        /// The channel values that map to 0 and 1 (`[0, 1]` by default;
        /// `[0, 255]` for bytes).
        range: [f64; 2],
        shading: Shading,
        opacity: f32,
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
    /// an error. On a polar chart (`Config.polar`) with a categorical angle
    /// axis the bars fan around the centre as wedges, a rose (#133): the
    /// group is vertical, the value is the radius.
    CategoryBars {
        /// The fill colour of the series' bars, opaque.
        color: Color,
        /// The look of the bars: material, corner radius, outline, texture,
        /// gloss and the brightness of a picked bar.
        style: CategoryBarStyle,
        /// Where the value labels go, how values are written and the drawn
        /// text of each bar.
        labels: CategoryBarSeriesLabels,
        /// What a value label says.
        label_format: CategoryBarLabelFormat,
        /// Sparse per-bar overrides, keyed by category id and this series'
        /// id. At most one per bar; an override's `style` replaces the
        /// series' style whole.
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
        style: BoxPlotStyle,
        /// Which labels the boxes show, how a median is written and the
        /// drawn text of each box.
        labels: BoxPlotSeriesLabels,
        /// The ragged column of the outliers, one group per category.
        outliers: Option<ColumnId>,
        /// Sparse per-box overrides, keyed by category id and this series'
        /// id. At most one per box; an override's `style` replaces the
        /// series' style whole.
        overrides: Option<Vec<BoxPlotOverride>>,
        /// The violin drawn behind each box: a host density curve per
        /// category, mirrored or one per side ([`Violin`]). With a violin,
        /// an empty `y_column` draws no box.
        violin: Option<Violin>,
        /// The raw points drawn beside each box, jittered across the slot
        /// ([`BoxPoints`]): a raincloud with a one-sided violin.
        points: Option<BoxPoints>,
    },
    /// Range bars: one bar per row, floating from a start value to an end
    /// value at a position, with optional whiskers from a low to a high
    /// value. One kind draws waterfall charts (the running total before and
    /// after each step), candlesticks (open to close, low to high), state
    /// timelines and Gantt charts (an interval per row), and floating
    /// min–max bars.
    ///
    /// `x_column` holds the positions and `y_column` the end values,
    /// whatever the direction (the box plot convention); `start` holds the
    /// start values. Every role is a column of one value per row, supplied
    /// by the host: the renderer draws the values it is given and computes
    /// none (a running total, a duration or a difference is host data). A
    /// row with a NaN position, start or end, or with equal start and end,
    /// draws no bar.
    ///
    /// The `RangeBars` series of one layer form its range group: they stand
    /// side by side in series order (planned beside actual), laid out by
    /// `Config::bar_groups`' direction, group width and gap ([`BarGroups`]),
    /// whose mode must be grouped. `Vertical` stands the bars up from a
    /// position axis on `bottom_x` (a waterfall, candles); `Horizontal` lays
    /// them along x from a position axis on `left_y` (a Gantt chart). The
    /// position axis is categorical (one slot per category) or linear
    /// numeric, where `slot_width` gives the slot in data units (a time
    /// axis); the value axis is linear or logarithmic.
    ///
    /// `color_by` chooses each bar's fill: the series colour, the direction
    /// of the given ends (`up` when end ≥ start, else `down`, and a host
    /// flagged total), or a host state index into a style list.
    /// `progress` shades the part of a bar past a host fraction; `connector`
    /// joins each bar's end to the next bar.
    ///
    /// Value labels: `labels.text` holds the drawn strings, which the host
    /// writes (the renderer never formats a value), one per row, drawn past
    /// each bar's end (or inside it) as `labels.placement` says, moved by
    /// `labels.offsets`. An empty string draws nothing.
    ///
    /// The columns must be GPU resident; a streamed source is rejected with
    /// an error. Polar charts (`Config.polar`) reject range bars.
    RangeBars {
        /// The fill colour of the series' bars under [`RangeColor::Single`],
        /// opaque; the legend swatch and the whisker colour by default.
        color: Color,
        /// The look of the bars: material, corner radius, outline, texture,
        /// gloss and the brightness of a picked bar.
        style: CategoryBarStyle,
        /// The column of the start values.
        start: ColumnId,
        /// The whiskers from a low to a high value (a candle's wick).
        whiskers: Option<RangeWhiskers>,
        /// How each bar's fill is chosen.
        color_by: RangeColor,
        /// The finished share of each bar (a Gantt task's progress).
        progress: Option<RangeProgress>,
        /// The line from each bar's end to the next bar (a waterfall).
        connector: Option<RangeConnector>,
        /// The slot of one row along a numeric position axis, in data units;
        /// `None` on a categorical axis, where the slot is one category.
        slot_width: Option<f64>,
        /// Where the value labels go and the drawn text of each bar. The
        /// renderer never writes the text; `decimals` and `value_suffix` are
        /// not read.
        labels: CategoryBarSeriesLabels,
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
        style: RadarSeriesStyle,
        /// Which vertices carry value labels, how a value is written and
        /// the drawn text of each vertex.
        labels: RadarSeriesLabels,
    },
    /// A ternary series on a ternary chart (`Config.ternary`): markers and/or
    /// a line through samples of three components, `x_column`, `y_column`
    /// and `c_column`, each an ordinary column, unnormalized. A sample is
    /// placed at `(x·L + y·T + c·R) / (x + y + c)`, the triangle's corners
    /// weighted by its components (see [`crate::ternary`]); the renderer
    /// normalizes only to place it and reports the original components. A
    /// sample with a negative or non-finite component, or a sum ≤ 0, is not
    /// drawn and breaks the line. The markers and the line take every
    /// option of a scatter and a line series (style tables, colour and size
    /// maps, dashes) except step modes and fills. With `region`, the series'
    /// samples are the vertices of closed polygons (a NaN row separates
    /// them) filled with [`crate::ternary::TernaryRegionStyle`] under every
    /// series' markers and lines. Streamed sources are rejected.
    Ternary {
        /// The third component's column, read on `right_y`.
        c_column: ColumnId,
        scatter: Option<DataScatterStyleConfig>,
        line: Option<DataLineStyleConfig>,
        /// The fill of the polygons the samples outline; `None` (omitted
        /// from JSON) fills nothing.
        region: Option<crate::ternary::TernaryRegionStyle>,
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
        arrows: Option<StreamlineArrows>,
        /// Colour the lines and arrowheads by a host column through
        /// `Config.colorbar`.
        color_map: Option<PointColorMap>,
    },
    /// Parallel coordinates (#141): one polyline per row across the axes of
    /// a parallel chart (`Config.parallel`), row `r` through
    /// `(axis i, (v_i − min_i) / (max_i − min_i))` for every axis, the
    /// normalization only to place the sample. `columns` names one
    /// column per axis, in the axes' order; `x_column` and `y_column` are
    /// its first two. A row with a non-finite value breaks at that axis.
    /// With `color_column`, each row's polyline takes the colour of that
    /// column's value through `Config.colorbar`; without, the line's
    /// colour. Drawn in the precise draw style only, solid and linear
    /// without a fill; the columns must be GPU resident. See
    /// [`Config::validate_parallel_series`](crate::config::Config::validate_parallel_series).
    Parallel {
        /// One column id per axis of the parallel chart, in order.
        columns: Vec<ColumnId>,
        /// The rows' line: its colour (without `color_column`) and width.
        line: DataLineStyleConfig,
        /// Colour each row by a host column through `Config.colorbar`.
        color_column: Option<ColumnId>,
    },
    /// An energy-level (Grotrian) diagram (#203): levels as strokes across
    /// the slots of a categorical `bottom_x` at their energies on `left_y`,
    /// ionization limits and the transitions the host listed. DataLess:
    /// every value is SSOT configuration
    /// ([`EnergyLevelSeries`](crate::energy_levels::EnergyLevelSeries)), so
    /// `x_column` and `y_column` stay empty and nothing is registered. Drawn
    /// through the CPU sink and composited after the GPU data. See
    /// [`crate::energy_levels`] and
    /// [`Config::validate_energy_levels`](crate::config::Config::validate_energy_levels).
    EnergyLevels(crate::energy_levels::EnergyLevelSeries),
    /// A field over host-given triangles (#210): a fill, contour lines or
    /// both over the triangles of a triangle index column, whose vertices
    /// are rows of `x_column` / `y_column` (and of `z_column`, the height,
    /// on a 3D chart). The values are given per vertex or per face; the
    /// renderer interpolates given vertex values across each triangle and
    /// draws the level set of that interpolant, and computes nothing the
    /// reader reads as data. See [`crate::tri_mesh`] and
    /// [`Config::validate_tri_mesh_series`](crate::config::Config::validate_tri_mesh_series).
    TriMesh(crate::tri_mesh::TriMeshSeries),
    /// A Feynman diagram (#234): vertices, the propagators that join them
    /// and their particles. DataLess: every value is SSOT configuration
    /// ([`FeynmanSeries`](crate::feynman::FeynmanSeries)), so `x_column` and
    /// `y_column` stay empty and nothing is registered. A vertex position
    /// is optional; the automatic layout places the rest. Drawn through the
    /// CPU sink and composited after the GPU data. See [`crate::feynman`]
    /// and [`Config::validate_feynman`](crate::config::Config::validate_feynman).
    Feynman(crate::feynman::FeynmanSeries),
    /// One channel of a monotonic group (#271): evenly spaced data the host
    /// lends on every call, drawn as a line through its M4 envelope per
    /// pixel column. `x_column` and `y_column` are the renderer-owned pool
    /// columns of the drawn envelope. See [`MonotonicLineSeries`] and
    /// [`crate::monotonic`].
    MonotonicLine(MonotonicLineSeries),
}

/// A [`DataRenderType::Parallel`] series' fields, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct ParallelRef<'a> {
    pub columns: &'a [ColumnId],
    pub line: &'a DataLineStyleConfig,
    pub color_column: Option<&'a str>,
}

/// Identifier of another series in the same chart: its `series_id`. Plain
/// `String`, like [`ColumnId`] — resolution against the chart's series list is
/// the renderer's job.
pub type SeriesRef = String;

impl SeriesConfig {
    /// Every plain column this series reads, once per occurrence (#160):
    /// its x and y columns (not a box plot's record), a 3D series' height
    /// column (`z_column`, #207), its markers' style,
    /// colour and size columns, its error bars' style and error columns, a
    /// ternary series' third component, a parallel series' further axes and
    /// colour, a streamline colour column, a box plot's densities and raw
    /// points, a range bar's further columns, a field's and an image's grid
    /// columns, and a vector field's components and colour. Records and
    /// ragged columns are visited by [`Self::visit_composite_columns`].
    pub fn visit_columns<'a>(&'a self, visit: &mut impl FnMut(&'a str)) {
        // A DataLess series reads no column (#203, #234).
        if self.is_dataless() {
            return;
        }
        visit(&self.x_column);
        if self.render_type.box_plot().is_none() {
            visit(&self.y_column);
        }
        // A 3D series' height column (#132). A flat chart keeps it unused,
        // but the series still names it: removing it removes the series,
        // and a streamed one is refused (#207).
        if let Some(z) = self.z_column.as_deref().filter(|z| !z.is_empty()) {
            visit(z);
        }
        let scatter = match &self.render_type {
            DataRenderType::Xy(xy) => xy.scatter.as_ref(),
            DataRenderType::Ternary { scatter, .. } => scatter.as_ref(),
            // A radar series' markers read no column of their own.
            _ => None,
        };
        if let Some(scatter) = scatter {
            if let Some(column) = scatter.point_style_index_column.as_deref() {
                visit(column);
            }
            if let Some(map) = scatter.point_color_map.as_ref() {
                visit(&map.column);
            }
            if let Some(map) = scatter.point_size_map.as_ref() {
                visit(&map.column);
            }
        }
        if let DataRenderType::Xy(XySeries {
            errors: Some(errors),
            ..
        }) = &self.render_type
        {
            if let Some(column) = errors.style.error_bar_style_index_column.as_deref() {
                visit(column);
            }
            for error in [errors.x.as_ref(), errors.y.as_ref()].into_iter().flatten() {
                match error {
                    ErrorRef::Symmetric { column } => visit(column),
                    ErrorRef::Asymmetric { lower, upper } => {
                        visit(lower);
                        visit(upper);
                    }
                }
            }
        }
        match &self.render_type {
            // Nothing beyond what is visited above.
            DataRenderType::Xy(_)
            | DataRenderType::EnergyLevels(_)
            | DataRenderType::Feynman(_)
            // Its envelope columns are its x and y columns (#271).
            | DataRenderType::MonotonicLine(_)
            | DataRenderType::Histogram { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::Radar { .. } => {}
            DataRenderType::Ternary { c_column, .. } => visit(c_column),
            // Its triangle index column and its value column (#210); the
            // height is the `z_column` visited above.
            DataRenderType::TriMesh(mesh) => {
                visit(&mesh.triangles);
                visit(&mesh.values.column);
            }
            // The first two axis columns are its x and y columns.
            DataRenderType::Parallel {
                columns,
                color_column,
                ..
            } => {
                for column in columns.iter().skip(2) {
                    visit(column);
                }
                if let Some(column) = color_column {
                    visit(column);
                }
            }
            DataRenderType::Streamlines { color_map, .. } => {
                if let Some(map) = color_map {
                    visit(&map.column);
                }
            }
            DataRenderType::BoxPlot { .. } => {
                if let Some(boxes) = self.render_type.box_plot() {
                    for id in boxes.plain_columns() {
                        visit(id);
                    }
                }
            }
            DataRenderType::RangeBars { .. } => {
                if let Some(bars) = self.render_type.range_bars() {
                    for id in bars.extra_columns() {
                        visit(id);
                    }
                }
            }
            DataRenderType::Field(field) => {
                for column in &field.matrix.columns {
                    visit(column);
                }
            }
            DataRenderType::RgbImage {
                red, green, blue, ..
            } => {
                for column in red
                    .columns
                    .iter()
                    .chain(&green.columns)
                    .chain(&blue.columns)
                {
                    visit(column);
                }
            }
            DataRenderType::Vector { components, style } => {
                match components {
                    VectorComponents::Points { u, v, w } => {
                        visit(u);
                        visit(v);
                        if let Some(w) = w {
                            visit(w);
                        }
                    }
                    VectorComponents::Grid { u, v } => {
                        for column in u.columns.iter().chain(&v.columns) {
                            visit(column);
                        }
                    }
                }
                match style.color_map.as_ref() {
                    Some(VectorColorMap::Column(column)) => visit(column),
                    Some(VectorColorMap::Grid(matrix)) => {
                        for column in &matrix.columns {
                            visit(column);
                        }
                    }
                    None => {}
                }
            }
        }
    }

    /// The record and ragged columns this series reads: a box plot's record
    /// (its `y_column`, when it names one) and its outlier and violin curve
    /// columns. Every other kind reads none.
    pub fn visit_composite_columns<'a>(&'a self, visit: &mut impl FnMut(&'a str)) {
        if let Some(boxes) = self.render_type.box_plot() {
            if !self.y_column.is_empty() {
                visit(&self.y_column);
            }
            for id in boxes.ragged_columns() {
                visit(id);
            }
        }
    }
}
