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

/// The major tick values of `axis` on `side`: its timestamp plan's majors
/// ([`timestamp_tick_plan`]), or the numeric walker's. A pass drawing more
/// than the ticks plans the axis once and reads [`major_tick_values_of`].
pub fn major_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
) -> Vec<f64> {
    major_tick_values_of(
        axis,
        timestamp_tick_plan(axis, side, plot, measure).as_ref(),
    )
}

/// The major tick values of `axis` whose timestamp plan in this pass is
/// `plan` ([`timestamp_tick_plan`] of the same axis, side, plot and
/// measure, #444): the plan's majors, or the numeric walker's without one.
pub fn major_tick_values_of(
    axis: &AxisOptions,
    plan: Option<&crate::chrome::time::TimestampTickPlan>,
) -> Vec<f64> {
    let mut values = match plan {
        Some(plan) => plan.majors.iter().map(|tick| tick.value).collect(),
        None => major_tick_values(axis),
    };
    // A broken axis draws no tick inside a cut (#200); each piece keeps the
    // axis' own spacing.
    values.retain(|&v| !crate::layout::value_in_cut(axis, v));
    values
}

/// The minor tick values of `axis` on `side`, as
/// [`major_tick_values_for_axis`] plans them.
pub fn minor_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
) -> Vec<f64> {
    minor_tick_values_of(
        axis,
        timestamp_tick_plan(axis, side, plot, measure).as_ref(),
    )
}

/// The minor tick values of `axis` whose timestamp plan in this pass is
/// `plan` ([`major_tick_values_of`], #444).
pub fn minor_tick_values_of(
    axis: &AxisOptions,
    plan: Option<&crate::chrome::time::TimestampTickPlan>,
) -> Vec<f64> {
    let mut values = match plan {
        Some(plan) => plan.minor_values.clone(),
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
        crate::chrome::axis_len(&side, plot),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{LabelFormat, TimestampLabelMode, TimestampTickPolicy};
    use crate::layout::Element;
    use crate::text::{RichText, TextExtents};
    use std::cell::Cell;

    /// A measure that counts the hour labels it measures. On a 30-day axis
    /// only the planner's rejected hourly candidates print an hour, so the
    /// count is the number of plans times the hour labels one plan
    /// measures.
    struct Counting {
        hours: Cell<usize>,
    }

    impl MeasureText for Counting {
        fn measure_plain(&self, text: &str, _: &str, size: f32) -> TextExtents {
            if text.contains(':') {
                self.hours.set(self.hours.get() + 1);
            }
            TextExtents {
                width: text.chars().count() as f32 * size * 0.6,
                ascent: size * 0.8,
                descent: size * 0.2,
            }
        }
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            self.measure_plain("", "", rt.font_size)
        }
    }

    /// A chart whose bottom axis is a 30-day automatic calendar axis with
    /// its grid, ticks and labels drawn.
    fn thirty_days() -> Config {
        let mut config = crate::defaults::config();
        let mut format = crate::defaults::timestamp_label_format();
        format.label = TimestampLabelMode::Auto;
        format.tick_policy = TimestampTickPolicy::AutoCalendar;
        config.bottom_x.min = 1_700_000_000.0;
        config.bottom_x.max = 1_700_000_000.0 + 30.0 * 86_400.0;
        config.bottom_x.label_style.format = LabelFormat::Timestamp(format);
        config.bottom_x.tick = crate::config::TickVisibility::Outside;
        config.grid.show_major_x = true;
        config.grid.show_minor_x = true;
        config
    }

    /// The hour labels one plan of the bottom axis measures.
    fn per_plan(config: &Config) -> usize {
        let plot = data_area_rect(&config.data_area().unwrap());
        let measure = Counting {
            hours: Cell::new(0),
        };
        let plan = timestamp_tick_plan(&config.bottom_x, Side::Bottom, &plot, &measure);
        assert!(plan.is_some_and(|plan| plan.majors.iter().all(|t| !t.label.contains(':'))));
        measure.hours.get()
    }

    /// #444: a chrome pass plans each axis once: the grid, the axis' ticks
    /// and labels and the labels of the axes across it read one plan.
    #[test]
    fn a_chrome_pass_plans_each_axis_once() {
        let config = thirty_days();
        let one = per_plan(&config);
        assert!(one > 0, "the planner rejects the hourly candidates");
        let measure = Counting {
            hours: Cell::new(0),
        };
        let planned = crate::chrome::emit_chart_chrome(
            &config,
            &measure,
            &mut crate::sink::Parts(&mut |_| {}),
            1.0,
        );
        assert_eq!(measure.hours.get(), one, "one plan of the bottom axis");
        assert_eq!(planned, [false, true, false, false]);
        assert_eq!(
            planned,
            timestamp_plan_sides(
                &config,
                &data_area_rect(&config.data_area().unwrap()),
                &measure
            )
        );
    }

    /// #444: placing one element (hit testing, selection, auto layout)
    /// plans each axis it reads once: the bottom axis' line and ticks, its
    /// labels, and the left axis' labels that keep clear of its labels.
    #[test]
    fn placing_an_axis_element_plans_each_axis_it_reads_once() {
        let config = thirty_days();
        let one = per_plan(&config);
        for element in [
            Element::Axis(Side::Bottom),
            Element::AxisLabel(Side::Bottom),
            Element::AxisLabel(Side::Left),
        ] {
            let measure = Counting {
                hours: Cell::new(0),
            };
            assert!(crate::chrome::emit_part_of(
                &config,
                &element,
                &measure,
                &mut crate::sink::Parts(&mut |_| {}),
                1.0,
            ));
            assert_eq!(measure.hours.get(), one, "{element:?}");
        }
    }
}
