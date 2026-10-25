//! Tick planning: the "nice" spacing rule every axis shares.
//!
//! [`nice_linear_step`] and [`nice_log_step`] are the one spacing rule. The
//! Cartesian axis helpers (`Chart::set_x_range` and the fits) call it through
//! [`axis_major_spacing`] with [`AXIS_LINEAR_TARGET`] and
//! [`AXIS_LOG_TARGET`]; [`AxisOptions::compute_nice_ticks`] calls it with the
//! caller's target and also widens the range to whole steps, which is what
//! the legacy document readers' fixed value axes and contour levels use.

use crate::config::{AxisOptions, AxisScale, Config, LogTickPolicy};
use crate::layout::guarded_log_range;

#[derive(Debug, Clone, PartialEq)]
pub struct TickPlan {
    pub min: f64,
    pub max: f64,
    pub major_spacing: f64,
    pub minor_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TickError {
    InvalidRange,
    NonPositiveLog,
    ZeroTargetCount,
}

/// Major ticks a linear Cartesian axis aims for: with
/// [`LooseRange::Above`] the step is 0.2, 0.5 or 1 × 10^k of the span, so a
/// span gets 5 to 10 major intervals.
pub const AXIS_LINEAR_TARGET: usize = 10;

/// Major ticks a logarithmic Cartesian axis aims for: 1 decade per major
/// tick up to 6 decades, 2 up to 12, then `ceil(decades / 6)`.
pub const AXIS_LOG_TARGET: usize = 7;

/// How the first stage of [`nice_linear_step`] rounds the span up to a
/// 1·2·5 × 10^k "loose range" before dividing it into steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LooseRange {
    /// The smallest of 1, 2, 5 and 10 × 10^k at least the span (Heckbert's
    /// closed classes). [`AxisOptions::compute_nice_ticks`] uses it, and
    /// with it the legacy document readers and contour levels.
    AtLeast,
    /// The smallest of 2, 5 and 10 × 10^k strictly above the span's leading
    /// digit class: a span of exactly 1, 2 or 5 × 10^k rounds up to the next
    /// class. With [`AXIS_LINEAR_TARGET`] this is the Cartesian axis rule.
    Above,
}

fn nice_parts(range: f64, round: bool, loose: LooseRange) -> (f64, f64) {
    let exp = range.log10().floor();
    let scale = 10f64.powf(exp);
    let frac = range / scale;
    let nice_frac = if round {
        if frac < 1.5 {
            1.0
        } else if frac < 3.0 {
            2.0
        } else if frac < 7.0 {
            5.0
        } else {
            10.0
        }
    } else {
        match loose {
            LooseRange::AtLeast => {
                if frac <= 1.0 {
                    1.0
                } else if frac <= 2.0 {
                    2.0
                } else if frac <= 5.0 {
                    5.0
                } else {
                    10.0
                }
            }
            LooseRange::Above => {
                if frac < 2.0 {
                    2.0
                } else if frac < 5.0 {
                    5.0
                } else {
                    10.0
                }
            }
        }
    };
    (nice_frac, scale)
}

fn nice_num(range: f64, round: bool, loose: LooseRange) -> f64 {
    let (nice_frac, scale) = nice_parts(range, round, loose);
    nice_frac * scale
}

fn finite_nice_value(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
    }
}

/// The 1·2·5 × 10^k step that divides `span` into about `target` ticks
/// (`target - 1` intervals; a target below 2 counts as 2).
///
/// The span is first rounded up to a loose range as `loose` says, divided by
/// `target - 1`, and the quotient rounded to the nearest 1, 2, 5 or 10 ×
/// 10^k. `None` when `span` is not finite and positive. A span too small for
/// the arithmetic (subnormal) is its own step; the result is always finite
/// and positive.
pub fn nice_linear_step(span: f64, target: usize, loose: LooseRange) -> Option<f64> {
    if !(span.is_finite() && span > 0.0) {
        return None;
    }
    let divisor = ((target as f64) - 1.0).max(1.0);
    let (loose_frac, scale) = nice_parts(span, false, loose);
    let range = loose_frac * scale;
    let raw_step = if range.is_infinite() {
        // A span above about 1e308 rounds up past f64::MAX; divide the power
        // of ten first so the step keeps its class.
        finite_nice_value(
            loose_frac * (scale / divisor),
            finite_nice_value(span / divisor, span),
        )
    } else {
        let range = finite_nice_value(range, span);
        finite_nice_value(range / divisor, range)
    };
    Some(finite_nice_value(nice_num(raw_step, true, loose), raw_step))
}

/// Whole decades per major tick for a log axis spanning `decades` decades,
/// aiming for about `target` ticks: `ceil(decades / (target - 1))`, at least
/// 1. `None` when `decades` is not finite and positive.
pub fn nice_log_step(decades: f64, target: usize) -> Option<f64> {
    if !(decades.is_finite() && decades > 0.0) {
        return None;
    }
    let divisor = ((target as f64) - 1.0).max(1.0);
    Some((decades / divisor).ceil().max(1.0))
}

/// The major spacing a Cartesian axis helper writes for `min..max`.
///
/// Linear: [`nice_linear_step`] of the span with [`AXIS_LINEAR_TARGET`] and
/// [`LooseRange::Above`] (span 72 → 10, 8 → 1, 0.7 → 0.1, 10 → 2).
/// Logarithmic: [`nice_log_step`] of the whole decades the range touches
/// with [`AXIS_LOG_TARGET`], in decades per major tick. Never fails: a range
/// the rule cannot measure gets 1.0.
pub fn axis_major_spacing(scale: &AxisScale, min: f64, max: f64) -> f64 {
    match scale {
        AxisScale::Linear => {
            nice_linear_step(max - min, AXIS_LINEAR_TARGET, LooseRange::Above).unwrap_or(1.0)
        }
        AxisScale::Logarithmic => {
            if !min.is_finite() || !max.is_finite() || min <= 0.0 || max <= min {
                return 1.0;
            }
            let decades = (max.log10().ceil() - min.log10().floor()).max(1.0);
            nice_log_step(decades, AXIS_LOG_TARGET).unwrap_or(1.0)
        }
    }
}

fn checked_plan(
    min: f64,
    max: f64,
    major_spacing: f64,
    minor_count: usize,
    data_min: f64,
    data_max: f64,
    logarithmic: bool,
) -> Result<TickPlan, TickError> {
    if !(min.is_finite()
        && max.is_finite()
        && major_spacing.is_finite()
        && major_spacing > 0.0
        && min < max
        && min <= data_min
        && max >= data_max
        && (!logarithmic || min > 0.0))
    {
        return Err(TickError::InvalidRange);
    }
    Ok(TickPlan {
        min,
        max,
        major_spacing,
        minor_count,
    })
}

fn compute_linear(
    data_min: f64,
    data_max: f64,
    target_count: usize,
) -> Result<TickPlan, TickError> {
    let step = nice_linear_step(data_max - data_min, target_count, LooseRange::AtLeast)
        .ok_or(TickError::InvalidRange)?;
    let candidate_min = (data_min / step).floor() * step;
    let candidate_max = (data_max / step).ceil() * step;
    let nice_min = if candidate_min.is_finite() && candidate_min <= data_min {
        candidate_min
    } else {
        data_min
    };
    let nice_max = if candidate_max.is_finite() && candidate_max >= data_max {
        candidate_max
    } else {
        data_max
    };
    checked_plan(nice_min, nice_max, step, 4, data_min, data_max, false)
}

fn compute_log(data_min: f64, data_max: f64, target_count: usize) -> Result<TickPlan, TickError> {
    let log_min = data_min.log10().floor();
    let log_max = data_max.log10().ceil();
    if !(log_min.is_finite() && log_max.is_finite()) {
        return Err(TickError::InvalidRange);
    }
    let decade_step =
        nice_log_step(log_max - log_min, target_count).ok_or(TickError::InvalidRange)?;
    let candidate_min = 10f64.powf(log_min);
    let candidate_max = 10f64.powf(log_max);
    let nice_min = if candidate_min.is_finite() && candidate_min > 0.0 {
        candidate_min
    } else {
        data_min
    };
    let nice_max = if candidate_max.is_finite() {
        candidate_max
    } else {
        data_max
    };
    checked_plan(nice_min, nice_max, decade_step, 8, data_min, data_max, true)
}

impl AxisOptions {
    pub fn compute_nice_ticks(
        scale: AxisScale,
        data_min: f64,
        data_max: f64,
        target_count: usize,
    ) -> Result<TickPlan, TickError> {
        if target_count == 0 {
            return Err(TickError::ZeroTargetCount);
        }
        if !(data_min.is_finite() && data_max.is_finite() && data_min < data_max) {
            return Err(TickError::InvalidRange);
        }
        if scale == AxisScale::Logarithmic && data_min <= 0.0 {
            return Err(TickError::NonPositiveLog);
        }
        match scale {
            AxisScale::Linear => compute_linear(data_min, data_max, target_count),
            AxisScale::Logarithmic => compute_log(data_min, data_max, target_count),
        }
    }

    pub fn auto_ticks(
        &mut self,
        data_min: f64,
        data_max: f64,
        target_count: usize,
    ) -> Result<(), TickError> {
        let plan =
            AxisOptions::compute_nice_ticks(self.scale.clone(), data_min, data_max, target_count)?;
        self.min = plan.min;
        self.max = plan.max;
        self.major_spacing = plan.major_spacing;
        self.minor_count = plan.minor_count;
        Ok(())
    }
}

impl Config {
    pub fn auto_ticks_all(
        &mut self,
        top_x: (f64, f64),
        bottom_x: (f64, f64),
        left_y: (f64, f64),
        right_y: (f64, f64),
        target_count: usize,
    ) -> Result<(), TickError> {
        // Compute all plans first (transactional).
        let plan_top = AxisOptions::compute_nice_ticks(
            self.top_x.scale.clone(),
            top_x.0,
            top_x.1,
            target_count,
        )?;
        let plan_bottom = AxisOptions::compute_nice_ticks(
            self.bottom_x.scale.clone(),
            bottom_x.0,
            bottom_x.1,
            target_count,
        )?;
        let plan_left = AxisOptions::compute_nice_ticks(
            self.left_y.scale.clone(),
            left_y.0,
            left_y.1,
            target_count,
        )?;
        let plan_right = AxisOptions::compute_nice_ticks(
            self.right_y.scale.clone(),
            right_y.0,
            right_y.1,
            target_count,
        )?;

        // Commit.
        apply_plan(&mut self.top_x, plan_top);
        apply_plan(&mut self.bottom_x, plan_bottom);
        apply_plan(&mut self.left_y, plan_left);
        apply_plan(&mut self.right_y, plan_right);
        Ok(())
    }
}

fn apply_plan(axis: &mut AxisOptions, p: TickPlan) {
    axis.min = p.min;
    axis.max = p.max;
    axis.major_spacing = p.major_spacing;
    axis.minor_count = p.minor_count;
}

// Tick walks.
//
// The walks every axis shares: the absolute linear grid, the log decades and
// the 2a..9a log minors, and the guards that keep a written spacing, minor
// count or decade step from blanking an axis or walking it without bound.
// None of them allocates; each is a `Copy` state with an iterator.

/// Slack, in steps, that keeps a tick landing on a range end through
/// floating-point noise (`floor(2.9999999999999996)`) on the grid.
pub const GRID_EPSILON: f64 = 1.0e-9;

/// Hard ceiling on majors the linear walker may emit. A finite-but-tiny
/// spacing (a mis-typed magnitude arriving through the SSoT) would otherwise
/// walk the whole range in near-zero steps, which reads as a frozen chart.
pub const MAX_MAJOR_INTERVALS: usize = 1000;

/// Minor subdivisions per major interval the linear walker draws at most,
/// counting the major itself: `minor_count` is clamped to one less.
pub const MAX_MINOR_SUBDIVISIONS: usize = 100;

/// Decades the log walkers visit at most.
pub const MAX_LOG_DECADES: usize = 1024;

/// The largest decade step the log walker takes; the renderer's
/// `RenderWarning::LogDecadeStepAdjusted` message states the same bound.
pub const MAX_LOG_DECADE_STEP: i32 = i32::MAX;

/// Why the linear tick walker did not walk a requested `major_spacing`.
///
/// New reasons may be added as new variants. The fields of existing variants
/// are frozen.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SpacingRejection {
    /// NaN or infinite.
    NonFinite,
    /// Zero or negative.
    NotPositive,
    /// Too small to move either end of the axis range at `f64` precision
    /// (`min + spacing == min` and `max - spacing == max`), so the walk could
    /// not advance.
    BelowResolution,
    /// The axis range holds more than `limit` intervals of this spacing.
    TooManyTicks {
        /// The most major intervals the walker draws (1000).
        limit: usize,
    },
}

fn range_is_walkable(min: f64, max: f64) -> bool {
    min.is_finite() && max.is_finite() && max > min
}

fn spacing_advances(min: f64, max: f64, spacing: f64) -> bool {
    let forward = min + spacing;
    let backward = max - spacing;
    (forward.is_finite() && forward > min) || (backward.is_finite() && backward < max)
}

/// Range a tick walker may iterate: finite ends, positive span. NaN/±inf
/// ends otherwise slip past plain `max <= min` comparisons and saturate the
/// integer step bounds (`inf as i64` = i64::MAX, an unbounded walk).
pub fn walkable_range(axis: &AxisOptions) -> bool {
    range_is_walkable(axis.min, axis.max)
}

/// The axis range measured in `spacing` units, or `None` when the range is
/// not walkable, the spacing is not finite and positive, or the count does
/// not fit an `f64`.
pub fn range_in_spacing_units(axis: &AxisOptions, spacing: f64) -> Option<f64> {
    if !walkable_range(axis) || !spacing.is_finite() || spacing <= 0.0 {
        return None;
    }
    let span = axis.max - axis.min;
    let units = if span.is_finite() {
        span / spacing
    } else {
        // Divide before subtracting so `-MAX..MAX` can still be measured when
        // the requested spacing is large enough to make the walk bounded.
        axis.max / spacing - axis.min / spacing
    };
    (units.is_finite() && units >= 0.0).then_some(units)
}

/// Whether one `spacing` moves either end of the axis range at `f64`
/// precision.
pub fn spacing_advances_range(axis: &AxisOptions, spacing: f64) -> bool {
    spacing_advances(axis.min, axis.max, spacing)
}

/// The spacing the linear walkers fall back to: [`axis_major_spacing`] for
/// the current range, or the span itself when that does not advance the
/// range at `f64` precision.
pub fn automatic_linear_spacing(axis: &AxisOptions) -> f64 {
    if !walkable_range(axis) {
        return 1.0;
    }
    let span = axis.max - axis.min;
    let mut spacing = if span.is_finite() {
        axis_major_spacing(&AxisScale::Linear, axis.min, axis.max)
    } else {
        let scale = axis.min.abs().max(axis.max.abs());
        let normalized_span = axis.max / scale - axis.min / scale;
        scale * (normalized_span / 8.0)
    };
    if !spacing.is_finite() || spacing <= 0.0 {
        spacing = if span.is_finite() && span > 0.0 {
            span
        } else {
            1.0
        };
    }
    if !spacing_advances_range(axis, spacing) && span.is_finite() && span > 0.0 {
        spacing = span;
    }
    spacing
}

/// The major spacing the linear walkers actually walk. The SSoT accepts
/// whatever the host sends (a live spacing input passes through 0 mid-edit),
/// so the guard lives here, in the one place shared by tick marks, grid
/// lines, minors, and label decimals, which therefore can never disagree. A
/// non-finite or ≤ 0 spacing, or one that would emit an absurd tick count,
/// falls back to [`automatic_linear_spacing`] instead of blanking the axis.
/// Callers must have validated the range as finite and non-empty.
pub fn effective_major_spacing(axis: &AxisOptions) -> f64 {
    match major_spacing_rejection(axis) {
        None => axis.major_spacing,
        Some(_) => automatic_linear_spacing(axis),
    }
}

/// Why [`effective_major_spacing`] does not walk `axis.major_spacing`, or
/// `None` when it does. The one decision behind both the fallback and the
/// renderer's warning about it, so the two cannot disagree.
///
/// The reason is meaningful for a walkable range only, the same precondition
/// [`effective_major_spacing`] has: any other range refuses every spacing
/// (reported as `TooManyTicks`, because its interval count cannot be measured)
/// and the walkers draw no linear ticks on it.
pub fn major_spacing_rejection(axis: &AxisOptions) -> Option<SpacingRejection> {
    let sp = axis.major_spacing;
    if !sp.is_finite() {
        Some(SpacingRejection::NonFinite)
    } else if sp <= 0.0 {
        Some(SpacingRejection::NotPositive)
    } else if !range_in_spacing_units(axis, sp)
        .is_some_and(|units| units <= MAX_MAJOR_INTERVALS as f64)
    {
        Some(SpacingRejection::TooManyTicks {
            limit: MAX_MAJOR_INTERVALS,
        })
    } else if !spacing_advances_range(axis, sp) {
        Some(SpacingRejection::BelowResolution)
    } else {
        None
    }
}

/// Minor ticks the linear walker draws per major interval: `minor_count`,
/// clamped so the walk stays O(majors × subdivisions) whatever arrives.
pub fn drawn_minor_count(axis: &AxisOptions) -> usize {
    axis.minor_count.min(MAX_MINOR_SUBDIVISIONS - 1)
}

fn clamped_exponent(value: f64) -> i32 {
    value.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// The decade step the log walker takes: `major_spacing` floored, at least 1
/// and at most [`MAX_LOG_DECADE_STEP`]; a non-finite spacing walks 1.
pub fn effective_log_decade_step(axis: &AxisOptions) -> i32 {
    let step = if axis.major_spacing.is_finite() {
        axis.major_spacing.max(1.0).floor()
    } else {
        1.0
    };
    step.min(f64::from(MAX_LOG_DECADE_STEP)) as i32
}

/// The major ladder a logarithmic axis walks, from its range, its
/// [`AxisOptions::log_ticks`] policy and constants: nothing per point.
///
/// [`LogTickPolicy::Decades`]: [`LogLadder::Decades`] of
/// [`effective_log_decade_step`], the walk every log axis took before the
/// policy existed. [`LogTickPolicy::Adaptive`] ignores `major_spacing`: over
/// the [`crate::layout::guarded_log_range`] of the axis range it takes
/// `n =` [`axis_major_spacing`], `Decades(n)` when `n > 1` (more than six
/// decades), and otherwise the first of `Decades(1)`,
/// [`LogLadder::OneTwoFive`] and [`LogLadder::AllMantissas`] that puts at
/// least [`LOG_LADDER_MIN_MAJORS`] majors inside the range, else
/// `AllMantissas`.
pub fn log_ladder(axis: &AxisOptions) -> LogLadder {
    match axis.log_ticks {
        LogTickPolicy::Decades => LogLadder::Decades(effective_log_decade_step(axis)),
        LogTickPolicy::Adaptive => {
            let (min, max) = guarded_log_range(axis.min, axis.max);
            let step = axis_major_spacing(&AxisScale::Logarithmic, min, max);
            if step > 1.0 {
                return LogLadder::Decades(step.min(f64::from(MAX_LOG_DECADE_STEP)) as i32);
            }
            [LogLadder::Decades(1), LogLadder::OneTwoFive]
                .into_iter()
                .find(|ladder| {
                    MajorTicks(log_major_walk(*ladder, min, max))
                        .take(LOG_LADDER_MIN_MAJORS)
                        .count()
                        == LOG_LADDER_MIN_MAJORS
                })
                .unwrap_or(LogLadder::AllMantissas)
        }
    }
}

/// The multiples of a step inside a range, anchored to absolute multiples
/// (`k · step`) rather than to the range's start, so tick values stay round
/// whatever the range ends are.
///
/// Index `k` runs from `ceil(min / step - GRID_EPSILON)` to
/// `floor(max / step + GRID_EPSILON)`, at most `max_intervals + 1` indices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearGrid {
    first: f64,
    last: f64,
    step: f64,
    max_intervals: usize,
}

impl LinearGrid {
    /// The grid of `step` over `min..max`, walking at most `max_intervals`
    /// intervals. `None` when the range is not finite with `min < max`, the
    /// step is not finite and positive, the step does not move either end of
    /// the range at `f64` precision, or no multiple of it lies in the range.
    pub fn new(min: f64, max: f64, step: f64, max_intervals: usize) -> Option<Self> {
        if !range_is_walkable(min, max)
            || !step.is_finite()
            || step <= 0.0
            || !spacing_advances(min, max, step)
        {
            return None;
        }
        let first = ((min / step) - GRID_EPSILON).ceil();
        let last = ((max / step) + GRID_EPSILON).floor();
        if !first.is_finite() || !last.is_finite() || last < first {
            return None;
        }
        Some(Self {
            first,
            last,
            step,
            max_intervals,
        })
    }

    /// The index of the first multiple, `ceil(min / step - GRID_EPSILON)`.
    pub fn first_index(&self) -> f64 {
        self.first
    }

    /// The index of the last multiple, `floor(max / step + GRID_EPSILON)`.
    pub fn last_index(&self) -> f64 {
        self.last
    }

    /// The step the grid walks.
    pub fn step(&self) -> f64 {
        self.step
    }

    /// The `(index, index · step)` pairs, in increasing order. A value that
    /// is not finite is skipped, and one equal to the previous value (an
    /// index past 2^53) is yielded once.
    pub fn iter(&self) -> LinearGridIter {
        LinearGridIter {
            grid: *self,
            next_offset: Some(0),
            previous: None,
        }
    }
}

/// The iterator of [`LinearGrid::iter`].
#[derive(Debug, Clone)]
pub struct LinearGridIter {
    grid: LinearGrid,
    next_offset: Option<usize>,
    previous: Option<f64>,
}

impl Iterator for LinearGridIter {
    type Item = (f64, f64);

    fn next(&mut self) -> Option<(f64, f64)> {
        while let Some(offset) = self.next_offset {
            self.next_offset = offset
                .checked_add(1)
                .filter(|next| *next <= self.grid.max_intervals);
            let index = self.grid.first + offset as f64;
            if !index.is_finite() || index > self.grid.last {
                self.next_offset = None;
                return None;
            }
            let value = index * self.grid.step;
            if !value.is_finite() || self.previous == Some(value) {
                continue;
            }
            self.previous = Some(value);
            return Some((index, value));
        }
        None
    }
}

/// Decade exponents `first..=last` by `step`, each with its value
/// `10^exponent` (`powf`, which keeps subnormal decades).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogDecades {
    next: Option<i32>,
    last: i32,
    step: i32,
}

impl LogDecades {
    /// The decades `first, first + step, …` up to `last`. Empty when
    /// `first > last`; `None` when `step` is below 1.
    pub fn new(first: i32, last: i32, step: i32) -> Option<Self> {
        (step >= 1).then_some(Self {
            next: Some(first),
            last,
            step,
        })
    }
}

impl Iterator for LogDecades {
    type Item = (i32, f64);

    fn next(&mut self) -> Option<(i32, f64)> {
        let exponent = self.next.filter(|exponent| *exponent <= self.last)?;
        self.next = exponent.checked_add(self.step);
        Some((exponent, 10f64.powf(f64::from(exponent))))
    }
}

/// The [`LogMultiples`] mask of the log minors: the multiples 2 to 9.
const LOG_MINOR_MULTIPLES: u16 = 0b11_1111_1100;

/// The [`LogMultiples`] mask of the decade ladder: the multiple 1.
const LOG_DECADE: u16 = 1 << 1;

/// The [`LogMultiples`] mask of the 1, 2 and 5 × 10^k ladder.
const LOG_ONE_TWO_FIVE: u16 = (1 << 1) | (1 << 2) | (1 << 5);

/// The [`LogMultiples`] mask of the 1 to 9 × 10^k ladder.
const LOG_ALL_MANTISSAS: u16 = 0b11_1111_1110;

/// The fewest majors inside the range at which [`log_ladder`] keeps a
/// sparser ladder under [`LogTickPolicy::Adaptive`]: a range that holds
/// fewer whole decades tries the 1-2-5 ladder, then the 1..9 ladder.
pub const LOG_LADDER_MIN_MAJORS: usize = 4;

/// Which multiples of every decade a logarithmic axis draws as majors
/// ([`log_ladder`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLadder {
    /// Every `step`-th whole decade `10^k`, `step` at least 1.
    Decades(i32),
    /// 1, 2 and 5 × 10^k.
    OneTwoFive,
    /// 1, 2, …, 9 × 10^k.
    AllMantissas,
}

impl LogLadder {
    /// The [`LogMultiples`] mask of the multiples the ladder's majors take
    /// in a decade; a [`LogLadder::Decades`] step above 1 skips whole
    /// decades as well.
    fn multiples(self) -> u16 {
        match self {
            LogLadder::Decades(_) => LOG_DECADE,
            LogLadder::OneTwoFive => LOG_ONE_TWO_FIVE,
            LogLadder::AllMantissas => LOG_ALL_MANTISSAS,
        }
    }
}

/// The multiples `m · a` of every decade `a` overlapping `min..max` whose
/// bit `m` is set in a mask, that fall inside the range, in increasing
/// order: anchored at the decade powers, so partial edge decades keep their
/// multiples. At most [`MAX_LOG_DECADES`] decades are visited.
///
/// Bit `m` of the mask (`m` in `1..=9`) selects `m · 10^k`; bit 0 and bits
/// 10 to 15 are ignored. Each value is `10^k` (`powf`, which keeps subnormal
/// decades) times `m`, so a multiple is the same `f64` whichever mask walks
/// it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogMultiples {
    min: f64,
    max: f64,
    decade: Option<i32>,
    last: i32,
    decades_left: usize,
    base: f64,
    multiple: u8,
    mask: u16,
}

impl LogMultiples {
    /// The multiples selected by `mask` over `min..max`, a finite,
    /// positive, ordered log range (a written one is guarded by
    /// [`crate::layout::guarded_log_range`]).
    pub fn new(min: f64, max: f64, mask: u16) -> Self {
        let first = clamped_exponent(min.log10().floor());
        Self {
            min,
            max,
            decade: Some(first),
            last: clamped_exponent(max.log10().floor()),
            decades_left: MAX_LOG_DECADES,
            base: 0.0,
            // Past 9: the next call opens the first decade.
            multiple: 10,
            mask,
        }
    }
}

impl Iterator for LogMultiples {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        loop {
            if self.multiple > 9 {
                let decade = self.decade.filter(|decade| *decade <= self.last)?;
                if self.decades_left == 0 {
                    self.decade = None;
                    return None;
                }
                self.decades_left -= 1;
                self.decade = decade.checked_add(1);
                self.base = 10f64.powf(f64::from(decade));
                self.multiple = 1;
            }
            let multiple = self.multiple;
            self.multiple += 1;
            if self.mask & (1 << multiple) == 0 {
                continue;
            }
            let value = self.base * f64::from(multiple);
            if value.is_finite() && value >= self.min && value <= self.max {
                return Some(value);
            }
        }
    }
}

/// The log minor ticks `2a..9a` of every decade `a` overlapping `min..max`
/// that fall inside it, in increasing order: anchored at the decade powers,
/// not at the majors, so partial edge decades (and multi-decade major steps)
/// keep their minors. At most [`MAX_LOG_DECADES`] decades are visited. The
/// [`LogMultiples`] of the multiples 2 to 9.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogMinors(LogMultiples);

impl LogMinors {
    /// The minors over `min..max`, a finite, positive, ordered log range
    /// (a written one is guarded by [`crate::layout::guarded_log_range`]).
    pub fn new(min: f64, max: f64) -> Self {
        Self(LogMultiples::new(min, max, LOG_MINOR_MULTIPLES))
    }
}

impl Iterator for LogMinors {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        self.0.next()
    }
}

/// The label ladders the renderer's tick-label pruning demotes an axis'
/// labels along, as [`LogMultiples`] masks, densest first: the axis'
/// [`log_ladder`] and the sparser ones after it, {1..9} → {1, 2, 5} → {1}.
/// Empty for a [`LogLadder::Decades`] ladder (always so under the default
/// [`LogTickPolicy::Decades`]) and on a linear or categorical axis, whose
/// labels are thinned by stride alone.
pub fn log_label_ladders(axis: &AxisOptions) -> &'static [u16] {
    if axis.scale != AxisScale::Logarithmic || axis.categories.is_some() {
        return &[];
    }
    match log_ladder(axis) {
        LogLadder::Decades(_) => &[],
        LogLadder::OneTwoFive => &[LOG_ONE_TWO_FIVE, LOG_DECADE],
        LogLadder::AllMantissas => &[LOG_ALL_MANTISSAS, LOG_ONE_TWO_FIVE, LOG_DECADE],
    }
}

/// The multiple `m` (1 to 9) when `value` is `m · 10^k` exactly as the log
/// walkers write it (`10^k` by `powf`, times `m`); `None` for any other
/// value (`3.5e4`), zero, a negative or a non-finite value.
pub fn log_multiple(value: f64) -> Option<u8> {
    if !(value.is_finite() && value > 0.0) {
        return None;
    }
    let exponent = clamped_exponent(value.log10().floor());
    // `log10` may land one decade off next to a power of ten.
    [
        exponent,
        exponent.saturating_sub(1),
        exponent.saturating_add(1),
    ]
    .into_iter()
    .find_map(|exponent| {
        let base = 10f64.powf(f64::from(exponent));
        let multiple = (value / base).round();
        ((1.0..=9.0).contains(&multiple) && base * multiple == value).then_some(multiple as u8)
    })
}

#[derive(Debug, Clone)]
enum MajorWalk {
    Empty,
    Linear(LinearGridIter),
    Log { decades: LogDecades, left: usize },
    LogMultiples(LogMultiples),
}

/// The majors of `ladder` over the guarded log range `min..max`: whole
/// decades by their exponents for [`LogLadder::Decades`], the ladder's
/// multiples of every decade otherwise.
fn log_major_walk(ladder: LogLadder, min: f64, max: f64) -> MajorWalk {
    match ladder {
        LogLadder::Decades(step) => {
            let first = clamped_exponent(min.log10().ceil());
            let last = clamped_exponent(max.log10().floor());
            LogDecades::new(first, last, step).map_or(MajorWalk::Empty, |decades| MajorWalk::Log {
                decades,
                left: MAX_LOG_DECADES,
            })
        }
        LogLadder::OneTwoFive | LogLadder::AllMantissas => {
            MajorWalk::LogMultiples(LogMultiples::new(min, max, ladder.multiples()))
        }
    }
}

/// The major tick values of an axis, from [`major_ticks`].
#[derive(Debug, Clone)]
pub struct MajorTicks(MajorWalk);

impl Iterator for MajorTicks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        match &mut self.0 {
            MajorWalk::Empty => None,
            MajorWalk::Linear(grid) => grid.next().map(|(_, value)| value),
            MajorWalk::Log { decades, left } => loop {
                if *left == 0 {
                    return None;
                }
                *left -= 1;
                let (_, value) = decades.next()?;
                if value.is_finite() && value > 0.0 {
                    return Some(value);
                }
            },
            MajorWalk::LogMultiples(multiples) => multiples.next(),
        }
    }
}

/// The major tick values an axis draws, in increasing order.
///
/// Categorical ([`AxisOptions::categories`]): every category position
/// `0, 1, …, n - 1`, whatever the scale and spacing. Linear: absolute multiples of [`effective_major_spacing`] inside the
/// range, at most [`MAX_MAJOR_INTERVALS`] + 1, none on a range that is not
/// [`walkable_range`]. Logarithmic: the [`log_ladder`] inside the
/// [`crate::layout::guarded_log_range`] of the axis range, over at most
/// [`MAX_LOG_DECADES`] decades: every `step`-th whole decade for
/// [`LogLadder::Decades`] (the [`effective_log_decade_step`] under the
/// default policy), the ladder's multiples of every decade otherwise.
pub fn major_ticks(axis: &AxisOptions) -> MajorTicks {
    if axis.categories.is_some() {
        let (min, max) = axis.data_range();
        return MajorTicks(
            LinearGrid::new(min, max, 1.0, MAX_MAJOR_INTERVALS)
                .map_or(MajorWalk::Empty, |grid| MajorWalk::Linear(grid.iter())),
        );
    }
    MajorTicks(match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                MajorWalk::Empty
            } else {
                LinearGrid::new(
                    axis.min,
                    axis.max,
                    effective_major_spacing(axis),
                    MAX_MAJOR_INTERVALS,
                )
                .map_or(MajorWalk::Empty, |grid| MajorWalk::Linear(grid.iter()))
            }
        }
        AxisScale::Logarithmic => {
            let (min, max) = guarded_log_range(axis.min, axis.max);
            log_major_walk(log_ladder(axis), min, max)
        }
    })
}

#[derive(Debug, Clone)]
enum MinorWalk {
    Empty,
    Linear {
        grid: LinearGridIter,
        subdivisions: f64,
    },
    Log(LogMinors),
    LogMultiples(LogMultiples),
}

/// The minor tick values of an axis, from [`minor_ticks`].
#[derive(Debug, Clone)]
pub struct MinorTicks(MinorWalk);

impl Iterator for MinorTicks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        match &mut self.0 {
            MinorWalk::Empty => None,
            MinorWalk::Linear { grid, subdivisions } => loop {
                let (index, value) = grid.next()?;
                // Multiples of the major spacing are the majors themselves.
                if index.rem_euclid(*subdivisions) != 0.0 {
                    return Some(value);
                }
            },
            MinorWalk::Log(minors) => minors.next(),
            MinorWalk::LogMultiples(minors) => minors.next(),
        }
    }
}

/// The minor tick values an axis draws, in increasing order, on the same
/// absolute grid the majors use, including the partial intervals before the
/// first and after the last major.
///
/// None when `minor_count` is 0 and on a categorical axis. Linear: [`drawn_minor_count`] subdivisions
/// of each [`effective_major_spacing`] interval, the majors left out.
/// Logarithmic, whatever the count: the multiples 2 to 9 of every decade that
/// are not [`log_ladder`] majors, over the
/// [`crate::layout::guarded_log_range`] of the axis range: [`LogMinors`] for
/// [`LogLadder::Decades`], 3, 4, 6, 7, 8 and 9 × 10^k for
/// [`LogLadder::OneTwoFive`], none for [`LogLadder::AllMantissas`].
pub fn minor_ticks(axis: &AxisOptions) -> MinorTicks {
    if axis.minor_count == 0 || axis.categories.is_some() {
        return MinorTicks(MinorWalk::Empty);
    }
    MinorTicks(match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                MinorWalk::Empty
            } else {
                let subdivisions = drawn_minor_count(axis) + 1;
                let step = effective_major_spacing(axis) / subdivisions as f64;
                let max_intervals = MAX_MAJOR_INTERVALS
                    .saturating_mul(subdivisions)
                    .saturating_add(1);
                LinearGrid::new(axis.min, axis.max, step, max_intervals).map_or(
                    MinorWalk::Empty,
                    |grid| MinorWalk::Linear {
                        grid: grid.iter(),
                        subdivisions: subdivisions as f64,
                    },
                )
            }
        }
        AxisScale::Logarithmic => {
            let (min, max) = guarded_log_range(axis.min, axis.max);
            match log_ladder(axis) {
                LogLadder::Decades(_) => MinorWalk::Log(LogMinors::new(min, max)),
                ladder => match LOG_MINOR_MULTIPLES & !ladder.multiples() {
                    0 => MinorWalk::Empty,
                    mask => MinorWalk::LogMultiples(LogMultiples::new(min, max, mask)),
                },
            }
        }
    })
}

// Invariant tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

    // plan.min <= data_min, plan.max >= data_max.
    #[test]
    fn linear_plan_contains_range() {
        let p = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.3, 9.7, 6).unwrap();
        assert!(p.min <= 0.3);
        assert!(p.max >= 9.7);
    }

    // plan.max > plan.min, plan.major_spacing > 0.
    #[test]
    fn linear_plan_non_degenerate() {
        let p = AxisOptions::compute_nice_ticks(AxisScale::Linear, -50.0, 50.0, 5).unwrap();
        assert!(p.max > p.min);
        assert!(p.major_spacing > 0.0);
    }

    #[test]
    fn log_plan_contains_range() {
        let p = AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, 0.003, 250.0, 5).unwrap();
        assert!(p.min <= 0.003);
        assert!(p.max >= 250.0);
        assert!(p.max > p.min);
        assert!(p.major_spacing > 0.0);
    }

    #[test]
    fn log_rejects_non_positive() {
        let e = AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, 0.0, 100.0, 5).unwrap_err();
        assert_eq!(e, TickError::NonPositiveLog);
    }

    #[test]
    fn rejects_invalid_range() {
        let e = AxisOptions::compute_nice_ticks(AxisScale::Linear, 5.0, 5.0, 5).unwrap_err();
        assert_eq!(e, TickError::InvalidRange);
    }

    #[test]
    fn rejects_every_non_finite_bound_and_reversed_ranges() {
        for (data_min, data_max) in [
            (f64::NAN, 1.0),
            (1.0, f64::NAN),
            (f64::NEG_INFINITY, 1.0),
            (1.0, f64::NEG_INFINITY),
            (f64::INFINITY, 1.0),
            (1.0, f64::INFINITY),
            (2.0, 1.0),
        ] {
            assert_eq!(
                AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, data_min, data_max, 5),
                Err(TickError::InvalidRange),
                "range ({data_min:?}, {data_max:?})"
            );
        }
    }

    #[test]
    fn target_count_then_range_then_log_domain_define_error_priority() {
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 10.0, 0),
            Err(TickError::ZeroTargetCount)
        );
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, f64::NAN, 0.0, 0),
            Err(TickError::ZeroTargetCount)
        );
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, -1.0, -2.0, 5),
            Err(TickError::InvalidRange)
        );
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Logarithmic, -1.0, 2.0, 5),
            Err(TickError::NonPositiveLog)
        );
    }

    #[test]
    fn extreme_finite_ranges_produce_finite_ordered_plans_or_fail_cleanly() {
        for (scale, data_min, data_max) in [
            (AxisScale::Linear, 0.0, f64::from_bits(1)),
            (AxisScale::Linear, 0.0, f64::MAX),
            (AxisScale::Logarithmic, f64::from_bits(1), f64::MAX),
        ] {
            let plan = AxisOptions::compute_nice_ticks(scale, data_min, data_max, 5).unwrap();
            assert!(plan.min.is_finite());
            assert!(plan.max.is_finite());
            assert!(plan.major_spacing.is_finite());
            assert!(plan.min < plan.max);
            assert!(plan.min <= data_min);
            assert!(plan.max >= data_max);
        }

        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Linear, -f64::MAX, f64::MAX, 5),
            Err(TickError::InvalidRange)
        );
    }

    // auto_ticks Err must leave self unchanged.
    #[test]
    fn auto_ticks_err_preserves_state() {
        let mut cfg = default_config();
        let before = cfg.top_x.clone();
        let r = cfg.top_x.auto_ticks(-f64::MAX, f64::MAX, 5);
        assert!(r.is_err());
        assert_eq!(cfg.top_x, before);
    }

    // auto_ticks_all Err must leave all four axes unchanged.
    #[test]
    fn auto_ticks_all_err_preserves_all() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.auto_ticks_all((0.0, 10.0), (0.0, 10.0), (f64::NAN, 5.0), (0.0, 10.0), 5);
        assert!(r.is_err());
        assert_eq!(cfg, before);
    }

    /// The Cartesian spacing rule, from the examples the axis docs give and
    /// the spans on the 1·2·5 class boundaries, where the closed and
    /// half-open first stages differ.
    #[test]
    fn axis_major_spacing_golden_linear() {
        let linear = |span: f64| axis_major_spacing(&AxisScale::Linear, 0.0, span);
        for (span, step) in [
            (72.0, 10.0),
            (8.0, 1.0),
            (0.7, 0.1),
            (10.0, 2.0),
            (100.0, 20.0),
            (1.0, 0.2),
            (2.0, 0.5),
            (20.0, 5.0),
            (3.0, 0.5),
            (25.0, 5.0),
            (50.0, 10.0),
            (999.999_999_999_999_9, 200.0),
            (f64::MAX, 2e307),
            (1.05e308, 2e307),
            (f64::from_bits(1), f64::from_bits(1)),
        ] {
            assert_eq!(linear(span), step, "span {span:?}");
        }
        for span in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            assert_eq!(linear(span), 1.0, "span {span:?}");
        }
        assert_eq!(axis_major_spacing(&AxisScale::Linear, 5.0, 1.0), 1.0);
    }

    #[test]
    fn axis_major_spacing_golden_log() {
        for decades in 1..=13 {
            let step = axis_major_spacing(&AxisScale::Logarithmic, 1.0, 10f64.powi(decades));
            let expected = match decades {
                1..=6 => 1.0,
                7..=12 => 2.0,
                _ => 3.0,
            };
            assert_eq!(step, expected, "{decades} decades");
            assert_eq!(
                nice_log_step(f64::from(decades), AXIS_LOG_TARGET),
                Some(expected)
            );
        }
        // Inside one decade the range still touches one whole decade.
        assert_eq!(axis_major_spacing(&AxisScale::Logarithmic, 2.0, 3.0), 1.0);
        for (min, max) in [
            (0.0, 10.0),
            (-1.0, 10.0),
            (10.0, 1.0),
            (f64::NAN, 10.0),
            (1.0, f64::INFINITY),
        ] {
            assert_eq!(
                axis_major_spacing(&AxisScale::Logarithmic, min, max),
                1.0,
                "{min:?}..{max:?}"
            );
        }
        assert_eq!(nice_log_step(0.0, AXIS_LOG_TARGET), None);
        assert_eq!(nice_log_step(f64::NAN, AXIS_LOG_TARGET), None);
    }

    #[test]
    fn nice_linear_step_rejects_spans_it_cannot_measure() {
        for span in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
            assert_eq!(nice_linear_step(span, 5, LooseRange::AtLeast), None);
            assert_eq!(nice_linear_step(span, 5, LooseRange::Above), None);
        }
    }

    /// `compute_nice_ticks` output recorded from the implementation before
    /// it was rebuilt on [`nice_linear_step`] and [`nice_log_step`]; the
    /// rebuild is bit-identical on every one of these.
    #[test]
    fn compute_nice_ticks_golden() {
        let plan = |min: f64, max: f64, major_spacing: f64, minor_count: usize| {
            Ok(TickPlan {
                min,
                max,
                major_spacing,
                minor_count,
            })
        };
        let linear = AxisScale::Linear;
        let log = AxisScale::Logarithmic;
        for (scale, data_min, data_max, target, expected) in [
            (linear.clone(), 0.0, 1.0, 5, plan(0.0, 1.0, 0.2, 4)),
            (linear.clone(), 0.0, 10.0, 5, plan(0.0, 10.0, 2.0, 4)),
            (linear.clone(), 0.0, 100.0, 6, plan(0.0, 100.0, 20.0, 4)),
            (linear.clone(), 0.3, 9.7, 6, plan(0.0, 10.0, 2.0, 4)),
            (linear.clone(), -50.0, 50.0, 5, plan(-60.0, 60.0, 20.0, 4)),
            (linear.clone(), 0.137, 9.42, 5, plan(0.0, 10.0, 2.0, 4)),
            (linear.clone(), -3.7, 12.9, 11, plan(-4.0, 14.0, 2.0, 4)),
            (
                linear.clone(),
                1e-9,
                3e-9,
                5,
                plan(1e-9, 3.0000000000000004e-9, 5e-10, 4),
            ),
            (
                linear.clone(),
                1e9,
                1.000_001e9,
                5,
                plan(1e9, 1.000_001e9, 200.0, 4),
            ),
            (linear.clone(), 0.0, 72.0, 8, plan(0.0, 80.0, 10.0, 4)),
            (linear.clone(), 0.0, 0.7, 3, plan(0.0, 1.0, 0.5, 4)),
            (linear.clone(), 0.0, 1025.0, 2001, plan(0.0, 1025.0, 1.0, 4)),
            (linear.clone(), 0.0, 1.0, 1, plan(0.0, 1.0, 1.0, 4)),
            (linear.clone(), 0.0, 1.0, 2, plan(0.0, 1.0, 1.0, 4)),
            (
                linear.clone(),
                -1e-300,
                1e-300,
                5,
                plan(-1e-300, 1e-300, 5e-301, 4),
            ),
            (
                linear.clone(),
                0.0,
                f64::from_bits(1),
                5,
                plan(0.0, f64::from_bits(1), f64::from_bits(1), 4),
            ),
            (
                linear.clone(),
                0.0,
                f64::MAX,
                5,
                plan(0.0, f64::MAX, 5e307, 4),
            ),
            (
                linear.clone(),
                0.0,
                1.5e307,
                5,
                plan(0.0, 1.5e307, 5e306, 4),
            ),
            (
                linear.clone(),
                0.0,
                999.999_999_999_999_9,
                5,
                plan(0.0, 1000.0, 200.0, 4),
            ),
            (linear.clone(), 12.5, 12.75, 5, plan(12.5, 12.8, 0.1, 4)),
            (
                linear.clone(),
                -8e307,
                8e307,
                5,
                plan(-1e308, 1e308, 5e307, 4),
            ),
            (linear.clone(), 0.0, 20.0, 10, plan(0.0, 20.0, 2.0, 4)),
            (linear.clone(), 0.0, 3.0, 10, plan(0.0, 3.0, 0.5, 4)),
            (log.clone(), 0.003, 250.0, 5, plan(0.001, 1000.0, 2.0, 8)),
            (log.clone(), 1.0, 10_000.0, 5, plan(1.0, 10_000.0, 1.0, 8)),
            (log.clone(), 1.0, 10_000.0, 1, plan(1.0, 10_000.0, 4.0, 8)),
            (log.clone(), 1e-30, 1e30, 7, plan(1e-30, 1e30, 10.0, 8)),
            (log.clone(), 2.0, 3.0, 5, plan(1.0, 10.0, 1.0, 8)),
            (log.clone(), 1e-5, 1e5, 7, plan(1e-5, 1e5, 2.0, 8)),
            (log.clone(), 1e-7, 1e6, 7, plan(1e-7, 1e6, 3.0, 8)),
            (
                log.clone(),
                f64::from_bits(1),
                f64::MAX,
                5,
                plan(f64::from_bits(1), f64::MAX, 159.0, 8),
            ),
        ] {
            assert_eq!(
                AxisOptions::compute_nice_ticks(scale.clone(), data_min, data_max, target),
                expected,
                "{scale:?} {data_min:?}..{data_max:?} target {target}"
            );
        }
    }

    /// Spans above about 1e308 used to lose their class when the loose range
    /// overflowed (a 1.05e308 span at target 5 stepped 2e307 and at target
    /// 10 stepped 1e307). The step is now the one the rule gives below the
    /// overflow.
    #[test]
    fn spans_near_f64_max_keep_their_step_class() {
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 1.05e308, 5),
            Ok(TickPlan {
                min: 0.0,
                max: 1.5e308,
                major_spacing: 5e307,
                minor_count: 4,
            })
        );
        assert_eq!(
            AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 1.05e308, 10),
            Ok(TickPlan {
                min: 0.0,
                max: 1.2e308,
                major_spacing: 2e307,
                minor_count: 4,
            })
        );
        assert_eq!(
            nice_linear_step(1.05e308, AXIS_LINEAR_TARGET, LooseRange::Above),
            Some(2e307)
        );
    }

    fn axis(
        scale: AxisScale,
        min: f64,
        max: f64,
        major_spacing: f64,
        minor_count: usize,
    ) -> AxisOptions {
        let mut axis = default_config().bottom_x;
        axis.scale = scale;
        axis.min = min;
        axis.max = max;
        axis.major_spacing = major_spacing;
        axis.minor_count = minor_count;
        axis
    }

    /// The walk is anchored to absolute multiples, and the slack keeps an end
    /// that floating-point noise puts just short of a multiple.
    #[test]
    fn linear_grid_walks_absolute_multiples_with_slack() {
        let grid = LinearGrid::new(0.0, 0.3, 0.1, 100).unwrap();
        assert_eq!((grid.first_index(), grid.last_index()), (0.0, 3.0));
        assert_eq!(grid.step(), 0.1);
        let indices: Vec<f64> = grid.iter().map(|(index, _)| index).collect();
        assert_eq!(indices, [0.0, 1.0, 2.0, 3.0]);

        let values: Vec<f64> = LinearGrid::new(0.137, 9.42, 2.0, 100)
            .unwrap()
            .iter()
            .map(|(_, value)| value)
            .collect();
        assert_eq!(values, [2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn linear_grid_refuses_what_it_cannot_walk_and_caps_the_rest() {
        for (min, max, step) in [
            (1.0, 1.0, 0.1),
            (2.0, 1.0, 0.1),
            (f64::NAN, 1.0, 0.1),
            (0.0, f64::INFINITY, 0.1),
            (0.0, 1.0, 0.0),
            (0.0, 1.0, -1.0),
            (0.0, 1.0, f64::NAN),
            (1.0e16, 1.0e16 + 2.0, 0.1),
            (0.25, 0.75, 1.0),
        ] {
            assert_eq!(
                LinearGrid::new(min, max, step, 100),
                None,
                "{min:?}..{max:?} by {step:?}"
            );
        }
        assert_eq!(
            LinearGrid::new(0.0, 1.0, 1e-6, 10).unwrap().iter().count(),
            11
        );
        assert_eq!(
            LinearGrid::new(0.0, 1.0, 1e-6, 0).unwrap().iter().count(),
            1
        );
    }

    #[test]
    fn log_decades_step_and_stop_at_the_last_exponent() {
        let decades: Vec<(i32, f64)> = LogDecades::new(-2, 3, 2).unwrap().collect();
        assert_eq!(decades, [(-2, 0.01), (0, 1.0), (2, 100.0)]);
        assert_eq!(LogDecades::new(3, 2, 1).unwrap().count(), 0);
        assert_eq!(LogDecades::new(0, 1, 0), None);
        assert_eq!(
            LogDecades::new(i32::MAX - 1, i32::MAX, i32::MAX)
                .unwrap()
                .count(),
            1
        );
        // `powf` keeps the subnormal decades.
        assert_eq!(
            LogDecades::new(-323, -323, 1).unwrap().next(),
            Some((-323, 1e-323))
        );
    }

    #[test]
    fn log_minors_cover_partial_edge_decades() {
        let minors: Vec<f64> = LogMinors::new(3.0, 50.0).collect();
        assert_eq!(
            minors,
            [3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 20.0, 30.0, 40.0, 50.0]
        );
    }

    /// The walk `LogMinors` took before it was built on `LogMultiples`:
    /// multiples 2 to 9 of every decade, the same guards.
    fn legacy_log_minors(min: f64, max: f64) -> Vec<f64> {
        let mut out = Vec::new();
        let first = clamped_exponent(min.log10().floor());
        let last = clamped_exponent(max.log10().floor());
        let mut decade = Some(first);
        let mut decades_left = MAX_LOG_DECADES;
        while let Some(d) = decade.filter(|d| *d <= last) {
            if decades_left == 0 {
                break;
            }
            decades_left -= 1;
            decade = d.checked_add(1);
            let base = 10f64.powf(f64::from(d));
            for multiple in 2..=9u8 {
                let value = base * f64::from(multiple);
                if value.is_finite() && value >= min && value <= max {
                    out.push(value);
                }
            }
        }
        out
    }

    /// `LogMinors` is `LogMultiples` of the multiples 2 to 9, bit for bit,
    /// on ordinary, partial, subnormal, overflowing and capped ranges.
    #[test]
    fn log_minors_are_the_multiples_two_to_nine() {
        for (min, max) in [
            (3.0, 50.0),
            (0.5, 2000.0),
            (1e-9, 1e9),
            (2.1e4, 2.9e4),
            (8e3, 1.2e4),
            (1e-12, 1e-11),
            (f64::from_bits(1), 1e-300),
            (1e300, f64::MAX),
            (f64::from_bits(1), f64::MAX),
        ] {
            let minors: Vec<u64> = LogMinors::new(min, max).map(f64::to_bits).collect();
            let multiples: Vec<u64> = LogMultiples::new(min, max, LOG_MINOR_MULTIPLES)
                .map(f64::to_bits)
                .collect();
            let legacy: Vec<u64> = legacy_log_minors(min, max)
                .into_iter()
                .map(f64::to_bits)
                .collect();
            assert_eq!(minors, multiples, "{min:?}..{max:?}");
            assert_eq!(minors, legacy, "{min:?}..{max:?}");
        }
    }

    /// A mask selects its multiples in every decade, and a multiple is the
    /// same value under every mask that walks it.
    #[test]
    fn log_multiples_walk_the_masked_multiples() {
        let one_two_five = (1 << 1) | (1 << 2) | (1 << 5);
        let values: Vec<f64> = LogMultiples::new(1.0, 100.0, one_two_five).collect();
        assert_eq!(values, [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]);
        let values: Vec<f64> = LogMultiples::new(3e3, 8e5, one_two_five).collect();
        assert_eq!(values, [5e3, 1e4, 2e4, 5e4, 1e5, 2e5, 5e5]);
        // Bit 0 and the bits past 9 select nothing.
        assert_eq!(LogMultiples::new(1.0, 1e3, 0xFC01).count(), 0);
        let all: Vec<u64> = LogMultiples::new(1e-5, 1e5, 0b11_1111_1110)
            .map(f64::to_bits)
            .collect();
        for value in LogMultiples::new(1e-5, 1e5, one_two_five) {
            assert!(all.contains(&value.to_bits()), "{value:?}");
        }
        for value in LogMinors::new(1e-5, 1e5) {
            assert!(all.contains(&value.to_bits()), "{value:?}");
        }
    }

    #[test]
    fn axis_walkers_follow_the_guards() {
        let majors: Vec<f64> = major_ticks(&axis(AxisScale::Linear, 0.0, 10.0, 2.5, 4)).collect();
        assert_eq!(majors, [0.0, 2.5, 5.0, 7.5, 10.0]);
        let minors: Vec<f64> = minor_ticks(&axis(AxisScale::Linear, 0.0, 1.0, 0.5, 1)).collect();
        assert_eq!(minors, [0.25, 0.75]);
        // A rejected spacing walks the automatic one.
        let fallback = axis(AxisScale::Linear, 0.0, 10.0, 0.0, 4);
        assert_eq!(
            major_spacing_rejection(&fallback),
            Some(SpacingRejection::NotPositive)
        );
        assert_eq!(effective_major_spacing(&fallback), 2.0);
        assert_eq!(major_ticks(&fallback).count(), 6);
        assert_eq!(
            major_spacing_rejection(&axis(AxisScale::Linear, 0.0, 10.0, 1e-9, 4)),
            Some(SpacingRejection::TooManyTicks {
                limit: MAX_MAJOR_INTERVALS
            })
        );
        assert_eq!(
            drawn_minor_count(&axis(AxisScale::Linear, 0.0, 1.0, 1.0, usize::MAX)),
            99
        );
        assert_eq!(
            major_ticks(&axis(AxisScale::Linear, 1.0, 1.0, 1.0, 4)).count(),
            0
        );
        assert_eq!(
            minor_ticks(&axis(AxisScale::Linear, 0.0, 1.0, 0.5, 0)).count(),
            0
        );

        let log = axis(AxisScale::Logarithmic, 0.5, 2000.0, 2.7, 8);
        assert_eq!(effective_log_decade_step(&log), 2);
        assert_eq!(log_ladder(&log), LogLadder::Decades(2));
        assert_eq!(major_ticks(&log).collect::<Vec<_>>(), [1.0, 100.0]);
        // 0.5..0.9, three whole decades, then 2000.
        assert_eq!(minor_ticks(&log).count(), 5 + 3 * 8 + 1);
        // A written range that is not a log range walks the guarded one.
        let unguarded = axis(AxisScale::Logarithmic, 0.0, f64::NAN, 1.0, 8);
        assert_eq!(major_ticks(&unguarded).collect::<Vec<_>>(), [1e-12, 1e-11]);
        assert_eq!(minor_ticks(&unguarded).count(), 8);
        // The same axes under `Adaptive`: the range decides, not the spacing.
        let mut adaptive = log.clone();
        adaptive.log_ticks = LogTickPolicy::Adaptive;
        assert_eq!(log_ladder(&adaptive), LogLadder::Decades(1));
        assert_eq!(
            major_ticks(&adaptive).collect::<Vec<_>>(),
            [1.0, 10.0, 100.0, 1000.0]
        );
        assert_eq!(minor_ticks(&adaptive).count(), 5 + 3 * 8 + 1);
    }

    fn adaptive(min: f64, max: f64) -> AxisOptions {
        let mut axis = axis(AxisScale::Logarithmic, min, max, 1.0, 8);
        axis.log_ticks = LogTickPolicy::Adaptive;
        axis
    }

    /// The ladders, majors and minors of the plan's table (#42) under
    /// `Adaptive`, with `LOG_LADDER_MIN_MAJORS = 4`.
    #[test]
    fn adaptive_log_ladders_golden() {
        assert_eq!(LOG_LADDER_MIN_MAJORS, 4);
        let decades = |min: f64, max: f64, step: f64| {
            let axis = axis(AxisScale::Logarithmic, min, max, step, 8);
            (
                major_ticks(&axis).collect::<Vec<_>>(),
                minor_ticks(&axis).collect::<Vec<_>>(),
            )
        };
        type Golden = (f64, f64, LogLadder, Vec<f64>, Vec<f64>);
        let table: [Golden; 9] = [
            (
                1e-9,
                1e9,
                LogLadder::Decades(3),
                decades(1e-9, 1e9, 3.0).0,
                decades(1e-9, 1e9, 3.0).1,
            ),
            (
                0.5,
                2000.0,
                LogLadder::Decades(1),
                vec![1.0, 10.0, 100.0, 1000.0],
                decades(0.5, 2000.0, 1.0).1,
            ),
            (
                1.0,
                1e3,
                LogLadder::Decades(1),
                vec![1.0, 10.0, 100.0, 1000.0],
                decades(1.0, 1e3, 1.0).1,
            ),
            (
                1.0,
                1e6,
                LogLadder::Decades(1),
                vec![1.0, 10.0, 100.0, 1e3, 1e4, 1e5, 1e6],
                decades(1.0, 1e6, 1.0).1,
            ),
            (
                1.0,
                100.0,
                LogLadder::OneTwoFive,
                vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0],
                vec![
                    3.0, 4.0, 6.0, 7.0, 8.0, 9.0, 30.0, 40.0, 60.0, 70.0, 80.0, 90.0,
                ],
            ),
            (
                3e3,
                8e5,
                LogLadder::OneTwoFive,
                vec![5e3, 1e4, 2e4, 5e4, 1e5, 2e5, 5e5],
                vec![
                    3e3, 4e3, 6e3, 7e3, 8e3, 9e3, 3e4, 4e4, 6e4, 7e4, 8e4, 9e4, 3e5, 4e5, 6e5, 7e5,
                    8e5,
                ],
            ),
            (
                1e4,
                8e4,
                LogLadder::AllMantissas,
                vec![1e4, 2e4, 3e4, 4e4, 5e4, 6e4, 7e4, 8e4],
                vec![],
            ),
            (
                8e3,
                1.2e4,
                LogLadder::AllMantissas,
                vec![8e3, 9e3, 1e4],
                vec![],
            ),
            (2.1e4, 2.9e4, LogLadder::AllMantissas, vec![], vec![]),
        ];
        for (min, max, ladder, majors, minors) in table {
            let axis = adaptive(min, max);
            assert_eq!(log_ladder(&axis), ladder, "{min:?}..{max:?}");
            assert_eq!(
                major_ticks(&axis).collect::<Vec<_>>(),
                majors,
                "{min:?}..{max:?} majors"
            );
            assert_eq!(
                minor_ticks(&axis).collect::<Vec<_>>(),
                minors,
                "{min:?}..{max:?} minors"
            );
        }
        // 2.1e4..2.9e4 draws no tick under either policy.
        assert_eq!(decades(2.1e4, 2.9e4, 1.0), (vec![], vec![]));
        // The 1-2-5 ladder would leave 1e4..8e4 three majors, one short.
        assert_eq!(
            LogMultiples::new(1e4, 8e4, LOG_ONE_TWO_FIVE).collect::<Vec<_>>(),
            [1e4, 2e4, 5e4]
        );
    }

    /// Guarded and extreme ranges: a written range that is not a log range
    /// is planned on its guarded one, and the ladders stay bounded at the
    /// ends of `f64`.
    #[test]
    fn adaptive_log_ladders_on_guarded_and_extreme_ranges() {
        // The guarded fallback decade 1e-12..1e-11 holds two decades.
        for (min, max) in [(0.0, f64::NAN), (-1.0, -2.0), (f64::NAN, f64::INFINITY)] {
            let axis = adaptive(min, max);
            let (gmin, gmax) = guarded_log_range(min, max);
            assert_eq!(log_ladder(&axis), LogLadder::OneTwoFive, "{min:?}..{max:?}");
            let majors: Vec<f64> = major_ticks(&axis).collect();
            assert_eq!(
                majors,
                LogMultiples::new(gmin, gmax, LOG_ONE_TWO_FIVE).collect::<Vec<_>>()
            );
            assert!(majors.len() >= LOG_LADDER_MIN_MAJORS, "{majors:?}");
        }
        for (min, max, ladder) in [
            (f64::from_bits(1), f64::MAX, LogLadder::Decades(106)),
            (1e300, f64::MAX, LogLadder::Decades(2)),
            (1e307, f64::MAX, LogLadder::OneTwoFive),
            (1e-320, 1e-300, LogLadder::Decades(4)),
        ] {
            let axis = adaptive(min, max);
            assert_eq!(log_ladder(&axis), ladder, "{min:?}..{max:?}");
            let majors: Vec<f64> = major_ticks(&axis).collect();
            assert!(!majors.is_empty(), "{min:?}..{max:?}");
            assert!(majors.iter().all(|v| v.is_finite() && *v > 0.0));
        }
        // 2e308 and above overflow and are not drawn.
        let top: Vec<f64> = major_ticks(&adaptive(1e307, f64::MAX)).collect();
        assert_eq!(top, [1e307, 2e307, 5e307, 1e308]);
    }

    fn sorted_bits(values: impl Iterator<Item = f64>) -> Vec<u64> {
        let mut out: Vec<u64> = values.map(f64::to_bits).collect();
        out.sort_unstable();
        out
    }

    /// A #72-style sweep over ranges inside one decade, across powers of
    /// ten and over many decades: wherever the adaptive ladder is
    /// `Decades(n)` it draws exactly the majors and minors of the default
    /// policy at `major_spacing = n`; every plan is sorted, disjoint and
    /// inside the range; a promoted 2·10^k or 5·10^k major is bit for bit
    /// the minor the decade ladder draws there; and a sparser ladder is
    /// kept only with enough majors.
    #[test]
    fn adaptive_log_ladders_sweep() {
        let mut decade_rows = 0;
        let mut multiple_rows = 0;
        for exponent in -15..=15 {
            for lead in [1.0, 1.5, 2.0, 3.7, 8.0, 9.99] {
                for span in [
                    0.01, 0.05, 0.2, 0.3, 0.6, 0.9, 1.0, 1.3, 2.0, 2.2, 3.0, 4.5, 5.9, 6.0, 6.5,
                    8.0, 12.4, 20.0, 60.0,
                ] {
                    let min = lead * 10f64.powi(exponent);
                    let max = min * 10f64.powf(span);
                    let axis = adaptive(min, max);
                    let ladder = log_ladder(&axis);
                    let majors: Vec<f64> = major_ticks(&axis).collect();
                    let minors: Vec<f64> = minor_ticks(&axis).collect();
                    let label = format!("{min:e}..{max:e} {ladder:?}");
                    for values in [&majors, &minors] {
                        assert!(values.windows(2).all(|w| w[0] < w[1]), "{label}");
                        assert!(values.len() <= MAX_LOG_DECADES * 9, "{label}");
                    }
                    // The multiples are range-checked by value; the decade
                    // walk takes its exponents from `log10` of the ends,
                    // which may land on a power of ten one ulp outside.
                    assert!(minors.iter().all(|v| *v >= min && *v <= max), "{label}");
                    let slack = match ladder {
                        LogLadder::Decades(_) => 1e-12,
                        _ => 0.0,
                    };
                    assert!(
                        majors
                            .iter()
                            .all(|v| *v >= min * (1.0 - slack) && *v <= max * (1.0 + slack)),
                        "{label}"
                    );
                    let major_bits = sorted_bits(majors.iter().copied());
                    assert!(
                        minors
                            .iter()
                            .all(|v| major_bits.binary_search(&v.to_bits()).is_err()),
                        "{label}: majors and minors overlap"
                    );
                    match ladder {
                        LogLadder::Decades(step) => {
                            decade_rows += 1;
                            let written =
                                self::axis(AxisScale::Logarithmic, min, max, f64::from(step), 8);
                            assert_eq!(log_ladder(&written), ladder, "{label}");
                            assert_eq!(
                                majors.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                major_ticks(&written).map(f64::to_bits).collect::<Vec<_>>(),
                                "{label}"
                            );
                            assert_eq!(
                                minors.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                minor_ticks(&written).map(f64::to_bits).collect::<Vec<_>>(),
                                "{label}"
                            );
                            if step == 1 {
                                assert!(majors.len() >= LOG_LADDER_MIN_MAJORS, "{label}");
                            }
                        }
                        LogLadder::OneTwoFive | LogLadder::AllMantissas => {
                            multiple_rows += 1;
                            // At most six decades: no wider range gets here.
                            assert_eq!(
                                axis_major_spacing(&AxisScale::Logarithmic, min, max),
                                1.0,
                                "{label}"
                            );
                            // A sparser ladder is kept only while the denser
                            // ones before it hold too few majors.
                            let count =
                                |ladder| MajorTicks(log_major_walk(ladder, min, max)).count();
                            assert!(count(LogLadder::Decades(1)) < LOG_LADDER_MIN_MAJORS);
                            if ladder == LogLadder::OneTwoFive {
                                assert!(majors.len() >= LOG_LADDER_MIN_MAJORS, "{label}");
                            } else {
                                assert!(count(LogLadder::OneTwoFive) < LOG_LADDER_MIN_MAJORS);
                            }
                            // Majors and minors are every multiple of every
                            // decade in the range, at the decade ladder's
                            // minor positions.
                            let mut both = majors.clone();
                            both.extend(&minors);
                            assert_eq!(
                                sorted_bits(both.into_iter()),
                                sorted_bits(LogMultiples::new(min, max, LOG_ALL_MANTISSAS)),
                                "{label}"
                            );
                            let decade_minors = sorted_bits(LogMinors::new(min, max));
                            for value in majors.iter().chain(&minors) {
                                let decade = 10f64.powf(value.log10().floor());
                                assert!(
                                    *value == decade
                                        || *value == 10f64.powf(value.log10().round())
                                        || decade_minors.binary_search(&value.to_bits()).is_ok(),
                                    "{label}: {value:e} is not where a decade ladder draws"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(
            decade_rows > 100 && multiple_rows > 100,
            "{decade_rows} / {multiple_rows}"
        );
    }

    /// The default policy is the walk every log axis took before the
    /// policy existed; `Adaptive` ignores `major_spacing`, and a minor
    /// count of 0 still blanks the minors.
    #[test]
    fn log_tick_policy_decades_is_the_written_decade_step() {
        for spacing in [0.5, 1.0, 2.7, 3.0, f64::NAN, f64::INFINITY, -4.0] {
            let log = axis(AxisScale::Logarithmic, 1e-9, 1e9, spacing, 8);
            assert_eq!(log.log_ticks, LogTickPolicy::Decades);
            assert_eq!(
                log_ladder(&log),
                LogLadder::Decades(effective_log_decade_step(&log))
            );
            let mut adaptive = log.clone();
            adaptive.log_ticks = LogTickPolicy::Adaptive;
            assert_eq!(log_ladder(&adaptive), LogLadder::Decades(3), "{spacing:?}");
        }
        let mut blank = adaptive(1.0, 100.0);
        blank.minor_count = 0;
        assert_eq!(minor_ticks(&blank).count(), 0);
        assert_eq!(major_ticks(&blank).count(), 7);
        // A linear or categorical axis ignores the policy.
        let mut linear = axis(AxisScale::Linear, 0.0, 10.0, 2.5, 4);
        let before: Vec<f64> = major_ticks(&linear).collect();
        linear.log_ticks = LogTickPolicy::Adaptive;
        assert_eq!(major_ticks(&linear).collect::<Vec<_>>(), before);
    }

    /// `log_multiple` names the multiple of every value the walkers write,
    /// over decades -307 to 307, and nothing else.
    #[test]
    fn log_multiple_names_every_walked_multiple_and_nothing_else() {
        for exponent in -307..=307 {
            let base = 10f64.powf(f64::from(exponent));
            for multiple in 1..=9u8 {
                let value = base * f64::from(multiple);
                assert_eq!(log_multiple(value), Some(multiple), "{multiple}e{exponent}");
            }
            for off in [base * 3.5, base * 1.25, base * 9.5] {
                assert_eq!(log_multiple(off), None, "{off:e}");
            }
        }
        for value in [
            3.5e4,
            0.0,
            -0.0,
            -1.0,
            -2e4,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::MAX,
            1.0 + f64::EPSILON,
        ] {
            assert_eq!(log_multiple(value), None, "{value:e}");
        }
        // Every major and minor an adaptive axis draws has its multiple.
        for (min, max) in [(1.0, 100.0), (3e3, 8e5), (1e4, 8e4), (1e-9, 1e9)] {
            let axis = adaptive(min, max);
            for value in major_ticks(&axis).chain(minor_ticks(&axis)) {
                assert!(log_multiple(value).is_some(), "{value:e}");
            }
        }
    }

    /// The label ladders start at the axis' own ladder and end at the
    /// decades; a decade ladder, the default policy and a linear axis have
    /// none.
    #[test]
    fn log_label_ladders_demote_to_the_decades() {
        assert_eq!(
            log_label_ladders(&adaptive(1e4, 8e4)),
            [LOG_ALL_MANTISSAS, LOG_ONE_TWO_FIVE, LOG_DECADE]
        );
        assert_eq!(
            log_label_ladders(&adaptive(3e3, 8e5)),
            [LOG_ONE_TWO_FIVE, LOG_DECADE]
        );
        assert!(log_label_ladders(&adaptive(1.0, 1e6)).is_empty());
        assert!(log_label_ladders(&adaptive(1e-9, 1e9)).is_empty());
        let mut decades = adaptive(1e4, 8e4);
        decades.log_ticks = LogTickPolicy::Decades;
        assert!(log_label_ladders(&decades).is_empty());
        let mut linear = adaptive(1e4, 8e4);
        linear.scale = AxisScale::Linear;
        assert!(log_label_ladders(&linear).is_empty());
        // Every ladder is a subset of the one before it.
        let ladders = log_label_ladders(&adaptive(1e4, 8e4));
        assert!(ladders.windows(2).all(|w| w[1] & !w[0] == 0));
    }

    // compute_nice_ticks is pure.
    #[test]
    fn compute_nice_ticks_pure() {
        let a = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        let b = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        assert_eq!(a, b);
    }
}
