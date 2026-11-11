//! User-facing `Chart` — wraps a `Config` with rescale / auto_fit helpers.
//!
//! `Config` is the SSoT for chart options (axes, labels, margins, colors).
//! Edit only the desired config fields for precise control. For example,
//! changing axis `min` / `max` directly preserves its tick and label settings.
//! Convenience helpers may derive related settings: `set_x_range` chooses
//! ticks and label formatting as well as bounds, and `with_title` can grow
//! the title band. Each helper documents the fields it adjusts.
//!
//! `Chart` adds one-line conveniences (`set_x_range`, `auto_fit_x`, …) plus
//! dirty-flag tracking: `data_dirty` (GPU transform only) and `raster_dirty`
//! (CPU axis raster) are set automatically based on the kind of `Config`
//! change, and cleared by the caller via `consume_*_dirty()`.

use crate::LayerRef;
use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, LegendEntryKind};
use crate::data_config::{BarOrientation, DataRenderType, SeriesConfig};
use crate::data_render::ColumnPool;
use crate::error::{FiggyError, Result};
use crate::format::LabelFormat;
use crate::text::{RichText, rich_segments_from_label};
use crate::text_render::CpuTextMeasure;

/// Apply min/max and scale-appropriate tick settings to one axis, for the
/// tick plan of `scale` (the axis' own scale or, for a mirrored axis, its
/// partner's). The spacing is chosen for the range the axis prints: its
/// shown range with a unit (#273), where `major_spacing` is read.
pub(crate) fn apply_axis_range(
    axis: &mut crate::config::AxisOptions,
    min: f64,
    max: f64,
    scale: &AxisScale,
) {
    // A categorical axis keeps its category range here: the `auto_fit_*`
    // family leaves it as is. A view moves it through `apply_view_range`.
    if axis.categories.is_some() {
        return;
    }
    axis.min = min;
    axis.max = max;
    match scale {
        AxisScale::Logarithmic => {
            axis.minor_count = 8;
            axis.label_style.format = LabelFormat::Power;
            axis.label_style.significant_digits = 1;
        }
        // #209: the tick walk reads the range alone; `minor_count` is a
        // switch, and the labels are percentages with each tail's digits.
        AxisScale::Probability(_) => {
            axis.minor_count = 1;
            axis.label_style.format = LabelFormat::Percent;
            axis.label_style.significant_digits = 1;
        }
        AxisScale::Linear => {
            axis.minor_count = 4;
            if !matches!(axis.label_style.format, LabelFormat::Timestamp(_)) {
                axis.label_style.format = LabelFormat::Decimal;
            }
            axis.label_style.significant_digits = 3;
        }
    }
    // After the label format, which decides whether the unit applies.
    let (shown_min, shown_max) = axis.shown_range();
    axis.major_spacing = crate::tick::axis_major_spacing(scale, shown_min, shown_max);
}

/// How a range reaches one axis: [`apply_view_range`] or
/// [`apply_axis_range`].
type ApplyRange = fn(&mut AxisOptions, f64, f64, &AxisScale);

/// Apply a view's range to one axis (#205): a categorical axis takes it as
/// its [`AxisOptions::category_window`] (`None` again for the full range),
/// a numeric one as [`apply_axis_range`] does.
pub(crate) fn apply_view_range(axis: &mut AxisOptions, min: f64, max: f64, scale: &AxisScale) {
    if let Some(n) = axis.category_count() {
        let full = (-0.5, n as f64 - 0.5);
        axis.category_window = ((min, max) != full).then_some([min, max]);
        return;
    }
    apply_axis_range(axis, min, max, scale);
}

/// Fill `rt.segments` with the label `s`: plain text with its `$…$` spans
/// as TeX ([`rich_segments_from_label`]).
fn fill_label_segments(rt: &mut RichText, s: &str) {
    rt.segments = rich_segments_from_label(s);
}

pub(crate) use crate::layout::guarded_log_range;

/// Positive-preserving padding for a log-scale axis: pad by
/// `padding_ratio · span` in log space on each side, then back to linear.
pub(crate) fn log_padded(min: f64, max: f64, padding_ratio: f64) -> (f64, f64) {
    if !(min.is_finite() && max.is_finite()) || min <= 0.0 || max <= min {
        return (min, max);
    }
    let lmin = min.log10();
    let lmax = max.log10();
    let span = lmax - lmin;
    let pad = span * padding_ratio.max(0.0);
    (10f64.powf(lmin - pad), 10f64.powf(lmax + pad))
}

/// The in-domain ends of a probability axis' fit (#209): the extent's ends
/// that lie in `(0, 1)`. An end outside takes the mirror of the other
/// (`1 − p`), which is where plotting positions put it (`0.5/n` and
/// `1 − 0.5/n`); the lower end may come from `min_positive` when the
/// minimum is at or below zero. Both ends outside fit nothing. No new pool
/// statistic (#209 open decision 6).
fn probability_fit_ends(ext: &FitExtent) -> Option<(f64, f64)> {
    let inside = |p: f64| p > 0.0 && p < 1.0;
    let lower = if inside(ext.min) {
        Some(ext.min)
    } else {
        ext.min_positive.filter(|&p| inside(p))
    };
    let upper = inside(ext.max).then_some(ext.max);
    let (min, max) = match (lower, upper) {
        (Some(min), Some(max)) => (min, max),
        (Some(min), None) => (min, 1.0 - min),
        (None, Some(max)) => (1.0 - max, max),
        (None, None) => return None,
    };
    (min < max).then_some((min, max))
}

/// Padding for a probability axis (#209): `padding_ratio · span` in `F⁻¹`
/// units on each side, then back to probabilities, so the padded range
/// stays inside `(0, 1)`.
fn probability_padded(scale: &AxisScale, min: f64, max: f64, padding_ratio: f64) -> (f64, f64) {
    let (Some(lo), Some(hi)) = (scale.forward(min), scale.forward(max)) else {
        return (min, max);
    };
    let pad = (hi - lo) * padding_ratio.max(0.0);
    match (scale.inverse(lo - pad), scale.inverse(hi + pad)) {
        (Some(a), Some(b)) if a > 0.0 && b < 1.0 && a < b => (a, b),
        _ => (min, max),
    }
}

/// One axis' fit input: the union of everything that must stay visible.
/// `min_positive` carries the log-axis safe floor (smallest positive bound)
/// so log fitting works even when the extent dips to zero or below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitExtent {
    pub min: f64,
    pub max: f64,
    pub min_positive: Option<f64>,
}

impl FitExtent {
    /// Neutral element for [`Self::union`] — folds away.
    pub const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        min_positive: None,
    };

    /// The extent of one value: a baseline a histogram or a stem series
    /// adds to its value axis (`min_positive` only when it is above zero).
    pub fn point(value: f64) -> Self {
        Self {
            min: value,
            max: value,
            min_positive: (value > 0.0).then_some(value),
        }
    }

    /// Widen to also cover `other`.
    pub fn union(&mut self, other: &FitExtent) {
        if other.min < self.min {
            self.min = other.min;
        }
        if other.max > self.max {
            self.max = other.max;
        }
        self.min_positive = match (self.min_positive, other.min_positive) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }

    /// A fit is only meaningful over a finite, non-degenerate span — the
    /// same gate the column-union fitters have always applied.
    fn is_fittable(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.max > self.min
    }

    /// The extent of the same values drawn `by` further along the axis: `by`
    /// data units on a linear axis, `by` decades (a factor of `10^by`) on a
    /// logarithmic one. A display shift of a line offset member
    /// (`Config::line_offset_of`); the values themselves are not moved.
    fn shifted(&self, by: f64, scale: &AxisScale) -> Self {
        if by == 0.0 {
            return *self;
        }
        if let AxisScale::Probability(_) = scale {
            // Shifted in `F⁻¹` units, as `shift_axis` shifts the axis
            // (#209); a value outside (0, 1) has no position and stays.
            let shift = |p: f64| {
                scale
                    .forward(p)
                    .and_then(|t| scale.inverse(t + by))
                    .unwrap_or(p)
            };
            return Self {
                min: shift(self.min),
                max: shift(self.max),
                min_positive: self.min_positive.map(shift),
            };
        }
        if matches!(scale, AxisScale::Logarithmic) {
            let factor = 10f64.powf(by);
            return Self {
                min: self.min * factor,
                max: self.max * factor,
                min_positive: self.min_positive.map(|value| value * factor),
            };
        }
        let min = self.min + by;
        Self {
            min,
            max: self.max + by,
            // A positive bound the shifted values keep: the shifted minimum,
            // else the shifted floor.
            min_positive: if min > 0.0 {
                Some(min)
            } else {
                self.min_positive
                    .map(|value| value + by)
                    .filter(|value| *value > 0.0)
            },
        }
    }
}

/// Validate the invariants required before a `Config` becomes renderer-owned.
///
/// Raster helpers remain defensive against invalid ad-hoc values, but the
/// renderer registry never publishes a zero target, a non-finite/reversed
/// range, or a non-positive logarithmic range.
pub(crate) fn validate_renderer_config(config: &Config) -> Result<()> {
    // Every rule lives in the model (#160); the renderer only maps the
    // failure to its error type.
    config.validate_chart().map_err(|error| match error.field {
        "chart_area" => FiggyError::InvalidChartArea {
            width: config.chart_area.0.width,
            height: config.chart_area.0.height,
        },
        field => FiggyError::InvalidConfig {
            field,
            kind: error.kind,
        },
    })
}

fn fitted_axis_range(
    axis: &AxisOptions,
    ext: &FitExtent,
    padding_ratio: f64,
) -> Option<(f64, f64)> {
    if !ext.is_fittable() {
        return None;
    }
    let range = match &axis.scale {
        AxisScale::Logarithmic => {
            let safe = if ext.min > 0.0 {
                ext.min
            } else {
                ext.min_positive?
            };
            log_padded(safe, ext.max, padding_ratio)
        }
        AxisScale::Probability(_) => {
            let (min, max) = probability_fit_ends(ext)?;
            probability_padded(&axis.scale, min, max, padding_ratio)
        }
        AxisScale::Linear => Chart::padded(ext.min, ext.max, padding_ratio),
    };
    (range.0.is_finite() && range.1.is_finite() && range.1 > range.0).then_some(range)
}

/// Fit one axis that has no partner, such as an overlay's y axis, to
/// `ext` with the auto-fit rules ([`fitted_axis_range`], [`apply_axis_range`]).
pub(crate) fn apply_axis_fit(
    axis: &mut crate::config::AxisOptions,
    ext: &FitExtent,
    padding_ratio: f64,
) {
    if let Some((min, max)) = fitted_axis_range(axis, ext, padding_ratio) {
        let scale = axis.scale.clone();
        apply_axis_range(axis, min, max, &scale);
    }
}

/// Apply GPU-reduced x/y fit bounds directly to an owned `Config`.
///
/// This is allocation-free and infallible after extent validation, allowing
/// the renderer to preflight every checked revision before atomically
/// publishing the two axes.
pub(crate) fn apply_auto_fit_all(
    config: &mut Config,
    x_extent: &FitExtent,
    y_extent: &FitExtent,
    padding_ratio: f64,
) {
    // A ternary chart's axes span the whole triangle, not a data extent; a
    // parallel chart's are the categorical positions and the normalized band
    // (`Chart::auto_fit_parallel_axes` fits its axes).
    if config.ternary().is_some() || config.parallel().is_some() {
        return;
    }
    let mirror = partner_mirrors(config);
    if let Some((min, max)) = fitted_axis_range(&config.bottom_x, x_extent, padding_ratio) {
        apply_pair_range(
            [&mut config.bottom_x, &mut config.top_x],
            mirror,
            min,
            max,
            apply_axis_range,
        );
    }
    if let Some((min, max)) = fitted_axis_range(&config.left_y, y_extent, padding_ratio) {
        apply_pair_range(
            [&mut config.left_y, &mut config.right_y],
            mirror,
            min,
            max,
            apply_axis_range,
        );
    }
}

/// Write `min..max` to both axes of a pair, the main axis (`bottom_x` or
/// `left_y`) and its partner (`top_x` or `right_y`), and derive their tick
/// and label settings.
///
/// On a chart whose partner axis frames the main one ([`partner_mirrors`]),
/// the partner first takes the main axis's `scale`, so both edges show the
/// same ticks: a logarithmic `left_y` gives `right_y` decades too. A second
/// independent value axis on the right is an overlay layer
/// ([`Config::overlays`]), whose `y_axis` keeps its own scale. Each axis
/// then takes the tick settings of its own scale, so where the partner is a
/// separate axis (the z axis of a 3D chart, the c axis of a ternary one) it
/// keeps its scale and gets the ticks of it. A categorical axis keeps its
/// scale and range under a fit ([`apply_axis_range`]); a view moves its
/// window ([`apply_view_range`]). `apply` is the one of the two. A
/// categorical partner of a numeric main axis is a separate axis whose
/// range is not in the main axis's units, so it keeps its window under a
/// view too.
fn apply_pair_range(
    pair: [&mut AxisOptions; 2],
    mirror: bool,
    min: f64,
    max: f64,
    apply: ApplyRange,
) {
    let [main, partner] = pair;
    if mirror && main.categories.is_none() && partner.categories.is_none() {
        partner.scale = main.scale.clone();
    }
    let partner_follows = main.categories.is_some() || partner.categories.is_none();
    let main_scale = main.scale.clone();
    apply(main, min, max, &main_scale);
    if partner_follows {
        let scale = partner.scale.clone();
        apply(partner, min, max, &scale);
    }
}

/// Whether `config`'s `top_x` and `right_y` frame `bottom_x` and `left_y`
/// (every chart but a 3D and a ternary one, where `right_y` is the z or the
/// c axis), so the range helpers give them the main axis's scale
/// ([`apply_pair_range`]).
fn partner_mirrors(config: &Config) -> bool {
    config.view_3d().is_none() && config.ternary().is_none()
}

/// The value-axis extent of the category bars `layer` of `config` draws,
/// from the pool's statistics, with its bar direction.
///
/// The bars grow from zero, so zero is always in it. Grouped bars span the
/// union of their value columns' ranges. Stacked bars span
/// `[Σ min(0, min_i), Σ max(0, max_i)]` over the group's value columns,
/// `min_i` and `max_i` being each column's own statistics
/// ([`ColumnSlot::min`] and [`ColumnSlot::max`]): positive values stack up
/// from zero and negative ones down, so no category's stack reaches past the
/// sum of the series' extremes, and a series with both signs adds its
/// positive part up and its negative part down. This is an upper bound,
/// exact when every series has its extremes in one category and wider when
/// they sit in different categories (two series peaking at 30 and 40 in
/// different categories fit 70 however low their other bars are). The
/// exact per-category totals are the shader's, computed from the pool on
/// every draw; the fit reads no value back, so the group's columns may be
/// uploaded in any batches. Percent stacks span `0..=100`: the shader draws
/// a category's shares from 0 to 100, and a percent group with a negative
/// value is refused when it is drawn. A layer without category bars gives
/// the empty extent.
///
/// [`ColumnSlot::min`]: crate::data_render::ColumnSlot::min
/// [`ColumnSlot::max`]: crate::data_render::ColumnSlot::max
pub fn category_bar_value_extent(
    pool: &ColumnPool,
    config: &Config,
    layer: LayerRef,
    series: &[SeriesConfig],
) -> Result<(FitExtent, crate::data_config::CategoryBarDirection)> {
    use crate::data_config::CategoryBarMode;
    let groups = config.bar_groups_of(layer);
    let members = || {
        series.iter().filter(move |s| {
            s.render_type.category_bars().is_some() && config.layer_of_series(&s.series_id) == layer
        })
    };
    let zero = FitExtent {
        min: 0.0,
        max: 0.0,
        min_positive: None,
    };
    let mut extent = FitExtent::EMPTY;
    if members().next().is_none() {
        return Ok((extent, groups.direction));
    }
    extent.union(&zero);
    match groups.mode {
        CategoryBarMode::Grouped => {
            for item in members() {
                extent.union(&Chart::slot_extent(
                    pool,
                    item.category_bar_columns(groups.direction).1,
                )?);
            }
        }
        CategoryBarMode::Stacked => {
            // Each column adds its positive extreme up from zero and its
            // negative extreme down; its smallest positive value bounds the
            // lowest positive bar end from below, the log-axis floor.
            let mut stack = zero;
            for item in members() {
                let column =
                    Chart::slot_extent(pool, item.category_bar_columns(groups.direction).1)?;
                if column.min.is_finite() {
                    stack.min += column.min.min(0.0);
                }
                if column.max.is_finite() {
                    stack.max += column.max.max(0.0);
                }
                stack.min_positive = match (stack.min_positive, column.min_positive) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            extent.union(&stack);
        }
        CategoryBarMode::PercentStacked => extent.union(&FitExtent {
            min: 0.0,
            max: 100.0,
            min_positive: Some(100.0),
        }),
    }
    Ok((extent, groups.direction))
}

/// The extents a range bar series `item` of `config` draws over, from the
/// pool's scalar statistics, with its direction: its value axis spans the
/// finite range of its start and end columns and, with whiskers, of its
/// low and high ones, zero not forced (a bar floats, and a price axis may
/// be logarithmic); its position axis, when it is numeric (the series has
/// a `slot_width`), spans its positions grown by half a slot on each side,
/// and is empty on a categorical axis, which no fit moves. Nothing per bar
/// is read back.
pub fn range_bar_extents(
    pool: &ColumnPool,
    config: &Config,
    item: &SeriesConfig,
) -> Result<(
    FitExtent,
    FitExtent,
    crate::data_config::CategoryBarDirection,
)> {
    let groups = config.bar_groups_of(config.layer_of_series(&item.series_id));
    let mut value = FitExtent::EMPTY;
    let mut position = FitExtent::EMPTY;
    let Some(bars) = item.render_type.range_bars() else {
        return Ok((value, position, groups.direction));
    };
    value.union(&Chart::slot_extent(pool, &item.y_column)?);
    value.union(&Chart::slot_extent(pool, bars.start)?);
    if let Some(w) = bars.whiskers {
        value.union(&Chart::slot_extent(pool, &w.low)?);
        value.union(&Chart::slot_extent(pool, &w.high)?);
    }
    if let Some(width) = bars.slot_width {
        let positions = Chart::slot_extent(pool, &item.x_column)?;
        if positions.min.is_finite() && positions.max.is_finite() {
            let (min, max) = (positions.min - width * 0.5, positions.max + width * 0.5);
            position.union(&FitExtent {
                min,
                max,
                min_positive: if min > 0.0 {
                    Some(min)
                } else {
                    (max > 0.0).then_some(max)
                },
            });
        }
    }
    Ok((value, position, groups.direction))
}

/// The value-axis extent of the box plots `layer` of `config` draws, from
/// the pool's scalar statistics, with their direction.
///
/// Each box plot series spans the finite range of its record's value fields
/// (every field but `sample_count`: the summary, the mean and the
/// confidence interval, as the former box plot fitted every supplied value)
/// and of its outlier column, hidden means and outliers included, so a style
/// toggle never moves the axis. The statistics are the ones the pool
/// measured when the record and the outliers were written; nothing per box
/// is read back. A layer without box plots gives the empty extent.
pub fn box_plot_value_extent(
    pool: &ColumnPool,
    config: &Config,
    layer: LayerRef,
    series: &[SeriesConfig],
) -> Result<(FitExtent, crate::data_config::CategoryBarDirection)> {
    let groups = config.bar_groups_of(layer);
    let mut extent = FitExtent::EMPTY;
    let mut include = |min: f64, max: f64| {
        if min.is_finite() && max.is_finite() && min <= max {
            let min_positive = if min > 0.0 {
                Some(min)
            } else if max > 0.0 {
                Some(max)
            } else {
                None
            };
            extent.union(&FitExtent {
                min,
                max,
                min_positive,
            });
        }
    };
    for item in series.iter().filter(|s| {
        s.render_type.box_plot().is_some() && config.layer_of_series(&s.series_id) == layer
    }) {
        let (_, record_id) = item.box_plot_columns();
        // A violin without boxes names no record.
        if !record_id.is_empty() {
            let record = pool
                .record(record_id)
                .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    kind: crate::RuleKind::BoxPlotRecordNotResident {
                        id: record_id.to_string(),
                    },
                })?;
            let stats = record.stats();
            for (k, (name, _)) in crate::data_render::BOX_PLOT_RECORD.iter().enumerate() {
                if *name != "sample_count" && stats.present[k] && stats.finite[k] > 0 {
                    include(stats.min[k], stats.max[k]);
                }
            }
        }
        let boxes = item
            .render_type
            .box_plot()
            .ok_or(FiggyError::internal("filtered to box plot series"))?;
        // The outliers and the violin curves: ragged columns of values.
        for id in boxes.ragged_columns() {
            let ragged = pool
                .ragged(id)
                .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    kind: crate::RuleKind::BoxPlotRaggedNotResident { id: id.to_string() },
                })?;
            let stats = ragged.stats();
            if stats.finite > 0 {
                include(stats.min, stats.max);
            }
        }
        // The raw points' values.
        if let Some(points) = boxes.points
            && let Some(slot) = pool.slot(&points.values)
        {
            include(slot.min, slot.max);
        }
    }
    Ok((extent, groups.direction))
}

/// A single figgy chart panel: a `Config`, the decoration selection its
/// raster highlights, and dirty-flag tracking.
pub struct Chart {
    config: Config,
    data_dirty: bool,
    raster_dirty: bool,
    /// The element the raster draws selected, by its element id
    /// (`Selectable::element_id`).
    selected: Option<String>,
    /// Moves on every raster-affecting change: [`Renderer::prepare`] redraws
    /// a view drawn at another stamp (#265). Each value is an issued number
    /// from one process-wide counter ([`issue_raster_stamp`]), so two charts
    /// never share one and a view cannot mistake another chart for its own.
    /// `None` once the counter is exhausted: an edit cannot fail, so
    /// `prepare` reports `FiggyError::CounterExhausted` instead.
    ///
    /// [`Renderer::prepare`]: crate::Renderer::prepare
    raster_stamp: Option<u64>,
}

/// The next chart raster stamp: an issued number from one process-wide
/// counter, incremented with a check and never wrapped, so no stamp is
/// reused. `None` once exhausted. See "Identity and staleness checks" in
/// `ARCHITECTURE.md`.
fn issue_raster_stamp() -> Option<u64> {
    static NEXT_RASTER_STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT_RASTER_STAMP
        .try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| current.checked_add(1),
        )
        .ok()
        .map(|previous| previous + 1)
}

impl Chart {
    /// New chart. Both dirty flags start true so the first frame draws.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            data_dirty: true,
            raster_dirty: true,
            selected: None,
            raster_stamp: issue_raster_stamp(),
        }
    }

    /// The raster changed: the dirty flag a host reads, and a new stamp for
    /// [`Renderer::prepare`](crate::Renderer::prepare).
    fn mark_raster(&mut self) {
        self.raster_dirty = true;
        self.raster_stamp = issue_raster_stamp();
    }

    /// The stamp of the chart's current raster content; `None` when the
    /// stamp counter is exhausted.
    pub(crate) fn raster_stamp(&self) -> Option<u64> {
        self.raster_stamp
    }

    /// The element the raster draws selected, by its element id.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Select the element `element_id` names (`Selectable::element_id`:
    /// `"axis_bottom"`, `"legend"`, `"overlay_0:legend"`, ...), or none.
    /// The next [`Renderer::prepare`](crate::Renderer::prepare) draws its
    /// selection box, as [`Renderer::refresh_axis_with_selection`](crate::Renderer::refresh_axis_with_selection)
    /// does. An id no element of the active layer has draws nothing.
    pub fn select(&mut self, element_id: Option<&str>) {
        if self.selected.as_deref() != element_id {
            self.selected = element_id.map(str::to_owned);
            self.mark_raster();
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// New chart with every pixel-based dimension multiplied by `scale`.
    /// Used for high-DPI export.
    pub fn scaled(&self, scale: f32) -> Self {
        Self::new(self.config.scaled(scale))
    }

    /// General mutable access. Sets both dirty flags because the caller may
    /// touch anything. Prefer `with_axis_range_change` / `with_decoration_change`
    /// when the kind of change is known.
    /// Only the caller's assignments change the config: this accessor does
    /// not fit ranges, choose ticks or fit the title band.
    pub fn config_mut(&mut self) -> &mut Config {
        self.data_dirty = true;
        self.mark_raster();
        &mut self.config
    }

    /// Mutate where only axis range / scale changes. Raster is invalidated
    /// because tick labels need to be repositioned within the same chart_area.
    /// Change only `min` / `max` here to preserve tick spacing, minor count
    /// and label formatting. Unlike [`Self::set_x_range`] / [`Self::set_y_range`],
    /// this accessor performs no coupled config adjustments. Change both axes
    /// of a pair explicitly when they should share the new range.
    pub fn with_axis_range_change<F: FnOnce(&mut Config)>(&mut self, f: F) {
        f(&mut self.config);
        self.data_dirty = true;
        self.mark_raster();
    }

    /// Decoration-only change (titles, colors, margins, …): raster only.
    /// Only the fields written by the closure change. In particular, assigning
    /// title segments here keeps `top_margin`; [`Self::with_title`] also fits
    /// that band. The caller still refreshes the view before preparing.
    pub fn with_decoration_change<F: FnOnce(&mut Config)>(&mut self, f: F) {
        f(&mut self.config);
        self.mark_raster();
    }

    // Dirty-flag interface.

    pub fn data_dirty(&self) -> bool {
        self.data_dirty
    }
    pub fn raster_dirty(&self) -> bool {
        self.raster_dirty
    }

    /// Clear `data_dirty` and return its previous value.
    pub fn consume_data_dirty(&mut self) -> bool {
        let was = self.data_dirty;
        self.data_dirty = false;
        was
    }
    pub fn consume_raster_dirty(&mut self) -> bool {
        let was = self.raster_dirty;
        self.raster_dirty = false;
        was
    }

    /// Manually flag the data as dirty — used when column data is mutated
    /// outside this chart and there's no automatic detection path.
    pub fn invalidate(&mut self) {
        self.data_dirty = true;
    }

    // Text builders (raster only). Each takes its text as a label, the way
    // matplotlib does: `$…$`, `\(…\)` and `$$…$$` spans are TeX, `\$` is a
    // dollar sign, and text whose TeX does not parse is shown as written
    // ([`crate::text::rich_segments_from_label`]).

    /// Set the chart title text as a label (plain text with `$…$` spans as
    /// TeX). For styled segments
    /// (bold, italic, …) edit `config_mut().chart_title.text` directly; a host
    /// that wants the same band fit then calls
    /// [`crate::layout::fit_chart_title_margin`] with
    /// [`CpuTextMeasure::for_style`].
    ///
    /// As a helper it also derives the coupled title band: when the new text
    /// is taller than `top_margin` (a multi-line title, `'\n'`), the band
    /// grows so the first line is not clipped at the top of the chart. A
    /// title that fits leaves the band unchanged.
    pub fn with_title(mut self, text: &str) -> Self {
        fill_label_segments(&mut self.config.chart_title.text, text);
        self.fit_chart_title_margin();
        self.mark_raster();
        self
    }

    /// Grow `chart_title.top_margin` when the title, measured with the font
    /// this chart's draw style renders, is taller than its band
    /// ([`crate::layout::fit_chart_title_margin`]). Returns whether the band
    /// changed, which marks the raster dirty.
    fn fit_chart_title_margin(&mut self) -> bool {
        let measure = CpuTextMeasure::for_style(&self.config.draw_style);
        let changed = crate::layout::fit_chart_title_margin(&mut self.config, &measure);
        if changed {
            self.mark_raster();
        }
        changed
    }

    /// Set the bottom X axis title text, a label with `$…$` spans as TeX.
    pub fn with_x_title(mut self, text: &str) -> Self {
        fill_label_segments(&mut self.config.bottom_x.title_option.text, text);
        self.mark_raster();
        self
    }

    /// Set the left Y axis title text, a label with `$…$` spans as TeX.
    pub fn with_y_title(mut self, text: &str) -> Self {
        fill_label_segments(&mut self.config.left_y.title_option.text, text);
        self.mark_raster();
        self
    }

    /// Append a legend entry to the legend's one-document content. The first
    /// call also makes the legend visible. The mark is built from `kind` as
    /// inline text segments carrying the series color as a per-segment
    /// override (`line_width` biases nothing today — the symbol glyph
    /// carries the look); entries are separated by explicit `'\n'` segments.
    /// For custom layouts (one-line legends, mid-text symbols) edit
    /// `config_mut().legend.content.segments` directly. `label` is a label
    /// with `$…$` spans as TeX.
    pub fn with_legend_entry(
        mut self,
        label: &str,
        color: Color,
        line_width: f32,
        kind: LegendEntryKind,
    ) -> Self {
        let _ = line_width;
        let symbol = crate::config::symbol_segments(&kind, color);
        crate::config::append_legend_entry_rich(
            &mut self.config.legend.content,
            symbol,
            rich_segments_from_label(label),
        );
        self.config.legend.visible = true;
        self.mark_raster();
        self
    }

    // Axis-range helpers.

    /// Set both X ranges (top + bottom) and derive their tick/label settings.
    ///
    /// This is a convenience helper for automatic setup. To keep the current
    /// ticks and formatting, write only `min` / `max` through
    /// [`Self::with_axis_range_change`] instead.
    ///
    /// `top_x` first takes `bottom_x`'s `scale` (on a 3D or ternary chart it
    /// keeps its own), then each axis branches on its scale:
    /// - **Linear**: `major_spacing` is
    ///   [`crate::tick::axis_major_spacing`] of the range: 0.2, 0.5 or 1 ×
    ///   10^k of the span, for 5 to 10 major intervals (span 72 → 10,
    ///   8 → 1, 0.7 → 0.1, 10 → 2). `minor_count = 4`, `format = Decimal`
    ///   (an existing timestamp format is retained), `sig_digits = 3`.
    /// - **Logarithmic**: `major_spacing` is the same function's decade step
    ///   (1 up to 6 decades, 2 up to 12, then `ceil(decades / 6)`),
    ///   `minor_count = 8` (2..9 within each decade), `format = Power` (10ⁿ
    ///   superscript), `sig_digits = 1`.
    ///
    /// An axis with a [`crate::config::AxisOptions::unit`] (#273) gets the
    /// spacing of its shown range, the units `major_spacing` is read in.
    pub fn set_x_range(&mut self, min: f64, max: f64) {
        self.x_range(min, max, apply_view_range);
    }

    /// The X pair's range through `apply`: [`apply_view_range`] for a view,
    /// which moves a categorical axis' window (#205), [`apply_axis_range`]
    /// for a fit, which leaves a categorical axis as it is.
    fn x_range(&mut self, min: f64, max: f64, apply: ApplyRange) {
        self.with_axis_range_change(|c| {
            let mirror = partner_mirrors(c);
            apply_pair_range([&mut c.bottom_x, &mut c.top_x], mirror, min, max, apply);
        });
    }

    /// Set both Y ranges (left + right) and derive their tick/label settings,
    /// the Y counterpart of [`Self::set_x_range`]. To preserve ticks and
    /// formatting, write only `min` / `max` through
    /// [`Self::with_axis_range_change`] instead.
    pub fn set_y_range(&mut self, min: f64, max: f64) {
        self.y_range(min, max, apply_view_range);
    }

    /// The Y pair's range through `apply` (see [`Self::x_range`]).
    fn y_range(&mut self, min: f64, max: f64, apply: ApplyRange) {
        self.with_axis_range_change(|c| {
            let mirror = partner_mirrors(c);
            apply_pair_range([&mut c.left_y, &mut c.right_y], mirror, min, max, apply);
        });
    }

    /// Set overlay layer `layer`'s Y range and derive its tick/label settings,
    /// the overlay counterpart of [`Self::set_y_range`]. Log/linear branching
    /// uses that axis' own scale. A `layer` past the overlay list is a no-op.
    pub fn set_overlay_y_range(&mut self, layer: usize, min: f64, max: f64) {
        self.overlay_y_range(layer, min, max, apply_view_range);
    }

    /// Overlay layer `layer`'s Y range through `apply` (see
    /// [`Self::x_range`]).
    fn overlay_y_range(&mut self, layer: usize, min: f64, max: f64, apply: ApplyRange) {
        let Some(scale) = self
            .config
            .overlays
            .get(layer)
            .map(|overlay| overlay.y_axis.scale.clone())
        else {
            return;
        };
        self.with_axis_range_change(|c| {
            if let Some(overlay) = c.overlays.get_mut(layer) {
                apply(&mut overlay.y_axis, min, max, &scale);
            }
        });
    }

    /// Fit overlay layer `layer`'s Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_y_extent`]. A `layer` past the overlay list is a
    /// no-op.
    pub fn auto_fit_overlay_y_extent(&mut self, layer: usize, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = self
            .config
            .overlays
            .get(layer)
            .and_then(|overlay| fitted_axis_range(&overlay.y_axis, ext, padding_ratio))
        else {
            return;
        };
        self.overlay_y_range(layer, min, max, apply_axis_range);
    }

    /// Auto-fit overlay layer `layer`'s Y axis to the union range of the
    /// given columns' cached statistics — the overlay counterpart of
    /// [`Self::auto_fit_y_union`].
    pub fn auto_fit_overlay_y_union(
        &mut self,
        pool: &ColumnPool,
        layer: usize,
        y_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in y_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_overlay_y_extent(layer, &ext, padding_ratio);
        Ok(())
    }

    /// Set subplot `index`'s X range (top + bottom) and derive its tick and
    /// label settings, the subplot counterpart of [`Self::set_x_range`]. A
    /// linked `bottom_x` writes the root of its link group, so every linked
    /// chart moves with it. An `index` past the subplot list is a no-op.
    pub fn set_subplot_x_range(&mut self, index: usize, min: f64, max: f64) {
        self.set_subplot_range(index, true, min, max, apply_view_range);
    }

    /// Set subplot `index`'s Y range (left + right); see
    /// [`Self::set_subplot_x_range`].
    pub fn set_subplot_y_range(&mut self, index: usize, min: f64, max: f64) {
        self.set_subplot_range(index, false, min, max, apply_view_range);
    }

    fn set_subplot_range(&mut self, index: usize, x: bool, min: f64, max: f64, apply: ApplyRange) {
        if index >= self.config.subplots.len() {
            return;
        }
        let root = self.config.subplot_link_root(index, x);
        self.with_axis_range_change(|config| {
            let pair = |chart: &mut Config| {
                let mirror = partner_mirrors(chart);
                let axes = if x {
                    [&mut chart.bottom_x, &mut chart.top_x]
                } else {
                    [&mut chart.left_y, &mut chart.right_y]
                };
                apply_pair_range(axes, mirror, min, max, apply);
            };
            match root {
                crate::CellRef::Parent => pair(config),
                crate::CellRef::Subplot(root) => pair(&mut config.subplots[root].config),
            }
            if root != crate::CellRef::Subplot(index) {
                pair(&mut config.subplots[index].config);
            }
            config.sync_subplot_links(None);
        });
    }

    /// Fit subplot `index`'s X axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_x_extent`]. An `index` past the list is a no-op.
    pub fn auto_fit_subplot_x_extent(&mut self, index: usize, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = self
            .config
            .subplot_config(index)
            .filter(|chart| chart.ternary().is_none() && chart.parallel().is_none())
            .and_then(|chart| fitted_axis_range(&chart.bottom_x, ext, padding_ratio))
        else {
            return;
        };
        self.set_subplot_range(index, true, min, max, apply_axis_range);
    }

    /// Fit subplot `index`'s Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_y_extent`].
    pub fn auto_fit_subplot_y_extent(&mut self, index: usize, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = self
            .config
            .subplot_config(index)
            .filter(|chart| chart.ternary().is_none() && chart.parallel().is_none())
            .and_then(|chart| fitted_axis_range(&chart.left_y, ext, padding_ratio))
        else {
            return;
        };
        self.set_subplot_range(index, false, min, max, apply_axis_range);
    }

    /// Auto-fit subplot `index`'s X axis to the union range of the given
    /// columns' cached statistics — the subplot counterpart of
    /// [`Self::auto_fit_x_union`].
    pub fn auto_fit_subplot_x_union(
        &mut self,
        pool: &ColumnPool,
        index: usize,
        x_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in x_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_subplot_x_extent(index, &ext, padding_ratio);
        Ok(())
    }

    /// Auto-fit subplot `index`'s Y axis to the union range of the given
    /// columns' cached statistics.
    pub fn auto_fit_subplot_y_union(
        &mut self,
        pool: &ColumnPool,
        index: usize,
        y_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in y_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_subplot_y_extent(index, &ext, padding_ratio);
        Ok(())
    }

    /// Pad `[min, max]` by `padding_ratio · (max - min)` on each side
    /// (e.g. `0.05` adds 5% on each side).
    fn padded(min: f64, max: f64, padding_ratio: f64) -> (f64, f64) {
        if !(min.is_finite() && max.is_finite()) || max <= min {
            return (min, max);
        }
        let span = max - min;
        let pad = span * padding_ratio.max(0.0);
        (min - pad, max + pad)
    }

    /// One column's contribution to a fit: min/max plus the cached
    /// `min_positive` scalar (the log-axis safe floor when the raw min is
    /// ≤ 0 — log10 of it would be undefined and would wreck decade ticks).
    /// All scalars were computed at upload; no data rescan. This is the
    /// single-column metadata path used by `auto_fit_x` / `auto_fit_y`.
    /// The web `auto_fit_all` path instead reduces each series' exact
    /// drawable x/y domain directly from the GPU column pool.
    pub fn slot_extent(pool: &ColumnPool, id: &str) -> Result<FitExtent> {
        let slot = pool
            .slot(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })?;
        Ok(FitExtent {
            min: slot.min,
            max: slot.max,
            min_positive: slot.min_positive,
        })
    }

    /// Fit the X axis to a prepared [`FitExtent`] — the shared tail of every
    /// auto-fit path. Hosts with an already-prepared extent, including the
    /// web exact-series reducer, call this directly. No-op on a degenerate
    /// extent (non-finite or zero span) and on a log axis with no positive
    /// bound to anchor to.
    /// A successful fit calls [`Self::set_x_range`], so it also chooses ticks
    /// and label formatting for both X axes. For range-only edits, write the
    /// chosen bounds through [`Self::with_axis_range_change`] instead.
    /// A ternary chart is left as it is: its axes span the triangle.
    pub fn auto_fit_x_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        if self.config.ternary().is_some() || self.config.parallel().is_some() {
            return;
        }
        let Some((min, max)) = fitted_axis_range(&self.config.bottom_x, ext, padding_ratio) else {
            return;
        };
        self.x_range(min, max, apply_axis_range);
    }

    /// Fit the Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_x_extent`].
    pub fn auto_fit_y_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        if self.config.ternary().is_some() || self.config.parallel().is_some() {
            return;
        }
        let Some((min, max)) = fitted_axis_range(&self.config.left_y, ext, padding_ratio) else {
            return;
        };
        self.y_range(min, max, apply_axis_range);
    }

    /// Fit the chart's z axis to a prepared extent. The colourbar owns the
    /// z scale, so a chart without one is intentionally left unchanged.
    /// A successful fit also chooses colourbar ticks and label formatting.
    /// To preserve them, directly edit only `colorbar.axis.min` / `max`.
    pub fn auto_fit_colorbar_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        let Some(axis) = self.config.colorbar.as_ref().map(|bar| &bar.axis) else {
            return;
        };
        let Some((min, max)) = fitted_axis_range(axis, ext, padding_ratio) else {
            return;
        };
        let scale = axis.scale.clone();
        self.with_axis_range_change(|config| {
            if let Some(bar) = config.colorbar.as_mut() {
                apply_axis_range(&mut bar.axis, min, max, &scale);
            }
        });
    }

    /// Auto-fit the colourbar to the union of matrix value columns.
    pub fn auto_fit_colorbar_union(
        &mut self,
        pool: &ColumnPool,
        z_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in z_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_colorbar_extent(&ext, padding_ratio);
        Ok(())
    }

    /// Auto-fit the X axis range from the pool's column metadata for `x_id`.
    /// Uses multiplicative (positive-preserving) padding for log scale.
    pub fn auto_fit_x(&mut self, pool: &ColumnPool, x_id: &str, padding_ratio: f64) -> Result<()> {
        let ext = Self::slot_extent(pool, x_id)?;
        self.auto_fit_x_extent(&ext, padding_ratio);
        Ok(())
    }

    /// Auto-fit the Y axis range from the pool's column metadata for `y_id`.
    /// Uses multiplicative padding for log scale.
    pub fn auto_fit_y(&mut self, pool: &ColumnPool, y_id: &str, padding_ratio: f64) -> Result<()> {
        let ext = Self::slot_extent(pool, y_id)?;
        self.auto_fit_y_extent(&ext, padding_ratio);
        Ok(())
    }

    /// Auto-fit the X axis to the union range of multiple columns — used when
    /// several series share the same X axis.
    pub fn auto_fit_x_union(
        &mut self,
        pool: &ColumnPool,
        x_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in x_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_x_extent(&ext, padding_ratio);
        Ok(())
    }

    /// Auto-fit the Y axis to the union range of multiple columns — used when
    /// several series share the same Y axis.
    pub fn auto_fit_y_union(
        &mut self,
        pool: &ColumnPool,
        y_ids: &[&str],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut ext = FitExtent::EMPTY;
        for id in y_ids {
            ext.union(&Self::slot_extent(pool, id)?);
        }
        self.auto_fit_y_extent(&ext, padding_ratio);
        Ok(())
    }

    /// Fit `layer`'s axes to prepared extents with proportional padding.
    ///
    /// `Base` (and `Annotations`, which draws on the base axes) and
    /// `Subplot(i)` fit that chart's X and Y axes. An overlay layer fits only
    /// its own Y axis and ignores `x`. A linked axis moves its whole link
    /// group, and subplots linked to a fitted parent axis follow it. Choose
    /// the series whose extents to union with [`Config::series_fit_roles`];
    /// [`Self::auto_fit_layer_union`] does both from cached column
    /// statistics.
    ///
    /// A ternary chart's axes span the whole triangle, not a data extent
    /// (`Config.ternary`): every fit leaves them as they are.
    pub fn auto_fit_layer_extent(
        &mut self,
        layer: LayerRef,
        x: &FitExtent,
        y: &FitExtent,
        padding_ratio: f64,
    ) {
        match layer {
            LayerRef::Base | LayerRef::Annotations => self.with_axis_range_change(|c| {
                apply_auto_fit_all(c, x, y, padding_ratio);
                c.sync_subplot_links(None);
            }),
            LayerRef::Overlay(index) => self.auto_fit_overlay_y_extent(index, y, padding_ratio),
            LayerRef::Subplot(index) => {
                self.auto_fit_subplot_x_extent(index, x, padding_ratio);
                self.auto_fit_subplot_y_extent(index, y, padding_ratio);
            }
            LayerRef::SubplotOverlay(index, overlay) => {
                let Some(subplot) = self.config.subplots.get(index) else {
                    return;
                };
                let mut inner = Chart::new(subplot.config.clone());
                inner.auto_fit_overlay_y_extent(overlay, y, padding_ratio);
                let Some(fitted) = inner.config.overlays.get(overlay) else {
                    return;
                };
                let axis = fitted.y_axis.clone();
                self.with_axis_range_change(|c| {
                    c.subplots[index].config.overlays[overlay].y_axis = axis;
                });
            }
        }
    }

    /// Fit `layer`'s axes to the cached column statistics of every series
    /// that takes part ([`Config::series_fit_roles`]): a linked axis fits the
    /// series of its whole link group, not only those of the chart `layer`
    /// names. Each series contributes its X and Y columns; a histogram's
    /// baseline joins its count axis. Error bars, cell half-widths and arrow
    /// tips are not in the column statistics: for those, measure the series
    /// with `Renderer::begin_series_fit_extent` and fit with
    /// [`Self::auto_fit_layer_extent`].
    ///
    /// A line offset member (`Config::line_offsets`) contributes its extents
    /// as it is drawn, moved by its shift ([`Self::line_offset_fit_extents`]).
    /// A ternary chart is left as it is ([`Self::auto_fit_layer_extent`]).
    pub fn auto_fit_layer_union(
        &mut self,
        pool: &ColumnPool,
        layer: LayerRef,
        series: &[SeriesConfig],
        padding_ratio: f64,
    ) -> Result<()> {
        // A parallel chart fits each of its axes to its columns (#141); the
        // categorical positions and the normalized band stay.
        if self.config.parallel().is_some()
            && matches!(layer, LayerRef::Base | LayerRef::Annotations)
        {
            return self.auto_fit_parallel_axes(pool, series, padding_ratio);
        }
        let mut x = FitExtent::EMPTY;
        let mut y = FitExtent::EMPTY;
        for cfg in series {
            let (fit_x, fit_y) = self.config.series_fit_roles(layer, &cfg.series_id);
            // A category bar series fits its value axis to its group's bars;
            // its category positions sit on a categorical axis no fit moves.
            if cfg.render_type.category_bars().is_some() {
                let own = self.config.layer_of_series(&cfg.series_id);
                let (extent, direction) =
                    category_bar_value_extent(pool, &self.config, own, series)?;
                match direction {
                    crate::data_config::CategoryBarDirection::Vertical if fit_y => y.union(&extent),
                    crate::data_config::CategoryBarDirection::Horizontal if fit_x => {
                        x.union(&extent)
                    }
                    _ => {}
                }
                continue;
            }
            // A range bar series fits its value axis to its ends and
            // whiskers, and a numeric position axis to its slots.
            if cfg.render_type.range_bars().is_some() {
                let (value, position, direction) = range_bar_extents(pool, &self.config, cfg)?;
                let (along_x, along_y) = match direction {
                    crate::data_config::CategoryBarDirection::Vertical => (position, value),
                    crate::data_config::CategoryBarDirection::Horizontal => (value, position),
                };
                if fit_x {
                    x.union(&along_x);
                }
                if fit_y {
                    y.union(&along_y);
                }
                continue;
            }
            // A box plot series fits its value axis to its records' and
            // outliers' statistics; its positions sit on a categorical axis.
            if cfg.render_type.box_plot().is_some() {
                let own = self.config.layer_of_series(&cfg.series_id);
                let (extent, direction) =
                    box_plot_value_extent(pool, &self.config, own, std::slice::from_ref(cfg))?;
                match direction {
                    crate::data_config::CategoryBarDirection::Vertical if fit_y => y.union(&extent),
                    crate::data_config::CategoryBarDirection::Horizontal if fit_x => {
                        x.union(&extent)
                    }
                    _ => {}
                }
                continue;
            }
            if fit_x || fit_y {
                let extent = |fit: bool, column: &str| {
                    if fit {
                        Self::slot_extent(pool, column)
                    } else {
                        Ok(FitExtent::EMPTY)
                    }
                };
                let (mut own_x, mut own_y) =
                    (extent(fit_x, &cfg.x_column)?, extent(fit_y, &cfg.y_column)?);
                // A stem's baseline joins its value axis (#206), before a
                // line offset member's shift.
                if let DataRenderType::Xy(crate::data_config::XySeries {
                    stem: Some(stem), ..
                }) = &cfg.render_type
                {
                    let baseline = FitExtent::point(stem.baseline);
                    match stem.direction {
                        crate::data_config::CategoryBarDirection::Vertical if fit_y => {
                            own_y.union(&baseline)
                        }
                        crate::data_config::CategoryBarDirection::Horizontal if fit_x => {
                            own_x.union(&baseline)
                        }
                        _ => {}
                    }
                }
                let (member_x, member_y) =
                    Self::line_offset_fit_extents(&self.config, &cfg.series_id, own_x, own_y);
                if fit_x {
                    x.union(&member_x);
                }
                if fit_y {
                    y.union(&member_y);
                }
            }
            if let DataRenderType::Histogram { bar } = &cfg.render_type {
                let baseline = FitExtent {
                    min: bar.baseline,
                    max: bar.baseline,
                    min_positive: (bar.baseline > 0.0).then_some(bar.baseline),
                };
                match bar.orientation {
                    BarOrientation::Vertical if fit_y => y.union(&baseline),
                    BarOrientation::Horizontal if fit_x => x.union(&baseline),
                    _ => {}
                }
            }
        }
        self.auto_fit_layer_extent(layer, &x, &y, padding_ratio);
        self.auto_fit_view_3d_z(pool, layer, series, padding_ratio)?;
        Ok(())
    }

    /// On a parallel chart (`Config.parallel`, #141), fit each parallel
    /// axis's range to the union of the cached statistics of the column
    /// every parallel series names for it, padded by `padding_ratio` (a
    /// logarithmic axis from the columns' smallest positive value): a
    /// display parameter from column statistics (rule 1), nothing read back.
    /// An axis no series names, or whose columns' extent is degenerate, is
    /// left as it is; a flat chart is left as it is.
    pub fn auto_fit_parallel_axes(
        &mut self,
        pool: &ColumnPool,
        series: &[SeriesConfig],
        padding_ratio: f64,
    ) -> Result<()> {
        let axes = match self.config.parallel() {
            Some(parallel) => parallel.axes.len(),
            None => return Ok(()),
        };
        for i in 0..axes {
            let mut ext = FitExtent::EMPTY;
            for cfg in series {
                if let Some(column) = cfg.render_type.parallel().and_then(|p| p.columns.get(i)) {
                    ext.union(&Self::slot_extent(pool, column)?);
                }
            }
            let Some(axis) = self
                .config
                .parallel()
                .map(|parallel| parallel.axes[i].axis_options(&self.config.left_y))
            else {
                return Ok(());
            };
            let Some((min, max)) = fitted_axis_range(&axis, &ext, padding_ratio) else {
                continue;
            };
            self.with_axis_range_change(|c| {
                if let Some(axis) = c.parallel_mut().and_then(|p| p.axes.get_mut(i)) {
                    axis.min = min;
                    axis.max = max;
                }
            });
        }
        Ok(())
    }

    /// On a 3D chart (`Config.view_3d`), fit the z axis (`right_y`) to the
    /// union of the height columns' cached statistics of the layer's
    /// heatmaps and contours and of the `z_column` of its scatter and line
    /// series (#132). A display parameter from column statistics (rule 1):
    /// nothing is read back. A flat chart is left as it is.
    pub fn auto_fit_view_3d_z(
        &mut self,
        pool: &ColumnPool,
        layer: LayerRef,
        series: &[SeriesConfig],
        padding_ratio: f64,
    ) -> Result<()> {
        let chart = match layer {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                match self.config.subplots.get(index) {
                    Some(subplot) => &subplot.config,
                    None => return Ok(()),
                }
            }
            _ => &self.config,
        };
        if chart.view_3d().is_none() {
            return Ok(());
        }
        let mut z = FitExtent::EMPTY;
        for cfg in series {
            let (fit_x, fit_y) = self.config.series_fit_roles(layer, &cfg.series_id);
            if !(fit_x || fit_y) {
                continue;
            }
            let matrix = match &cfg.render_type {
                DataRenderType::Field(crate::data_config::FieldSeries { matrix, .. }) => matrix,
                _ => {
                    if let Some(id) = cfg.z_column.as_deref() {
                        let anchors = Self::slot_extent(pool, id)?;
                        z.union(&anchors);
                        // A `Data`-scaled 3D arrow's tips are its anchor
                        // heights plus `factor · w` (#211, D7): the bound
                        // from the two columns' statistics holds every tip.
                        if let Some(tips) = Self::vector_tip_heights(pool, cfg, &anchors)? {
                            z.union(&tips);
                        }
                    }
                    continue;
                }
            };
            for id in &matrix.columns {
                z.union(&Self::slot_extent(pool, id)?);
            }
        }
        let Some((min, max)) = fitted_axis_range(&chart.right_y, &z, padding_ratio) else {
            return Ok(());
        };
        let scale = chart.right_y.scale.clone();
        self.with_axis_range_change(|c| {
            let chart = match layer {
                LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                    &mut c.subplots[index].config
                }
                _ => c,
            };
            apply_axis_range(&mut chart.right_y, min, max, &scale);
        });
        Ok(())
    }

    /// The heights a `Data`-scaled 3D point vector's tips can reach, from
    /// its anchor heights' extent and its `w` column's statistics:
    /// `[z_min + f·w_min, z_max + f·w_max]`, ordered for a negative factor.
    /// A display parameter (rule 1): a superset of the tips, never the tips.
    /// `None` for any other series.
    fn vector_tip_heights(
        pool: &ColumnPool,
        cfg: &SeriesConfig,
        anchors: &FitExtent,
    ) -> Result<Option<FitExtent>> {
        use crate::data_config::{VectorComponents, VectorScale};
        let DataRenderType::Vector {
            components: VectorComponents::Points { w: Some(w), .. },
            style,
        } = &cfg.render_type
        else {
            return Ok(None);
        };
        let VectorScale::Data { factor } = style.scale else {
            return Ok(None);
        };
        let w = Self::slot_extent(pool, w)?;
        let (low, high) = if factor >= 0.0 {
            (w.min, w.max)
        } else {
            (w.max, w.min)
        };
        let min = anchors.min + factor * low;
        let max = anchors.max + factor * high;
        if !(min.is_finite() && max.is_finite()) {
            return Ok(None);
        }
        let mut tips = FitExtent::point(min);
        tips.union(&FitExtent::point(max));
        Ok(Some(tips))
    }

    /// The `x` and `y` extents of series `series_id` of `config` as they are
    /// drawn: moved by its line offset (`Config::line_offset_of`) on the axes
    /// that draw it, a factor of `10^shift` on a logarithmic axis. A series
    /// that is not a member keeps both. The fit of a layer unions these, so
    /// the base range covers every member where it is drawn.
    pub fn line_offset_fit_extents(
        config: &Config,
        series_id: &str,
        x: FitExtent,
        y: FitExtent,
    ) -> (FitExtent, FitExtent) {
        let Some([dx, dy]) = config.line_offset_of(series_id) else {
            return (x, y);
        };
        let (chart, y_axis) = match config.layer_of_series(series_id) {
            LayerRef::Subplot(index) => {
                let chart = &config.subplots[index].config;
                (chart, &chart.left_y)
            }
            LayerRef::SubplotOverlay(index, overlay) => {
                let chart = &config.subplots[index].config;
                (chart, &chart.overlays[overlay].y_axis)
            }
            LayerRef::Overlay(overlay) => (config, &config.overlays[overlay].y_axis),
            LayerRef::Base | LayerRef::Annotations => (config, &config.left_y),
        };
        (
            x.shifted(dx, &chart.bottom_x.scale),
            y.shifted(dy, &y_axis.scale),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probability axis (#209) validates on a chart axis and a subplot:
    /// the renderer draws it (the refusal of the model PR is gone).
    #[test]
    fn probability_axes_validate_for_drawing() {
        let probability = AxisScale::Probability(crate::config::ProbabilityScale::Normal);
        let on = |axis: &mut AxisOptions| {
            axis.scale = probability.clone();
            axis.min = 0.01;
            axis.max = 0.99;
        };
        let base = crate::default::default_config();
        let mut left = base.clone();
        on(&mut left.left_y);
        let mut subplot = base.clone();
        let mut inner = base.clone();
        on(&mut inner.bottom_x);
        subplot.subplots.push(crate::Subplot {
            config: inner,
            series: vec![],
            link_x: None,
            link_y: None,
            marker: None,
        });
        for config in [left, subplot] {
            validate_renderer_config(&config).expect("a probability axis draws");
        }
    }

    /// The fit of a probability axis keeps its ends inside (0, 1): an end
    /// at 0 or 1 takes the mirror of the other, the padding is in `F⁻¹`
    /// units, and an extent with no in-domain end fits nothing.
    #[test]
    fn a_probability_fit_stays_inside_the_unit_interval() {
        let mut axis = crate::default::default_config().left_y;
        axis.scale = AxisScale::Probability(crate::config::ProbabilityScale::Normal);
        let fit = |min: f64, max: f64, min_positive: Option<f64>| {
            fitted_axis_range(
                &axis,
                &FitExtent {
                    min,
                    max,
                    min_positive,
                },
                0.05,
            )
        };
        let (lo, hi) = fit(0.01, 0.99, Some(0.01)).expect("in domain");
        assert!(lo > 0.0 && lo < 0.01 && hi > 0.99 && hi < 1.0, "{lo} {hi}");
        assert!((lo - (1.0 - hi)).abs() < 1e-12, "symmetric: {lo} {hi}");
        let (lo, hi) = fit(0.0, 0.98, None).expect("mirrored lower end");
        assert!(lo > 0.0 && lo < 0.02 && hi < 1.0, "{lo} {hi}");
        let (lo, hi) = fit(0.0, 1.0, Some(0.005)).expect("min_positive lower end");
        assert!(
            lo > 0.0 && lo < 0.005 && hi > 0.995 && hi < 1.0,
            "{lo} {hi}"
        );
        assert_eq!(fit(-1.0, 2.0, None), None);
        assert_eq!(fit(0.0, 1.0, Some(1.5)), None);
    }

    use crate::default::default_config;

    fn dummy_config() -> Config {
        default_config()
    }

    #[test]
    fn chart_consume_clears_dirty() {
        let mut c = Chart::new(dummy_config());
        assert!(c.data_dirty());
        assert!(c.raster_dirty());
        assert!(c.consume_data_dirty());
        assert!(!c.data_dirty());
        assert!(c.consume_raster_dirty());
        assert!(!c.raster_dirty());
    }

    #[test]
    fn chart_set_x_range_marks_dirty() {
        let mut c = Chart::new(dummy_config());
        c.consume_data_dirty();
        c.consume_raster_dirty();

        c.set_x_range(0.0, 100.0);
        assert!(c.data_dirty());
        assert!(c.raster_dirty());
        assert_eq!(c.config().bottom_x.min, 0.0);
        assert_eq!(c.config().bottom_x.max, 100.0);
        assert_eq!(c.config().top_x.min, 0.0);
        assert_eq!(c.config().top_x.max, 100.0);
    }

    #[test]
    fn direct_range_edits_preserve_custom_ticks_and_other_config() {
        for scale in [AxisScale::Linear, AxisScale::Logarithmic] {
            let mut config = dummy_config();
            for axis in [&mut config.bottom_x, &mut config.top_x] {
                axis.scale = scale.clone();
                axis.min = 1.0;
                axis.max = 1000.0;
                axis.major_spacing = 3.0;
                axis.minor_count = 2;
                axis.label_style.format = LabelFormat::Scientific;
                axis.label_style.significant_digits = 6;
            }
            // The top axis is independently configured, even when sharing bounds.
            config.top_x.major_spacing = 2.0;
            let mut expected = config.clone();
            expected.bottom_x.min = 10.0;
            expected.bottom_x.max = 100.0;
            expected.top_x.min = 10.0;
            expected.top_x.max = 100.0;

            for general_access in [false, true] {
                let mut chart = Chart::new(config.clone());
                chart.consume_data_dirty();
                chart.consume_raster_dirty();
                let edit = |cfg: &mut Config| {
                    for axis in [&mut cfg.bottom_x, &mut cfg.top_x] {
                        axis.min = 10.0;
                        axis.max = 100.0;
                    }
                };
                if general_access {
                    edit(chart.config_mut());
                } else {
                    chart.with_axis_range_change(edit);
                }
                assert_eq!(chart.config(), &expected);
                assert!(chart.data_dirty());
                assert!(chart.raster_dirty());
            }
        }
    }

    #[test]
    fn linear_range_preserves_timestamp_label_format() {
        let mut c = Chart::new(dummy_config());
        c.config_mut().bottom_x.label_style.format =
            LabelFormat::Timestamp(crate::format::TimestampLabelFormat::default());
        c.config_mut().top_x.label_style.format =
            LabelFormat::Timestamp(crate::format::TimestampLabelFormat::default());

        c.set_x_range(0.0, 100.0);

        assert!(matches!(
            c.config().bottom_x.label_style.format,
            LabelFormat::Timestamp(_)
        ));
        assert!(matches!(
            c.config().top_x.label_style.format,
            LabelFormat::Timestamp(_)
        ));
    }

    #[test]
    fn chart_with_decoration_change_only_raster() {
        let mut c = Chart::new(dummy_config());
        c.consume_data_dirty();
        c.consume_raster_dirty();

        c.with_decoration_change(|cfg| {
            cfg.chart_title.top_margin = 50.0;
        });
        assert!(!c.data_dirty(), "decoration change must not set data_dirty");
        assert!(c.raster_dirty());
    }

    // Fitting the title band is a decoration change like any other margin
    // edit: it marks only the raster dirty, and only when the band moved.
    #[test]
    fn fit_chart_title_margin_marks_the_raster_dirty_only_when_it_grows() {
        let mut c = Chart::new(dummy_config());
        c.with_decoration_change(|cfg| {
            cfg.chart_title.text.segments = crate::text::rich_segments_from_text("one\ntwo\nthree");
        });
        c.consume_data_dirty();
        c.consume_raster_dirty();

        assert!(c.fit_chart_title_margin());
        assert!(c.config().chart_title.top_margin > 32.0);
        assert!(c.raster_dirty());
        assert!(!c.data_dirty());

        c.consume_raster_dirty();
        assert!(!c.fit_chart_title_margin(), "already fits");
        assert!(!c.raster_dirty());
    }

    #[test]
    fn chart_padded_helper() {
        let (lo, hi) = Chart::padded(0.0, 10.0, 0.1);
        assert!((lo - (-1.0)).abs() < 1e-9);
        assert!((hi - 11.0).abs() < 1e-9);

        // Invalid input passes through unchanged.
        let (a, b) = Chart::padded(5.0, 5.0, 0.1);
        assert_eq!(a, 5.0);
        assert_eq!(b, 5.0);
    }

    #[test]
    fn auto_fit_y_extent_linear_pads_both_sides() {
        let mut c = Chart::new(dummy_config());
        let ext = FitExtent {
            min: 0.0,
            max: 10.0,
            min_positive: Some(0.5),
        };
        c.auto_fit_y_extent(&ext, 0.1);
        assert!((c.config().left_y.min - (-1.0)).abs() < 1e-9);
        assert!((c.config().left_y.max - 11.0).abs() < 1e-9);
    }

    #[test]
    fn auto_fit_y_extent_log_falls_back_to_min_positive() {
        let mut c = Chart::new(dummy_config());
        c.config_mut().left_y.scale = AxisScale::Logarithmic;
        let ext = FitExtent {
            min: -5.0,
            max: 100.0,
            min_positive: Some(0.5),
        };
        c.auto_fit_y_extent(&ext, 0.0);
        assert!((c.config().left_y.min - 0.5).abs() < 1e-9);
        assert!((c.config().left_y.max - 100.0).abs() < 1e-9);

        // No positive bound at all: the fit must decline, not panic or set
        // an unusable range.
        let before = (c.config().left_y.min, c.config().left_y.max);
        let ext = FitExtent {
            min: -5.0,
            max: -1.0,
            min_positive: None,
        };
        c.auto_fit_y_extent(&ext, 0.0);
        assert_eq!((c.config().left_y.min, c.config().left_y.max), before);
    }

    /// Every pair helper gives the partner axis (`top_x`, `right_y`) the
    /// main axis's scale on a framed chart, and writes each axis's ticks
    /// from its scale: `set_x_range` / `set_y_range`, the auto-fit applied
    /// to an owned config and the subplot setters. A logarithmic `left_y`
    /// gives a linear `right_y` decades, and a linear one turns a
    /// logarithmic partner linear. On a 3D chart `right_y` is the z axis: it
    /// keeps its own scale and gets the ticks of it. Deriving a linear
    /// partner's ticks from a logarithmic main axis gave it one-unit majors
    /// with eight minors across 1..1000.
    #[test]
    fn pair_range_helpers_give_the_partner_the_main_axis_scale() {
        use crate::Subplot;
        let ticks = |axis: &AxisOptions| {
            (
                axis.major_spacing,
                axis.minor_count,
                axis.label_style.format.clone(),
            )
        };
        let expected = |scale: &AxisScale| match scale {
            AxisScale::Linear => (
                crate::tick::axis_major_spacing(&AxisScale::Linear, 1.0, 1000.0),
                4,
                LabelFormat::Decimal,
            ),
            AxisScale::Logarithmic => (1.0, 8, LabelFormat::Power),
            AxisScale::Probability(_) => unreachable!("a linear and log sweep"),
        };
        for view_3d in [false, true] {
            for (main, partner) in [
                (AxisScale::Logarithmic, AxisScale::Linear),
                (AxisScale::Linear, AxisScale::Logarithmic),
            ] {
                let mut base = dummy_config();
                if view_3d {
                    base.set_view_3d(Some(crate::View3d::default()));
                }
                base.bottom_x.scale = main.clone();
                base.left_y.scale = main.clone();
                base.top_x.scale = partner.clone();
                base.right_y.scale = partner.clone();
                // A framed chart's partners mirror the main axes; a 3D
                // chart's top_x and right_y (its z axis) keep their scales.
                let drawn = if view_3d { &partner } else { &main };
                let check = |cfg: &Config, helper: &str| {
                    let case =
                        format!("{helper}, {main:?} main, {partner:?} partner, view_3d {view_3d}");
                    for (name, axis, scale) in [
                        ("bottom_x", &cfg.bottom_x, &main),
                        ("top_x", &cfg.top_x, drawn),
                        ("left_y", &cfg.left_y, &main),
                        ("right_y", &cfg.right_y, drawn),
                    ] {
                        assert_eq!(axis.scale, *scale, "{case}: {name} scale");
                        assert_eq!((axis.min, axis.max), (1.0, 1000.0), "{case}: {name} range");
                        assert_eq!(
                            ticks(axis),
                            expected(scale),
                            "{case}: {name} ticks by its scale"
                        );
                    }
                };

                let mut c = Chart::new(base.clone());
                c.set_x_range(1.0, 1000.0);
                c.set_y_range(1.0, 1000.0);
                check(c.config(), "set_x_range / set_y_range");

                let mut fitted = base.clone();
                let ext = FitExtent {
                    min: 1.0,
                    max: 1000.0,
                    min_positive: Some(1.0),
                };
                apply_auto_fit_all(&mut fitted, &ext, &ext, 0.0);
                check(&fitted, "apply_auto_fit_all");

                let mut parent = dummy_config();
                parent.subplots = vec![Subplot {
                    config: base.clone(),
                    ..Default::default()
                }];
                let mut c = Chart::new(parent);
                c.set_subplot_x_range(0, 1.0, 1000.0);
                c.set_subplot_y_range(0, 1.0, 1000.0);
                check(
                    &c.config().subplots[0].config,
                    "set_subplot_x_range / set_subplot_y_range",
                );
            }
        }
    }

    /// A categorical partner keeps its linear scale and its category range
    /// when the main axis is logarithmic: only a numeric partner mirrors.
    #[test]
    fn a_categorical_partner_keeps_its_scale() {
        let mut cfg = dummy_config();
        cfg.left_y.scale = AxisScale::Logarithmic;
        cfg.right_y.categories = Some(vec![crate::config::Category::new("a", "A")]);
        let before = cfg.right_y.clone();
        let mut c = Chart::new(cfg);
        c.set_y_range(1.0, 1000.0);
        assert_eq!(c.config().right_y, before);
    }

    /// A view moves the window of a categorical partner of a categorical
    /// main axis: both edges show the same categories (#205).
    #[test]
    fn a_view_moves_both_windows_of_a_categorical_pair() {
        let mut cfg = dummy_config();
        cfg.bottom_x.categories = categories(3);
        cfg.top_x.categories = categories(3);
        let mut c = Chart::new(cfg);
        c.set_x_range(0.5, 2.5);
        assert_eq!(c.config().bottom_x.category_window, Some([0.5, 2.5]));
        assert_eq!(c.config().top_x.category_window, Some([0.5, 2.5]));
    }

    #[test]
    fn auto_fit_extent_degenerate_is_noop() {
        let mut c = Chart::new(dummy_config());
        let before = (c.config().bottom_x.min, c.config().bottom_x.max);
        c.auto_fit_x_extent(&FitExtent::EMPTY, 0.1);
        c.auto_fit_x_extent(
            &FitExtent {
                min: 3.0,
                max: 3.0,
                min_positive: Some(3.0),
            },
            0.1,
        );
        assert_eq!((c.config().bottom_x.min, c.config().bottom_x.max), before);
    }

    #[test]
    fn auto_fit_colorbar_extent_uses_the_colourbar_axis() {
        let mut config = dummy_config();
        config.colorbar = Some(crate::default::default_colorbar_options());
        let mut chart = Chart::new(config);
        chart.auto_fit_colorbar_extent(
            &FitExtent {
                min: 2.0,
                max: 8.0,
                min_positive: Some(2.0),
            },
            0.1,
        );
        let axis = &chart.config().colorbar.as_ref().unwrap().axis;
        assert!((axis.min - 1.4).abs() < 1e-9);
        assert!((axis.max - 8.6).abs() < 1e-9);
    }

    #[test]
    fn auto_fit_colorbar_extent_without_a_colourbar_is_a_noop() {
        let config = dummy_config();
        assert!(config.colorbar.is_none());
        let mut chart = Chart::new(config.clone());
        chart.auto_fit_colorbar_extent(
            &FitExtent {
                min: 2.0,
                max: 8.0,
                min_positive: Some(2.0),
            },
            0.1,
        );
        assert_eq!(chart.config(), &config);
    }

    // A malformed math segment tree is refused at the renderer boundary in
    // every text a `Config` draws outside the annotations (which their own
    // validation checks), with the field that holds it.
    #[test]
    fn validate_renderer_config_refuses_a_malformed_math_tree() {
        use crate::text::{MathKind, MathNode, MathSlot, RichSegment};
        let mut cfg = dummy_config();
        cfg.colorbar = Some(crate::default::default_colorbar_options());
        cfg.overlays.push(crate::OverlayLayer {
            legend: Some(cfg.legend.clone()),
            ..Default::default()
        });
        assert!(validate_renderer_config(&cfg).is_ok());
        // A slot child of a node that is not open.
        let broken = vec![
            RichSegment {
                math_node: Some(MathNode {
                    id: 0,
                    kind: MathKind::Math(crate::text::MathStyle::Text),
                }),
                ..RichSegment::plain('\u{200B}')
            },
            RichSegment {
                math_parent: Some(MathSlot { id: 3, slot: 0 }),
                ..RichSegment::plain('x')
            },
        ];
        type Pick = fn(&mut Config) -> &mut crate::text::RichText;
        let fields: [(&str, Pick); 9] = [
            ("chart_title", |c| &mut c.chart_title.text),
            ("legend", |c| &mut c.legend.content),
            ("top_x", |c| &mut c.top_x.title_option.text),
            ("bottom_x", |c| &mut c.bottom_x.title_option.text),
            ("left_y", |c| &mut c.left_y.title_option.text),
            ("right_y", |c| &mut c.right_y.title_option.text),
            ("colorbar.axis", |c| {
                &mut c.colorbar.as_mut().unwrap().axis.title_option.text
            }),
            ("overlays.y_axis", |c| {
                &mut c.overlays[0].y_axis.title_option.text
            }),
            ("overlays.legend", |c| {
                &mut c.overlays[0].legend.as_mut().unwrap().content
            }),
        ];
        for (field, pick) in fields {
            let mut c = cfg.clone();
            pick(&mut c).segments = broken.clone();
            match validate_renderer_config(&c) {
                Err(FiggyError::InvalidConfig {
                    field: f,
                    kind: reason,
                }) => {
                    assert_eq!(f, field);
                    assert!(
                        matches!(reason, crate::RuleKind::InvalidMath { error } if error.kind == crate::text::MathErrorKind::UnknownParent),
                        "{reason}"
                    );
                }
                other => panic!("{field}: {other:?}"),
            }
        }
        let mut c = cfg.clone();
        c.annotations.items.push(crate::annotation::Annotation::new(
            crate::annotation::AnnotationKind::Text {
                anchor: crate::annotation::Anchor::Ratio { x: 0.5, y: 0.5 },
                text: crate::text::RichText {
                    segments: broken,
                    ..crate::text::RichText::plain("", Color::BLACK, 12.0, "")
                },
                box_style: None,
                align: Default::default(),
            },
        ));
        assert!(matches!(
            validate_renderer_config(&c),
            Err(FiggyError::InvalidConfig {
                field: "annotations",
                ..
            })
        ));
    }

    // Issue #6: every renderer edit entry point (`register_chart`,
    // `set_chart_config`, `set_chart_state`, `set_chart_view_state`) runs this
    // gate before touching state. A negative title margin is a coordinate and
    // passes; one that would carry the data area above the canvas origin, or a
    // non-finite one, is refused and the previous config stays published.
    #[test]
    fn validate_renderer_config_takes_title_margins_as_coordinates() {
        let mut cfg = dummy_config();
        cfg.colorbar = Some(crate::default::default_colorbar_options());
        assert_eq!(cfg.chart_area.0.y, 0);
        assert!(validate_renderer_config(&cfg).is_ok());

        let with_title = |margin: f32| {
            let mut c = cfg.clone();
            c.chart_title.top_margin = margin;
            c
        };
        assert!(validate_renderer_config(&with_title(-5.0)).is_ok());
        for margin in [-1e38, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
            assert!(
                matches!(
                    validate_renderer_config(&with_title(margin)),
                    Err(FiggyError::InvalidConfig {
                        field: "layout",
                        ..
                    })
                ),
                "title margin {margin} must be refused at the renderer boundary"
            );
        }
    }

    fn categories(n: usize) -> Option<Vec<crate::config::Category>> {
        Some(
            (0..n)
                .map(|i| crate::config::Category::new(format!("c{i}"), format!("C{i}")))
                .collect(),
        )
    }

    /// A categorical axis is held to its category rules on every axis the
    /// renderer reads, its `min` / `max` are not checked, and the axes that
    /// cannot be categorical yet say so.
    #[test]
    fn validate_renderer_config_holds_categorical_axes_to_their_rules() {
        let with = |edit: &dyn Fn(&mut Config)| {
            let mut cfg = dummy_config();
            edit(&mut cfg);
            validate_renderer_config(&cfg)
        };
        assert!(with(&|c| c.bottom_x.categories = categories(3)).is_ok());
        // No count limit on a categorical axis (CLAUDE.md rule 10, #107).
        assert!(with(&|c| c.bottom_x.categories = categories(5000)).is_ok());
        // An inverted categorical axis walks its categories from the far end.
        assert!(
            with(&|c| {
                c.left_y.categories = categories(3);
                c.left_y.inverted = true;
            })
            .is_ok()
        );
        // `min` / `max` are ignored on a categorical axis.
        assert!(
            with(&|c| {
                c.left_y.categories = categories(2);
                c.left_y.min = 5.0;
                c.left_y.max = f64::NAN;
            })
            .is_ok()
        );
        for (field, edit) in [
            (
                "top_x",
                (|c: &mut Config| c.top_x.categories = categories(0)) as fn(&mut Config),
            ),
            ("bottom_x", |c| {
                c.bottom_x.categories = categories(2);
                c.bottom_x.scale = AxisScale::Logarithmic;
            }),
            ("left_y", |c| {
                c.left_y.categories = categories(2);
                c.left_y.categories.as_mut().unwrap()[0].id = String::new();
            }),
            ("right_y", |c| {
                c.right_y.categories = categories(2);
                c.right_y.categories.as_mut().unwrap()[1].id = "c0".into();
            }),
        ] {
            assert!(
                matches!(with(&edit), Err(FiggyError::InvalidConfig { field: f, .. }) if f == field),
                "{field}"
            );
        }
        let bar = with(&|c| {
            let mut bar = crate::default::default_colorbar_options();
            bar.axis.categories = categories(2);
            c.colorbar = Some(bar);
        });
        assert!(matches!(
            bar,
            Err(FiggyError::InvalidConfig {
                field: "colorbar.axis",
                kind: crate::RuleKind::ColorbarCategorical
            })
        ));
        let overlay = with(&|c| {
            let mut layer = crate::OverlayLayer::default();
            layer.y_axis.categories = categories(0);
            c.overlays.push(layer);
        });
        assert!(matches!(
            overlay,
            Err(FiggyError::InvalidConfig {
                field: "overlays.y_axis",
                ..
            })
        ));
        let overlay = with(&|c| {
            let mut layer = crate::OverlayLayer::default();
            layer.y_axis.categories = categories(4);
            c.overlays.push(layer);
        });
        assert!(overlay.is_ok(), "{overlay:?}");
    }

    /// The fit helpers leave a categorical axis as it is; a view's range
    /// moves its category window (#205), and the full range clears it.
    #[test]
    fn a_categorical_axis_keeps_its_range_on_fits_and_takes_a_view_as_its_window() {
        let mut cfg = dummy_config();
        cfg.bottom_x.categories = categories(3);
        let before = cfg.bottom_x.clone();
        let mut chart = Chart::new(cfg);
        chart.auto_fit_x_extent(
            &FitExtent {
                min: 1.0,
                max: 2.0,
                min_positive: Some(1.0),
            },
            0.1,
        );
        assert_eq!(chart.config().bottom_x, before);
        // The numeric top axis of the pair still follows.
        assert_eq!(chart.config().top_x.min, 1.0 - 0.1);
        chart.set_x_range(0.5, 2.5);
        let axis = &chart.config().bottom_x;
        assert_eq!(axis.category_window, Some([0.5, 2.5]));
        assert_eq!(
            (axis.min, axis.max),
            (before.min, before.max),
            "min / max kept"
        );
        chart.set_x_range(-0.5, 2.5);
        assert_eq!(chart.config().bottom_x, before);
    }

    // The colourbar's z axis goes through the same renderer-side gate as the
    // four chart axes. A log colourbar starting at zero has no `t` for its
    // lowest colour, exactly as a log x axis starting at zero has no screen
    // position for its lowest point.
    #[test]
    fn validate_renderer_config_holds_the_colorbar_axis_to_the_axis_rules() {
        let with_bar = |edit: fn(&mut crate::config::ColorBarOptions)| {
            let mut cfg = dummy_config();
            let mut bar = crate::default::default_colorbar_options();
            edit(&mut bar);
            cfg.colorbar = Some(bar);
            cfg
        };

        assert!(validate_renderer_config(&with_bar(|_| {})).is_ok());

        let non_finite = validate_renderer_config(&with_bar(|bar| bar.axis.max = f64::NAN));
        assert!(matches!(
            non_finite,
            Err(FiggyError::InvalidConfig {
                field: "colorbar.axis",
                ..
            })
        ));

        let reversed = validate_renderer_config(&with_bar(|bar| {
            bar.axis.min = 10.0;
            bar.axis.max = 1.0;
        }));
        assert!(matches!(
            reversed,
            Err(FiggyError::InvalidConfig {
                field: "colorbar.axis",
                ..
            })
        ));

        let log_at_zero = validate_renderer_config(&with_bar(|bar| {
            bar.axis.scale = AxisScale::Logarithmic;
            bar.axis.min = 0.0;
            bar.axis.max = 100.0;
        }));
        assert!(matches!(
            log_at_zero,
            Err(FiggyError::InvalidConfig {
                field: "colorbar.axis",
                kind: crate::RuleKind::LogAxisBounds
            })
        ));

        // A positive log range is fine, and a hidden bar is still validated —
        // an invisible z scale that field series read from must still be sane.
        assert!(
            validate_renderer_config(&with_bar(|bar| {
                bar.visible = false;
                bar.axis.scale = AxisScale::Logarithmic;
                bar.axis.min = 1e-3;
                bar.axis.max = 1e3;
            }))
            .is_ok()
        );
    }

    /// LINK_ROOT_IN_RANGE (#152): a subplot range edit writes the root of
    /// the subplot's link chain, which is the parent or a listed subplot for
    /// any links (none, down to the parent, self, forward, cycles, targets
    /// past the list), with 0, 1 and 64 subplots and indices past the list.
    #[test]
    fn subplot_range_edits_follow_any_links_without_panicking() {
        use crate::{CellRef, Subplot};
        for n in [0usize, 1, 64] {
            let patterns: [&dyn Fn(usize) -> Option<CellRef>; 8] = [
                &|_| None,
                &|i| Some(i.checked_sub(1).map_or(CellRef::Parent, CellRef::Subplot)),
                &|i| Some(CellRef::Subplot(i)),
                &|i| Some(CellRef::Subplot(i + 1)),
                &|i| Some(CellRef::Subplot(if i ^ 1 < n { i ^ 1 } else { i })),
                &|i| Some(CellRef::Subplot((i + 1) % n.max(1))),
                &|_| Some(CellRef::Subplot(n)),
                &|_| Some(CellRef::Subplot(usize::MAX)),
            ];
            for (k, x) in patterns.iter().enumerate() {
                let y = patterns[(k + 3) % patterns.len()];
                let mut cfg = dummy_config();
                cfg.subplots = (0..n)
                    .map(|i| Subplot {
                        link_x: x(i),
                        link_y: y(i),
                        ..Default::default()
                    })
                    .collect();
                let mut c = Chart::new(cfg);
                for index in 0..n + 2 {
                    c.set_subplot_x_range(index, -1.0 - index as f64, 2.0);
                    c.set_subplot_y_range(index, 0.5, 9.0 + index as f64);
                }
                c.set_subplot_x_range(usize::MAX, 0.0, 1.0);
                assert_eq!(c.config().subplots.len(), n);
                if n > 0 && k == 0 {
                    // Unlinked: each subplot keeps its own last edit.
                    assert_eq!(c.config().subplots[n - 1].config.bottom_x.min, -(n as f64));
                }
            }
        }
    }
}
