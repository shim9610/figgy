//! Tick labels: which labels an axis or the colourbar shows, where each is
//! placed, and how it is drawn.

use super::*;
use crate::axis_render::*;

// Tick labels.

pub(crate) enum NumericTickText {
    Plain(String),
    Power(RichText),
}

pub(crate) struct NumericTickLabel {
    pub(crate) position: (f32, f32),
    /// The box the label is drawn in, which decides whether it collides
    /// with its neighbours.
    pub(crate) bounds: RectF,
    pub(crate) text: NumericTickText,
}

impl NumericTickText {
    pub(crate) fn as_tick_text(&self) -> TickText<'_> {
        match self {
            NumericTickText::Plain(text) => TickText::Plain(text),
            NumericTickText::Power(text) => TickText::Power(text),
        }
    }
}

/// Keep numeric labels readable at the current scale without changing the
/// document's requested ticks/grid or shrinking the font. The same placement
/// and glyph metrics are used for measuring and drawing, including exponents.
pub(crate) fn visible_numeric_labels(
    axis: &AxisOptions,
    side: Side,
    fp: FontPolicy,
    majors: &[f64],
    position: impl Fn(f64) -> (f32, f32),
) -> Vec<NumericTickLabel> {
    let ls = &axis.label_style;
    prune_numeric_labels(axis, fp, majors, position, |position, metrics| {
        label_rect(
            label_origin(
                side.clone(),
                position,
                axis.major_tick_length,
                (ls.label_offset_x, ls.label_offset_y),
                metrics,
            ),
            metrics,
        )
    })
}

/// [`visible_numeric_labels`] with each label's drawn box given by
/// `bounds(position, metrics)`: when two kept neighbours' boxes come closer
/// than half an em both across and along the screen, the labels are thinned
/// to every 2nd, 5th, 10th... label. A Cartesian axis' labels line up, so
/// this compares them along the axis; a polar ray's labels step along the
/// ray at any angle.
pub(crate) fn prune_numeric_labels(
    axis: &AxisOptions,
    fp: FontPolicy,
    majors: &[f64],
    position: impl Fn(f64) -> (f32, f32),
    bounds: impl Fn((f32, f32), crate::text_render::TextMetrics) -> RectF,
) -> Vec<NumericTickLabel> {
    let ls = &axis.label_style;
    let labels: Vec<_> = majors
        .iter()
        .filter_map(|&value| {
            let (text, metrics) = if matches!(ls.format, LabelFormat::Power) {
                let text = format_tick_power(value, ls.significant_digits, ls);
                let metrics = measure_rich_text(&text, fp);
                (NumericTickText::Power(text), metrics)
            } else {
                let text = format_tick_value(
                    value,
                    &ls.format,
                    ls.significant_digits,
                    &axis.scale,
                    effective_major_spacing(axis),
                    axis.min,
                    axis.max,
                );
                let metrics =
                    measure_plain_text(&text, &ls.label_font, ls.font_size, false, false, fp);
                (NumericTickText::Plain(text), metrics)
            };
            let position = position(value);
            let bounds = bounds(position, metrics);
            [bounds.x, bounds.y, bounds.width, bounds.height]
                .into_iter()
                .all(f32::is_finite)
                .then_some(NumericTickLabel {
                    position,
                    bounds,
                    text,
                })
        })
        .collect();
    let stride = label_stride(labels.len(), ls.font_size, |index| labels[index].bounds);
    labels
        .into_iter()
        .enumerate()
        .filter_map(|(index, label)| (index % stride == 0).then_some(label))
        .collect()
}

/// The stride an axis keeps every `stride`-th of its `count` labels by:
/// the first of 1, 2, 5, 10, 20, 50... at which every kept label's box
/// (`bounds(index)`) stays half an em of `font_size` clear of the previous
/// kept one, either across or along the screen.
pub(crate) fn label_stride(count: usize, font_size: f32, bounds: impl Fn(usize) -> RectF) -> usize {
    // Half an em also scales with HiDPI/PNG export, unlike a fixed pixel gap.
    let gap = font_size * 0.5;
    let mut stride = 1;
    loop {
        let mut previous: Option<RectF> = None;
        let apart = |a: (f32, f32), b: (f32, f32)| b.0 >= a.1 + gap || a.0 >= b.1 + gap;
        let fits = (0..count).step_by(stride).all(|index| {
            let b = bounds(index);
            let fits = previous.is_none_or(|a| {
                apart((a.x, a.x + a.width), (b.x, b.x + b.width))
                    || apart((a.y, a.y + a.height), (b.y, b.y + b.height))
            });
            previous = Some(b);
            fits
        });
        if fits {
            return stride;
        }
        // Regular 1/2/5 multiples avoid alternating crowded and sparse labels.
        let decade = 10usize.pow(stride.ilog10());
        stride = match stride / decade {
            1 => 2 * decade,
            2 => 5 * decade,
            _ => 10 * decade,
        };
    }
}

/// Every tick label of a categorical axis with its tick position, in draw
/// order: `categories[i].label` at category `i`, measured and placed like a
/// plain numeric label in the axis' label style, and thinned by the same
/// [`label_stride`] rule when neighbours crowd. An empty label keeps its
/// slot in the stride and draws nothing. Nothing is collected: the stride is
/// measured on the categories themselves.
pub(crate) fn each_category_tick_label(
    axis: &AxisOptions,
    categories: &[crate::config::Category],
    side: Side,
    fp: FontPolicy,
    position: impl Fn(f64) -> (f32, f32),
    mut each: impl FnMut(TickText<'_>, (f32, f32)),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    let placed = |index: usize| {
        let text = categories[index].label.as_str();
        let position = position(index as f64);
        let metrics = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
        let origin = label_origin(
            side.clone(),
            position,
            axis.major_tick_length,
            (ls.label_offset_x, ls.label_offset_y),
            metrics,
        );
        (text, position, label_rect(origin, metrics))
    };
    let stride = label_stride(categories.len(), ls.font_size, |index| placed(index).2);
    for index in (0..categories.len()).step_by(stride) {
        let (text, position, bounds) = placed(index);
        let finite = [bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f32::is_finite);
        if !text.is_empty() && finite {
            each(TickText::Plain(text), position);
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlainTickLabel {
    pub(super) value: f64,
    pub(super) text: String,
}

pub(super) fn visible_timestamp_labels(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    fp: FontPolicy,
    majors: &[f64],
) -> Vec<PlainTickLabel> {
    let Some(cfg) = crate::time_axis::timestamp_format(axis) else {
        return Vec::new();
    };
    let labels: Vec<PlainTickLabel> =
        if let Some(plan) = timestamp_tick_plan(axis, side.clone(), plot, fp) {
            plan.majors
                .into_iter()
                .map(|tick| PlainTickLabel {
                    value: tick.value,
                    text: tick.label,
                })
                .collect()
        } else {
            majors
                .iter()
                .filter_map(|v| {
                    crate::time_axis::format_timestamp_numeric_tick(
                        *v,
                        cfg,
                        axis.min,
                        axis.max,
                        effective_major_spacing(axis),
                    )
                    .map(|text| PlainTickLabel { value: *v, text })
                })
                .collect()
        };

    prune_overlapping_plain_labels(axis, side, plot, fp, labels)
}

pub(super) fn prune_overlapping_plain_labels(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    fp: FontPolicy,
    labels: Vec<PlainTickLabel>,
) -> Vec<PlainTickLabel> {
    let mut intervals: Vec<(f32, f32, PlainTickLabel)> = labels
        .into_iter()
        .map(|label| {
            let pos = value_to_screen(label.value, axis, side.clone(), plot);
            let (start, end) = plain_tick_label_interval(&label.text, pos, side.clone(), axis, fp);
            (start, end, label)
        })
        .filter(|(start, end, _)| start.is_finite() && end.is_finite())
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut last_end = f32::NEG_INFINITY;
    for (start, end, label) in intervals {
        if start >= last_end + LABEL_COLLISION_GAP {
            last_end = end;
            out.push(label);
        }
    }
    out
}

pub(super) fn plain_tick_label_interval(
    text: &str,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
) -> (f32, f32) {
    let ls: &LabelStyle = &axis.label_style;
    let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
    let (origin_x, origin_y) = label_origin(
        side.clone(),
        tick_pos,
        axis.major_tick_length,
        (ls.label_offset_x, ls.label_offset_y),
        m,
    );
    match side {
        Side::Top | Side::Bottom => (origin_x, origin_x + m.width),
        Side::Left | Side::Right => (origin_y - m.ascent, origin_y + m.descent),
    }
}

pub(super) fn plain_seg(c: char) -> RichSegment {
    RichSegment::plain(c)
}

/// The text of one drawn tick label: plain, or a Power-format `RichText`
/// with its exponent.
pub(crate) enum TickText<'a> {
    Plain(&'a str),
    Power(&'a RichText),
}

/// Where a tick label at `tick_pos` is drawn: its baseline origin and its
/// glyph extents. The emits place the label there, so a label's pixels and
/// its selection and hit geometry come from one origin.
pub(crate) fn tick_label_placement(
    text: &TickText<'_>,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
) -> ((f32, f32), crate::text::TextExtents) {
    let ls: &LabelStyle = &axis.label_style;
    let m = match text {
        TickText::Plain(text) => {
            measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp)
        }
        TickText::Power(rt) => measure_rich_text(rt, fp),
    };
    let origin = label_origin(
        side,
        tick_pos,
        axis.major_tick_length,
        (ls.label_offset_x, ls.label_offset_y),
        m,
    );
    (origin, m)
}

/// Place one tick label at its tick, moved by `offset` (the detached-axis
/// translation), above the data.
pub(crate) fn place_tick_text(
    text: &TickText<'_>,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
    offset: (f32, f32),
    sink: &mut dyn Sink,
) {
    let (origin, extents) = tick_label_placement(text, tick_pos, side, axis, fp);
    let ls: &LabelStyle = &axis.label_style;
    let placed = PlacedText {
        text: match text {
            TickText::Plain(text) => TextRef::Plain {
                text,
                family: &ls.label_font,
                size: ls.font_size,
                color: ls.color,
            },
            TickText::Power(rt) => TextRef::Rich(rt),
        },
        origin: (origin.0 + offset.0, origin.1 + offset.1),
        extents,
        rotation: None,
    };
    sink.text(Slot::Above, &placed);
}

/// The colourbar's tick labels along the strip `rect`
/// (`Element::ColorBarLabel`), moved by the bar axis' detached translation.
pub(crate) fn emit_colorbar_tick_labels(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    fp: FontPolicy,
    sink: &mut dyn Sink,
) {
    let majors = major_tick_values(axis);
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_colorbar_tick_label(axis, side.clone(), rect, fp, &majors, |text, pos| {
        place_tick_text(&text, pos, side.clone(), axis, fp, offset, sink);
    });
}

/// Every tick label the axis on `side` draws, with its tick position, in
/// draw order, before the detached-axis translation.
pub(crate) fn each_axis_tick_label(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    fp: FontPolicy,
    majors: &[f64],
    mut each: impl FnMut(TickText<'_>, (f32, f32)),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    if let Some(categories) = &axis.categories {
        each_category_tick_label(
            axis,
            categories,
            side.clone(),
            fp,
            |value| value_to_screen(value, axis, side.clone(), plot),
            each,
        );
        return;
    }
    if let LabelFormat::Timestamp(_) = &ls.format {
        for label in visible_timestamp_labels(axis, side.clone(), plot, fp, majors) {
            let pos = value_to_screen(label.value, axis, side.clone(), plot);
            each(TickText::Plain(&label.text), pos);
        }
    } else {
        for label in visible_numeric_labels(axis, side.clone(), fp, majors, |value| {
            value_to_screen(value, axis, side.clone(), plot)
        }) {
            each(label.text.as_tick_text(), label.position);
        }
    }
}

/// Every tick label the colourbar draws along the strip `rect`, with its
/// tick position, in draw order. Timestamps are not offered: a z value is a
/// magnitude, and the timestamp planner needs an axis pixel length and
/// collision pruning that belong to a time axis. `LabelFormat::Timestamp` on a
/// colourbar therefore formats through the numeric path rather than being
/// silently dropped.
pub(crate) fn each_colorbar_tick_label(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    fp: FontPolicy,
    majors: &[f64],
    mut each: impl FnMut(TickText<'_>, (f32, f32)),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    for label in visible_numeric_labels(axis, side.clone(), fp, majors, |value| {
        point_on_rect_side(axis_fraction(value, axis), &side, rect)
    }) {
        each(label.text.as_tick_text(), label.position);
    }
}
