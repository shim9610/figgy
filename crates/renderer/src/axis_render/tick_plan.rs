//! Tick planning: the major spacing an axis walks, its major and minor tick
//! values, the log decade step, the timestamp plan, and the warnings for the
//! tick settings the walkers replace.

use super::*;

/// Hard ceiling on majors the linear walker may emit. A finite-but-tiny
/// spacing (a mis-typed magnitude arriving through the SSoT) would otherwise
/// walk the whole range in near-zero steps — millions of pushes on the
/// raster path, which reads as a frozen chart.
pub(super) const MAX_MAJOR_INTERVALS: usize = 1000;

pub(super) const MAX_MINOR_SUBDIVISIONS: usize = 100;

pub(super) const MAX_LOG_DECADES: usize = 1024;

/// The largest decade step the log walker takes; `RenderWarning`'s message
/// states the same bound.
pub(crate) const MAX_LOG_DECADE_STEP: i32 = i32::MAX;

pub(super) fn range_in_spacing_units(axis: &AxisOptions, spacing: f64) -> Option<f64> {
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

pub(super) fn spacing_advances_range(axis: &AxisOptions, spacing: f64) -> bool {
    let forward = axis.min + spacing;
    let backward = axis.max - spacing;
    (forward.is_finite() && forward > axis.min) || (backward.is_finite() && backward < axis.max)
}

pub(super) fn automatic_linear_spacing(axis: &AxisOptions) -> f64 {
    if !walkable_range(axis) {
        return 1.0;
    }
    let span = axis.max - axis.min;
    let mut spacing = if span.is_finite() {
        crate::chart::nice_spacing(span)
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

/// The major spacing the raster actually walks. The SSoT accepts whatever
/// the host sends (a live spacing input passes through 0 mid-edit), so the
/// guard lives here — in the one place shared by tick marks, grid lines,
/// minors, and label decimals, which therefore can never disagree. A
/// non-finite or ≤ 0 spacing, or one that would emit an absurd tick count,
/// falls back to the auto spacing for the current range instead of blanking
/// the axis. Callers must have validated the range as finite and non-empty.
pub(crate) fn effective_major_spacing(axis: &AxisOptions) -> f64 {
    match major_spacing_rejection(axis) {
        None => axis.major_spacing,
        Some(_) => automatic_linear_spacing(axis),
    }
}

/// Why [`effective_major_spacing`] does not walk `axis.major_spacing`, or
/// `None` when it does. The one decision behind both the fallback and its
/// [`RenderWarning::MajorSpacingReplaced`], so the two cannot disagree.
///
/// The reason is meaningful for a walkable range only, the same precondition
/// [`effective_major_spacing`] has: any other range refuses every spacing
/// (reported as `TooManyTicks`, because its interval count cannot be measured)
/// and the walkers draw no linear ticks on it.
pub(super) fn major_spacing_rejection(axis: &AxisOptions) -> Option<SpacingRejection> {
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
pub(super) fn drawn_minor_count(axis: &AxisOptions) -> usize {
    axis.minor_count.min(MAX_MINOR_SUBDIVISIONS - 1)
}

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

pub(super) fn effective_log_decade_step(axis: &AxisOptions) -> i32 {
    let step = if axis.major_spacing.is_finite() {
        axis.major_spacing.max(1.0).floor()
    } else {
        1.0
    };
    step.min(f64::from(MAX_LOG_DECADE_STEP)) as i32
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

pub(super) const LABEL_COLLISION_GAP: f32 = 8.0;

pub(super) fn timestamp_tick_plan(
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

pub(super) fn major_tick_values(axis: &AxisOptions) -> Vec<f64> {
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
