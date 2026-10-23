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
use crate::text::{RichText, rich_segments_from_text};
use crate::text_render::CpuTextMeasure;

/// Apply min/max and scale-appropriate tick settings to one axis.
pub(crate) fn apply_axis_range(
    axis: &mut crate::config::AxisOptions,
    min: f64,
    max: f64,
    log: bool,
) {
    axis.min = min;
    axis.max = max;
    if log {
        axis.major_spacing = crate::tick::axis_major_spacing(&AxisScale::Logarithmic, min, max);
        axis.minor_count = 8;
        axis.label_style.format = LabelFormat::Power;
        axis.label_style.significant_digits = 1;
    } else {
        axis.major_spacing = crate::tick::axis_major_spacing(&AxisScale::Linear, min, max);
        axis.minor_count = 4;
        if !matches!(axis.label_style.format, LabelFormat::Timestamp(_)) {
            axis.label_style.format = LabelFormat::Decimal;
        }
        axis.label_style.significant_digits = 3;
    }
}

/// Fill `rt.segments` with one plain RichSegment per char (all style flags off).
fn fill_plain_segments(rt: &mut RichText, s: &str) {
    rt.segments = rich_segments_from_text(s);
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
}

/// Validate the invariants required before a `Config` becomes renderer-owned.
///
/// Raster helpers remain defensive against invalid ad-hoc values, but the
/// renderer registry never publishes a zero target, a non-finite/reversed
/// range, or a non-positive logarithmic range.
pub(crate) fn validate_renderer_config(config: &Config) -> Result<()> {
    let area = &config.chart_area.0;
    if area.width == 0 || area.height == 0 {
        return Err(FiggyError::InvalidChartArea {
            width: area.width,
            height: area.height,
        });
    }

    // The colourbar's z axis is validated with the four chart axes, by the same
    // rules: it is an `AxisOptions` and its range means the same thing. A log
    // colourbar with a non-positive lower bound has no `t` for its lowest
    // colour, which is the same defect as a log x axis starting at zero.
    let colorbar_axis = config.colorbar.as_ref().map(|bar| &bar.axis);
    for (field, axis) in [
        ("top_x", &config.top_x),
        ("bottom_x", &config.bottom_x),
        ("left_y", &config.left_y),
        ("right_y", &config.right_y),
    ]
    .into_iter()
    .chain(colorbar_axis.map(|axis| ("colorbar.axis", axis)))
    {
        if !axis.min.is_finite() || !axis.max.is_finite() {
            return Err(FiggyError::InvalidConfig {
                field,
                reason: "axis bounds must be finite",
            });
        }
        if axis.max <= axis.min {
            return Err(FiggyError::InvalidConfig {
                field,
                reason: "axis max must be greater than min",
            });
        }
        if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
            return Err(FiggyError::InvalidConfig {
                field,
                reason: "logarithmic axis bounds must be positive",
            });
        }
    }

    // Overlay Y axes are ranges of the same kind, and the layer membership
    // must be unambiguous before a series is drawn on one panel.
    for overlay in &config.overlays {
        let axis = &overlay.y_axis;
        if !axis.min.is_finite() || !axis.max.is_finite() {
            return Err(FiggyError::InvalidConfig {
                field: "overlays.y_axis",
                reason: "axis bounds must be finite",
            });
        }
        if axis.max <= axis.min {
            return Err(FiggyError::InvalidConfig {
                field: "overlays.y_axis",
                reason: "axis max must be greater than min",
            });
        }
        if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
            return Err(FiggyError::InvalidConfig {
                field: "overlays.y_axis",
                reason: "logarithmic axis bounds must be positive",
            });
        }
    }
    config
        .validate_overlays()
        .map_err(|_| FiggyError::InvalidConfig {
            field: "overlays",
            reason: "a series id may belong to at most one overlay layer",
        })?;

    // The subplot rules come before the layout check, which also expands
    // each subplot, so a broken link or nesting reports as what it is.
    config
        .validate_subplots()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "subplots",
            reason,
        })?;
    config.validate().map_err(|_| FiggyError::InvalidConfig {
        field: "layout",
        reason: "margins and visual dimensions must produce a non-empty data area",
    })?;
    // Malformed annotation styles are rejected like any other config field;
    // non-finite values and anchors are not malformed, the raster skips them.
    config
        .annotations
        .validate()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "annotations",
            reason,
        })?;
    if !config.overlays.is_empty() {
        config.expand().map_err(|_| FiggyError::InvalidConfig {
            field: "overlays",
            reason: "the overlay layers must share one non-empty data area",
        })?;
    }
    if !config.subplots.is_empty() {
        // Every subplot is a chart of its own, held to the same rules with
        // its links applied.
        for index in 0..config.subplots.len() {
            let subplot = config
                .subplot_config(index)
                .ok_or(FiggyError::InvalidConfig {
                    field: "subplots",
                    reason: "subplot index out of range",
                })?;
            validate_renderer_config(&subplot)?;
        }
    }
    Ok(())
}

fn fitted_axis_range(
    axis: &AxisOptions,
    ext: &FitExtent,
    padding_ratio: f64,
) -> Option<(f64, f64)> {
    if !ext.is_fittable() {
        return None;
    }
    let range = if matches!(axis.scale, AxisScale::Logarithmic) {
        let safe = if ext.min > 0.0 {
            ext.min
        } else {
            ext.min_positive?
        };
        log_padded(safe, ext.max, padding_ratio)
    } else {
        Chart::padded(ext.min, ext.max, padding_ratio)
    };
    (range.0.is_finite() && range.1.is_finite() && range.1 > range.0).then_some(range)
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
    if let Some((min, max)) = fitted_axis_range(&config.bottom_x, x_extent, padding_ratio) {
        let log = matches!(config.bottom_x.scale, AxisScale::Logarithmic);
        apply_axis_range(&mut config.bottom_x, min, max, log);
        apply_axis_range(&mut config.top_x, min, max, log);
    }
    if let Some((min, max)) = fitted_axis_range(&config.left_y, y_extent, padding_ratio) {
        let log = matches!(config.left_y.scale, AxisScale::Logarithmic);
        apply_axis_range(&mut config.left_y, min, max, log);
        apply_axis_range(&mut config.right_y, min, max, log);
    }
}

/// A single figgy chart panel: a `Config` plus dirty-flag tracking.
pub struct Chart {
    config: Config,
    data_dirty: bool,
    raster_dirty: bool,
}

impl Chart {
    /// New chart. Both dirty flags start true so the first frame draws.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            data_dirty: true,
            raster_dirty: true,
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
        self.raster_dirty = true;
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
        self.raster_dirty = true;
    }

    /// Decoration-only change (titles, colors, margins, …): raster only.
    /// Only the fields written by the closure change. In particular, assigning
    /// title segments here keeps `top_margin`; [`Self::with_title`] also fits
    /// that band. The caller still refreshes the view before preparing.
    pub fn with_decoration_change<F: FnOnce(&mut Config)>(&mut self, f: F) {
        f(&mut self.config);
        self.raster_dirty = true;
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

    // Text builders (raster only).

    /// Set the chart title text as plain segments. For styled segments
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
        fill_plain_segments(&mut self.config.chart_title.text, text);
        self.fit_chart_title_margin();
        self.raster_dirty = true;
        self
    }

    /// Grow `chart_title.top_margin` when the title, measured with the font
    /// this chart's draw style renders, is taller than its band
    /// ([`crate::layout::fit_chart_title_margin`]). Returns whether the band
    /// changed, which marks the raster dirty.
    fn fit_chart_title_margin(&mut self) -> bool {
        let measure = CpuTextMeasure::for_style(&self.config.draw_style);
        let changed = crate::layout::fit_chart_title_margin(&mut self.config, &measure);
        self.raster_dirty |= changed;
        changed
    }

    /// Set the bottom X axis title text.
    pub fn with_x_title(mut self, text: &str) -> Self {
        fill_plain_segments(&mut self.config.bottom_x.title_option.text, text);
        self.raster_dirty = true;
        self
    }

    /// Set the left Y axis title text.
    pub fn with_y_title(mut self, text: &str) -> Self {
        fill_plain_segments(&mut self.config.left_y.title_option.text, text);
        self.raster_dirty = true;
        self
    }

    /// Append a legend entry to the legend's one-document content. The first
    /// call also makes the legend visible. The mark is built from `kind` as
    /// inline text segments carrying the series color as a per-segment
    /// override (`line_width` biases nothing today — the symbol glyph
    /// carries the look); entries are separated by explicit `'\n'` segments.
    /// For custom layouts (one-line legends, mid-text symbols) edit
    /// `config_mut().legend.content.segments` directly.
    pub fn with_legend_entry(
        mut self,
        label: &str,
        color: Color,
        line_width: f32,
        kind: LegendEntryKind,
    ) -> Self {
        let _ = line_width;
        let symbol = crate::config::symbol_segments(&kind, color);
        crate::config::append_legend_entry(&mut self.config.legend.content, symbol, label);
        self.config.legend.visible = true;
        self.raster_dirty = true;
        self
    }

    // Axis-range helpers.

    /// Set both X ranges (top + bottom) and derive their tick/label settings.
    ///
    /// This is a convenience helper for automatic setup. To keep the current
    /// ticks and formatting, write only `min` / `max` through
    /// [`Self::with_axis_range_change`] instead.
    ///
    /// Branches on the bottom axis's scale:
    /// - **Linear**: `major_spacing` is
    ///   [`crate::tick::axis_major_spacing`] of the range: 0.2, 0.5 or 1 ×
    ///   10^k of the span, for 5 to 10 major intervals (span 72 → 10,
    ///   8 → 1, 0.7 → 0.1, 10 → 2). `minor_count = 4`, `format = Decimal`
    ///   (an existing timestamp format is retained), `sig_digits = 3`.
    /// - **Logarithmic**: `major_spacing` is the same function's decade step
    ///   (1 up to 6 decades, 2 up to 12, then `ceil(decades / 6)`),
    ///   `minor_count = 8` (2..9 within each decade), `format = Power` (10ⁿ
    ///   superscript), `sig_digits = 1`.
    pub fn set_x_range(&mut self, min: f64, max: f64) {
        let log = matches!(self.config.bottom_x.scale, AxisScale::Logarithmic);
        self.with_axis_range_change(|c| {
            apply_axis_range(&mut c.bottom_x, min, max, log);
            apply_axis_range(&mut c.top_x, min, max, log);
        });
    }

    /// Set both Y ranges (left + right) and derive their tick/label settings.
    /// Log/linear branching uses the left axis's scale and mirrors
    /// [`Self::set_x_range`]. To preserve ticks and formatting, write only
    /// `min` / `max` through [`Self::with_axis_range_change`] instead.
    pub fn set_y_range(&mut self, min: f64, max: f64) {
        let log = matches!(self.config.left_y.scale, AxisScale::Logarithmic);
        self.with_axis_range_change(|c| {
            apply_axis_range(&mut c.left_y, min, max, log);
            apply_axis_range(&mut c.right_y, min, max, log);
        });
    }

    /// Set overlay layer `layer`'s Y range and derive its tick/label settings,
    /// the overlay counterpart of [`Self::set_y_range`]. Log/linear branching
    /// uses that axis' own scale. A `layer` past the overlay list is a no-op.
    pub fn set_overlay_y_range(&mut self, layer: usize, min: f64, max: f64) {
        let Some(log) = self
            .config
            .overlays
            .get(layer)
            .map(|overlay| matches!(overlay.y_axis.scale, AxisScale::Logarithmic))
        else {
            return;
        };
        self.with_axis_range_change(|c| {
            if let Some(overlay) = c.overlays.get_mut(layer) {
                apply_axis_range(&mut overlay.y_axis, min, max, log);
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
        self.set_overlay_y_range(layer, min, max);
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
        self.set_subplot_range(index, true, min, max);
    }

    /// Set subplot `index`'s Y range (left + right); see
    /// [`Self::set_subplot_x_range`].
    pub fn set_subplot_y_range(&mut self, index: usize, min: f64, max: f64) {
        self.set_subplot_range(index, false, min, max);
    }

    fn set_subplot_range(&mut self, index: usize, x: bool, min: f64, max: f64) {
        if index >= self.config.subplots.len() {
            return;
        }
        let root = self.config.subplot_link_root(index, x);
        self.with_axis_range_change(|config| {
            let pair = |chart: &mut Config| {
                let (main, secondary) = if x {
                    (&mut chart.bottom_x, &mut chart.top_x)
                } else {
                    (&mut chart.left_y, &mut chart.right_y)
                };
                let log = matches!(main.scale, AxisScale::Logarithmic);
                apply_axis_range(main, min, max, log);
                apply_axis_range(secondary, min, max, log);
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
            .and_then(|chart| fitted_axis_range(&chart.bottom_x, ext, padding_ratio))
        else {
            return;
        };
        self.set_subplot_x_range(index, min, max);
    }

    /// Fit subplot `index`'s Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_y_extent`].
    pub fn auto_fit_subplot_y_extent(&mut self, index: usize, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = self
            .config
            .subplot_config(index)
            .and_then(|chart| fitted_axis_range(&chart.left_y, ext, padding_ratio))
        else {
            return;
        };
        self.set_subplot_y_range(index, min, max);
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
    pub fn auto_fit_x_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = fitted_axis_range(&self.config.bottom_x, ext, padding_ratio) else {
            return;
        };
        self.set_x_range(min, max);
    }

    /// Fit the Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_x_extent`].
    pub fn auto_fit_y_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        let Some((min, max)) = fitted_axis_range(&self.config.left_y, ext, padding_ratio) else {
            return;
        };
        self.set_y_range(min, max);
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
        let log = matches!(axis.scale, AxisScale::Logarithmic);
        self.with_axis_range_change(|config| {
            if let Some(bar) = config.colorbar.as_mut() {
                apply_axis_range(&mut bar.axis, min, max, log);
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
    pub fn auto_fit_layer_union(
        &mut self,
        pool: &ColumnPool,
        layer: LayerRef,
        series: &[SeriesConfig],
        padding_ratio: f64,
    ) -> Result<()> {
        let mut x = FitExtent::EMPTY;
        let mut y = FitExtent::EMPTY;
        for cfg in series {
            let (fit_x, fit_y) = self.config.series_fit_roles(layer, &cfg.series_id);
            if fit_x {
                x.union(&Self::slot_extent(pool, &cfg.x_column)?);
            }
            if fit_y {
                y.union(&Self::slot_extent(pool, &cfg.y_column)?);
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            cfg.chart_title.text.segments = rich_segments_from_text("one\ntwo\nthree");
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
                reason: "logarithmic axis bounds must be positive"
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
}
