//! The tick settings the axis raster replaces, reported as warnings.

use super::*;

// Tick-guard warnings.
//
// The walkers above substitute tick settings that would blank an axis or walk
// it without bound. `tick_warnings` reports those substitutions by asking the
// walkers' own guards (`major_spacing_rejection`, `drawn_minor_count`,
// `effective_log_decade_step`) and the draw path's own gates
// (`timestamp_tick_plan`, `drawn_colorbar`, the adaptive log policy) rather
// than restating them.

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
    let plot = data_area_rect(&da);
    let measure = CpuTextMeasure::for_style(&config.draw_style);
    for (which, axis, side) in [
        (WarningAxis::BottomX, &config.bottom_x, Side::Bottom),
        (WarningAxis::TopX, &config.top_x, Side::Top),
        (WarningAxis::LeftY, &config.left_y, Side::Left),
        (WarningAxis::RightY, &config.right_y, Side::Right),
    ] {
        // The plan the raster uses: calendar ticks walk neither
        // `major_spacing` nor `minor_count`.
        if timestamp_tick_plan(axis, side, &plot, &measure).is_some() {
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
    // A categorical axis walks its category positions; it has no spacing,
    // count or decade step to replace.
    if axis.categories.is_some() {
        return;
    }
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
            // The adaptive ladder plans the majors from the range and does
            // not walk `major_spacing`, so there is nothing to substitute.
            if axis.log_ticks == LogTickPolicy::Adaptive {
                return;
            }
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
