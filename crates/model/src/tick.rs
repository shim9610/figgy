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

fn nice_num(range: f64, round: bool) -> f64 {
    let exp = range.log10().floor();
    let frac = range / 10f64.powf(exp);
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
        if frac <= 1.0 {
            1.0
        } else if frac <= 2.0 {
            2.0
        } else if frac <= 5.0 {
            5.0
        } else {
            10.0
        }
    };
    nice_frac * 10f64.powf(exp)
}

fn finite_nice_value(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        fallback
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
    let span = data_max - data_min;
    if !(span.is_finite() && span > 0.0) {
        return Err(TickError::InvalidRange);
    }
    let range = finite_nice_value(nice_num(span, false), span);
    let divisor = ((target_count as f64) - 1.0).max(1.0);
    let raw_step = finite_nice_value(range / divisor, range);
    let step = finite_nice_value(nice_num(raw_step, true), raw_step);
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
    let decades = log_max - log_min;
    if !(log_min.is_finite() && log_max.is_finite() && decades.is_finite() && decades > 0.0) {
        return Err(TickError::InvalidRange);
    }
    let divisor = ((target_count as f64) - 1.0).max(1.0);
    let decade_step = (decades / divisor).ceil().max(1.0);
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
    fn rejects_zero_target_count() {
        let e = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 10.0, 0).unwrap_err();
        assert_eq!(e, TickError::ZeroTargetCount);
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

    // compute_nice_ticks is pure.
    #[test]
    fn compute_nice_ticks_pure() {
        let a = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        let b = AxisOptions::compute_nice_ticks(AxisScale::Linear, 0.0, 100.0, 6).unwrap();
        assert_eq!(a, b);
    }
}
