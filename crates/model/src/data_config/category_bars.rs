//! Categorical bar series, bar groups and the value-label helpers they share.

use super::*;

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

pub(super) fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
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
pub(super) fn validate_value_label_offsets(
    offsets: &[[f32; 2]],
    categories: usize,
) -> Result<(), crate::RuleKind> {
    if !offsets.is_empty() && offsets.len() != categories {
        return Err(crate::RuleKind::ValueLabelOffsetCount);
    }
    if offsets
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || v.abs() > MAX_VALUE_LABEL_OFFSET)
    {
        return Err(crate::RuleKind::ValueLabelOffset);
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
    /// The value label offsets of a category bar, box plot, range bar or
    /// radar series (`labels.offsets`), or `None` for every other kind.
    pub fn value_label_offsets(&self) -> Option<&[[f32; 2]]> {
        match &self.render_type {
            DataRenderType::CategoryBars { labels, .. } => Some(&labels.offsets),
            DataRenderType::BoxPlot { labels, .. } => Some(&labels.offsets),
            DataRenderType::RangeBars { labels, .. } => Some(&labels.offsets),
            DataRenderType::Radar { labels, .. } => Some(&labels.offsets),
            _ => None,
        }
    }

    /// The drawn strings of a category bar, box plot, range bar or radar
    /// series' value labels (`labels.text`), or `None` for every other kind.
    pub fn value_label_text(&self) -> Option<&[String]> {
        match &self.render_type {
            DataRenderType::CategoryBars { labels, .. } => Some(&labels.text),
            DataRenderType::BoxPlot { labels, .. } => Some(&labels.text),
            DataRenderType::RangeBars { labels, .. } => Some(&labels.text),
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
    ///
    /// A Feynman series (#234) is rejected here: its label is named by
    /// `part` and row (D7), so it moves through
    /// [`FeynmanSeries::drag_label`](crate::feynman::FeynmanSeries::drag_label),
    /// from the selection's `PickedDataRef::FeynmanLabel { part, index }`.
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
            DataRenderType::RangeBars { labels, .. } => &mut labels.offsets,
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
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.material.supported_on_flat_body() {
            return Err(crate::RuleKind::CategoricalChartBarMaterial);
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
            Err(crate::RuleKind::BarStyle)
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
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if finite_in(self.group_width, 0.1, 0.95) && finite_in(self.bar_gap, 0.0, 24.0) {
            Ok(())
        } else {
            Err(crate::RuleKind::BarGroups)
        }
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
) -> Result<(), crate::RuleKind> {
    let Some(bars) = series.render_type.category_bars() else {
        return Ok(());
    };
    if !valid_bar_color(bars.color, true) {
        return Err(crate::RuleKind::BarColor);
    }
    bars.style.validate()?;
    if bars.labels.decimals > 6 || !single_line_within(&bars.labels.value_suffix, 24) {
        return Err(crate::RuleKind::BarLabelFormat);
    }
    if !bars.labels.text.is_empty() && bars.labels.text.len() != categories.len() {
        return Err(crate::RuleKind::BarLabelTextCount);
    }
    validate_value_label_offsets(&bars.labels.offsets, categories.len())?;
    if bars
        .labels
        .text
        .iter()
        .any(|text| !single_line_within(text, MAX_CATEGORY_BAR_LABEL_CHARS))
    {
        return Err(crate::RuleKind::BarLabel);
    }
    for (index, o) in bars.overrides.iter().enumerate() {
        if o.target.series_id != series.series_id
            || !categories.iter().any(|c| c.id == o.target.category_id)
            || bars.overrides[..index].iter().any(|e| e.target == o.target)
        {
            return Err(crate::RuleKind::BarOverride);
        }
        if o.color.is_some_and(|c| !valid_bar_color(c, true)) {
            return Err(crate::RuleKind::BarColor);
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
    /// of one layer are its bar group: any number of them (#107), each
    /// with its own value column, drawn against a categorical axis
    /// (`bottom_x` for vertical bars, `left_y` for horizontal ones) and a
    /// linear numeric value axis, with no cap on the number of bars (#107),
    /// and each series passes [`validate_category_bar_series`] against the
    /// axis' categories. Horizontal bars cannot sit on an overlay layer,
    /// whose own axis is a value axis on y. On a polar chart the bars are
    /// a rose (#133): wedges of the categorical angle axis (`bottom_x`)
    /// along the linear radius axis, so the group is vertical. Whether a
    /// column holds one value per category is data: the renderer checks it
    /// against the pool when it prepares.
    pub fn validate_category_bars(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
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
        for layer in self.panel_layers() {
            let members = || bars().filter(|s| self.layer_of_series(&s.series_id) == layer);
            if members().next().is_none() {
                continue;
            }
            let groups = self.bar_groups_of(layer);
            let horizontal = groups.direction == CategoryBarDirection::Horizontal;
            let overlay = matches!(layer, LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..));
            if horizontal && overlay {
                return Err(crate::RuleKind::HorizontalBarsOnOverlay);
            }
            // The axes as the layer draws them: a subplot with its links.
            let subplot = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    self.subplot_config(index)
                }
                LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => None,
            };
            let chart = subplot.as_ref().unwrap_or(self);
            if horizontal && chart.polar().is_some() {
                return Err(crate::RuleKind::RoseDirection);
            }
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
                return Err(crate::RuleKind::CategoryBarsAxis);
            };
            if value_axis.categories.is_some() || value_axis.scale != AxisScale::Linear {
                return Err(crate::RuleKind::CategoryBarsValueAxis);
            }
            for (k, item) in members().enumerate() {
                validate_category_bar_series(item, categories)?;
                let (_, values) = item.category_bar_columns(groups.direction);
                if members()
                    .take(k)
                    .any(|earlier| earlier.category_bar_columns(groups.direction).1 == values)
                {
                    return Err(crate::RuleKind::CategoryBarsValueColumns);
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
