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

/// The timestamp tick plan of `axis` on `side` of `plot`, its labels
/// measured with `measure`; `None` for an axis without one.
///
/// When `measure` is the plan cache of an [`crate::layout::auto_layout`]
/// call ([`TickPlanCache`], #465), a plan made earlier in the call from the
/// same input is handed out again instead of being planned anew.
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
    let axis_len = crate::chrome::axis_len(&side, plot);
    match measure.tick_plans() {
        Some(cache) => cache.plan(axis, &side, axis_len),
        None => plan_timestamp_ticks(axis, &side, axis_len, measure),
    }
}

/// [`timestamp_tick_plan`] made afresh, along `axis_len` px.
fn plan_timestamp_ticks(
    axis: &AxisOptions,
    side: &Side,
    axis_len: f32,
    measure: &dyn MeasureText,
) -> Option<crate::chrome::time::TimestampTickPlan> {
    let ls = &axis.label_style;
    crate::chrome::time::build_timestamp_tick_plan(axis, axis_len, LABEL_COLLISION_GAP, |text| {
        let m = measure.measure_plain(text, &ls.label_font, ls.font_size);
        match side {
            Side::Top | Side::Bottom => m.width,
            Side::Left | Side::Right => m.height(),
        }
    })
}

/// Whether a label on `side` is measured by its height (a vertical axis)
/// rather than its width: the one thing a plan reads from the side.
fn measured_by_height(side: &Side) -> bool {
    matches!(side, Side::Left | Side::Right)
}

/// The full input of one timestamp tick plan of a non-categorical axis
/// ([`plan_timestamp_ticks`] under one measure): everything
/// [`crate::chrome::time::build_timestamp_tick_plan`] reads from the axis,
/// the side and the axis length. Floats are compared by their bits, so a
/// plan is reused only for the same input bit for bit.
///
/// - the scale, and the range `min` and `max` (the candidates' values);
/// - whether the range is walkable ([`walkable_range`], which also reads
///   the axis' unit);
/// - the timestamp label format (unit, zone, label mode, fraction digits,
///   tick policy, step and period labels);
/// - the label font and size, and whether a label is measured by its
///   width or its height (the side);
/// - the axis length in px.
///
/// The collision gap is the constant [`LABEL_COLLISION_GAP`], and the
/// measure is the one the cache was made over.
struct PlanInput {
    scale: AxisScale,
    min: u64,
    max: u64,
    walkable: bool,
    format: crate::format::TimestampLabelFormat,
    font: String,
    font_size: u32,
    by_height: bool,
    axis_len: u32,
}

impl PlanInput {
    /// The input of a plan of `axis`, whose label format is `format`, on
    /// `side` along `axis_len` px. It copies the axis' configuration (rule
    /// 2 allows copies of the SSOT), never data.
    fn of(
        axis: &AxisOptions,
        format: &crate::format::TimestampLabelFormat,
        side: &Side,
        axis_len: f32,
    ) -> Self {
        Self {
            scale: axis.scale.clone(),
            min: axis.min.to_bits(),
            max: axis.max.to_bits(),
            walkable: walkable_range(axis),
            format: format.clone(),
            font: axis.label_style.label_font.clone(),
            font_size: axis.label_style.font_size.to_bits(),
            by_height: measured_by_height(side),
            axis_len: axis_len.to_bits(),
        }
    }

    /// Whether this is the input of a plan of `axis` (label format
    /// `format`) on `side` along `axis_len` px, compared field by field
    /// without building a second input.
    fn is(
        &self,
        axis: &AxisOptions,
        format: &crate::format::TimestampLabelFormat,
        side: &Side,
        axis_len: f32,
    ) -> bool {
        self.axis_len == axis_len.to_bits()
            && self.min == axis.min.to_bits()
            && self.max == axis.max.to_bits()
            && self.by_height == measured_by_height(side)
            && self.font_size == axis.label_style.font_size.to_bits()
            && self.scale == axis.scale
            && self.walkable == walkable_range(axis)
            && self.font == axis.label_style.label_font
            && self.format == *format
    }
}

/// The timestamp tick plans of one [`crate::layout::auto_layout`] call
/// (#465), and the measure they are made with.
///
/// Auto layout measures a chart in several passes, and each probe of a
/// pass plans the axes it reads again. Wrapped around the call's measure,
/// this hands every probe the plan an earlier probe made from the same
/// input (`PlanInput`: the axis, the span, the pixel length and the
/// label font), and plans afresh, and keeps, every other. It is made at
/// the start of the call and dropped when the call returns, so no plan
/// outlives the call or meets another measure. A plan is reused only for
/// an identical input, so the layout, the plans and the frames are the
/// ones planning afresh gives.
///
/// It measures text exactly as the measure it wraps: every
/// [`MeasureText`] method is handed through.
pub struct TickPlanCache<'m> {
    measure: &'m dyn MeasureText,
    /// The plans made so far in the call, each with its input. A layout
    /// plans a few axes at a few lengths, so a lookup walks the list.
    plans: std::cell::RefCell<Vec<(PlanInput, Option<crate::chrome::time::TimestampTickPlan>)>>,
    #[cfg(test)]
    hits: std::cell::Cell<usize>,
    #[cfg(test)]
    misses: std::cell::Cell<usize>,
}

impl<'m> TickPlanCache<'m> {
    /// An empty cache over `measure`, for one auto layout call.
    pub(crate) fn over(measure: &'m dyn MeasureText) -> Self {
        Self {
            measure,
            plans: std::cell::RefCell::new(Vec::new()),
            #[cfg(test)]
            hits: std::cell::Cell::new(0),
            #[cfg(test)]
            misses: std::cell::Cell::new(0),
        }
    }

    /// The timestamp tick plan of the non-categorical `axis` on `side`
    /// along `axis_len` px: the one made earlier in the call from the same
    /// input, or a new one, which is kept.
    fn plan(
        &self,
        axis: &AxisOptions,
        side: &Side,
        axis_len: f32,
    ) -> Option<crate::chrome::time::TimestampTickPlan> {
        // An axis without a timestamp format has no plan, and finding
        // that out costs less than a lookup.
        let format = crate::chrome::time::timestamp_format(axis)?;
        let kept = self
            .plans
            .borrow()
            .iter()
            .find(|(input, _)| input.is(axis, format, side, axis_len))
            .map(|(_, plan)| plan.clone());
        if let Some(plan) = kept {
            #[cfg(test)]
            self.hits.set(self.hits.get() + 1);
            return plan;
        }
        #[cfg(test)]
        self.misses.set(self.misses.get() + 1);
        // The borrow above has ended: planning only measures text.
        let plan = plan_timestamp_ticks(axis, side, axis_len, self.measure);
        self.plans
            .borrow_mut()
            .push((PlanInput::of(axis, format, side, axis_len), plan.clone()));
        plan
    }

    /// How many plans the call handed out again, and how many it made.
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.hits.get(), self.misses.get())
    }
}

impl MeasureText for TickPlanCache<'_> {
    fn measure_rich(&self, rt: &crate::text::RichText) -> crate::text::TextExtents {
        self.measure.measure_rich(rt)
    }

    fn ink_boxes(
        &self,
        rt: &crate::text::RichText,
        origin: (f32, f32),
        each: &mut dyn FnMut(RectF),
    ) {
        self.measure.ink_boxes(rt, origin, each);
    }

    fn measure_plain(&self, text: &str, family: &str, size: f32) -> crate::text::TextExtents {
        self.measure.measure_plain(text, family, size)
    }

    fn font_line(&self, family: &str, size: f32) -> crate::text::TextExtents {
        self.measure.font_line(family, size)
    }

    fn tick_plans(&self) -> Option<&TickPlanCache<'_>> {
        Some(self)
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

    /// #465: the plan cache of one auto layout call hands an identical
    /// probe the plan made first, measuring nothing, and plans every
    /// input that differs in anything the plan reads afresh.
    #[test]
    fn the_plan_cache_hands_out_a_plan_only_for_an_identical_input() {
        let config = thirty_days();
        let plot = data_area_rect(&config.data_area().unwrap());
        let one = per_plan(&config);
        let measure = Counting {
            hours: Cell::new(0),
        };
        let cache = TickPlanCache::over(&measure);
        let fresh = |axis: &AxisOptions, side: Side, plot: &RectF| {
            let plan = timestamp_tick_plan(axis, side, plot, &NoCache(&measure));
            format!("{plan:?}")
        };
        let cached = |axis: &AxisOptions, side: Side, plot: &RectF| {
            format!("{:?}", timestamp_tick_plan(axis, side, plot, &cache))
        };

        let first = cached(&config.bottom_x, Side::Bottom, &plot);
        assert_eq!(cache.counts(), (0, 1));
        assert_eq!(measure.hours.get(), one);
        for _ in 0..3 {
            assert_eq!(cached(&config.bottom_x, Side::Bottom, &plot), first);
        }
        assert_eq!(cache.counts(), (3, 1), "identical probes hit");
        assert_eq!(measure.hours.get(), one, "a hit measures nothing");
        // The top side measures by width as the bottom does: same input.
        assert_eq!(cached(&config.bottom_x, Side::Top, &plot), first);
        assert_eq!(cache.counts(), (4, 1));

        // Every input the plan reads: each change is a miss, and the plan
        // handed out is the one planning afresh gives.
        let mut changed: Vec<(AxisOptions, Side, RectF)> = Vec::new();
        let base = || (config.bottom_x.clone(), Side::Bottom, plot);
        let mut wider = base();
        wider.2.width += 1.0;
        changed.push(wider);
        let mut nudged = base();
        nudged.2.width = f32::from_bits(plot.width.to_bits() + 1);
        changed.push(nudged);
        let mut later = base();
        later.0.min += 86_400.0;
        changed.push(later);
        let mut longer = base();
        longer.0.max += 86_400.0 * 300.0;
        changed.push(longer);
        let mut bigger = base();
        bigger.0.label_style.font_size += 1.0;
        changed.push(bigger);
        let mut font = base();
        font.0.label_style.label_font = "serif".into();
        changed.push(font);
        let mut pattern = base();
        if let LabelFormat::Timestamp(format) = &mut pattern.0.label_style.format {
            format.label = TimestampLabelMode::Pattern("%H:%M".into());
        }
        changed.push(pattern);
        let mut periods = base();
        if let LabelFormat::Timestamp(format) = &mut periods.0.label_style.format {
            format.period_labels = true;
        }
        changed.push(periods);
        let mut across = base();
        across.1 = Side::Left;
        across.2.height = plot.width;
        changed.push(across);
        let mut unwalkable = base();
        unwalkable.0.max = unwalkable.0.min;
        changed.push(unwalkable);
        let mut log = base();
        log.0.scale = AxisScale::Logarithmic;
        changed.push(log);
        let count = changed.len();
        for (axis, side, plot) in &changed {
            assert_eq!(
                cached(axis, side.clone(), plot),
                fresh(axis, side.clone(), plot),
                "{side:?} {plot:?}"
            );
        }
        assert_eq!(cache.counts(), (4, 1 + count), "each changed input misses");
        for (axis, side, plot) in &changed {
            assert_eq!(
                cached(axis, side.clone(), plot),
                fresh(axis, side.clone(), plot)
            );
        }
        assert_eq!(cache.counts(), (4 + count, 1 + count), "and then hits");

        // A numeric or categorical axis is never kept.
        assert_eq!(cached(&config.left_y, Side::Left, &plot), "None");
        assert_eq!(cache.counts(), (4 + count, 1 + count));
    }

    /// A measure handed through without the plan cache: what planning
    /// afresh measures with.
    struct NoCache<'a>(&'a dyn MeasureText);

    impl MeasureText for NoCache<'_> {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            self.0.measure_rich(rt)
        }
        fn measure_plain(&self, text: &str, family: &str, size: f32) -> TextExtents {
            self.0.measure_plain(text, family, size)
        }
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
