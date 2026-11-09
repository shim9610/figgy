//! Box plots, violins and their raw points.

use super::*;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxPlotLabels {
    None,
    Median,
    MedianAndCount,
}

/// The shape of a box's outlier markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoxOutlierShape {
    Circle,
    Square,
}

/// The look of a box: [`DataRenderType::BoxPlot`]'s `style`, and a complete
/// per-box override. Widths and sizes are logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::box_plot_style")
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

impl BoxPlotStyle {
    /// The rules a box style is held to at every level (series, override):
    /// a material with a look on a flat body, a corner radius in `0..=3`,
    /// outline, median and whisker widths in `0..=8`, a cap ratio in `0..=1`,
    /// a notch depth in `0..=0.45`, mean and outlier sizes in `1..=24`,
    /// texture strength and gloss in `0..=1`, a texture scale in `0.1..=8`,
    /// an emphasis brightness in `0..=0.5` and colours with channels in
    /// `0..=1`.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
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
            return Err(crate::RuleKind::BoxMaterial);
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
            Err(crate::RuleKind::BoxStyle)
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

/// The violin of a [`DataRenderType::BoxPlot`] series: one body per
/// category drawn from the host's density curves, behind a box narrowed to
/// `box_width` of the slot.
///
/// The host computes the density (a KDE evaluated on its own grid): the
/// renderer draws the curve it is given, scaled to the slot by each half's
/// `density_scale`, and estimates nothing (`CLAUDE.md` rule 1).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Violin {
    pub body: ViolinBody,
    /// The share of the slot's thickness the box takes, `0..=1`, centred on
    /// the member's middle; 0 hides the box.
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::violin_box_width")
    )]
    pub box_width: f32,
}

impl Violin {
    /// The box takes a quarter of the slot by default.
    pub const DEFAULT_BOX_WIDTH: f32 = crate::defaults::violin_box_width();

    /// A violin of `body` with the default box width.
    pub fn new(body: ViolinBody) -> Self {
        Self {
            body,
            box_width: Self::DEFAULT_BOX_WIDTH,
        }
    }

    /// The halves the violin draws, low side first: a symmetric body's one
    /// half drawn on both sides, or each present half of a split one.
    pub fn halves(&self) -> impl Iterator<Item = (ViolinSide, &ViolinHalf)> {
        let (both, low, high) = match &self.body {
            ViolinBody::Symmetric(half) => (Some(half), None, None),
            ViolinBody::Split { low, high } => (None, low.as_ref(), high.as_ref()),
        };
        both.map(|h| (ViolinSide::Both, h))
            .into_iter()
            .chain(low.map(|h| (ViolinSide::Low, h)))
            .chain(high.map(|h| (ViolinSide::High, h)))
    }
}

/// The shape of a violin: one curve mirrored about the category position,
/// or a curve per side whose halves differ (a split violin, or a
/// raincloud's half violin with the points on the other side).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ViolinBody {
    /// One curve drawn on both sides: one scale and one style.
    Symmetric(ViolinHalf),
    /// A curve per side, at least one: `low` below the category position
    /// (left of a vertical violin, under a horizontal one), `high` above
    /// it.
    Split {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        low: Option<ViolinHalf>,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        high: Option<ViolinHalf>,
    },
}

/// Which side of the category position a violin half is drawn on. A pick
/// and a selection of a split violin's half name it (`Low` or `High` in
/// [`PickedDataRef::HistogramBin`](crate::config::PickedDataRef::HistogramBin)'s
/// `half`); a symmetric body is `Both` and is never named there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ViolinSide {
    /// A symmetric body, mirrored.
    Both,
    Low,
    High,
}

/// One density curve of a violin.
///
/// `curve` is a ragged column with one group per category: the positions
/// along the value axis the host evaluated its density at, ascending
/// within a group. `density` is a plain column index-aligned with the
/// curve's values: the density at each position, drawn as the half width.
/// A group that does not ascend draws its segments as given and picks
/// undefined; a NaN breaks the outline.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ViolinHalf {
    pub curve: ColumnId,
    pub density: ColumnId,
    /// The density drawn at the full half width of the slot; a larger
    /// density is drawn wider. Finite and positive ([`Self::fit_density_scale`]).
    pub density_scale: f64,
    #[cfg_attr(feature = "serde", serde(default = "crate::defaults::violin_style"))]
    pub style: ViolinStyle,
}

impl ViolinHalf {
    /// Set `density_scale` to `max`, typically the density column's maximum
    /// statistic, so the widest point of the series fills the half slot.
    ///
    /// A display helper, like [`PointSizeMap::fit_domain`]: it writes one
    /// value. A non-finite or non-positive `max` leaves the scale unchanged
    /// and returns `false`.
    pub fn fit_density_scale(&mut self, max: f64) -> bool {
        if !max.is_finite() || max <= 0.0 {
            return false;
        }
        self.density_scale = max;
        true
    }
}

/// The look of a violin half. Widths are logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::violin_style")
)]
pub struct ViolinStyle {
    /// The fill, opaque; `None` takes the series' colour (or a box
    /// override's, for its category).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub fill: Option<Color>,
    pub outline: bool,
    pub outline_width: f32,
    pub outline_color: Color,
}

/// The raw points of a [`DataRenderType::BoxPlot`] series: one marker per
/// row of two plain columns, at its category's slot, jittered across it.
///
/// `positions` holds each point's category position (the index the
/// series' `x_column` uses) and `values` its value. The jitter is the
/// renderer's: a hash of the point's index, the same on every draw and
/// pick, so a point never moves while the chart is panned or exported
/// (`CLAUDE.md` rule 1). There is no limit on the number of points.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoxPoints {
    pub positions: ColumnId,
    pub values: ColumnId,
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::box_points_style")
    )]
    pub style: BoxPointsStyle,
}

/// The look and the placement of a box plot's raw points. Sizes are
/// logical pixels; `jitter` and `offset` are shares of the half slot.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "crate::defaults::box_points_style")
)]
pub struct BoxPointsStyle {
    pub color: Color,
    /// The marker's diameter, `1..=24`.
    pub size: f32,
    pub shape: BoxOutlierShape,
    pub filled: bool,
    /// How far a point may move from its centre line, `0..=1` of the half
    /// slot, spread evenly on both sides.
    pub jitter: f32,
    /// Where the centre line sits from the member's middle, `-1..=1` of the
    /// half slot: negative on the low side, positive on the high side.
    pub offset: f32,
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
    serde(default = "crate::defaults::box_plot_series_labels")
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

/// The fields of a [`DataRenderType::BoxPlot`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct BoxPlotRef<'a> {
    pub color: Color,
    pub style: &'a BoxPlotStyle,
    pub labels: &'a BoxPlotSeriesLabels,
    pub outliers: Option<&'a str>,
    pub overrides: &'a [BoxPlotOverride],
    pub violin: Option<&'a Violin>,
    pub points: Option<&'a BoxPoints>,
}

impl<'a> BoxPlotRef<'a> {
    /// The ragged columns the series reads besides its record: the outliers
    /// and each violin half's curve (a symmetric curve once per half it
    /// names).
    pub fn ragged_columns(self) -> impl Iterator<Item = &'a str> + 'a {
        self.outliers.into_iter().chain(
            self.violin
                .into_iter()
                .flat_map(Violin::halves)
                .map(|(_, half)| half.curve.as_str()),
        )
    }

    /// The plain columns the series reads besides its positions: each
    /// violin half's density and the points' positions and values.
    pub fn plain_columns(self) -> impl Iterator<Item = &'a str> + 'a {
        self.violin
            .into_iter()
            .flat_map(Violin::halves)
            .map(|(_, half)| half.density.as_str())
            .chain(
                self.points
                    .into_iter()
                    .flat_map(|p| [p.positions.as_str(), p.values.as_str()]),
            )
    }

    /// Whether the series draws boxes: always without a violin, and with
    /// one only when it names a record and gives the box a width.
    pub fn draws_boxes(self, record: &str) -> bool {
        !record.is_empty() && self.violin.is_none_or(|v| v.box_width > 0.0)
    }
}

impl DataRenderType {
    /// The third component's column of a ternary series, or `None` for every
    /// other kind.
    /// The surface look of a heatmap kind (`Heatmap`, `HeatmapContour`),
    /// `None` for every other kind and for a heatmap without one.
    pub fn surface_style(&self) -> Option<&SurfaceStyle> {
        match self {
            DataRenderType::Field(crate::data_config::FieldSeries {
                fill: Some(_),
                surface,
                ..
            }) => surface.as_ref(),
            _ => None,
        }
    }

    pub fn ternary_c_column(&self) -> Option<&str> {
        match self {
            DataRenderType::Ternary { c_column, .. } => Some(c_column),
            _ => None,
        }
    }

    /// A parallel series' columns, line and colour column (#141), or
    /// `None` for every other kind.
    pub fn parallel(&self) -> Option<ParallelRef<'_>> {
        match self {
            DataRenderType::Parallel {
                columns,
                line,
                color_column,
            } => Some(ParallelRef {
                columns,
                line,
                color_column: color_column.as_deref(),
            }),
            _ => None,
        }
    }

    /// The three channel grids of an [`RgbImage`](DataRenderType::RgbImage)
    /// series, red first, or `None` for every other kind.
    pub fn rgb_image(&self) -> Option<[&MatrixRef; 3]> {
        match self {
            DataRenderType::RgbImage {
                red, green, blue, ..
            } => Some([red, green, blue]),
            _ => None,
        }
    }

    /// A ternary series' region fill, when it has one.
    pub fn ternary_region(&self) -> Option<&crate::ternary::TernaryRegionStyle> {
        match self {
            DataRenderType::Ternary { region, .. } => region.as_ref(),
            _ => None,
        }
    }

    /// The fields of a box plot series, or `None` for every other kind.
    pub fn box_plot(&self) -> Option<BoxPlotRef<'_>> {
        match self {
            DataRenderType::BoxPlot {
                color,
                style,
                labels,
                outliers,
                overrides,
                violin,
                points,
            } => Some(BoxPlotRef {
                color: *color,
                style,
                labels,
                outliers: outliers.as_deref(),
                overrides: overrides.as_deref().unwrap_or(&[]),
                violin: violin.as_ref(),
                points: points.as_ref(),
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
) -> Result<(), crate::RuleKind> {
    let Some(boxes) = series.render_type.box_plot() else {
        return Ok(());
    };
    if !valid_bar_color(boxes.color, true) {
        return Err(crate::RuleKind::BoxColor);
    }
    boxes.style.validate()?;
    if boxes.labels.decimals > 6 || !single_line_within(&boxes.labels.value_suffix, 24) {
        return Err(crate::RuleKind::BoxLabelFormat);
    }
    if !boxes.labels.text.is_empty() && boxes.labels.text.len() != categories.len() {
        return Err(crate::RuleKind::BoxLabelTextCount);
    }
    validate_value_label_offsets(&boxes.labels.offsets, categories.len())?;
    if boxes
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_BOX_PLOT_LABEL_CHARS))
    {
        return Err(crate::RuleKind::BoxLabel);
    }
    let (positions, record) = series.box_plot_columns();
    if positions.is_empty()
        || (record.is_empty() && boxes.violin.is_none())
        || boxes.outliers.is_some_and(str::is_empty)
    {
        return Err(crate::RuleKind::BoxPlotColumns);
    }
    if let Some(violin) = boxes.violin {
        if !finite_in(violin.box_width, 0.0, 1.0) {
            return Err(crate::RuleKind::ViolinBoxWidth);
        }
        if violin.halves().next().is_none() {
            return Err(crate::RuleKind::ViolinHalves);
        }
        for (_, half) in violin.halves() {
            if half.curve.is_empty() || half.density.is_empty() || half.curve == half.density {
                return Err(crate::RuleKind::ViolinHalfColumns);
            }
            if !half.density_scale.is_finite() || half.density_scale <= 0.0 {
                return Err(crate::RuleKind::ViolinDensityScale);
            }
            let style = &half.style;
            if !finite_in(style.outline_width, 0.0, 8.0)
                || !valid_bar_color(style.outline_color, false)
                || style.fill.is_some_and(|c| !valid_bar_color(c, true))
            {
                return Err(crate::RuleKind::ViolinOutline);
            }
        }
    }
    if let Some(points) = boxes.points {
        let style = &points.style;
        if points.positions.is_empty()
            || points.values.is_empty()
            || points.positions == points.values
        {
            return Err(crate::RuleKind::BoxPointsColumns);
        }
        if !finite_in(style.size, 1.0, 24.0)
            || !finite_in(style.jitter, 0.0, 1.0)
            || !finite_in(style.offset, -1.0, 1.0)
            || !valid_bar_color(style.color, false)
        {
            return Err(crate::RuleKind::BoxPointsStyle);
        }
    }
    for (index, o) in boxes.overrides.iter().enumerate() {
        if o.target.series_id != series.series_id
            || !categories.iter().any(|c| c.id == o.target.category_id)
            || boxes.overrides[..index]
                .iter()
                .any(|e| e.target == o.target)
        {
            return Err(crate::RuleKind::BoxOverride);
        }
        if o.color.is_some_and(|c| !valid_bar_color(c, true)) {
            return Err(crate::RuleKind::BoxColor);
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
    /// [`CategoryBarMode::Grouped`] (boxes do not stack). Any number of
    /// them (#107), drawn against a categorical axis (`bottom_x` for vertical boxes, `left_y`
    /// for horizontal ones) and a linear numeric value axis, with no cap on
    /// the number of boxes (#107); a layer draws either
    /// category bars or box plots. Each series passes
    /// [`validate_box_plot_series`] against the axis' categories.
    /// Horizontal boxes cannot sit on an overlay layer, and a polar chart
    /// draws no box plots. Whether the record holds one row per category is
    /// data: the renderer checks it against the pool when it prepares.
    pub fn validate_box_plots(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        use crate::layer::LayerRef;
        let boxes = || series.iter().filter(|s| s.render_type.box_plot().is_some());
        if boxes().next().is_none() {
            return Ok(());
        }
        if self.polar().is_some() {
            return Err(crate::RuleKind::PolarBoxPlot);
        }
        for layer in self.panel_layers() {
            let members = || boxes().filter(|s| self.layer_of_series(&s.series_id) == layer);
            if members().next().is_none() {
                continue;
            }
            if series.iter().any(|s| {
                s.render_type.category_bars().is_some()
                    && self.layer_of_series(&s.series_id) == layer
            }) {
                return Err(crate::RuleKind::BarsAndBoxesInLayer);
            }
            let groups = self.bar_groups_of(layer);
            if groups.mode != CategoryBarMode::Grouped {
                return Err(crate::RuleKind::BoxPlotsStacked);
            }
            let horizontal = groups.direction == CategoryBarDirection::Horizontal;
            let overlay = matches!(layer, LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..));
            if horizontal && overlay {
                return Err(crate::RuleKind::HorizontalBoxPlotsOnOverlay);
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
                return Err(crate::RuleKind::BoxPlotsAxis);
            };
            if value_axis.categories.is_some() || value_axis.scale != AxisScale::Linear {
                return Err(crate::RuleKind::BoxPlotsValueAxis);
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
