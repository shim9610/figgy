//! Tick planning: the "nice" spacing rule every axis shares.
//!
//! [`nice_linear_step`] and [`nice_log_step`] are the one spacing rule. The
//! Cartesian axis helpers (`Chart::set_x_range` and the fits) call it through
//! [`axis_major_spacing`] with [`AXIS_LINEAR_TARGET`] and
//! [`AXIS_LOG_TARGET`]; [`AxisOptions::compute_nice_ticks`] calls it with the
//! caller's target and also widens the range to whole steps, which is what
//! the bounded chart families and contour levels use.

use crate::config::{AxisOptions, AxisScale, Config};

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
    /// with it the bounded chart families and contour levels.
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

    // compute_nice_ticks is pure.
    #[test]
    fn compute_nice_ticks_pure() {
        let a = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        let b = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        assert_eq!(a, b);
    }
}
