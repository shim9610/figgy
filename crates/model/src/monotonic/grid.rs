//! The x-to-column arithmetic of a monotonic group (#271): the column of a
//! sample, the first sample of a column and the visible slice, all by plain
//! arithmetic on `start`, `interval`, the x range and the plot width. No
//! search over the data.

use super::{MonotonicError, MonotonicMapping};
use crate::config::AxisScale;

/// The column grid of an envelope: `x = start + i * interval`, and envelope
/// column `c` holds the samples whose position
/// `t = (x - x_min) / x_per_pixel + margin - shift` lies in `[c, c + 1)`.
/// Envelope column `margin` is the plot's first pixel column: the grid
/// spans `margin` whole columns beyond each edge of the plot
/// ([`MonotonicMapping::margin_px`]).
///
/// `x_min`, `x_max` and `x_per_pixel` are the anchor the full call built the
/// grid on. A slide drops whole columns from the front (`shift` grows) and
/// never moves the anchor, so the columns stay on the data grid they were
/// built on (#271). Every boundary is derived from the same position, so the
/// reduction and a pick always agree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonotonicGrid {
    start: f64,
    interval: f64,
    x_min: f64,
    x_max: f64,
    x_per_pixel: f64,
    /// The envelope's columns: the plot's and the margin on each side.
    width: usize,
    /// The columns beyond each edge of the plot.
    margin: usize,
    /// The x of envelope column 0's left edge: `x_min - margin ×
    /// x_per_pixel`.
    origin: f64,
    shift: usize,
}

/// The samples of the current x range: the visible samples `first..end`
/// (`x_min <= x < x_max`) that the group holds, and the sample on each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleSlice {
    /// The first visible sample.
    pub first: usize,
    /// One past the last visible sample.
    pub end: usize,
    /// The last sample left of the range, when the group holds it.
    pub lead: Option<usize>,
    /// The first sample at or right of the range's end, when the group
    /// holds it.
    pub trail: Option<usize>,
}

impl VisibleSlice {
    /// The number of visible samples.
    pub fn samples(&self) -> usize {
        self.end.saturating_sub(self.first)
    }
}

impl MonotonicGrid {
    /// The grid of a group with this `start` and `interval` on `mapping`.
    ///
    /// Fails for a non-finite `start`, an `interval` that is not finite and
    /// positive, an x range that is not finite and increasing, a zero width,
    /// and an x mapping that is not linear and unbroken (refused for this
    /// source, #271).
    pub fn new(
        start: f64,
        interval: f64,
        mapping: &MonotonicMapping,
    ) -> Result<Self, MonotonicError> {
        if !(start.is_finite() && interval.is_finite() && interval > 0.0) {
            return Err(MonotonicError::InvalidGrid { start, interval });
        }
        let MonotonicMapping {
            x_min,
            x_max,
            plot_width_px,
            margin_px,
            ..
        } = *mapping;
        let width = plot_width_px as usize;
        let x_per_pixel = (x_max - x_min) / f64::from(plot_width_px);
        if !(x_min.is_finite()
            && x_max.is_finite()
            && x_min < x_max
            && width > 0
            && x_per_pixel.is_finite()
            && x_per_pixel > 0.0)
        {
            return Err(MonotonicError::InvalidMapping {
                x_min,
                x_max,
                plot_width_px,
            });
        }
        if mapping.x_scale != AxisScale::Linear || mapping.x_breaks.is_broken() {
            return Err(MonotonicError::UnsupportedXMapping);
        }
        let margin = margin_px as usize;
        let width = margin
            .checked_mul(2)
            .and_then(|both| both.checked_add(width))
            .ok_or(MonotonicError::InvalidMapping {
                x_min,
                x_max,
                plot_width_px,
            })?;
        Ok(Self {
            start,
            interval,
            x_min,
            x_max,
            x_per_pixel,
            width,
            margin,
            origin: x_min - margin as f64 * x_per_pixel,
            shift: 0,
        })
    }

    /// The x of sample 0.
    pub fn start(&self) -> f64 {
        self.start
    }

    /// The x distance between two samples.
    pub fn interval(&self) -> f64 {
        self.interval
    }

    /// The number of envelope columns: the plot's pixel columns and the
    /// margin on each side.
    pub fn width(&self) -> usize {
        self.width
    }

    /// The number of pixel columns of the plot.
    pub fn plot_width(&self) -> usize {
        self.width - 2 * self.margin
    }

    /// The envelope columns beyond each edge of the plot: envelope column
    /// `margin` is the plot's first pixel column.
    pub fn margin(&self) -> usize {
        self.margin
    }

    /// The x units one column spans.
    pub fn x_per_pixel(&self) -> f64 {
        self.x_per_pixel
    }

    /// The whole columns slides have dropped since the full call.
    pub fn shift(&self) -> usize {
        self.shift
    }

    /// The x range the columns are aligned to: the full call's range moved
    /// by the dropped columns.
    pub fn aligned_range(&self) -> (f64, f64) {
        let moved = self.shift as f64 * self.x_per_pixel;
        (self.x_min + moved, self.x_max + moved)
    }

    /// The x of sample `i`.
    #[inline]
    pub fn x(&self, i: usize) -> f64 {
        self.start + i as f64 * self.interval
    }

    /// The position of sample `i` on the anchor, in columns.
    #[inline]
    fn anchored(&self, i: usize) -> f64 {
        (self.x(i) - self.origin) / self.x_per_pixel
    }

    /// The position of sample `i` in columns from the envelope's column 0.
    /// Non-decreasing in `i`.
    #[inline]
    pub fn position(&self, i: usize) -> f64 {
        self.anchored(i) - self.shift as f64
    }

    /// The envelope column of sample `i`, `None` left of column 0.
    pub fn column(&self, i: usize) -> Option<usize> {
        let c = self.anchored(i).floor() - self.shift as f64;
        (c >= 0.0).then_some(c as usize)
    }

    /// The column an x falls in, `None` left of column 0.
    pub fn column_of_x(&self, x: f64) -> Option<usize> {
        let c = ((x - self.origin) / self.x_per_pixel).floor() - self.shift as f64;
        (c >= 0.0).then_some(c as usize)
    }

    /// The smallest `i < limit` whose `key` satisfies `pred`, else `limit`.
    /// `key` is non-decreasing in `i`, so the arithmetic estimate needs at
    /// most a step or two of correction.
    fn first_where(
        &self,
        limit: usize,
        estimate: f64,
        key: impl Fn(usize) -> f64,
        pred: impl Fn(f64) -> bool,
    ) -> usize {
        let mut i = if estimate.is_nan() || estimate <= 0.0 {
            0
        } else if estimate >= limit as f64 {
            limit
        } else {
            estimate as usize
        };
        while i > 0 && pred(key(i - 1)) {
            i -= 1;
        }
        while i < limit && !pred(key(i)) {
            i += 1;
        }
        i
    }

    /// The first sample of envelope column `c` (the smallest `i` with
    /// `position(i) >= c`), or `limit` when it is not below `limit`.
    pub fn column_start(&self, c: usize, limit: usize) -> usize {
        let edge = (self.shift as f64) + c as f64;
        let estimate =
            ((self.origin + edge * self.x_per_pixel - self.start) / self.interval).ceil();
        self.first_where(limit, estimate, |i| self.anchored(i), |t| t >= edge)
    }

    /// The first sample with `x >= x`, or `limit`.
    fn first_at_x(&self, x: f64, limit: usize) -> usize {
        let estimate = ((x - self.start) / self.interval).ceil();
        self.first_where(limit, estimate, |i| self.x(i), |v| v >= x)
    }

    /// The visible slice of `x_min..x_max` in a group of `len` samples.
    pub fn visible(&self, len: usize, x_min: f64, x_max: f64) -> VisibleSlice {
        let i0 = self.first_at_x(x_min, len.saturating_add(1));
        let e = self.first_at_x(x_max, len);
        let first = i0.min(len);
        VisibleSlice {
            first,
            end: e.max(first),
            lead: (i0 >= 1 && i0 <= len).then(|| i0 - 1),
            trail: (e < len).then_some(e),
        }
    }

    /// The grid after `dropped` more whole columns left the front.
    pub(crate) fn shifted(&self, dropped: usize) -> Self {
        Self {
            shift: self.shift.saturating_add(dropped),
            ..*self
        }
    }
}
