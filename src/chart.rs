//! User-facing `Chart` — wraps a `Config` with rescale / auto_fit helpers.
//!
//! `Config` is the SSoT for chart options (axes, labels, margins, colors).
//! `Chart` adds one-line conveniences (`set_x_range`, `auto_fit_x`, …) plus
//! dirty-flag tracking: `data_dirty` (GPU transform only) and `raster_dirty`
//! (CPU axis raster) are set automatically based on the kind of `Config`
//! change, and cleared by the caller via `consume_*_dirty()`.

use crate::color::Color;
use crate::config::{AxisScale, Config, LegendEntry, LegendEntryKind};
use crate::data_render::ColumnPool;
use crate::error::{FiggyError, Result};
use crate::format::LabelFormat;
use crate::text::{rich_segments_from_text, RichText};

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
        axis.label_style.format = LabelFormat::Decimal;
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
    if decades <= 6.0 { 1.0 }
    else if decades <= 12.0 { 2.0 }
    else { (decades / 6.0).ceil() }
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

    pub fn data_dirty(&self) -> bool { self.data_dirty }
    pub fn raster_dirty(&self) -> bool { self.raster_dirty }

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

    /// Add a legend entry. The first call also makes the legend visible.
    pub fn with_legend_entry(
        mut self,
        label: &str,
        color: Color,
        line_width: f32,
        kind: LegendEntryKind,
    ) -> Self {
        self.config.legend.visible = true;
        let label = RichText::plain(
            label,
            Color::BLACK,
            self.config.legend.font_size,
            String::new(),
        );
        self.config.legend.entries.push(LegendEntry {
            label,
            color,
            line_width,
            kind,
        });
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

    /// Auto-fit the X axis range from the pool's column metadata for `x_id`.
    /// Uses multiplicative (positive-preserving) padding for log scale.
    pub fn auto_fit_x(
        &mut self,
        pool: &ColumnPool,
        x_id: &str,
        padding_ratio: f64,
    ) -> Result<()> {
        let slot = pool
            .slot(x_id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: x_id.to_string() })?;
        let log = matches!(self.config.bottom_x.scale, AxisScale::Logarithmic);
        let (lo, hi) = if log {
            log_padded(slot.min, slot.max, padding_ratio)
        } else {
            Self::padded(slot.min, slot.max, padding_ratio)
        };
        self.set_x_range(lo, hi);
        Ok(())
    }

    /// Auto-fit the Y axis range from the pool's column metadata for `y_id`.
    /// Uses multiplicative padding for log scale.
    pub fn auto_fit_y(
        &mut self,
        pool: &ColumnPool,
        y_id: &str,
        padding_ratio: f64,
    ) -> Result<()> {
        let slot = pool
            .slot(y_id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: y_id.to_string() })?;
        let log = matches!(self.config.left_y.scale, AxisScale::Logarithmic);
        let (lo, hi) = if log {
            log_padded(slot.min, slot.max, padding_ratio)
        } else {
            Self::padded(slot.min, slot.max, padding_ratio)
        };
        self.set_y_range(lo, hi);
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
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for id in y_ids {
            let slot = pool
                .slot(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })?;
            if slot.min < lo { lo = slot.min; }
            if slot.max > hi { hi = slot.max; }
        }
        if !(lo.is_finite() && hi.is_finite()) || hi <= lo {
            return Ok(());
        }
        let log = matches!(self.config.left_y.scale, AxisScale::Logarithmic);
        let (lo, hi) = if log {
            log_padded(lo, hi, padding_ratio)
        } else {
            Self::padded(lo, hi, padding_ratio)
        };
        self.set_y_range(lo, hi);
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
}
