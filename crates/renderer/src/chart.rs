//! User-facing `Chart` — wraps a `Config` with rescale / auto_fit helpers.
//!
//! `Config` is the SSoT for chart options (axes, labels, margins, colors).
//! `Chart` adds one-line conveniences (`set_x_range`, `auto_fit_x`, …) plus
//! dirty-flag tracking: `data_dirty` (GPU transform only) and `raster_dirty`
//! (CPU axis raster) are set automatically based on the kind of `Config`
//! change, and cleared by the caller via `consume_*_dirty()`.

use crate::color::Color;
use crate::config::{AxisScale, Config, LegendEntryKind};
use crate::data_render::ColumnPool;
use crate::error::{FiggyError, Result};
use crate::format::LabelFormat;
use crate::text::{RichText, rich_segments_from_text};

/// Apply min/max and scale-appropriate tick settings to one axis.
fn apply_axis_range(axis: &mut crate::config::AxisOptions, min: f64, max: f64, log: bool) {
    axis.min = min;
    axis.max = max;
    if log {
        axis.major_spacing = log_decade_step(min, max);
        axis.minor_count = 8;
        axis.label_style.format = LabelFormat::Power;
        axis.label_style.significant_digits = 1;
    } else {
        axis.major_spacing = nice_spacing(max - min);
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

/// Major spacing for a logarithmic axis, in decades.
///
/// `min`/`max` are in linear space; both shader and axis raster interpret
/// `major_spacing` as "decades per major tick". Branches aim for 5–8 labels.
pub(crate) fn log_decade_step(min: f64, max: f64) -> f64 {
    if !min.is_finite() || !max.is_finite() || min <= 0.0 || max <= min {
        return 1.0;
    }
    let log_min = min.log10().floor();
    let log_max = max.log10().ceil();
    let decades = (log_max - log_min).max(1.0);
    if decades <= 6.0 {
        1.0
    } else if decades <= 12.0 {
        2.0
    } else {
        (decades / 6.0).ceil()
    }
}

/// Fallback lower bound for invalid manual log-axis ranges.
pub(crate) const LOG_RANGE_FALLBACK_MIN: f64 = 1.0e-12;

/// Renderer-side guard for log-axis ranges.
pub(crate) fn guarded_log_range(min: f64, max: f64) -> (f64, f64) {
    let min = if min.is_finite() && min > 0.0 {
        min
    } else {
        LOG_RANGE_FALLBACK_MIN
    };
    let mut max = if max.is_finite() && max > 0.0 {
        max
    } else {
        min * 10.0
    };
    if max <= min {
        max = min * 10.0;
    }
    (min, max)
}

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

/// "Nice" tick spacing for a given data span — aims for 5–10 major ticks
/// using the 1·2·5 × 10^k sequence common in scientific plots.
///
/// Examples: span=72 → 10, span=8 → 1, span=0.7 → 0.1.
pub(crate) fn nice_spacing(span: f64) -> f64 {
    if !span.is_finite() || span <= 0.0 {
        return 1.0;
    }
    let exp = span.log10().floor();
    let base = 10f64.powf(exp);
    let frac = span / base;
    // frac is in [1, 10); the 1·2·5 branches keep tick count in 5..=10.
    if frac >= 5.0 {
        base
    } else if frac >= 2.0 {
        0.5 * base
    } else {
        0.2 * base
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

    /// Widen to also cover the single bound `v`.
    fn include(&mut self, v: f64) {
        if v < self.min {
            self.min = v;
        }
        if v > self.max {
            self.max = v;
        }
        if v > 0.0 && self.min_positive.is_none_or(|p| v < p) {
            self.min_positive = Some(v);
        }
    }

    /// A fit is only meaningful over a finite, non-degenerate span — the
    /// same gate the column-union fitters have always applied.
    fn is_fittable(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.max > self.min
    }
}

/// Pairwise fit extent of a value column with errorbar offsets, matching the
/// GPU arithmetic exactly (`errorbar_columnar.wgsl`: `lo = v - err_lo`,
/// `hi = v + err_hi`). This cannot be derived from per-column min/max — the
/// widest bound is not necessarily at a value extreme — so it needs one pass
/// over the actual pairs. Non-finite values are skipped (NaN-gap
/// convention); a non-finite error entry contributes the bare value (the
/// shader draws no bar there but the marker still shows). Offsets past the
/// error columns' length default to 0, mirroring "no bar". Returns `None`
/// when no finite value exists.
pub fn errorbar_extent(vals: &[f32], err_lo: &[f32], err_hi: &[f32]) -> Option<FitExtent> {
    let mut ext = FitExtent::EMPTY;
    let mut any = false;
    for (i, &v) in vals.iter().enumerate() {
        let v = v as f64;
        if !v.is_finite() {
            continue;
        }
        let lo_off = err_lo
            .get(i)
            .map(|&e| e as f64)
            .filter(|e| e.is_finite())
            .unwrap_or(0.0);
        let hi_off = err_hi
            .get(i)
            .map(|&e| e as f64)
            .filter(|e| e.is_finite())
            .unwrap_or(0.0);
        ext.include(v - lo_off);
        ext.include(v + hi_off);
        any = true;
    }
    any.then_some(ext)
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
    pub fn config_mut(&mut self) -> &mut Config {
        self.data_dirty = true;
        self.raster_dirty = true;
        &mut self.config
    }

    /// Mutate where only axis range / scale changes. Raster is invalidated
    /// because tick labels need to be repositioned within the same chart_area.
    pub fn with_axis_range_change<F: FnOnce(&mut Config)>(&mut self, f: F) {
        f(&mut self.config);
        self.data_dirty = true;
        self.raster_dirty = true;
    }

    /// Decoration-only change (titles, colors, margins, …): raster only.
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
    /// (bold, italic, …) edit `config_mut().chart_title.text` directly.
    pub fn with_title(mut self, text: &str) -> Self {
        fill_plain_segments(&mut self.config.chart_title.text, text);
        self.raster_dirty = true;
        self
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

    /// Set both X axes (top + bottom) at once.
    ///
    /// Branches on the bottom axis's scale:
    /// - **Linear**: `major_spacing = nice_spacing(max - min)` (1·2·5 sequence),
    ///   `minor_count = 4`, `format = Decimal`, `sig_digits = 3`.
    /// - **Logarithmic**: `major_spacing = log_decade_step(min, max)`
    ///   (1 / 2 / N decades), `minor_count = 8` (2..9 within each decade),
    ///   `format = Power` (10ⁿ superscript), `sig_digits = 1`.
    pub fn set_x_range(&mut self, min: f64, max: f64) {
        let log = matches!(self.config.bottom_x.scale, AxisScale::Logarithmic);
        self.with_axis_range_change(|c| {
            apply_axis_range(&mut c.bottom_x, min, max, log);
            apply_axis_range(&mut c.top_x, min, max, log);
        });
    }

    /// Set both Y axes (left + right). Log/linear branching mirrors X.
    pub fn set_y_range(&mut self, min: f64, max: f64) {
        let log = matches!(self.config.left_y.scale, AxisScale::Logarithmic);
        self.with_axis_range_change(|c| {
            apply_axis_range(&mut c.left_y, min, max, log);
            apply_axis_range(&mut c.right_y, min, max, log);
        });
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
    /// All scalars were computed at upload; no data rescan. Public so hosts
    /// aggregating their own [`FitExtent`] unions (wasm `auto_fit_all`)
    /// build them from the same source.
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
    /// auto-fit path. Hosts that aggregate extents themselves (e.g. the wasm
    /// `auto_fit_all`, which folds errorbar bounds into the union) call this
    /// directly. No-op on a degenerate extent (non-finite or zero span) and
    /// on a log axis with no positive bound to anchor to.
    pub fn auto_fit_x_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        if !ext.is_fittable() {
            return;
        }
        let log = matches!(self.config.bottom_x.scale, AxisScale::Logarithmic);
        let (lo, hi) = if log {
            let safe = if ext.min > 0.0 {
                ext.min
            } else {
                match ext.min_positive {
                    Some(p) => p,
                    None => return,
                }
            };
            log_padded(safe, ext.max, padding_ratio)
        } else {
            Self::padded(ext.min, ext.max, padding_ratio)
        };
        self.set_x_range(lo, hi);
    }

    /// Fit the Y axis to a prepared [`FitExtent`] — see
    /// [`Self::auto_fit_x_extent`].
    pub fn auto_fit_y_extent(&mut self, ext: &FitExtent, padding_ratio: f64) {
        if !ext.is_fittable() {
            return;
        }
        let log = matches!(self.config.left_y.scale, AxisScale::Logarithmic);
        let (lo, hi) = if log {
            let safe = if ext.min > 0.0 {
                ext.min
            } else {
                match ext.min_positive {
                    Some(p) => p,
                    None => return,
                }
            };
            log_padded(safe, ext.max, padding_ratio)
        } else {
            Self::padded(ext.min, ext.max, padding_ratio)
        };
        self.set_y_range(lo, hi);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

    fn dummy_config() -> Config {
        default_config()
    }

    #[test]
    fn chart_dirty_starts_true() {
        let c = Chart::new(dummy_config());
        assert!(c.data_dirty());
        assert!(c.raster_dirty());
    }

    #[test]
    fn chart_consume_clears_dirty() {
        let mut c = Chart::new(dummy_config());
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

    /// The widest bound is not at a value extreme — per-column min/max
    /// composition would get this wrong (0−5 = −5), the pairwise pass right.
    #[test]
    fn errorbar_extent_is_pairwise_not_minmax_composed() {
        let ext = errorbar_extent(&[0.0, 10.0], &[1.0, 5.0], &[1.0, 5.0]).unwrap();
        assert_eq!(ext.min, -1.0);
        assert_eq!(ext.max, 15.0);
    }

    /// NaN values are gaps (skipped); NaN errors draw no bar, so they
    /// contribute the bare value. Offsets past the error column's end
    /// default to zero.
    #[test]
    fn errorbar_extent_nan_and_length_conventions() {
        let nan = f32::NAN;
        let ext = errorbar_extent(&[nan, 2.0, 4.0], &[100.0, nan], &[100.0, nan]).unwrap();
        assert_eq!(ext.min, 2.0);
        assert_eq!(ext.max, 4.0);

        assert!(errorbar_extent(&[nan, nan], &[1.0, 1.0], &[1.0, 1.0]).is_none());
    }

    #[test]
    fn errorbar_extent_asymmetric_and_min_positive() {
        let ext = errorbar_extent(&[10.0], &[2.0], &[7.0]).unwrap();
        assert_eq!(ext.min, 8.0);
        assert_eq!(ext.max, 17.0);
        assert_eq!(ext.min_positive, Some(8.0));

        // Bound dips below zero: min_positive keeps the smallest positive
        // bound for the log-axis safe floor.
        let ext = errorbar_extent(&[1.0, 3.0], &[2.0, 1.0], &[0.0, 0.0]).unwrap();
        assert_eq!(ext.min, -1.0);
        assert_eq!(ext.min_positive, Some(1.0));
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
}
