//! Tick planning: the major spacing an axis walks, its major and minor tick
//! values, the log decade step, the timestamp plan, and the warnings for the
//! tick settings the walkers replace.

use super::*;

// The spacing guards and walk limits are the model's (`crate::tick`), so
// every axis family walks the same grid.
pub(crate) use crate::tick::effective_major_spacing;
pub(super) use crate::tick::{
    MAX_LOG_DECADES, MAX_MAJOR_INTERVALS, drawn_minor_count, effective_log_decade_step,
    major_spacing_rejection, spacing_advances_range,
};

/// The first and last whole decade exponents inside the guarded log range.
/// It stays here, beside `chart::guarded_log_range`, until that guard moves
/// to the model.
pub(super) fn log_exponent_bounds(axis: &AxisOptions) -> (i32, i32) {
    let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
    let first = min
        .log10()
        .ceil()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let last = max
        .log10()
        .floor()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    (first, last)
}

pub(super) fn major_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Vec<f64> {
    if let Some(plan) = timestamp_tick_plan(axis, side, da, fp) {
        return plan.majors.into_iter().map(|tick| tick.value).collect();
    }
    major_tick_values(axis)
}

pub(super) fn minor_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Vec<f64> {
    if let Some(plan) = timestamp_tick_plan(axis, side, da, fp) {
        return plan.minor_values;
    }
    minor_tick_values(axis)
}

pub(crate) const LABEL_COLLISION_GAP: f32 = 8.0;

pub(crate) fn timestamp_tick_plan(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Option<crate::time_axis::TimestampTickPlan> {
    let ls = &axis.label_style;
    crate::time_axis::build_timestamp_tick_plan(
        axis,
        axis_pixel_len(side.clone(), da),
        LABEL_COLLISION_GAP,
        |text| {
            let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
            match side {
                Side::Top | Side::Bottom => m.width,
                Side::Left | Side::Right => m.height(),
            }
        },
    )
}

pub(super) fn axis_pixel_len(side: Side, da: &DataArea) -> f32 {
    match side {
        Side::Top | Side::Bottom => da.width as f32,
        Side::Left | Side::Right => da.height as f32,
    }
}

pub(crate) fn major_tick_values(axis: &AxisOptions) -> Vec<f64> {
    let mut out = Vec::new();
    match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                return out;
            }
            // Anchor ticks to ABSOLUTE multiples of the spacing, not to
            // axis.min: uniform-margin fits make the range ends arbitrary
            // (0.137…), but tick values must stay nice regardless.
            out.extend(
                linear_grid_values(axis, effective_major_spacing(axis), MAX_MAJOR_INTERVALS)
                    .into_iter()
                    .map(|(_, value)| value),
            );
        }
        AxisScale::Logarithmic => {
            // major_spacing = decade step.
            let (start_exp, end_exp) = log_exponent_bounds(axis);
            let step = effective_log_decade_step(axis);
            let mut exponent = start_exp;
            for _ in 0..MAX_LOG_DECADES {
                if exponent > end_exp {
                    break;
                }
                let value = 10f64.powi(exponent);
                if value.is_finite() && value > 0.0 {
                    out.push(value);
                }
                let Some(next) = exponent.checked_add(step) else {
                    break;
                };
                if next <= exponent {
                    break;
                }
                exponent = next;
            }
        }
    }
    out
}

/// Minor tick positions on the same ABSOLUTE grid the majors use — including
/// the partial intervals before the first and after the last major (range
/// ends are arbitrary under uniform-margin fits, and subdividing only
/// between consecutive majors left those edge strips empty).
pub(super) fn minor_tick_values(axis: &AxisOptions) -> Vec<f64> {
    let mut out = Vec::new();
    if axis.minor_count == 0 {
        return out;
    }
    match axis.scale {
        AxisScale::Linear => {
            // The subdivision count is SSoT input too — clamp it so the
            // walk stays O(majors × subdivisions) no matter what arrives.
            if !walkable_range(axis) {
                return out;
            }
            let subdivisions = drawn_minor_count(axis) + 1;
            let step = effective_major_spacing(axis) / subdivisions as f64;
            let max_intervals = MAX_MAJOR_INTERVALS
                .saturating_mul(subdivisions)
                .saturating_add(1);
            for (index, value) in linear_grid_values(axis, step, max_intervals) {
                // Multiples of the major spacing are the majors themselves.
                if index.rem_euclid(subdivisions as f64) == 0.0 {
                    continue;
                }
                out.push(value);
            }
        }
        AxisScale::Logarithmic => {
            // 2a..9a for EVERY decade overlapping the range, anchored at the
            // decade powers — not at the majors, so partial edge decades
            // (and multi-decade major steps) keep their minors.
            let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
            let mut exponent =
                min.log10()
                    .floor()
                    .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            let last_decade =
                max.log10()
                    .floor()
                    .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            for _ in 0..MAX_LOG_DECADES {
                if exponent > last_decade {
                    break;
                }
                let a = 10f64.powi(exponent);
                for k in 2..=9 {
                    let v = a * (k as f64);
                    if v.is_finite() && v >= min && v <= max {
                        out.push(v);
                    }
                }
                let Some(next) = exponent.checked_add(1) else {
                    break;
                };
                exponent = next;
            }
        }
    }
    out
}

// Tick-guard warnings.
//
// The walkers above substitute tick settings that would blank an axis or walk
// it without bound. `tick_warnings` reports those substitutions by asking the
// walkers' own guards (`major_spacing_rejection`, `drawn_minor_count`,
// `effective_log_decade_step`) and the draw path's own gates
// (`timestamp_tick_plan`, `drawn_colorbar`) rather than restating them.

/// The tick settings the axis raster of `config` replaces with ones it can
/// draw. Empty when every setting is drawn as written, and when the raster
/// draws no axes at all (no data area).
///
/// The decoration layer walks all four chart axes whatever their tick and
/// label visibility, and the colourbar's axis whenever the bar is drawn, so
/// those are the axes reported. Depends on `config` alone; the renderer
/// derives it exactly when it rasterizes a `ChartView`.
pub(crate) fn tick_warnings(config: &Config) -> Vec<RenderWarning> {
    let mut out = Vec::new();
    let Ok(config) = raster_frame_config(config) else {
        return out;
    };
    let Ok(da) = config.data_area() else {
        return out;
    };
    let fp = FontPolicy::for_style(&config.draw_style);
    for (which, axis, side) in [
        (WarningAxis::BottomX, &config.bottom_x, Side::Bottom),
        (WarningAxis::TopX, &config.top_x, Side::Top),
        (WarningAxis::LeftY, &config.left_y, Side::Left),
        (WarningAxis::RightY, &config.right_y, Side::Right),
    ] {
        // The plan the raster uses: calendar ticks walk neither
        // `major_spacing` nor `minor_count`.
        if timestamp_tick_plan(axis, side, &da, fp).is_some() {
            continue;
        }
        push_walker_warnings(&mut out, which, axis);
    }
    // The colourbar walks the numeric walkers directly (no calendar plan).
    if let Some((bar, _)) = drawn_colorbar(&config, &da) {
        push_walker_warnings(&mut out, WarningAxis::ColorBar, &bar.axis);
    }
    out
}

/// The substitutions [`major_tick_values`] / [`minor_tick_values`] make for
/// one axis.
pub(super) fn push_walker_warnings(
    out: &mut Vec<RenderWarning>,
    which: WarningAxis,
    axis: &AxisOptions,
) {
    match axis.scale {
        AxisScale::Linear => {
            // Both linear walkers draw nothing on any other range.
            if !walkable_range(axis) {
                return;
            }
            if let Some(reason) = major_spacing_rejection(axis) {
                out.push(RenderWarning::MajorSpacingReplaced {
                    axis: which,
                    requested: axis.major_spacing,
                    used: effective_major_spacing(axis),
                    reason,
                });
            }
            let used = drawn_minor_count(axis);
            if used != axis.minor_count {
                out.push(RenderWarning::MinorCountClamped {
                    axis: which,
                    requested: axis.minor_count,
                    used,
                });
            }
        }
        AxisScale::Logarithmic => {
            // Log minors are 2a..9a in every decade whatever the count; only
            // the decade step of the majors is substituted.
            let used = f64::from(effective_log_decade_step(axis));
            // `!=`, not a negated `==`: a NaN request was substituted too.
            if used != axis.major_spacing {
                out.push(RenderWarning::LogDecadeStepAdjusted {
                    axis: which,
                    requested: axis.major_spacing,
                    used,
                });
            }
        }
    }
}
