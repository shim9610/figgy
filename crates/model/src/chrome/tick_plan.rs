//! Tick planning: the major spacing an axis walks, its major and minor tick
//! values, the log decade step, the timestamp plan, and the warnings for the
//! tick settings the walkers replace.

use super::*;

// The spacing guards and walk limits are the model's (`crate::tick`), so
// every axis family walks the same grid.
pub use crate::tick::effective_major_spacing;
pub use crate::tick::{
    drawn_minor_count, effective_log_decade_step, major_spacing_rejection, walkable_range,
};

pub fn major_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
) -> Vec<f64> {
    let mut values = match timestamp_tick_plan(axis, side, plot, measure) {
        Some(plan) => plan.majors.into_iter().map(|tick| tick.value).collect(),
        None => major_tick_values(axis),
    };
    // A broken axis draws no tick inside a cut (#200); each piece keeps the
    // axis' own spacing.
    values.retain(|&v| !crate::layout::value_in_cut(axis, v));
    values
}

pub fn minor_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
) -> Vec<f64> {
    let mut values = match timestamp_tick_plan(axis, side, plot, measure) {
        Some(plan) => plan.minor_values,
        None => minor_tick_values(axis),
    };
    values.retain(|&v| !crate::layout::value_in_cut(axis, v));
    values
}

pub const LABEL_COLLISION_GAP: f32 = 8.0;

pub fn timestamp_tick_plan(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
) -> Option<crate::chrome::time::TimestampTickPlan> {
    // A categorical axis ignores its label format: its ticks are the
    // category positions.
    if axis.categories.is_some() {
        return None;
    }
    let ls = &axis.label_style;
    crate::chrome::time::build_timestamp_tick_plan(
        axis,
        axis_pixel_len(side.clone(), plot),
        LABEL_COLLISION_GAP,
        |text| {
            let m = measure.measure_plain(text, &ls.label_font, ls.font_size);
            match side {
                Side::Top | Side::Bottom => m.width,
                Side::Left | Side::Right => m.height(),
            }
        },
    )
}

pub fn axis_pixel_len(side: Side, plot: &RectF) -> f32 {
    match side {
        Side::Top | Side::Bottom => plot.width,
        Side::Left | Side::Right => plot.height,
    }
}

/// The major tick values the numeric walker draws: [`crate::tick::major_ticks`],
/// anchored to absolute multiples of the spacing (or whole decades), so tick
/// values stay round whatever the range ends are.
pub fn major_tick_values(axis: &AxisOptions) -> Vec<f64> {
    crate::tick::major_ticks(axis).collect()
}

/// The minor tick values on the majors' absolute grid, including the partial
/// intervals before the first and after the last major:
/// [`crate::tick::minor_ticks`].
pub fn minor_tick_values(axis: &AxisOptions) -> Vec<f64> {
    crate::tick::minor_ticks(axis).collect()
}
