//! Range bars: waterfalls, candlesticks, intervals and Gantt charts.

use super::*;

/// The whiskers of a [`DataRenderType::RangeBars`] series: a stroke from
/// the low to the high value at the middle of each bar, under the bar (a
/// candle's wick).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeWhiskers {
    /// The column of the low values.
    pub low: ColumnId,
    /// The column of the high values.
    pub high: ColumnId,
    /// The stroke width in logical pixels, `0..=24`.
    pub width: f32,
    /// The stroke colour, opaque; the series colour when `None`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub color: Option<Color>,
}

/// How the bars of a [`DataRenderType::RangeBars`] series are filled.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RangeColor {
    /// Every bar in the series colour.
    Single,
    /// By the direction of the bar's given ends: `up` when end ≥ start,
    /// else `down`; a bar whose `total` flag column reads 1 takes the total
    /// colour (a waterfall's subtotals). The renderer compares the two
    /// values it was given; it computes no change.
    Direction {
        up: Color,
        down: Color,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        total: Option<RangeTotal>,
    },
    /// By a host state index per bar: the bar of state `k` (the column's
    /// value rounded) takes `styles[k]`; a state with no style takes the
    /// series colour.
    State {
        /// The column of the state indices.
        column: ColumnId,
        styles: Vec<RangeStateStyle>,
    },
    /// By the bar's row: the bar of row `i` (its row in the series'
    /// columns, as a pick reports it) takes `colors[i % colors.len()]`, a
    /// timeline's one colour per task. Empty draws every bar in the series
    /// colour. A colour mapping by row index: it reads no column and
    /// computes no value, and the list cycles, so every row draws.
    ByRow {
        /// The fill colours, opaque.
        colors: Vec<Color>,
    },
}

impl RangeColor {
    /// Whether this is [`RangeColor::Single`], which a document omits.
    pub fn is_single(&self) -> bool {
        matches!(self, RangeColor::Single)
    }
}

/// The total flag of [`RangeColor::Direction`]: a column that reads 1 for a
/// total or subtotal bar (any other value is a step), and its colour.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeTotal {
    pub column: ColumnId,
    pub color: Color,
}

/// The look of one state of [`RangeColor::State`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeStateStyle {
    /// The fill colour, opaque.
    pub color: Color,
}

/// The progress of a [`DataRenderType::RangeBars`] series: each bar is drawn
/// whole from its start to `start + p · (end − start)`, `p` the bar's value
/// in `column` clamped to `0..=1`, and past that at `remaining_opacity`. A
/// NaN progress draws the bar whole.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeProgress {
    pub column: ColumnId,
    /// The opacity of the unfinished part, `0..=1`.
    pub remaining_opacity: f32,
}

/// The connector of a [`DataRenderType::RangeBars`] series: a line at each
/// bar's end value, from the bar's edge to the next bar's (by row), as a
/// waterfall joins its steps. The last bar and a bar next to a missing one
/// have none.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeConnector {
    /// The line colour; channels in `0..=1`.
    pub color: Color,
    /// The line width in logical pixels, `0..=24`.
    pub width: f32,
}

/// One dependency link of a [`RangeLinkSeries`] (#357): from the bar of
/// row `from` to the bar of row `to` of the series' bars, finish to start.
/// A row at or past the bars' row count draws and picks nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeLink {
    /// The predecessor's row.
    pub from: u32,
    /// The successor's row.
    pub to: u32,
    /// Where the link leaves the predecessor's edge, in logical pixels
    /// from the predecessor's start (clamped to the bar when drawn); `None`
    /// is the series' default, `inset`. A user's drag writes it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub exit_offset: Option<f32>,
}

impl RangeLink {
    /// A link from row `from` to row `to` at the default exit.
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            from,
            to,
            exit_offset: None,
        }
    }
}

/// How close to the default exit, in logical pixels, a dragged exit snaps
/// back to it ([`RangeLinkSeries::drag_link_exit`]).
pub const RANGE_LINK_EXIT_SNAP: f32 = 3.0;

/// The fields of a [`DataRenderType::RangeLinks`] series (#357): the
/// dependency links between the bars of the range bar series `bars`, drawn
/// in `color`, `width` logical pixels wide: out of the predecessor's edge
/// facing the successor, `inset` pixels from its start, down to the
/// successor's centre line and into its start, ending in an open chevron
/// `head` pixels long. The series is DataLess: its links are SSOT
/// (`links`), and it reads no column (`x_column` and `y_column` empty).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeLinkSeries {
    /// The `series_id` of the range bar series whose rows the links join,
    /// in the same layer.
    pub bars: String,
    /// The stroke and head colour; channels in `0..=1`.
    pub color: Color,
    /// The stroke width in logical pixels, `0..=`[`MAX_RANGE_STROKE_WIDTH`].
    pub width: f32,
    /// The open chevron's length in logical pixels, `0..=32`.
    pub head: f32,
    /// How far from the predecessor's start a link leaves its edge by
    /// default, in logical pixels, `0..=64`.
    pub inset: f32,
    /// The links, any number of them.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub links: Vec<RangeLink>,
}

impl RangeLinkSeries {
    /// Set link `index`'s exit offset: `None`, `Some(0)` and a non-finite
    /// value clear it back to the default exit. Returns `false`, changing
    /// nothing, for an index past the links or a negative offset.
    pub fn set_link_exit_offset(&mut self, index: usize, offset: Option<f32>) -> bool {
        let offset = offset.filter(|o| o.is_finite() && *o != 0.0);
        if offset.is_some_and(|o| o < 0.0) {
            return false;
        }
        let Some(link) = self.links.get_mut(index) else {
            return false;
        };
        link.exit_offset = offset;
        true
    }

    /// Drag link `index`'s exit by `along` logical pixels along the
    /// predecessor's edge (toward its end when positive). Every link that
    /// leaves the same bar at the same exit moves with it, so a shared
    /// trunk moves as one; an exit within [`RANGE_LINK_EXIT_SNAP`] of the
    /// default snaps back to it (`None`), and one dragged before the bar's
    /// start stops half a pixel in from it, since an offset of 0 means the
    /// default.
    ///
    /// `limit` is the predecessor's drawn length in logical pixels, the
    /// farthest an exit moves (the draw clamps the exit to the bar's end):
    /// the stored offset stops there, so dragging back moves the exit at
    /// once. The renderer holds no row values on the CPU, so the length
    /// comes from the link's pick (`PickedData::HistogramBin::extent_px`,
    /// in display pixels, divided by the display scale); with `None` the
    /// offset is not clamped and only the draw stops the exit.
    pub fn drag_link_exit(
        &mut self,
        index: usize,
        along: f32,
        limit: Option<f32>,
    ) -> crate::layout::NudgeResult {
        use crate::layout::{NudgeReject, NudgeResult};
        let Some(&picked) = self.links.get(index) else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        if !along.is_finite() {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let inset = self.inset;
        let current = picked.exit_offset.unwrap_or(inset);
        let limit = limit.filter(|l| l.is_finite()).map(|l| l.max(0.5));
        // An offset stored past the bar moves from where the exit is drawn.
        let from = limit.map_or(current, |l| current.min(l));
        let moved = (from + along).max(0.5);
        let moved = limit.map_or(moved, |l| moved.min(l));
        let offset = ((moved - inset).abs() > RANGE_LINK_EXIT_SNAP).then_some(moved);
        for link in &mut self.links {
            if link.from == picked.from && link.exit_offset.unwrap_or(inset) == current {
                link.exit_offset = offset;
            }
        }
        NudgeResult::Moved
    }
}

/// The fields of a [`DataRenderType::RangeBars`] series.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RangeBarSeries {
    /// The fill colour of the series' bars under [`RangeColor::Single`],
    /// opaque; the legend swatch and the whisker colour by default.
    pub color: Color,
    /// The look of the bars: material, corner radius, outline, texture,
    /// gloss and the brightness of a picked bar.
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::category_bar_style")
    )]
    pub style: CategoryBarStyle,
    /// The column of the start values.
    pub start: ColumnId,
    /// The whiskers from a low to a high value (a candle's wick).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub whiskers: Option<RangeWhiskers>,
    /// How each bar's fill is chosen.
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "crate::defaults::range_color",
            skip_serializing_if = "RangeColor::is_single"
        )
    )]
    pub color_by: RangeColor,
    /// The finished share of each bar (a Gantt task's progress).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub progress: Option<RangeProgress>,
    /// The line from each bar's end to the next bar (a waterfall).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub connector: Option<RangeConnector>,
    /// The slot of one row along a numeric position axis, in data units;
    /// `None` on a categorical axis, where the slot is one category.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub slot_width: Option<f64>,
    /// Where the value labels go and the drawn text of each bar. The
    /// renderer never writes the text; `decimals` and `value_suffix` are
    /// not read.
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::category_bar_series_labels")
    )]
    pub labels: CategoryBarSeriesLabels,
}

impl DataRenderType {
    /// The fields of a range bar series, or `None` for every other kind.
    pub fn range_bars(&self) -> Option<&RangeBarSeries> {
        match self {
            DataRenderType::RangeBars(bars) => Some(bars),
            _ => None,
        }
    }

    /// The fields of a range link series, or `None` for every other kind.
    pub fn range_links(&self) -> Option<&RangeLinkSeries> {
        match self {
            DataRenderType::RangeLinks(links) => Some(links),
            _ => None,
        }
    }

    /// The fields of a range link series to edit, or `None` for every
    /// other kind.
    pub fn range_links_mut(&mut self) -> Option<&mut RangeLinkSeries> {
        match self {
            DataRenderType::RangeLinks(links) => Some(links),
            _ => None,
        }
    }
}

/// The most a range link's arrowhead is long and wide, in logical pixels.
pub const MAX_RANGE_LINK_HEAD: f32 = 32.0;
/// The largest exit inset of a range link, in logical pixels.
pub const MAX_RANGE_LINK_INSET: f32 = 64.0;

/// The rules one range link series is held to among the chart's `series`
/// (#357): its bars series is a range bar series of the same layer, it
/// reads no column, its width, head and inset are in range, its colour's
/// channels in `0..=1`, and every exit offset is finite and not negative.
/// A link's rows are checked against the bars when drawn: one past them
/// draws nothing, and `series_draw_info` reports it as `truncated`.
pub fn validate_range_link_series(
    config: &crate::config::Config,
    series: &[SeriesConfig],
    item: &SeriesConfig,
) -> Result<(), crate::RuleKind> {
    let Some(links) = item.render_type.range_links() else {
        return Ok(());
    };
    let bars = series
        .iter()
        .find(|s| s.series_id == links.bars && s.render_type.range_bars().is_some());
    match bars {
        Some(bars)
            if config.layer_of_series(&bars.series_id)
                == config.layer_of_series(&item.series_id) => {}
        _ => return Err(crate::RuleKind::RangeLinkBars),
    }
    if !item.x_column.is_empty() || !item.y_column.is_empty() || item.z_column.is_some() {
        return Err(crate::RuleKind::RangeLinkColumns);
    }
    let offset_ok = |o: Option<f32>| o.is_none_or(|o| o.is_finite() && o >= 0.0);
    if !finite_in(links.width, 0.0, MAX_RANGE_STROKE_WIDTH)
        || !finite_in(links.head, 0.0, MAX_RANGE_LINK_HEAD)
        || !finite_in(links.inset, 0.0, MAX_RANGE_LINK_INSET)
        || !valid_bar_color(links.color, false)
        || !links.links.iter().all(|l| offset_ok(l.exit_offset))
    {
        return Err(crate::RuleKind::RangeLinkStyle);
    }
    Ok(())
}

impl RangeBarSeries {
    /// Every column the series reads besides its positions and end values
    /// (`x_column`, `y_column`): the start, the whiskers' low and high, the
    /// total flag or the state index, and the progress, in that order.
    pub fn extra_columns(&self) -> impl Iterator<Item = &str> + '_ {
        let colour = match &self.color_by {
            RangeColor::Single => None,
            RangeColor::Direction { total, .. } => total.as_ref().map(|t| t.column.as_str()),
            RangeColor::State { column, .. } => Some(column.as_str()),
            RangeColor::ByRow { .. } => None,
        };
        let whiskers = self.whiskers.as_ref();
        std::iter::once(self.start.as_str())
            .chain(whiskers.map(|w| w.low.as_str()))
            .chain(whiskers.map(|w| w.high.as_str()))
            .chain(colour)
            .chain(self.progress.as_ref().map(|p| p.column.as_str()))
    }
}

/// The most a range bar's whisker or connector is wide, in logical pixels.
pub const MAX_RANGE_STROKE_WIDTH: f32 = 24.0;

/// The rules one range bar series is held to: an opaque colour and valid
/// style, every column id not empty and distinct, whisker and connector
/// widths in `0..=`[`MAX_RANGE_STROKE_WIDTH`], opaque whisker, direction,
/// state and row colours, a connector colour with channels in `0..=1`, a
/// remaining opacity in `0..=1`, label decimals `0..=6` and a one-line
/// value suffix of at most 24 characters, `labels.text` of one-line strings
/// of at most [`MAX_CATEGORY_BAR_LABEL_CHARS`] characters, and
/// `labels.offsets` empty or one per string of `labels.text`, and opaque
/// `labels.contrast_ink` colours.
pub fn validate_range_bar_series(series: &SeriesConfig) -> Result<(), crate::RuleKind> {
    let Some(bars) = series.render_type.range_bars() else {
        return Ok(());
    };
    if !valid_bar_color(bars.color, true) {
        return Err(crate::RuleKind::RangeBarColor);
    }
    bars.style.validate()?;
    let columns = || {
        [series.x_column.as_str(), series.y_column.as_str()]
            .into_iter()
            .chain(bars.extra_columns())
    };
    if columns().any(str::is_empty) {
        return Err(crate::RuleKind::RangeBarColumns);
    }
    if columns()
        .enumerate()
        .any(|(k, id)| columns().take(k).any(|earlier| earlier == id))
    {
        return Err(crate::RuleKind::RangeBarColumnsDistinct);
    }
    if bars.whiskers.as_ref().is_some_and(|w| {
        !finite_in(w.width, 0.0, MAX_RANGE_STROKE_WIDTH)
            || w.color.is_some_and(|c| !valid_bar_color(c, true))
    }) {
        return Err(crate::RuleKind::RangeBarWhiskers);
    }
    if bars.connector.as_ref().is_some_and(|c| {
        !finite_in(c.width, 0.0, MAX_RANGE_STROKE_WIDTH) || !valid_bar_color(c.color, false)
    }) {
        return Err(crate::RuleKind::RangeBarConnector);
    }
    let opaque = match &bars.color_by {
        RangeColor::Single => true,
        RangeColor::Direction { up, down, total } => {
            valid_bar_color(*up, true)
                && valid_bar_color(*down, true)
                && total
                    .as_ref()
                    .is_none_or(|t| valid_bar_color(t.color, true))
        }
        RangeColor::State { styles, .. } => styles.iter().all(|s| valid_bar_color(s.color, true)),
        RangeColor::ByRow { colors } => colors.iter().all(|c| valid_bar_color(*c, true)),
    };
    if !opaque {
        return Err(crate::RuleKind::RangeBarStateColors);
    }
    if bars
        .progress
        .as_ref()
        .is_some_and(|p| !finite_in(p.remaining_opacity, 0.0, 1.0))
    {
        return Err(crate::RuleKind::RangeBarRemainingOpacity);
    }
    if bars.slot_width.is_some_and(|w| !(w.is_finite() && w > 0.0)) {
        return Err(crate::RuleKind::RangeBarSlotWidth);
    }
    let labels = &bars.labels;
    if labels.decimals > 6 || !single_line_within(&labels.value_suffix, 24) {
        return Err(crate::RuleKind::RangeBarLabelFormat);
    }
    if labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_CATEGORY_BAR_LABEL_CHARS))
    {
        return Err(crate::RuleKind::RangeBarLabel);
    }
    if labels
        .contrast_ink
        .is_some_and(|inks| !inks.iter().all(|c| valid_bar_color(*c, true)))
    {
        return Err(crate::RuleKind::RangeBarContrastInk);
    }
    validate_value_label_offsets(&labels.offsets, labels.text.len())?;
    Ok(())
}

impl crate::config::Config {
    /// The rules the range bar series of a chart are held to, with the
    /// series list it draws.
    ///
    /// The [`DataRenderType::RangeBars`] series of one layer are its range
    /// group: any number of them, side by side in series order, laid out by
    /// the layer's `bar_groups` ([`crate::config::Config::bar_groups_of`]),
    /// whose mode must be [`CategoryBarMode::Grouped`]. A layer draws
    /// either category bars, box plots or range bars. The position axis
    /// (`bottom_x` for vertical bars, `left_y` for horizontal ones) is
    /// categorical, with no `slot_width`, or linear numeric, with a
    /// `slot_width`; the value axis is numeric, linear or logarithmic. A
    /// connector needs its series to be the only range bar series of its
    /// layer (it would cross its neighbours). Each series passes
    /// [`validate_range_bar_series`]. Horizontal range bars cannot sit on an
    /// overlay layer, and a polar chart draws no range bars. Whether every
    /// column holds one value per row is data: the renderer checks it
    /// against the pool when it prepares.
    pub fn validate_range_bars(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        use crate::layer::LayerRef;
        let ranges = || {
            series
                .iter()
                .filter(|s| s.render_type.range_bars().is_some())
        };
        if ranges().next().is_none() {
            return Ok(());
        }
        if self.polar().is_some() {
            return Err(crate::RuleKind::PolarRangeBars);
        }
        for layer in self.panel_layers() {
            let members = || ranges().filter(|s| self.layer_of_series(&s.series_id) == layer);
            if members().next().is_none() {
                continue;
            }
            if series.iter().any(|s| {
                (s.render_type.category_bars().is_some() || s.render_type.box_plot().is_some())
                    && self.layer_of_series(&s.series_id) == layer
            }) {
                return Err(crate::RuleKind::RangeBarsWithOtherBars);
            }
            let groups = self.bar_groups_of(layer);
            if groups.mode != CategoryBarMode::Grouped {
                return Err(crate::RuleKind::RangeBarsStacked);
            }
            let horizontal = groups.direction == CategoryBarDirection::Horizontal;
            let overlay = matches!(layer, LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..));
            if horizontal && overlay {
                return Err(crate::RuleKind::HorizontalRangeBarsOnOverlay);
            }
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            // The axes the layer's series are scaled by: an overlay's own
            // or, linked, its chart's.
            let (x_axis, y_axis) = chart.scaling_axes(match layer {
                LayerRef::Overlay(index) | LayerRef::SubplotOverlay(_, index) => Some(index),
                _ => None,
            });
            let (position_axis, value_axis) = if horizontal {
                (y_axis, x_axis)
            } else {
                (x_axis, y_axis)
            };
            if value_axis.categories.is_some() {
                return Err(crate::RuleKind::RangeBarsValueAxis);
            }
            let categorical = position_axis.categories.is_some();
            if !categorical && position_axis.scale != AxisScale::Linear {
                return Err(crate::RuleKind::RangeBarsPositionAxis);
            }
            let connectors = members().any(|s| {
                s.render_type
                    .range_bars()
                    .is_some_and(|b| b.connector.is_some())
            });
            if connectors && members().nth(1).is_some() {
                return Err(crate::RuleKind::RangeBarConnectorLayer);
            }
            for item in members() {
                validate_range_bar_series(item)?;
                let slot = item.render_type.range_bars().and_then(|b| b.slot_width);
                if categorical == slot.is_some() {
                    return Err(crate::RuleKind::RangeBarSlotWidthAxis);
                }
            }
        }
        Ok(())
    }
}

impl crate::config::Config {
    /// The rules the range link series of a chart are held to, with the
    /// series list it draws: each passes [`validate_range_link_series`].
    pub fn validate_range_links(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        for item in series {
            validate_range_link_series(self, series, item)?;
        }
        Ok(())
    }
}
