//! Tick planning: the "nice" spacing rule every axis shares.
//!
//! [`nice_linear_step`] and [`nice_log_step`] are the one spacing rule. The
//! Cartesian axis helpers (`Chart::set_x_range` and the fits) call it through
//! [`axis_major_spacing`] with [`AXIS_LINEAR_TARGET`] and
//! [`AXIS_LOG_TARGET`]; [`AxisOptions::compute_nice_ticks`] calls it with the
//! caller's target and also widens the range to whole steps, which is what
//! the legacy document readers' fixed value axes and contour levels use.

use crate::config::{AxisOptions, AxisScale, AxisUnit, Config, LogTickPolicy};
use crate::layout::{guarded_log_range, guarded_probability_range};

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
    /// A probability axis range with an end outside `(0, 1)` (#209).
    OutsideProbability,
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
        // The probability walk takes its majors from the range (#209).
        AxisScale::Probability(_) => 1.0,
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

/// `p` widened to the next tail decade below it (#209): `10^⌊log₁₀ p⌋` in
/// the lower half, `1 − 10^⌈log₁₀(1 − p)⌉` in the upper one, and at most
/// back to 0.5. An exact decade (0.001, 0.999) stays.
fn probability_floor(p: f64) -> f64 {
    if p < 0.5 {
        let widened = 10f64.powf(p.log10().floor());
        return if widened > 0.0 && widened <= p {
            widened
        } else {
            p
        };
    }
    let tail = 10f64.powf((1.0 - p).log10().ceil());
    if tail >= 0.5 {
        0.5
    } else {
        (1.0 - tail).min(p)
    }
}

/// A probability axis plan (#209): the range widened outward to the next
/// tail decade on both ends (`0.003..0.997` becomes `0.001..0.999`), as a
/// log plan widens to whole decades. The majors come from the range, so the
/// spacing is 1 and the walk ignores it; one minor is a switch.
fn compute_probability(data_min: f64, data_max: f64) -> Result<TickPlan, TickError> {
    let nice_min = probability_floor(data_min);
    let nice_max = 1.0 - probability_floor(1.0 - data_max);
    let plan = checked_plan(nice_min, nice_max, 1.0, 1, data_min, data_max, false)?;
    if plan.min > 0.0 && plan.max < 1.0 {
        Ok(plan)
    } else {
        Err(TickError::InvalidRange)
    }
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
        if matches!(scale, AxisScale::Probability(_)) && !(data_min > 0.0 && data_max < 1.0) {
            return Err(TickError::OutsideProbability);
        }
        match scale {
            AxisScale::Linear => compute_linear(data_min, data_max, target_count),
            AxisScale::Logarithmic => compute_log(data_min, data_max, target_count),
            AxisScale::Probability(_) => compute_probability(data_min, data_max),
        }
    }

    /// Set the range to `data_min..data_max` widened to whole steps, with
    /// the spacing and minor count of [`Self::compute_nice_ticks`]. With a
    /// [`Self::unit`] (#273) the steps are planned in shown units and the
    /// widened range is mapped back to data units.
    pub fn auto_ticks(
        &mut self,
        data_min: f64,
        data_max: f64,
        target_count: usize,
    ) -> Result<(), TickError> {
        let plan = shown_plan(self, data_min, data_max, target_count)?;
        apply_plan(self, plan);
        Ok(())
    }
}

/// [`AxisOptions::compute_nice_ticks`] over the shown values of
/// `data_min..data_max` (#273), with the widened range mapped back to data
/// units and the spacing left in shown units, the units `major_spacing` is
/// read in. Without a unit, the plan of the data range itself.
fn shown_plan(
    axis: &AxisOptions,
    data_min: f64,
    data_max: f64,
    target_count: usize,
) -> Result<TickPlan, TickError> {
    let (min, max) = axis.shown_interval(data_min, data_max);
    let plan = AxisOptions::compute_nice_ticks(axis.scale.clone(), min, max, target_count)?;
    let (min, max) = axis.data_interval(plan.min, plan.max);
    Ok(TickPlan { min, max, ..plan })
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
        let plan_top = shown_plan(&self.top_x, top_x.0, top_x.1, target_count)?;
        let plan_bottom = shown_plan(&self.bottom_x, bottom_x.0, bottom_x.1, target_count)?;
        let plan_left = shown_plan(&self.left_y, left_y.0, left_y.1, target_count)?;
        let plan_right = shown_plan(&self.right_y, right_y.0, right_y.1, target_count)?;

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
///
/// The range is the one the ticks are planned over: the axis'
/// [`AxisOptions::shown_range`], which is `min..max` without a unit (#273).
pub fn walkable_range(axis: &AxisOptions) -> bool {
    let (min, max) = axis.shown_range();
    range_is_walkable(min, max)
}

/// The axis range measured in `spacing` units, or `None` when the range is
/// not walkable, the spacing is not finite and positive, or the count does
/// not fit an `f64`. Shown units: the [`AxisOptions::shown_range`] in a
/// spacing read in shown units (#273).
pub fn range_in_spacing_units(axis: &AxisOptions, spacing: f64) -> Option<f64> {
    if !walkable_range(axis) || !spacing.is_finite() || spacing <= 0.0 {
        return None;
    }
    let (min, max) = axis.shown_range();
    let span = max - min;
    let units = if span.is_finite() {
        span / spacing
    } else {
        // Divide before subtracting so `-MAX..MAX` can still be measured when
        // the requested spacing is large enough to make the walk bounded.
        max / spacing - min / spacing
    };
    (units.is_finite() && units >= 0.0).then_some(units)
}

/// Whether one `spacing` moves either end of the axis range at `f64`
/// precision, in shown units ([`AxisOptions::shown_range`]).
pub fn spacing_advances_range(axis: &AxisOptions, spacing: f64) -> bool {
    let (min, max) = axis.shown_range();
    spacing_advances(min, max, spacing)
}

/// The spacing the linear walkers fall back to: [`axis_major_spacing`] for
/// the current range, or the span itself when that does not advance the
/// range at `f64` precision. Shown units ([`AxisOptions::shown_range`]).
pub fn automatic_linear_spacing(axis: &AxisOptions) -> f64 {
    if !walkable_range(axis) {
        return 1.0;
    }
    let (min, max) = axis.shown_range();
    let span = max - min;
    let mut spacing = if span.is_finite() {
        axis_major_spacing(&AxisScale::Linear, min, max)
    } else {
        let scale = min.abs().max(max.abs());
        let normalized_span = max / scale - min / scale;
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

/// The major spacing the linear walkers actually walk, in shown units on an
/// axis with a [`AxisOptions::unit`] (#273). The SSoT accepts
/// whatever the host sends (a live spacing input passes through 0 mid-edit),
/// so the guard lives here, in the one place shared by tick marks, grid
/// lines, minors, and label decimals, which therefore can never disagree. A
/// non-finite or ≤ 0 spacing, or one that would emit an absurd tick count,
/// falls back to [`automatic_linear_spacing`] instead of blanking the axis.
/// Callers must have validated the range as finite and non-empty.
pub fn effective_major_spacing(axis: &AxisOptions) -> f64 {
    // Pieces with scales of their own (#205) format their labels at the
    // finest spacing any piece walks.
    if let Some((pieces, count)) = piece_ticks(axis) {
        return pieces[..count]
            .iter()
            .map(|p| p.spacing)
            .fold(f64::INFINITY, f64::min);
    }
    let spacing = match major_spacing_rejection(axis) {
        None => axis.major_spacing,
        Some(_) => automatic_linear_spacing(axis),
    };
    spacing_for_pieces(axis, spacing)
}

/// The spacing a broken linear axis (#200) walks: `spacing` stepped down
/// the 1, 2, 5 × 10^k ladder while some piece would hold fewer than two
/// majors, so a short piece is not left with one tick and one label. The
/// majors stay on absolute multiples, and crowded labels are still thinned
/// on the walk. An unbroken, logarithmic or categorical axis, and a step
/// the range could not walk, keep `spacing`.
fn spacing_for_pieces(axis: &AxisOptions, spacing: f64) -> f64 {
    if axis.scale != AxisScale::Linear || axis.categories.is_some() {
        return spacing;
    }
    let map = crate::layout::BreakMap::cuts(axis);
    if !map.is_broken() || !(spacing.is_finite() && spacing > 0.0) {
        return spacing;
    }
    let span = axis.max - axis.min;
    // The pieces are cut in data units and walked in shown ones (#273).
    // Majors on absolute multiples of `step` inside `lo..=hi`.
    let majors = |lo: f64, hi: f64, step: f64| {
        let tolerance = 1e-9;
        ((hi / step + tolerance).floor() - (lo / step - tolerance).ceil() + 1.0).max(0.0)
    };
    // A piece with no span (a cut that reaches past an end of the range,
    // after a zoom) holds no ticks and draws no length: it is not short.
    let short = |step: f64| {
        map.raw_pieces().any(|(a, b)| {
            let (p, q) = axis.shown_interval(axis.min + a * span, axis.min + b * span);
            (b - a).abs() > 1e-12 && majors(p.min(q), p.max(q), step) < 2.0
        })
    };
    let mut step = spacing;
    // Each rung divides the step by 2 or 2.5; the interval budget below
    // ends the walk long before the bound.
    for _ in 0..64 {
        if !short(step) {
            break;
        }
        let decade = 10f64.powf(step.log10().floor());
        let leading = step / decade;
        let next = if leading > 5.0 + 1e-9 {
            5.0 * decade
        } else if leading > 2.0 + 1e-9 {
            2.0 * decade
        } else if leading > 1.0 + 1e-9 {
            decade
        } else {
            0.5 * decade
        };
        if !(next.is_finite() && next > 0.0)
            || !range_in_spacing_units(axis, next)
                .is_some_and(|units| units <= MAX_MAJOR_INTERVALS as f64)
            || !spacing_advances_range(axis, next)
        {
            break;
        }
        step = next;
    }
    step
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
            // Decades of the shown value (#273).
            let (min, max) = axis.shown_range();
            let (min, max) = guarded_log_range(min, max);
            let step = axis_major_spacing(&AxisScale::Logarithmic, min, max);
            if step > 1.0 {
                return LogLadder::Decades(step.min(f64::from(MAX_LOG_DECADE_STEP)) as i32);
            }
            [LogLadder::Decades(1), LogLadder::OneTwoFive]
                .into_iter()
                .find(|ladder| {
                    MajorTicks(log_major_walk(*ladder, min, max), None)
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
        self.walk(false)
    }

    /// [`Self::iter`], or the same pairs from the last index down when
    /// `descending`: the walk of a shown range whose unit has a negative
    /// scale, so the data values still come in increasing order (#273).
    fn walk(&self, descending: bool) -> LinearGridIter {
        LinearGridIter {
            grid: *self,
            next_offset: Some(0),
            previous: None,
            descending,
        }
    }
}

/// The iterator of [`LinearGrid::iter`].
#[derive(Debug, Clone)]
pub struct LinearGridIter {
    grid: LinearGrid,
    next_offset: Option<usize>,
    previous: Option<f64>,
    descending: bool,
}

impl Iterator for LinearGridIter {
    type Item = (f64, f64);

    fn next(&mut self) -> Option<(f64, f64)> {
        while let Some(offset) = self.next_offset {
            self.next_offset = offset
                .checked_add(1)
                .filter(|next| *next <= self.grid.max_intervals);
            let index = if self.descending {
                self.grid.last - offset as f64
            } else {
                self.grid.first + offset as f64
            };
            if !index.is_finite() || index > self.grid.last || index < self.grid.first {
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
    if axis.categories.is_some() {
        return &[];
    }
    if let AxisScale::Probability(_) = axis.scale {
        // The tails' ladders (#209), demoted the same way; 0.5 is on all.
        return match probability_ladder(axis) {
            Some(LOG_ALL_MANTISSAS) => &[LOG_ALL_MANTISSAS, LOG_ONE_TWO_FIVE, LOG_DECADE],
            Some(LOG_ONE_TWO_FIVE) => &[LOG_ONE_TWO_FIVE, LOG_DECADE],
            _ => &[],
        };
    }
    if axis.scale != AxisScale::Logarithmic {
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

/// The multiple `m` (1 to 9) a tick label is demoted by in the label
/// pruning: [`log_multiple`] of the value on a logarithmic axis; on a
/// probability axis (#209) that of its tail `min(p, 1 − p)`, matched to
/// `m · 10^k` within 1e-9 of the tail because `1 − p` of an upper tick is
/// not the walked tail exactly, and 1 for 0.5, which every ladder keeps.
/// `None` on a linear axis.
pub fn ladder_multiple(value: f64, scale: &AxisScale) -> Option<u8> {
    match scale {
        AxisScale::Linear => None,
        AxisScale::Logarithmic => log_multiple(value),
        AxisScale::Probability(_) => {
            if !(value.is_finite() && value > 0.0 && value < 1.0) {
                return None;
            }
            if value == 0.5 {
                return Some(1);
            }
            let tail = if value < 0.5 { value } else { 1.0 - value };
            let exponent = clamped_exponent(tail.log10().floor());
            [
                exponent,
                exponent.saturating_sub(1),
                exponent.saturating_add(1),
            ]
            .into_iter()
            .find_map(|exponent| {
                let base = 10f64.powf(f64::from(exponent));
                let multiple = (tail / base).round();
                ((1.0..=9.0).contains(&multiple) && (base * multiple - tail).abs() <= 1e-9 * tail)
                    .then_some(multiple as u8)
            })
        }
    }
}

/// The ladders a probability axis's majors take on its tails (#209),
/// sparsest first: the decades, 1-2-5 and 1 to 9 × 10^k of the tail
/// `min(p, 1 − p)`, each with 0.5.
const PROBABILITY_LADDERS: [u16; 3] = [LOG_DECADE, LOG_ONE_TWO_FIVE, LOG_ALL_MANTISSAS];

/// The tail mask a probability axis walks its majors on (#209): the first of
/// [`PROBABILITY_LADDERS`] that puts at least [`LOG_LADDER_MIN_MAJORS`]
/// majors inside the [`guarded_probability_range`] of the axis range, or
/// `None` when even the densest does not, and the walk is a linear grid in
/// `p`.
fn probability_ladder(axis: &AxisOptions) -> Option<u16> {
    let (min, max) = guarded_probability_range(axis.min, axis.max);
    PROBABILITY_LADDERS.into_iter().find(|mask| {
        ProbabilityTicks::new(min, max, *mask, true)
            .take(LOG_LADDER_MIN_MAJORS)
            .count()
            == LOG_LADDER_MIN_MAJORS
    })
}

/// The multiples `m · a` of every decade `a` of a probability axis's tails
/// (#209), the decade powers walked downward: `max` to `min`, multiples 9 to
/// 1, at most [`MAX_LOG_DECADES`] decades. The upper tail of
/// [`ProbabilityTicks`], whose `1 − t` then rises.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TailDescent {
    min: f64,
    max: f64,
    decade: Option<i32>,
    last: i32,
    decades_left: usize,
    base: f64,
    multiple: u8,
    mask: u16,
}

impl TailDescent {
    fn new(min: f64, max: f64, mask: u16) -> Self {
        Self {
            min,
            max,
            decade: Some(clamped_exponent(max.log10().floor())),
            last: clamped_exponent(min.log10().floor()),
            decades_left: MAX_LOG_DECADES,
            base: 0.0,
            // Below 1: the next call opens the first decade.
            multiple: 0,
            mask,
        }
    }
}

impl Iterator for TailDescent {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        loop {
            if self.multiple < 1 {
                let decade = self.decade.filter(|decade| *decade >= self.last)?;
                if self.decades_left == 0 {
                    self.decade = None;
                    return None;
                }
                self.decades_left -= 1;
                self.decade = decade.checked_sub(1);
                self.base = 10f64.powf(f64::from(decade));
                self.multiple = 9;
            }
            let multiple = self.multiple;
            self.multiple -= 1;
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

/// The ticks of a probability axis over `min..max` (#209), in increasing
/// order: the masked tail multiples of the lower half (`m · 10^k < 0.5`),
/// then 0.5 when `with_half` and it is inside, then `1 − t` for the masked
/// tail multiples `t < 0.5` of the upper half, the tail walked downward. A
/// `Copy` state that allocates nothing, bounded by [`MAX_LOG_DECADES`]
/// decades on each side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbabilityTicks {
    lower: LogMultiples,
    half: bool,
    upper: TailDescent,
}

impl ProbabilityTicks {
    /// The ticks of `mask` (a [`LogMultiples`] mask) over `min..max`, a
    /// probability range strictly inside `(0, 1)` (a written one is guarded
    /// by [`guarded_probability_range`]).
    pub fn new(min: f64, max: f64, mask: u16, with_half: bool) -> Self {
        // `1 − max` rounds a little above the walked tail of an upper range
        // end (1 − 0.999 > 0.001): the relative slack keeps that tick.
        Self {
            lower: LogMultiples::new(min, max.min(0.5), mask),
            half: with_half && min <= 0.5 && max >= 0.5,
            upper: TailDescent::new((1.0 - max) * (1.0 - 1e-9), (1.0 - min).min(0.5), mask),
        }
    }
}

impl Iterator for ProbabilityTicks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        if let Some(value) = self.lower.by_ref().find(|value| *value < 0.5) {
            return Some(value);
        }
        if self.half {
            self.half = false;
            return Some(0.5);
        }
        self.upper
            .by_ref()
            .find(|tail| *tail < 0.5)
            .map(|tail| 1.0 - tail)
    }
}

#[derive(Debug, Clone)]
enum MajorWalk {
    Empty,
    Linear(LinearGridIter),
    /// One grid per piece of a broken axis whose pieces have scales of
    /// their own (#205), walked in axis order.
    Pieces {
        grids: [Option<LinearGridIter>; crate::config::MAX_AXIS_BREAKS + 1],
        at: usize,
    },
    Log {
        decades: LogDecades,
        left: usize,
    },
    LogMultiples(LogMultiples),
    Probability(ProbabilityTicks),
}

/// One piece of a broken linear axis whose pieces have scales of their own
/// (#205): its value range, low then high, and the major spacing its ticks
/// walk. Shown units on an axis with a [`AxisOptions::unit`] (#273): the
/// range is the piece's [`AxisOptions::shown_interval`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PieceTicks {
    pub low: f64,
    pub high: f64,
    pub spacing: f64,
}

/// The pieces of a broken linear axis whose pieces have scales of their
/// own (#205), each with the spacing its ticks walk, in axis order; `None`
/// when the pieces share one scale (they walk the axis' one grid) or the
/// axis is not a walkable linear one. A piece's target is its share of
/// [`AXIS_LINEAR_TARGET`] majors, at least two. It walks the axis'
/// `major_spacing` when that places between half and twice its target in
/// it, otherwise the nice step for its own span and target: a short span
/// drawn tall reads at its own scale.
pub fn piece_ticks(
    axis: &AxisOptions,
) -> Option<([PieceTicks; crate::config::MAX_AXIS_BREAKS + 1], usize)> {
    if axis.scale != AxisScale::Linear || axis.categories.is_some() || !walkable_range(axis) {
        return None;
    }
    let map = crate::layout::BreakMap::cuts(axis);
    if !map.is_broken() || !map.per_piece() {
        return None;
    }
    let span = axis.max - axis.min;
    let mut out = [PieceTicks {
        low: 0.0,
        high: 0.0,
        spacing: 1.0,
    }; crate::config::MAX_AXIS_BREAKS + 1];
    let mut count = 0;
    for (drawn, raw) in map.pieces().zip(map.raw_pieces()) {
        let (a, b) = (axis.min + raw.0 * span, axis.min + raw.1 * span);
        let (a, b) = axis.shown_interval(a, b);
        let (low, high) = (a.min(b), a.max(b));
        if !(high > low) {
            continue;
        }
        let share = (drawn.1 - drawn.0).max(0.0);
        let target = ((AXIS_LINEAR_TARGET as f64 * share).round() as usize).max(2);
        let own = nice_linear_step(high - low, target, LooseRange::Above).unwrap_or(high - low);
        let host = axis.major_spacing;
        let fits = host.is_finite()
            && host > 0.0
            && LinearGrid::new(low, high, host, MAX_MAJOR_INTERVALS).is_some_and(|grid| {
                let n = grid.last_index() - grid.first_index() + 1.0;
                n >= (target as f64 / 2.0).max(1.0) && n <= 2.0 * target as f64
            });
        out[count] = PieceTicks {
            low,
            high,
            spacing: if fits { host } else { own },
        };
        count += 1;
    }
    (count > 0).then_some((out, count))
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
///
/// The walk runs over the shown range of an axis with a unit (#273) and
/// maps each tick back to its data position, `(shown - offset) / scale`.
#[derive(Debug, Clone)]
pub struct MajorTicks(MajorWalk, Option<AxisUnit>);

impl Iterator for MajorTicks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        loop {
            let shown = self.0.next()?;
            match self.1 {
                None => return Some(shown),
                Some(unit) => {
                    let value = unit.value(shown);
                    if value.is_finite() {
                        return Some(value);
                    }
                }
            }
        }
    }
}

impl MajorWalk {
    /// The next value of the walk, in the units it walks.
    fn next(&mut self) -> Option<f64> {
        match self {
            MajorWalk::Empty => None,
            MajorWalk::Linear(grid) => grid.next().map(|(_, value)| value),
            MajorWalk::Pieces { grids, at } => loop {
                let grid = grids.get_mut(*at)?;
                match grid.as_mut().and_then(|g| g.next()) {
                    Some((_, value)) => return Some(value),
                    None => *at += 1,
                }
            },
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
            MajorWalk::Probability(ticks) => ticks.next(),
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
///
/// With a [`AxisOptions::unit`] (#273) the linear and logarithmic walks run
/// over the [`AxisOptions::shown_range`], `major_spacing` read in shown
/// units, and each tick is yielded at its data position
/// `(shown - offset) / scale`, still in increasing data order (a negative
/// scale walks the shown grid downward). [`shown_tick_value`] gives back
/// the walked shown value a label prints.
pub fn major_ticks(axis: &AxisOptions) -> MajorTicks {
    if let Some(categories) = &axis.categories {
        // One tick per category: the walk is bounded by the category count,
        // which has no cap (#107), not by the numeric tick budget.
        let (min, max) = axis.data_range();
        return MajorTicks(
            LinearGrid::new(min, max, 1.0, categories.len())
                .map_or(MajorWalk::Empty, |grid| MajorWalk::Linear(grid.iter())),
            None,
        );
    }
    let unit = axis.effective_unit();
    let descending = unit.is_some_and(|unit| unit.scale < 0.0);
    if let Some((pieces, count)) = piece_ticks(axis) {
        let mut grids: [Option<LinearGridIter>; crate::config::MAX_AXIS_BREAKS + 1] =
            [None, None, None, None];
        for (k, piece) in pieces[..count].iter().enumerate() {
            grids[piece_slot(k, count, descending)] =
                LinearGrid::new(piece.low, piece.high, piece.spacing, MAX_MAJOR_INTERVALS)
                    .map(|grid| grid.walk(descending));
        }
        return MajorTicks(MajorWalk::Pieces { grids, at: 0 }, unit);
    }
    let walk = match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                MajorWalk::Empty
            } else {
                let (min, max) = axis.shown_range();
                LinearGrid::new(min, max, effective_major_spacing(axis), MAX_MAJOR_INTERVALS)
                    .map_or(MajorWalk::Empty, |grid| {
                        MajorWalk::Linear(grid.walk(descending))
                    })
            }
        }
        AxisScale::Logarithmic => {
            // Decades of the shown value; a log unit's scale is positive.
            let (min, max) = axis.shown_range();
            let (min, max) = guarded_log_range(min, max);
            log_major_walk(log_ladder(axis), min, max)
        }
        AxisScale::Probability(_) => {
            let (min, max) = guarded_probability_range(axis.min, axis.max);
            match probability_ladder(axis) {
                Some(mask) => MajorWalk::Probability(ProbabilityTicks::new(min, max, mask, true)),
                // Too narrow for the densest ladder: a linear grid in `p`.
                None => nice_linear_step(max - min, AXIS_LINEAR_TARGET, LooseRange::Above)
                    .and_then(|step| LinearGrid::new(min, max, step, MAX_MAJOR_INTERVALS))
                    .map_or(MajorWalk::Empty, |grid| MajorWalk::Linear(grid.iter())),
            }
        }
    };
    MajorTicks(walk, unit)
}

/// The walk slot of piece `k` of `count` (in axis order): `k`, or from the
/// last slot down when the shown walk descends (#273), so the data values
/// still come in increasing order.
fn piece_slot(k: usize, count: usize, descending: bool) -> usize {
    if descending { count - 1 - k } else { k }
}

/// The value an axis prints for the major tick at data value `value`
/// (#273): the walked shown value of [`major_ticks`], so a label reads
/// `0.5`, not `0.49999999999999994`, after the round trip through the data
/// position. Without a unit, `value` itself.
///
/// A linear tick is snapped to the multiple of the spacing it was walked
/// at (each piece's own on a broken axis with scales of their own), a
/// logarithmic one to its `m · 10^k`; a value that is not near one is
/// `scale * value + offset` as it is.
pub fn shown_tick_value(axis: &AxisOptions, value: f64) -> f64 {
    let Some(unit) = axis.effective_unit() else {
        return value;
    };
    let shown = unit.shown(value);
    if !shown.is_finite() {
        return shown;
    }
    match axis.scale {
        AxisScale::Logarithmic => snap_log_multiple(shown).unwrap_or(shown),
        _ => {
            // `k · step` is the product the grid walks; `+ 0.0` keeps a
            // zero tick from printing as `-0`.
            let snap = |step: f64| {
                let k = shown / step;
                (step.is_finite() && step > 0.0 && (k - k.round()).abs() < 1e-6)
                    .then(|| k.round() * step + 0.0)
            };
            match piece_ticks(axis) {
                Some((pieces, count)) => pieces[..count].iter().find_map(|p| snap(p.spacing)),
                None => snap(effective_major_spacing(axis)),
            }
            .unwrap_or(shown)
        }
    }
}

/// `m · 10^k` (`10^k` by `powf`, times `m` from 1 to 9, the product the
/// log walkers write) within a relative 1e-9 of `value`, or `None`.
fn snap_log_multiple(value: f64) -> Option<f64> {
    if !(value.is_finite() && value > 0.0) {
        return None;
    }
    let exponent = clamped_exponent(value.log10().floor());
    [
        exponent,
        exponent.saturating_sub(1),
        exponent.saturating_add(1),
    ]
    .into_iter()
    .find_map(|exponent| {
        let base = 10f64.powf(f64::from(exponent));
        let multiple = (value / base).round();
        let snapped = base * multiple;
        ((1.0..=9.0).contains(&multiple) && (snapped - value).abs() <= 1e-9 * value)
            .then_some(snapped)
    })
}

#[derive(Debug, Clone)]
enum MinorWalk {
    Empty,
    Linear {
        grid: LinearGridIter,
        subdivisions: f64,
    },
    /// One minor grid per piece (#205), as [`MajorWalk::Pieces`].
    Pieces {
        grids: [Option<LinearGridIter>; crate::config::MAX_AXIS_BREAKS + 1],
        at: usize,
        subdivisions: f64,
    },
    Log(LogMinors),
    LogMultiples(LogMultiples),
    Probability(ProbabilityTicks),
}

/// The minor tick values of an axis, from [`minor_ticks`].
#[derive(Debug, Clone)]
pub struct MinorTicks(MinorWalk, Option<AxisUnit>);

impl Iterator for MinorTicks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        // As [`MajorTicks`]: the walk is in shown units, the ticks at their
        // data positions (#273).
        loop {
            let shown = self.0.next()?;
            match self.1 {
                None => return Some(shown),
                Some(unit) => {
                    let value = unit.value(shown);
                    if value.is_finite() {
                        return Some(value);
                    }
                }
            }
        }
    }
}

impl MinorWalk {
    /// The next value of the walk, in the units it walks.
    fn next(&mut self) -> Option<f64> {
        match self {
            MinorWalk::Empty => None,
            MinorWalk::Linear { grid, subdivisions } => loop {
                let (index, value) = grid.next()?;
                // Multiples of the major spacing are the majors themselves.
                if index.rem_euclid(*subdivisions) != 0.0 {
                    return Some(value);
                }
            },
            MinorWalk::Pieces {
                grids,
                at,
                subdivisions,
            } => loop {
                let grid = grids.get_mut(*at)?;
                match grid.as_mut().and_then(|g| g.next()) {
                    Some((index, value)) => {
                        if index.rem_euclid(*subdivisions) != 0.0 {
                            return Some(value);
                        }
                    }
                    None => *at += 1,
                }
            },
            MinorWalk::Log(minors) => minors.next(),
            MinorWalk::LogMultiples(minors) => minors.next(),
            MinorWalk::Probability(minors) => minors.next(),
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
        return MinorTicks(MinorWalk::Empty, None);
    }
    let unit = axis.effective_unit();
    let descending = unit.is_some_and(|unit| unit.scale < 0.0);
    if let Some((pieces, count)) = piece_ticks(axis) {
        let subdivisions = drawn_minor_count(axis) + 1;
        let max_intervals = MAX_MAJOR_INTERVALS
            .saturating_mul(subdivisions)
            .saturating_add(1);
        let mut grids: [Option<LinearGridIter>; crate::config::MAX_AXIS_BREAKS + 1] =
            [None, None, None, None];
        for (k, piece) in pieces[..count].iter().enumerate() {
            grids[piece_slot(k, count, descending)] = LinearGrid::new(
                piece.low,
                piece.high,
                piece.spacing / subdivisions as f64,
                max_intervals,
            )
            .map(|grid| grid.walk(descending));
        }
        return MinorTicks(
            MinorWalk::Pieces {
                grids,
                at: 0,
                subdivisions: subdivisions as f64,
            },
            unit,
        );
    }
    let walk = match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                MinorWalk::Empty
            } else {
                let subdivisions = drawn_minor_count(axis) + 1;
                let step = effective_major_spacing(axis) / subdivisions as f64;
                let max_intervals = MAX_MAJOR_INTERVALS
                    .saturating_mul(subdivisions)
                    .saturating_add(1);
                let (min, max) = axis.shown_range();
                LinearGrid::new(min, max, step, max_intervals).map_or(MinorWalk::Empty, |grid| {
                    MinorWalk::Linear {
                        grid: grid.walk(descending),
                        subdivisions: subdivisions as f64,
                    }
                })
            }
        }
        AxisScale::Logarithmic => {
            let (min, max) = axis.shown_range();
            let (min, max) = guarded_log_range(min, max);
            match log_ladder(axis) {
                LogLadder::Decades(_) => MinorWalk::Log(LogMinors::new(min, max)),
                ladder => match LOG_MINOR_MULTIPLES & !ladder.multiples() {
                    0 => MinorWalk::Empty,
                    mask => MinorWalk::LogMultiples(LogMultiples::new(min, max, mask)),
                },
            }
        }
        AxisScale::Probability(_) => {
            // The next denser tail ladder less the majors (#209); none on the
            // densest ladder or the linear grid.
            let (min, max) = guarded_probability_range(axis.min, axis.max);
            let denser = match probability_ladder(axis) {
                Some(LOG_DECADE) => LOG_ONE_TWO_FIVE & !LOG_DECADE,
                Some(LOG_ONE_TWO_FIVE) => LOG_ALL_MANTISSAS & !LOG_ONE_TWO_FIVE,
                _ => 0,
            };
            match denser {
                0 => MinorWalk::Empty,
                mask => MinorWalk::Probability(ProbabilityTicks::new(min, max, mask, false)),
            }
        }
    };
    MinorTicks(walk, unit)
}

// Invariant tests.

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut cfg = crate::defaults::config();
        let before = cfg.top_x.clone();
        let r = cfg.top_x.auto_ticks(-f64::MAX, f64::MAX, 5);
        assert!(r.is_err());
        assert_eq!(cfg.top_x, before);
    }

    // auto_ticks_all Err must leave all four axes unchanged.
    #[test]
    fn auto_ticks_all_err_preserves_all() {
        let mut cfg = crate::defaults::config();
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
        let mut axis = crate::defaults::config().bottom_x;
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
                                |ladder| MajorTicks(log_major_walk(ladder, min, max), None).count();
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

#[cfg(test)]
mod piece_spacing_tests {
    use super::*;
    use crate::config::{AxisBreak, AxisBreaks};

    /// A short piece gets at least two majors: the spacing steps down the
    /// 1-2-5 ladder from what the range alone would pick, the ticks stay
    /// round, and an unbroken axis keeps its spacing (#200).
    #[test]
    fn a_short_piece_keeps_two_majors() {
        let mut axis = crate::defaults::axis_options_y();
        axis.min = 0.0;
        axis.max = 100.0;
        axis.major_spacing = 20.0;
        assert_eq!(effective_major_spacing(&axis), 20.0);
        axis.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(30.0, 85.0)],
            ..crate::defaults::axis_breaks()
        });
        assert_eq!(effective_major_spacing(&axis), 10.0);
        let majors: Vec<f64> = major_ticks(&axis)
            .filter(|&v| !crate::layout::value_in_cut(&axis, v))
            .collect();
        assert_eq!(majors, vec![0.0, 10.0, 20.0, 30.0, 90.0, 100.0]);
        // A piece of 0.3 units on a 0..100 axis.
        axis.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(0.3, 99.0)],
            ..crate::defaults::axis_breaks()
        });
        let step = effective_major_spacing(&axis);
        assert!(step <= 0.2 + 1e-12, "{step}");
        let k = (step / 10f64.powf(step.log10().floor())).round();
        assert!([1.0, 2.0, 5.0].contains(&k), "{step} is off the ladder");
    }
}

#[cfg(test)]
mod zoomed_break_tests {
    // #205: a view that cuts into a break leaves a piece with no span; the
    // walk keeps the axis' spacing instead of stepping down for it.
    #[test]
    fn a_range_that_cuts_into_a_break_keeps_its_spacing() {
        let mut axis = crate::defaults::config().left_y;
        axis.min = 19.5;
        axis.max = 24.0;
        axis.major_spacing =
            crate::tick::axis_major_spacing(&crate::config::AxisScale::Linear, 19.5, 24.0);
        axis.minor_count = 4;
        for sizing in [
            crate::config::PieceSizing::ByValue,
            crate::config::PieceSizing::Weights(vec![1.0, 9.0]),
        ] {
            axis.breaks = Some(crate::config::AxisBreaks {
                ranges: vec![crate::config::AxisBreak::new(0.25, 19.6)],
                sizing: sizing.clone(),
                ..crate::defaults::axis_breaks()
            });
            assert_eq!(super::effective_major_spacing(&axis), 0.5, "{sizing:?}");
            let majors = super::major_ticks(&axis).count();
            assert!(majors <= 10, "{sizing:?}: {majors} majors");
        }
    }
}
