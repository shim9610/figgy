//! Axis strokes: the axis line and its tick marks, placed through the sink,
//! and the value-to-screen mapping along an axis that every axis element
//! shares.

use super::*;

/// One axis: its line and ticks, then its tick labels, every part moved by
/// the detached-axis offset (`axis.line_offset`, perpendicular to the
/// axis). The data area and grid stay put; tick positions along the axis are
/// unaffected. `cross` is where the line crosses the perpendicular axis
/// ([`crate::layout::axis_cross`]); only the line moves there.
pub fn emit_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    emit_axis_strokes(axis, side.clone(), plot, cross, measure, sink);
    emit_axis_tick_labels(axis, side, plot, measure, sink);
}

/// The line and the major and minor ticks of `axis` on `side`
/// (`Element::Axis`), from the walk [`each_axis_stroke`] reports, offset by
/// the detached-axis translation. Tick stroker tags share one running index
/// per side (majors first, minors after) so every tick gets its own wobble
/// shape.
pub fn emit_axis_strokes(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let majors = major_tick_values_for_axis(axis, side.clone(), plot, measure);
    let dash = axis.line_style.pattern();
    each_axis_stroke(
        axis,
        side.clone(),
        plot,
        cross,
        measure,
        &majors,
        |stroke, a, b| {
            let tag = match stroke {
                AxisStroke::Line => format!("axis_{}", side_tag(&side)),
                AxisStroke::Tick(i) => format!("tick_{}_{i}", side_tag(&side)),
                AxisStroke::Break(k, end) => format!("axis_break_{}_{k}_{end}", side_tag(&side)),
                AxisStroke::Frame(k, end) => format!("axis_frame_{}_{k}_{end}", side_tag(&side)),
            };
            let (off_x, off_y) = match stroke {
                AxisStroke::Frame(..) => (0.0, 0.0),
                _ => (off_x, off_y),
            };
            // A break mark is drawn in its own style, solid; every other
            // stroke in the axis line's.
            let (width, color, dash) = match (stroke, &axis.breaks) {
                (
                    AxisStroke::Break(..),
                    Some(crate::config::AxisBreaks {
                        mark: crate::config::BreakMark::Slash { width, color, .. },
                        ..
                    }),
                ) => (
                    *width,
                    *color,
                    crate::line::LineStylePreset::Solid.pattern(),
                ),
                _ => (axis.line_width, axis.line_color, dash),
            };
            sink.stroke(
                Slot::Above,
                [a.0 + off_x, a.1 + off_y],
                [b.0 + off_x, b.1 + off_y],
                &StrokeStyle {
                    width,
                    color,
                    dash,
                    tag: &tag,
                },
            );
        },
    );
}

/// The tick labels of `axis` on `side` (`Element::AxisLabel`): every label
/// [`each_axis_tick_label`] walks, placed at its tick and moved by the
/// detached-axis translation; a categorical axis' labels come from
/// [`emit_category_tick_labels`].
pub fn emit_axis_tick_labels(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    if let Some(categories) = &axis.categories {
        emit_category_tick_labels(axis, categories, side, plot, measure, sink);
        return;
    }
    let majors = major_tick_values_for_axis(axis, side.clone(), plot, measure);
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_axis_tick_label(axis, side.clone(), plot, measure, &majors, |text, pos| {
        place_tick_text(&text, pos, side.clone(), axis, measure, offset, sink);
    });
}

/// The tick labels of a categorical `axis` on `side`, part of the same
/// `Element::AxisLabel` as numeric labels: `categories[i].label` at category
/// `i` ([`each_category_tick_label`]), placed by [`place_tick_text`] in the
/// axis' label style and moved by the detached-axis translation, so hover,
/// hit, selection, both drag offsets and the picked highlight are the
/// numeric labels'.
pub fn emit_category_tick_labels(
    axis: &AxisOptions,
    categories: &[crate::config::Category],
    side: Side,
    plot: &RectF,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let offset = axis_offset(side.clone(), axis.line_offset);
    each_category_tick_label(
        axis,
        categories,
        side.clone(),
        measure,
        |value| value_to_screen(value, axis, side.clone(), plot),
        |text, pos| place_tick_text(&text, pos, side.clone(), axis, measure, offset, sink),
    );
}

/// Where `value` falls along its axis ([`crate::layout::axis_fraction`]).
pub fn axis_fraction(value: f64, axis: &AxisOptions) -> f32 {
    crate::layout::axis_fraction(value, axis)
}

/// The plot rectangle of an integer data area: what the Cartesian axis,
/// tick and grid walks take. Exact for every data area (its corners are
/// integers far below 2^24).
pub fn data_area_rect(da: &DataArea) -> RectF {
    RectF::from_rect(da)
}

pub fn value_to_screen(value: f64, axis: &AxisOptions, side: Side, plot: &RectF) -> (f32, f32) {
    point_on_rect_side(axis_fraction(value, axis), &side, plot)
}

/// Which stroke of an axis a segment is: its line, or tick `i` in the
/// running per-axis index (majors first, minors after).
#[derive(Clone, Copy)]
pub enum AxisStroke {
    Line,
    Tick(usize),
    /// One slash of break mark `k` (#200): `0` at the gap's start, `1` at
    /// its end, in the axis' visual order.
    Break(usize, usize),
    /// The frame of a `Panels` piece along gap `k` when the axis has
    /// `frame_each_panel` (#200): the edge at the gap's start (`0`) or end
    /// (`1`), across the plot from the axis' side to the opposite one. Not
    /// moved by the axis' `line_offset`: it is an edge of the plot.
    Frame(usize, usize),
}

/// Every segment [`emit_axis_strokes`] places for `axis` on `side`, before
/// the detached-axis translation: the line (crossing the perpendicular axis
/// at `cross`), then the major and minor ticks at the edge.
pub fn each_axis_stroke(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    measure: &dyn MeasureText,
    majors: &[f64],
    mut each: impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    if axis.line_visible {
        let (p0, p1) = axis_anchor(side.clone(), plot, cross);
        let map = crate::layout::BreakMap::of(axis);
        if map.is_broken() {
            each_broken_axis_line(axis, &map, side.clone(), plot, p0, p1, &mut each);
        } else {
            each(AxisStroke::Line, p0, p1);
        }
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    let minors = minor_tick_values_for_axis(axis, side.clone(), plot, measure);
    let ticks = majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)));
    for (i, (value, length)) in ticks.enumerate() {
        let pos = value_to_screen(value, axis, side.clone(), plot);
        if let Some((a, b)) = tick_segment(pos, side.clone(), length, &axis.tick) {
            each(AxisStroke::Tick(i), a, b);
        }
    }
}

/// The line of a broken axis (#200) from `p0` to `p1` (its
/// [`axis_anchor`]): one segment per piece, then the slashes of each gap's
/// mark when the axis draws one, then on a `Panels` axis with
/// `frame_each_panel` the pieces' edges along each gap. The pieces' bands
/// are in visual order: from the left on a horizontal axis and from the
/// bottom on a vertical one.
fn each_broken_axis_line(
    axis: &AxisOptions,
    map: &crate::layout::BreakMap,
    side: Side,
    plot: &RectF,
    p0: (f32, f32),
    p1: (f32, f32),
    each: &mut impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    let horizontal = matches!(side, Side::Top | Side::Bottom);
    // The point at visual fraction `f` along the line: `p0` is the left end
    // of a horizontal line and the top end of a vertical one.
    let at = |f: f64| {
        let f = f as f32;
        if horizontal {
            (p0.0 + (p1.0 - p0.0) * f, p0.1)
        } else {
            (p0.0, p1.1 + (p0.1 - p1.1) * f)
        }
    };
    let visual = |(a, b): (f64, f64)| {
        if axis.inverted {
            (1.0 - b, 1.0 - a)
        } else {
            (a, b)
        }
    };
    for band in map.pieces() {
        let (a, b) = visual(band);
        each(AxisStroke::Line, at(a), at(b));
    }
    if let Some(crate::config::AxisBreaks {
        style: crate::config::AxisBreakStyle::Panels,
        frame_each_panel: true,
        ..
    }) = &axis.breaks
    {
        // From the plot's edge on the axis' side to the opposite edge, at
        // the edge lines' pixel centres (`axis_anchor`).
        let ((x0, y0), (x1, y1)) = (
            axis_anchor(Side::Left, plot, None).0,
            axis_anchor(Side::Right, plot, None).1,
        );
        for (k, knot) in map.knots().iter().enumerate() {
            let (a, b) = visual((knot.start, knot.end));
            for (end, f) in [a, b].into_iter().enumerate() {
                let (x, y) = at(f);
                let (u, v) = if horizontal {
                    ((x, y0), (x, y1))
                } else {
                    ((x0, y), (x1, y))
                };
                each(AxisStroke::Frame(k, end), u, v);
            }
        }
    }
    let Some(crate::config::AxisBreaks {
        mark: crate::config::BreakMark::Slash { size, .. },
        ..
    }) = &axis.breaks
    else {
        return;
    };
    // Each slash leans 60 degrees from the line: across it by `size`, along
    // it by half that.
    let (along, across) = (size * 0.25, size * 0.5);
    for (k, knot) in map.knots().iter().enumerate() {
        let (a, b) = visual((knot.start, knot.end));
        for (end, f) in [a, b].into_iter().enumerate() {
            let (x, y) = at(f);
            let (u, v) = if horizontal {
                ((x - along, y + across), (x + along, y - across))
            } else {
                ((x - across, y + along), (x + across, y - along))
            };
            each(AxisStroke::Break(k, end), u, v);
        }
    }
}

/// Every segment the colourbar's axis is stroked along the strip `rect`,
/// before the detached-axis translation: the line, then the ticks.
pub fn each_colorbar_axis_stroke(
    axis: &AxisOptions,
    side: Side,
    rect: &RectF,
    majors: &[f64],
    mut each: impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    // The bar's axis runs along the strip's outer long edge — the same side
    // of the strip as the bar is of the chart, which is what makes
    // `tick_segment`'s "outward = away from the data area" come out right.
    if axis.line_visible {
        let low = point_on_rect_side(0.0, &side, rect);
        let high = point_on_rect_side(1.0, &side, rect);
        each(AxisStroke::Line, low, high);
    }
    if axis.tick == TickVisibility::None {
        return;
    }
    // `major_tick_values` / `minor_tick_values` are the axis walkers, so a
    // logarithmic bar gets decades and 2a..9a minors with no extra code here.
    let minors = minor_tick_values(axis);
    let ticks = majors
        .iter()
        .map(|v| (*v, axis.major_tick_length))
        .chain(minors.iter().map(|v| (*v, axis.minor_tick_length)));
    for (i, (value, length)) in ticks.enumerate() {
        let pos = point_on_rect_side(axis_fraction(value, axis), &side, rect);
        if let Some((a, b)) = tick_segment(pos, side.clone(), length, &axis.tick) {
            each(AxisStroke::Tick(i), a, b);
        }
    }
}

/// The segment a tick at `pos` is stroked along, or `None` when ticks are
/// hidden.
pub fn tick_segment(
    pos: (f32, f32),
    side: Side,
    length: f32,
    visibility: &TickVisibility,
) -> Option<((f32, f32), (f32, f32))> {
    // outward direction = away from the data area.
    let (dx_out, dy_out) = match side {
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
    };
    let outside = (pos.0 + dx_out * length, pos.1 + dy_out * length);
    let inside = (pos.0 - dx_out * length, pos.1 - dy_out * length);
    match visibility {
        TickVisibility::None => None,
        TickVisibility::Outside => Some((pos, outside)),
        TickVisibility::Inside => Some((pos, inside)),
        TickVisibility::Both => Some((inside, outside)),
    }
}

#[cfg(test)]
mod break_tests {
    //! The chrome of a broken axis (#200): no tick inside a cut, the line
    //! drawn per piece and a slash pair at each gap.
    use super::*;
    use crate::config::{AxisBreak, AxisBreaks, BreakMark};
    use crate::default::default_axis_options_y;
    use crate::text::{RichText, TextExtents};

    struct NoMeasure;
    impl MeasureText for NoMeasure {
        fn measure_rich(&self, _: &RichText) -> TextExtents {
            TextExtents {
                width: 0.0,
                ascent: 0.0,
                descent: 0.0,
            }
        }
    }

    fn broken(mark: BreakMark) -> AxisOptions {
        let mut axis = default_axis_options_y();
        axis.min = 0.0;
        axis.max = 100.0;
        axis.major_spacing = 10.0;
        axis.minor_count = 1;
        axis.line_visible = true;
        axis.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(15.0, 85.0)],
            gap: 0.1,
            mark,
            ..AxisBreaks::default()
        });
        axis
    }

    const PLOT: RectF = RectF {
        x: 50.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    };

    #[test]
    fn no_tick_falls_inside_a_cut() {
        let axis = broken(BreakMark::None);
        let majors = major_tick_values_for_axis(&axis, Side::Left, &PLOT, &NoMeasure);
        assert_eq!(majors, vec![0.0, 10.0, 90.0, 100.0]);
        let minors = minor_tick_values_for_axis(&axis, Side::Left, &PLOT, &NoMeasure);
        assert!(
            minors.iter().all(|&v| !(v > 15.0 && v < 85.0)),
            "{minors:?}"
        );
        assert!(!minors.is_empty());
    }

    #[test]
    fn the_line_is_drawn_per_piece_with_a_slash_pair_per_gap() {
        for (mark, slashes) in [(BreakMark::None, 0), (BreakMark::default(), 2)] {
            let axis = broken(mark);
            let majors = major_tick_values_for_axis(&axis, Side::Left, &PLOT, &NoMeasure);
            let (mut lines, mut marks) = (Vec::new(), Vec::new());
            each_axis_stroke(
                &axis,
                Side::Left,
                &PLOT,
                None,
                &NoMeasure,
                &majors,
                |stroke, a, b| match stroke {
                    AxisStroke::Line => lines.push((a, b)),
                    AxisStroke::Break(k, end) => marks.push((k, end, a, b)),
                    AxisStroke::Tick(_) | AxisStroke::Frame(..) => {}
                },
            );
            assert_eq!(lines.len(), 2, "one segment per piece");
            assert_eq!(marks.len(), slashes);
            // Neither segment crosses the gap: the lower piece ends where
            // the gap starts and the upper one begins where it ends.
            let map = crate::layout::BreakMap::of(&axis);
            let knot = map.knots()[0];
            let y_at = |f: f64| PLOT.y + PLOT.height - 0.5 - (PLOT.height - 1.0) * f as f32;
            let (lower, upper) = (lines[0], lines[1]);
            assert!((lower.1.1 - y_at(knot.start)).abs() < 1e-3, "{lower:?}");
            assert!((upper.0.1 - y_at(knot.end)).abs() < 1e-3, "{upper:?}");
            for (_, end, a, b) in &marks {
                let y = if *end == 0 {
                    y_at(knot.start)
                } else {
                    y_at(knot.end)
                };
                assert!(
                    ((a.1 + b.1) / 2.0 - y).abs() < 1e-3,
                    "slash centred on its gap edge"
                );
            }
        }
    }

    /// The frames of the pieces along each gap: only on a `Panels` axis
    /// that asks for them, one across the plot at each edge of the gap.
    #[test]
    fn a_panels_axis_frames_each_piece_along_the_gap_when_asked() {
        use crate::config::AxisBreakStyle;
        for (style, frame, expected) in [
            (AxisBreakStyle::Panels, true, 2),
            (AxisBreakStyle::Panels, false, 0),
            (AxisBreakStyle::Continuous, true, 0),
        ] {
            for side in [Side::Left, Side::Bottom] {
                let mut axis = broken(BreakMark::None);
                if let Some(breaks) = axis.breaks.as_mut() {
                    breaks.style = style;
                    breaks.frame_each_panel = frame;
                }
                let mut frames = Vec::new();
                each_axis_stroke(
                    &axis,
                    side.clone(),
                    &PLOT,
                    None,
                    &NoMeasure,
                    &[],
                    |stroke, a, b| {
                        if let AxisStroke::Frame(k, end) = stroke {
                            frames.push((k, end, a, b));
                        }
                    },
                );
                assert_eq!(frames.len(), expected, "{style:?} frame {frame}");
                let knot = crate::layout::BreakMap::of(&axis).knots()[0];
                for (k, end, a, b) in frames {
                    assert_eq!(k, 0);
                    let f = if end == 0 { knot.start } else { knot.end } as f32;
                    if matches!(side, Side::Left) {
                        let y = PLOT.y + PLOT.height - 0.5 - (PLOT.height - 1.0) * f;
                        assert!((a.1 - y).abs() < 1e-3 && (b.1 - y).abs() < 1e-3);
                        assert_eq!((a.0, b.0), (PLOT.x + 0.5, PLOT.x + PLOT.width - 0.5));
                    } else {
                        let x = PLOT.x + 0.5 + (PLOT.width - 1.0) * f;
                        assert!((a.0 - x).abs() < 1e-3 && (b.0 - x).abs() < 1e-3);
                        assert_eq!((a.1, b.1), (PLOT.y + 0.5, PLOT.y + PLOT.height - 0.5));
                    }
                }
            }
        }
    }
}
