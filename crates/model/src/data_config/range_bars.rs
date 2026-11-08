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
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RangeColor {
    /// Every bar in the series colour.
    #[default]
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

/// The fields of a [`DataRenderType::RangeBars`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct RangeBarsRef<'a> {
    pub color: Color,
    pub style: &'a CategoryBarStyle,
    pub start: &'a str,
    pub whiskers: Option<&'a RangeWhiskers>,
    pub color_by: &'a RangeColor,
    pub progress: Option<&'a RangeProgress>,
    pub connector: Option<&'a RangeConnector>,
    pub slot_width: Option<f64>,
    pub labels: &'a CategoryBarSeriesLabels,
}

impl DataRenderType {
    /// The fields of a range bar series, or `None` for every other kind.
    pub fn range_bars(&self) -> Option<RangeBarsRef<'_>> {
        match self {
            DataRenderType::RangeBars {
                color,
                style,
                start,
                whiskers,
                color_by,
                progress,
                connector,
                slot_width,
                labels,
            } => Some(RangeBarsRef {
                color: *color,
                style,
                start,
                whiskers: whiskers.as_ref(),
                color_by,
                progress: progress.as_ref(),
                connector: connector.as_ref(),
                slot_width: *slot_width,
                labels,
            }),
            _ => None,
        }
    }
}

impl<'a> RangeBarsRef<'a> {
    /// Every column the series reads besides its positions and end values
    /// (`x_column`, `y_column`): the start, the whiskers' low and high, the
    /// total flag or the state index, and the progress, in that order.
    pub fn extra_columns(self) -> impl Iterator<Item = &'a str> + 'a {
        let colour = match self.color_by {
            RangeColor::Single => None,
            RangeColor::Direction { total, .. } => total.as_ref().map(|t| t.column.as_str()),
            RangeColor::State { column, .. } => Some(column.as_str()),
        };
        std::iter::once(self.start)
            .chain(self.whiskers.map(|w| w.low.as_str()))
            .chain(self.whiskers.map(|w| w.high.as_str()))
            .chain(colour)
            .chain(self.progress.map(|p| p.column.as_str()))
    }
}

/// The most a range bar's whisker or connector is wide, in logical pixels.
pub const MAX_RANGE_STROKE_WIDTH: f32 = 24.0;

/// The rules one range bar series is held to: an opaque colour and valid
/// style, every column id not empty and distinct, whisker and connector
/// widths in `0..=`[`MAX_RANGE_STROKE_WIDTH`], opaque whisker, direction
/// and state colours, a connector colour with channels in `0..=1`, a
/// remaining opacity in `0..=1`, label decimals `0..=6` and a one-line
/// value suffix of at most 24 characters, `labels.text` of one-line strings
/// of at most [`MAX_CATEGORY_BAR_LABEL_CHARS`] characters, and
/// `labels.offsets` empty or one per string of `labels.text`.
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
    if bars.whiskers.is_some_and(|w| {
        !finite_in(w.width, 0.0, MAX_RANGE_STROKE_WIDTH)
            || w.color.is_some_and(|c| !valid_bar_color(c, true))
    }) {
        return Err(crate::RuleKind::RangeBarWhiskers);
    }
    if bars.connector.is_some_and(|c| {
        !finite_in(c.width, 0.0, MAX_RANGE_STROKE_WIDTH) || !valid_bar_color(c.color, false)
    }) {
        return Err(crate::RuleKind::RangeBarConnector);
    }
    let opaque = match bars.color_by {
        RangeColor::Single => true,
        RangeColor::Direction { up, down, total } => {
            valid_bar_color(*up, true)
                && valid_bar_color(*down, true)
                && total
                    .as_ref()
                    .is_none_or(|t| valid_bar_color(t.color, true))
        }
        RangeColor::State { styles, .. } => styles.iter().all(|s| valid_bar_color(s.color, true)),
    };
    if !opaque {
        return Err(crate::RuleKind::RangeBarStateColors);
    }
    if bars
        .progress
        .is_some_and(|p| !finite_in(p.remaining_opacity, 0.0, 1.0))
    {
        return Err(crate::RuleKind::RangeBarRemainingOpacity);
    }
    if bars.slot_width.is_some_and(|w| !(w.is_finite() && w > 0.0)) {
        return Err(crate::RuleKind::RangeBarSlotWidth);
    }
    let labels = bars.labels;
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
            let position_axis = if horizontal {
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
