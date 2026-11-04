//! Tick labels: which labels an axis or the colourbar shows, where each is
//! placed, and how it is drawn.

use super::*;

// Tick labels.

pub enum NumericTickText {
    Plain(String),
    Power(RichText),
}

pub struct NumericTickLabel {
    /// The major tick value the label shows.
    pub value: f64,
    pub position: (f32, f32),
    /// The box the label is drawn in, which decides whether it collides
    /// with its neighbours.
    pub bounds: RectF,
    pub text: NumericTickText,
}

impl NumericTickText {
    pub fn as_tick_text(&self) -> TickText<'_> {
        match self {
            NumericTickText::Plain(text) => TickText::Plain(text),
            NumericTickText::Power(text) => TickText::Power(text),
        }
    }
}

/// Keep numeric labels readable at the current scale without changing the
/// document's requested ticks/grid or shrinking the font. The same placement
/// and glyph metrics are used for measuring and drawing, including exponents.
pub fn visible_numeric_labels(
    axis: &AxisOptions,
    side: Side,
    measure: &dyn MeasureText,
    majors: &[f64],
    position: impl Fn(f64) -> (f32, f32),
) -> Vec<NumericTickLabel> {
    let ls = &axis.label_style;
    prune_numeric_labels(axis, measure, majors, position, |position, metrics| {
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
///
/// On an adaptive log axis ([`crate::tick::log_label_ladders`]) crowded
/// labels are first demoted to the next sparser ladder, {1..9} → {1, 2, 5}
/// → {1} × 10^k, as long as that keeps at least two labels; the stride then
/// thins the ladder that is left, every n-th decade on the decade ladder.
pub fn prune_numeric_labels(
    axis: &AxisOptions,
    measure: &dyn MeasureText,
    majors: &[f64],
    position: impl Fn(f64) -> (f32, f32),
    bounds: impl Fn((f32, f32), TextExtents) -> RectF,
) -> Vec<NumericTickLabel> {
    let ls = &axis.label_style;
    let labels: Vec<_> = majors
        .iter()
        .filter_map(|&value| {
            let (text, metrics) = if matches!(ls.format, LabelFormat::Power) {
                let text = format_tick_power(value, ls.significant_digits, ls);
                let metrics = measure.measure_rich(&text);
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
                let metrics = measure.measure_plain(&text, &ls.label_font, ls.font_size);
                (NumericTickText::Plain(text), metrics)
            };
            let position = position(value);
            let bounds = bounds(position, metrics);
            [bounds.x, bounds.y, bounds.width, bounds.height]
                .into_iter()
                .all(f32::is_finite)
                .then_some(NumericTickLabel {
                    value,
                    position,
                    bounds,
                    text,
                })
        })
        .collect();
    // The ladder the labels are demoted to; `None` keeps every label.
    let on_ladder = |label: &NumericTickLabel, mask: u16| {
        crate::tick::log_multiple(label.value).is_some_and(|multiple| mask & (1 << multiple) != 0)
    };
    let kept = |ladder: Option<u16>, label: &NumericTickLabel| {
        ladder.is_none_or(|mask| on_ladder(label, mask))
    };
    let kept_bounds = |ladder: Option<u16>, index: usize| {
        labels
            .iter()
            .filter(|label| kept(ladder, *label))
            .nth(index)
            .map_or(labels[index].bounds, |label| label.bounds)
    };
    let mut ladder = None;
    for &mask in crate::tick::log_label_ladders(axis) {
        let count = labels
            .iter()
            .filter(|label| on_ladder(*label, mask))
            .count();
        if count < 2 {
            break;
        }
        ladder = Some(mask);
        if label_stride(count, ls.font_size, |index| kept_bounds(ladder, index)) == 1 {
            break;
        }
    }
    let stride = match ladder {
        None => label_stride(labels.len(), ls.font_size, |index| labels[index].bounds),
        Some(_) => label_stride(
            labels.iter().filter(|label| kept(ladder, *label)).count(),
            ls.font_size,
            |index| kept_bounds(ladder, index),
        ),
    };
    // On a broken linear axis (#200) the stride counts on the unbroken
    // walk, so each piece keeps the round multiples it would keep without
    // the cut, not a cadence carried over from the piece before.
    let grid = (ladder.is_none()
        && axis.scale == crate::config::AxisScale::Linear
        && axis.categories.is_none()
        && crate::layout::BreakMap::of(axis).is_broken())
    .then(|| {
        crate::tick::major_ticks(axis)
            .next()
            .map(|first| (first, effective_major_spacing(axis)))
    })
    .flatten()
    .filter(|&(_, spacing)| spacing.is_finite() && spacing > 0.0);
    let grid_index = |index: usize, value: f64| {
        grid.and_then(|(first, spacing)| {
            let k = (value - first) / spacing;
            ((k - k.round()).abs() < 1e-6 && k >= -0.5).then(|| k.round() as usize)
        })
        .unwrap_or(index)
    };
    labels
        .into_iter()
        .filter(|label| kept(ladder, label))
        .enumerate()
        .filter_map(|(index, label)| {
            (grid_index(index, label.value) % stride == 0).then_some(label)
        })
        .collect()
}

/// The stride an axis keeps every `stride`-th of its `count` labels by:
/// the first of 1, 2, 5, 10, 20, 50... at which every kept label's box
/// (`bounds(index)`) stays half an em of `font_size` clear of the previous
/// kept one, either across or along the screen.
pub fn label_stride(count: usize, font_size: f32, bounds: impl Fn(usize) -> RectF) -> usize {
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
pub fn each_category_tick_label(
    axis: &AxisOptions,
    categories: &[crate::config::Category],
    side: Side,
    measure: &dyn MeasureText,
    position: impl Fn(f64) -> (f32, f32),
    mut each: impl FnMut(TickText<'_>, (f32, f32)),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    // A category's rich label (#203) is drawn, and measured, in place of
    // its plain one.
    let text_of = |index: usize| match &categories[index].rich_label {
        Some(rich) => TickText::Power(rich),
        None => TickText::Plain(categories[index].label.as_str()),
    };
    let placed = |index: usize| {
        let text = text_of(index);
        let position = position(index as f64);
        let metrics = match &text {
            TickText::Plain(plain) => measure.measure_plain(plain, &ls.label_font, ls.font_size),
            TickText::Power(rich) => measure.measure_rich(rich),
        };
        let origin = label_origin(
            side.clone(),
            position,
            axis.major_tick_length,
            (ls.label_offset_x, ls.label_offset_y),
            metrics,
        );
        let empty = match &text {
            TickText::Plain(plain) => plain.is_empty(),
            TickText::Power(rich) => rich.segments.is_empty(),
        };
        (empty, position, label_rect(origin, metrics))
    };
    let stride = label_stride(categories.len(), ls.font_size, |index| placed(index).2);
    for index in (0..categories.len()).step_by(stride) {
        let (empty, position, bounds) = placed(index);
        let finite = [bounds.x, bounds.y, bounds.width, bounds.height]
            .into_iter()
            .all(f32::is_finite);
        if !empty && finite {
            each(text_of(index), position);
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlainTickLabel {
    pub value: f64,
    pub text: String,
}

pub fn visible_timestamp_labels(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
    majors: &[f64],
) -> Vec<PlainTickLabel> {
    let Some(cfg) = crate::chrome::time::timestamp_format(axis) else {
        return Vec::new();
    };
    let labels: Vec<PlainTickLabel> =
        if let Some(plan) = timestamp_tick_plan(axis, side.clone(), plot, measure) {
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
                    crate::chrome::time::format_timestamp_numeric_tick(
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

    prune_overlapping_plain_labels(axis, side, plot, measure, labels)
}

pub fn prune_overlapping_plain_labels(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
    labels: Vec<PlainTickLabel>,
) -> Vec<PlainTickLabel> {
    let mut intervals: Vec<(f32, f32, PlainTickLabel)> = labels
        .into_iter()
        .map(|label| {
            let pos = value_to_screen(label.value, axis, side.clone(), plot);
            let (start, end) =
                plain_tick_label_interval(&label.text, pos, side.clone(), axis, measure);
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

pub fn plain_tick_label_interval(
    text: &str,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    measure: &dyn MeasureText,
) -> (f32, f32) {
    let ls: &LabelStyle = &axis.label_style;
    let m = measure.measure_plain(text, &ls.label_font, ls.font_size);
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

pub fn plain_seg(c: char) -> RichSegment {
    RichSegment::plain(c)
}

/// The text of one drawn tick label: plain, or a Power-format `RichText`
/// with its exponent.
pub enum TickText<'a> {
    Plain(&'a str),
    Power(&'a RichText),
}

/// Where a tick label at `tick_pos` is drawn: its baseline origin and its
/// glyph extents. The emits place the label there, so a label's pixels and
/// its selection and hit geometry come from one origin.
pub fn tick_label_placement(
    text: &TickText<'_>,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    measure: &dyn MeasureText,
) -> ((f32, f32), crate::text::TextExtents) {
    let ls: &LabelStyle = &axis.label_style;
    let m = match text {
        TickText::Plain(text) => measure.measure_plain(text, &ls.label_font, ls.font_size),
        TickText::Power(rt) => measure.measure_rich(rt),
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
pub fn place_tick_text(
    text: &TickText<'_>,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    measure: &dyn MeasureText,
    offset: (f32, f32),
    sink: &mut dyn Sink,
) {
    let (origin, extents) = tick_label_placement(text, tick_pos, side, axis, measure);
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
pub fn emit_colorbar_tick_labels(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let majors = major_tick_values(axis);
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_colorbar_tick_label(axis, side.clone(), rect, measure, &majors, |text, pos| {
        place_tick_text(&text, pos, side.clone(), axis, measure, offset, sink);
    });
}

/// Every tick label the axis on `side` draws, with its tick position, in
/// draw order, before the detached-axis translation.
pub fn each_axis_tick_label(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
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
            measure,
            |value| value_to_screen(value, axis, side.clone(), plot),
            each,
        );
        return;
    }
    if let LabelFormat::Timestamp(_) = &ls.format {
        for label in visible_timestamp_labels(axis, side.clone(), plot, measure, majors) {
            let pos = value_to_screen(label.value, axis, side.clone(), plot);
            each(TickText::Plain(&label.text), pos);
        }
    } else {
        for label in visible_numeric_labels(axis, side.clone(), measure, majors, |value| {
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
pub fn each_colorbar_tick_label(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    measure: &dyn MeasureText,
    majors: &[f64],
    mut each: impl FnMut(TickText<'_>, (f32, f32)),
) {
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) {
        return;
    }
    for label in visible_numeric_labels(axis, side.clone(), measure, majors, |value| {
        point_on_rect_side(axis_fraction(value, axis), &side, rect)
    }) {
        each(label.text.as_tick_text(), label.position);
    }
}

/// The gap between the tick label band and a category group's bracket, and
/// between the bracket and its header, as a share of the tick label font
/// size, so a scaled config (an export at 2x) places them scaled.
const GROUP_GAP: f32 = 0.5;
/// How far a group bracket's ends turn toward the axis, and how far a
/// bracket stops short of its run's edges, as shares of the font size.
const GROUP_BRACKET_END: f32 = 0.33;
const GROUP_BRACKET_INSET: f32 = 0.15;

/// Place the category group headers of the categorical axis on `side`
/// (`Top` or `Bottom`) under `plot` (#203): past the tick label band, a
/// bracket along each run when it asks for one and the header centred on
/// the run, moved by the groups' offset. Nothing for an axis without groups
/// or without labels drawn.
pub fn emit_category_groups(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
    sink: &mut dyn crate::sink::Sink,
) {
    use crate::sink::{PlacedText, Slot, StrokeStyle, TextRef};
    let (Some(categories), Some(groups)) = (&axis.categories, &axis.category_groups) else {
        return;
    };
    let ls = &axis.label_style;
    if !(ls.visible && ls.label_visible) || !matches!(side, Side::Top | Side::Bottom) {
        return;
    }
    // The tick label band's far edge: the tallest drawn label past the
    // ticks.
    let mut band = 0.0f32;
    each_category_tick_label(
        axis,
        categories,
        side.clone(),
        measure,
        |value| value_to_screen(value, axis, side.clone(), plot),
        |text, _| {
            let metrics = match &text {
                TickText::Plain(plain) => {
                    measure.measure_plain(plain, &ls.label_font, ls.font_size)
                }
                TickText::Power(rich) => measure.measure_rich(rich),
            };
            band = band.max(metrics.height());
        },
    );
    let below = side == Side::Bottom;
    let edge = if below { plot.y + plot.height } else { plot.y };
    let gap = GROUP_GAP * ls.font_size;
    let inset = GROUP_BRACKET_INSET * ls.font_size;
    let reach = axis.major_tick_length.max(0.0) + band + gap;
    let [ox, oy] = groups.offset;
    let bracket_y = if below { edge + reach } else { edge - reach } + oy;
    let end = GROUP_BRACKET_END * ls.font_size;
    let toward = if below { -end } else { end };
    let index = |id: &str| categories.iter().position(|c| c.id == id);
    for group in &groups.groups {
        let (Some(first), Some(last)) = (index(&group.first), index(&group.last)) else {
            continue;
        };
        let x = |v: f64| value_to_screen(v, axis, side.clone(), plot).0;
        let (a, b) = (x(first as f64 - 0.5), x(last as f64 + 0.5));
        let (x0, x1) = (a.min(b) + inset + ox, a.max(b) - inset + ox);
        if !(x0.is_finite() && x1.is_finite() && bracket_y.is_finite()) {
            continue;
        }
        if group.bracket {
            // In the header's colour: the bracket is part of the header.
            let style = StrokeStyle {
                width: axis.line_width.max(1.0),
                color: group.label.color,
                dash: &[],
                tag: "category_group",
            };
            sink.stroke(Slot::Above, [x0, bracket_y], [x1, bracket_y], &style);
            sink.stroke(
                Slot::Above,
                [x0, bracket_y],
                [x0, bracket_y + toward],
                &style,
            );
            sink.stroke(
                Slot::Above,
                [x1, bracket_y],
                [x1, bracket_y + toward],
                &style,
            );
        }
        let extents = measure.measure_rich(&group.label);
        let baseline = if below {
            bracket_y + gap + extents.ascent
        } else {
            bracket_y - gap - extents.descent
        };
        sink.text(
            Slot::Above,
            &PlacedText {
                text: TextRef::Rich(&group.label),
                origin: ((x0 + x1) / 2.0 - extents.width / 2.0, baseline),
                extents,
                rotation: None,
            },
        );
    }
}
