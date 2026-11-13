//! Axis strokes: the axis line and its tick marks, placed through the sink,
//! and the value-to-screen mapping along an axis that every axis element
//! shares.

use super::*;

/// One axis: its line and ticks, then its tick labels, every part moved by
/// the detached-axis offset (`axis.line_offset`, perpendicular to the
/// axis). The data area and grid stay put; tick positions along the axis are
/// unaffected. `cross` is where the line crosses the perpendicular axis
/// ([`crate::layout::axis_cross`]); only the line moves there. `across` is
/// the perpendicular axis, whose gaps a broken axis' frames stop at (#249).
/// `scale` is
/// the display or export scale `plot` and the axis' pixel values carry
/// (1 for a config drawn as stored): a broken axis snaps its piece frames
/// to whole logical pixels through it (#205).
/// `corners` are the axes whose tick labels share this axis' corners
/// ([`LabelClearance::corners`]).
#[allow(clippy::too_many_arguments)]
pub fn emit_axis(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    across: Option<&AxisOptions>,
    corners: [Option<CornerAxis<'_>>; 2],
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
    scale: f32,
) {
    emit_axis_strokes(
        axis,
        side.clone(),
        plot,
        cross,
        across,
        measure,
        sink,
        scale,
    );
    let clearance = LabelClearance {
        cross,
        scale,
        corners,
    };
    emit_axis_tick_labels_clear_of(axis, side, plot, &clearance, measure, sink);
}

/// The line and the major and minor ticks of `axis` on `side`
/// (`Element::Axis`), from the walk [`each_axis_stroke`] reports, offset by
/// the detached-axis translation. Tick stroker tags share one running index
/// per side (majors first, minors after) so every tick gets its own wobble
/// shape. `scale` is the scale the pixel values carry ([`emit_axis`]).
#[allow(clippy::too_many_arguments)]
pub fn emit_axis_strokes(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    across: Option<&AxisOptions>,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
    scale: f32,
) {
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let majors = major_tick_values_for_axis(axis, side.clone(), plot, measure);
    let dash = axis.line_style.pattern();
    each_axis_stroke(
        axis,
        side.clone(),
        plot,
        cross,
        across,
        measure,
        &majors,
        scale,
        |stroke, a, b| {
            let tag = match stroke {
                AxisStroke::Line => format!("axis_{}", side_tag(&side)),
                AxisStroke::Tick(i) => format!("tick_{}_{i}", side_tag(&side)),
                AxisStroke::Break(k, end) => format!("axis_break_{}_{k}_{end}", side_tag(&side)),
                AxisStroke::Frame(k, end) => format!("axis_frame_{}_{k}_{end}", side_tag(&side)),
            };
            // A break mark is drawn in its own width, solid, in its colour
            // or the line's; every other stroke in the axis line's style.
            let (width, color, dash) = match (stroke, &axis.breaks) {
                (
                    AxisStroke::Break(..),
                    Some(crate::config::AxisBreaks {
                        mark: crate::config::BreakMark::Slash { width, color, .. },
                        ..
                    }),
                ) => (
                    *width,
                    color.unwrap_or(axis.line_color),
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
    emit_axis_tick_labels_clear_of(axis, side, plot, &LabelClearance::LONE, measure, sink);
}

/// An axis across a rectangular chart's axis whose tick labels share one
/// of its corners (#272): the axis, its side and where its line crosses
/// ([`crate::layout::axis_cross`]).
#[derive(Clone, Copy)]
pub struct CornerAxis<'a> {
    pub axis: &'a AxisOptions,
    pub side: &'a Side,
    pub cross: Option<f32>,
}

/// What the tick labels of an axis keep clear of (#272): the slashes of its
/// own break marks, and at its two ends the tick labels of the axes across
/// it.
pub struct LabelClearance<'a> {
    /// Where the axis line crosses ([`crate::layout::axis_cross`]) and the
    /// scale its break marks snap with: [`each_axis_stroke`]'s.
    pub cross: Option<f32>,
    pub scale: f32,
    /// The axes across this one whose labels meet its labels at its ends:
    /// `bottom_x` and `top_x` across a y axis, `left_y` and `right_y`
    /// across an x axis. `None` for an axis standing alone (a parallel
    /// chart's category axes, a test's lone axis).
    pub corners: [Option<CornerAxis<'a>>; 2],
}

impl LabelClearance<'static> {
    /// An axis with no axes across it, crossing at its edge at scale 1.
    pub const LONE: LabelClearance<'static> = LabelClearance {
        cross: None,
        scale: 1.0,
        corners: [None, None],
    };
}

/// The tick labels of `axis` on `side`, as [`emit_axis_tick_labels`]
/// places them, less the labels that would collide with what `clearance`
/// names (#272), as the axis' [`LabelClearanceMode`] allows
/// (`label_style.clearance`):
///
/// - with [`LabelClearanceMode::Auto`] (the default), a label whose box
///   overlaps a break mark's slash is dropped
///   ([`each_kept_axis_tick_label`]);
/// - where two axes meet, the labels of one axis yield to the labels of
///   the other ([`yields_corner`]): with the yielding axis' `Auto`, a label
///   whose box overlaps the box of the other axis' label nearest that
///   corner is dropped, as the labels along one axis are thinned so they
///   do not collide, as long as the axis keeps at least two labels.
///   The other axis' setting does not matter there;
/// - with [`LabelClearanceMode::Keep`], the axis drops no label for
///   either.
///
/// Two boxes overlap here when they come within a small visual gap of
/// each other (`boxes_touch`): 2 px in total at the default 18 px tick
/// label font, each box's half read as a share of its own font
/// (`visual_gap_for`), so it scales with the display or export and a 2x
/// export keeps the 1x labels. Boxes the gap apart or more stay: labels
/// that look separate are kept.
///
/// Drawing, hit testing, selection and auto layout's measurement all come
/// through here, so they see the same labels.
pub fn emit_axis_tick_labels_clear_of(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    clearance: &LabelClearance<'_>,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    if let Some(categories) = &axis.categories {
        emit_category_tick_labels(axis, categories, side, plot, measure, sink);
        return;
    }
    let majors = major_tick_values_for_axis(axis, side.clone(), plot, measure);
    let offset = axis_offset(side.clone(), axis.line_offset);
    // Each corner box with its axis' font, which its half of the visual
    // gap is read from.
    let corners = clearance.corners.map(|corner| {
        corner.and_then(|corner| {
            corner_label_box(axis, &side, plot, corner, clearance.scale, measure)
                .map(|b| (b, corner.axis.label_style.font_size))
        })
    });
    let font = axis.label_style.font_size;
    // At rest on both axes: a drag of either label set moves it as one.
    let touching = |rest: &RectF| {
        corners
            .iter()
            .flatten()
            .any(|&(corner, across)| boxes_touch((rest, font), (&corner, across)))
    };
    // The corners never take an axis below two labels, as the log ladders
    // never demote below two: an axis left with one label, or none, keeps
    // its corner labels.
    let (mut count, mut yielding) = (0usize, 0usize);
    if corners.iter().any(Option::is_some) {
        each_kept_axis_tick_label(
            axis,
            side.clone(),
            plot,
            clearance.cross,
            clearance.scale,
            measure,
            &majors,
            |_, _, rest| {
                count += 1;
                yielding += usize::from(touching(&rest));
            },
        );
    }
    let yields = yielding > 0 && count - yielding >= 2;
    each_kept_axis_tick_label(
        axis,
        side.clone(),
        plot,
        clearance.cross,
        clearance.scale,
        measure,
        &majors,
        |text, pos, rest| {
            if yields && touching(&rest) {
                return;
            }
            place_tick_text(&text, pos, side.clone(), axis, measure, offset, sink);
        },
    );
}

/// Every tick label [`each_axis_tick_label`] walks for `axis` on `side`,
/// with its box where it rests: before the label offsets a drag writes
/// (`label_offset_x`, `label_offset_y`) and the detached-axis translation,
/// and with the fixed [`crate::layout::LABEL_GAP`] past the tick read as
/// the share of the font it is of the default tick label font
/// (`LABEL_GAP_FONT`, 18 px) and its height read from the font
/// (0.7 em, the height of its digits): the gap and the glyphs' rounded extents
/// are what an export's scale does not scale, so read this way a 2x
/// export keeps the labels the 1x one keeps.
/// With the axis' [`LabelClearanceMode::Auto`] (the default), the labels
/// whose resting box overlaps one of the axis' break mark slashes
/// ([`each_break_mark_box`]; `cross` and `scale` are
/// [`each_axis_stroke`]'s), or comes within the visual gap of it read
/// from the label's font (`boxes_touch`), are left out; with
/// [`LabelClearanceMode::Keep`] none is. Which labels stay is decided
/// at rest, so a drag moves the drawn set as one, as it moves the box,
/// the hit region and the selection with it (#272).
#[allow(clippy::too_many_arguments)]
pub fn each_kept_axis_tick_label(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    scale: f32,
    measure: &dyn MeasureText,
    majors: &[f64],
    mut each: impl FnMut(TickText<'_>, (f32, f32), RectF),
) {
    let ls = &axis.label_style;
    let keep = ls.clearance == LabelClearanceMode::Keep;
    each_axis_tick_label(axis, side.clone(), plot, measure, majors, |text, pos| {
        let (origin, extents) = tick_label_placement(&text, pos, side.clone(), axis, measure);
        let gap_in = crate::layout::LABEL_GAP * (1.0 - ls.font_size / LABEL_GAP_FONT);
        let unscaled = match side {
            Side::Bottom => (0.0, -gap_in),
            Side::Top => (0.0, gap_in),
            Side::Left => (gap_in, 0.0),
            Side::Right => (-gap_in, 0.0),
        };
        let drawn = crate::layout::label_rect(
            (
                origin.0 - ls.label_offset_x + unscaled.0,
                origin.1 - ls.label_offset_y + unscaled.1,
            ),
            extents,
        );
        // Its height read from the font, from the edge or the middle that
        // is placed by the tick: glyph extents round to whole pixels at
        // each scale, the font size does not.
        let height = label_decision_height(ls.font_size);
        let y = match side {
            Side::Bottom => drawn.y,
            Side::Top => drawn.y + drawn.height - height,
            Side::Left | Side::Right => drawn.y + (drawn.height - height) * 0.5,
        };
        let rest = RectF {
            x: drawn.x,
            y,
            width: drawn.width,
            height,
        };
        let mut clear = true;
        if !keep {
            each_break_mark_box(axis, &side, plot, cross, scale, |mark| {
                clear &= !boxes_touch((&rest, ls.font_size), (&mark, ls.font_size));
            });
        }
        if clear {
            each(text, pos, rest);
        }
    });
}

/// The tick label font size [`crate::layout::LABEL_GAP`] is set for: the
/// default tick label font.
const LABEL_GAP_FONT: f32 = 18.0;

/// The height a tick label of font size `font` is read with where it
/// decides what it keeps clear of (#272): the height of its digits,
/// 0.7 em. Read from the font size, it is the same share of the font at
/// every export scale, where the glyphs' measured extents round to whole
/// pixels.
pub(crate) fn label_decision_height(font: f32) -> f32 {
    font.max(0.0) * 0.7
}

/// Whether the labels of `axis` on `side` yield where they meet the labels
/// of `across` (#272): a categorical axis never does (every category is
/// named, #205); otherwise a y axis yields to the x axis, and an x axis
/// only to a categorical y axis. So at a corner exactly one axis yields,
/// or neither when both are categorical. Whether the yielding axis drops
/// a label there is its own [`LabelClearanceMode`]'s call
/// ([`emit_axis_tick_labels_clear_of`]).
pub fn yields_corner(axis: &AxisOptions, side: &Side, across: &AxisOptions) -> bool {
    axis.categories.is_none()
        && (matches!(side, Side::Left | Side::Right) || across.categories.is_some())
}

/// The resting box ([`each_kept_axis_tick_label`]) of the label of
/// `corner` that reaches farthest toward `side` (#272): the leftmost label
/// of an x axis for a left axis, the lowest label of a y axis for a bottom
/// axis. The labels of one axis keep clear of each other, so that label is
/// the one nearest the corner. `None` when `axis` does not yield to it
/// ([`yields_corner`]), when `axis` keeps every label
/// ([`LabelClearanceMode::Keep`]), or when `corner` draws no label.
/// `scale` is the one its break marks snap with ([`each_axis_stroke`]).
fn corner_label_box(
    axis: &AxisOptions,
    side: &Side,
    plot: &RectF,
    corner: CornerAxis<'_>,
    scale: f32,
    measure: &dyn MeasureText,
) -> Option<RectF> {
    if axis.label_style.clearance == LabelClearanceMode::Keep
        || !yields_corner(axis, side, corner.axis)
    {
        return None;
    }
    let across = corner.axis;
    let majors = major_tick_values_for_axis(across, corner.side.clone(), plot, measure);
    // How far a box reaches toward `side`: the larger, the nearer.
    let reach = |b: &RectF| match side {
        Side::Left => -b.x,
        Side::Right => b.x + b.width,
        Side::Top => -b.y,
        Side::Bottom => b.y + b.height,
    };
    let mut nearest: Option<RectF> = None;
    each_kept_axis_tick_label(
        across,
        corner.side.clone(),
        plot,
        corner.cross,
        scale,
        measure,
        &majors,
        |_, _, rest| {
            if nearest.is_none_or(|b| reach(&rest) > reach(&b)) {
                nearest = Some(rest);
            }
        },
    );
    nearest
}

/// The visual gap two tick labels in the default tick label font
/// (`LABEL_GAP_FONT`, 18 px) keep where they decide what they keep clear
/// of (#272): 2 px in total, between the two boxes. Closer than that, the
/// eye reads them as touching.
pub(crate) const LABEL_VISUAL_GAP: f32 = 2.0;

/// The visual gap (`LABEL_VISUAL_GAP`, 2 px) between a decision box of a tick
/// label in an `a_font` px font and one in a `b_font` px font (#272): the
/// sum of each box's half of the gap, read as the share of its own font it
/// is of the default 18 px font,
/// `LABEL_VISUAL_GAP / 2 * (a_font + b_font) / 18`. So it is 2 px between
/// two 18 px labels and symmetric in the two fonts. A break mark's slash
/// is read with the font of the labels it cuts.
///
/// The labels' gap past the tick and their glyphs' rounded extents are what
/// an export's scale does not scale, and the model's walks do not know the
/// display or export scale `s` (hit testing reads a config with no display
/// scale); the fonts are scaled with the config, so read from the fonts
/// the gap is `s` times the 1x gap wherever the labels are decided, as the
/// gap past the tick is ([`each_kept_axis_tick_label`]). So a 2x export
/// keeps the labels the 1x one keeps. The renderer's radar value labels
/// keep the same gap from each other.
pub fn visual_gap_for(a_font: f32, b_font: f32) -> f32 {
    0.5 * LABEL_VISUAL_GAP * (a_font.max(0.0) + b_font.max(0.0)) / LABEL_GAP_FONT
}

/// Whether two decision boxes, each with the font size its half of the gap
/// is read from ([`visual_gap_for`]), touch to the eye (#272): whether they
/// come closer than the visual gap both across and along the screen
/// ([`boxes_within`]). So overlapping boxes and boxes 1 px apart at 18 px
/// touch; boxes exactly the gap apart, or farther, along either direction
/// do not.
pub(crate) fn boxes_touch((a, a_font): (&RectF, f32), (b, b_font): (&RectF, f32)) -> bool {
    boxes_within(a, b, visual_gap_for(a_font, b_font))
}

/// Whether boxes `a` and `b` come closer than `gap` both across and along
/// the screen (with `gap` 0: whether they overlap).
pub(crate) fn boxes_within(a: &RectF, b: &RectF, gap: f32) -> bool {
    a.x < b.x + b.width + gap
        && b.x < a.x + a.width + gap
        && a.y < b.y + b.height + gap
        && b.y < a.y + a.height + gap
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

/// Where `value` falls along its axis ([`crate::layout::axis_fraction`])
/// drawn along `side` of `plot`.
pub fn axis_fraction(value: f64, axis: &AxisOptions, side: &Side, plot: &RectF) -> f32 {
    crate::layout::axis_fraction(value, axis, axis_len(side, plot))
}

/// The drawn length of an axis on `side` of `plot` (#249): the plot's
/// width for an x axis, its height for a y axis.
pub fn axis_len(side: &Side, plot: &RectF) -> f32 {
    match side {
        Side::Top | Side::Bottom => plot.width,
        Side::Left | Side::Right => plot.height,
    }
}

/// The plot rectangle of an integer data area: what the Cartesian axis,
/// tick and grid walks take. Exact for every data area (its corners are
/// integers far below 2^24).
pub fn data_area_rect(da: &DataArea) -> RectF {
    RectF::from_rect(da)
}

pub fn value_to_screen(value: f64, axis: &AxisOptions, side: Side, plot: &RectF) -> (f32, f32) {
    point_on_rect_side(axis_fraction(value, axis, &side, plot), &side, plot)
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
    /// The frame of a piece along gap `k` when the axis has
    /// `frame_each_panel` (#200): the edge at the gap's start (`0`) or end
    /// (`1`), across the plot from the axis' side to the opposite one. Part
    /// of the axis element, so it moves with the axis' `line_offset`.
    Frame(usize, usize),
}

/// Every segment [`emit_axis_strokes`] places for `axis` on `side`, before
/// the detached-axis translation: the line (crossing the perpendicular axis
/// at `cross`), then the major and minor ticks at the edge. `across` is the
/// perpendicular axis (`bottom_x` for a y axis, `left_y` for an x axis):
/// a broken axis' piece frames stop at its gaps (#249). `scale` is the
/// scale the pixel values carry: a broken axis' piece frames are snapped to
/// whole logical pixels, then scaled (#205), so a 2x frame is the 1x frame
/// doubled; nothing else reads it.
#[allow(clippy::too_many_arguments)]
pub fn each_axis_stroke(
    axis: &AxisOptions,
    side: Side,
    plot: &RectF,
    cross: Option<f32>,
    across: Option<&AxisOptions>,
    measure: &dyn MeasureText,
    majors: &[f64],
    scale: f32,
    mut each: impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    if axis.line_visible {
        let (p0, p1) = axis_anchor(side.clone(), plot, cross);
        let map = crate::layout::BreakMap::of(axis, axis_len(&side, plot));
        if map.is_broken() {
            each_broken_axis_line(
                axis,
                &map,
                side.clone(),
                plot,
                (p0, p1),
                across,
                scale,
                &mut each,
            );
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
    let map = crate::layout::BreakMap::of(axis, axis_len(&side, plot));
    for (i, (value, length)) in ticks.enumerate() {
        let mut pos = value_to_screen(value, axis, side.clone(), plot);
        // A tick on a piece's end stands on that piece's frame row (#249).
        let f = f64::from(axis_fraction(value, axis, &side, plot));
        if let Some(v) = gap_edge_pixel(axis, &map, &side, plot, scale, f) {
            match side {
                Side::Top | Side::Bottom => pos.0 = v,
                Side::Left | Side::Right => pos.1 = v,
            }
        }
        if let Some((a, b)) = tick_segment(pos, side.clone(), length, &axis.tick) {
            each(AxisStroke::Tick(i), a, b);
        }
    }
}

/// The centre of a stroke `width` config pixels wide at pixel edge `v`, on
/// the logical pixel grid of `scale` and inside the piece that lies in pixel
/// direction `dir` (+1 right or down, -1 left or up) (#249).
fn snap_into_piece(width: f32, v: f32, dir: f32, scale: f32) -> f32 {
    let half = (width / scale).max(1.0) * 0.5;
    ((v / scale).round() + dir * half) * scale
}

/// `scale` when it is a usable display or export scale, else 1.
fn usable_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// Where a tick at visual fraction `f` of a broken `axis` on `side` of
/// `plot` stands when `f` is a gap edge: on the row (or column) of that
/// piece's frame, inside the piece (#249). `None` off the gap edges.
fn gap_edge_pixel(
    axis: &AxisOptions,
    map: &crate::layout::BreakMap,
    side: &Side,
    plot: &RectF,
    scale: f32,
    f: f64,
) -> Option<f32> {
    if !map.is_broken() {
        return None;
    }
    let horizontal = matches!(side, Side::Top | Side::Bottom);
    let scale = usable_scale(scale);
    for knot in map.knots() {
        let (a, b) = if axis.inverted {
            (1.0 - knot.end, 1.0 - knot.start)
        } else {
            (knot.start, knot.end)
        };
        for (closes, edge) in [(true, a), (false, b)] {
            if (f - edge).abs() > 1e-6 {
                continue;
            }
            let edge = edge as f32;
            return Some(if horizontal {
                let dir = if closes { -1.0 } else { 1.0 };
                snap_into_piece(axis.line_width, plot.x + plot.width * edge, dir, scale)
            } else {
                let dir = if closes { 1.0 } else { -1.0 };
                snap_into_piece(
                    axis.line_width,
                    plot.y + plot.height * (1.0 - edge),
                    dir,
                    scale,
                )
            });
        }
    }
    None
}

/// The line of a broken axis (#200) from `p0` to `p1` (its
/// [`axis_anchor`]): one segment per piece, then the slashes of each gap's
/// mark when the axis draws one, then with `frame_each_panel` the pieces'
/// edges along each gap, each cut at the gaps of the broken axis `across`
/// (#249), so the gaps of two broken axes meet as a cross. The pieces' bands
/// are in visual order: from the left on a horizontal axis and from the
/// bottom on a vertical one. A frame is snapped to whole logical pixels
/// (`scale` logical pixels per config pixel) and then scaled (#205): its
/// stroke covers whole pixels at 1x instead of blurring over two rows, and
/// at 2x it is the 1x stroke doubled.
#[allow(clippy::too_many_arguments)]
fn each_broken_axis_line(
    axis: &AxisOptions,
    map: &crate::layout::BreakMap,
    side: Side,
    plot: &RectF,
    (p0, p1): ((f32, f32), (f32, f32)),
    across: Option<&AxisOptions>,
    scale: f32,
    each: &mut impl FnMut(AxisStroke, (f32, f32), (f32, f32)),
) {
    let horizontal = matches!(side, Side::Top | Side::Bottom);
    // The point at visual fraction `f` along the line: `p0` is the left end
    // of a horizontal line and the top end of a vertical one.
    let visual = |(a, b): (f64, f64)| {
        if axis.inverted {
            (1.0 - b, 1.0 - a)
        } else {
            (a, b)
        }
    };
    // The display or export scale the frames and the pieces' ends snap to:
    // a gap edge sits on the logical pixel grid, so the line, the frames and
    // a 2x export (the 1x one doubled) all end on the same pixel (#249).
    let scale = usable_scale(scale);
    // The pixel of visual fraction `f` across the plot (x, left to right)
    // or up it (y, bottom to top): where the data pieces end.
    let plot_x = |f: f64| plot.x + plot.width * f as f32;
    let plot_y = |f: f64| plot.y + plot.height * (1.0 - f as f32);
    let end_at = |f: f64| piece_end(horizontal, plot, (p0, p1), scale, f);
    for band in map.pieces() {
        let (a, b) = visual(band);
        each(AxisStroke::Line, end_at(a), end_at(b));
    }
    if let Some(crate::config::AxisBreaks {
        frame_each_panel: true,
        ..
    }) = &axis.breaks
    {
        // From the axis' edge line (`axis_anchor`) to the opposite one,
        // stopping a line width short of the opposite line's centre (the
        // width scales with the export): the frame meets that line
        // without sharing its ink, so each pixel belongs to one axis.
        let ((mut x0, mut y0), (mut x1, mut y1)) = (
            axis_anchor(Side::Left, plot, None).0,
            axis_anchor(Side::Right, plot, None).1,
        );
        let short = axis.line_width.max(0.5);
        match side {
            Side::Left => x1 -= short,
            Side::Right => x0 += short,
            Side::Bottom => y0 += short,
            Side::Top => y1 -= short,
        }
        // A frame stands on its piece's edge in the plot (where the data
        // pieces end, `BreakMap` over the plot's width or height), on the
        // logical pixel grid and inside its own piece: a stroke of `w`
        // logical pixels centred at `k ± w / 2` covers whole pixels (the
        // raster floors a thinner stroke to one pixel), the same grid
        // scaled keeps 2x the 1x frame doubled, and both sides of a gap
        // keep the gap's own width between their frames (#249).
        // `dir` is the pixel direction into the piece: +1 right or down.
        let snap_in = |width: f32, v: f32, dir: f32| snap_into_piece(width, v, dir, scale);
        // The perpendicular axis' pieces along a frame, as `(from, to)` in
        // its visual fractions (0 at the left or bottom): the frame is
        // drawn over each piece and stops at each gap. An unbroken `across`
        // is one piece.
        let across_side = if horizontal { Side::Left } else { Side::Bottom };
        let other = across
            .map(|o| crate::layout::BreakMap::of(o, axis_len(&across_side, plot)))
            .filter(crate::layout::BreakMap::is_broken);
        let other_visual = |(a, b): (f64, f64)| match across {
            Some(o) if o.inverted => (1.0 - b, 1.0 - a),
            _ => (a, b),
        };
        // Where visual fraction `f` of `across` falls along a frame, on the
        // perpendicular axis' own frame grid when it frames its pieces, so
        // the two frames meet; the plot's ends stay the frame's own ends.
        let other_framed = across
            .and_then(|o| o.breaks.as_ref())
            .is_some_and(|b| b.frame_each_panel);
        let other_width = across.map_or(axis.line_width, |o| o.line_width);
        // `start` is whether `f` starts its piece (the piece lies above or
        // to the right of it) or ends it.
        let along = |f: f64, from: f32, to: f32, start: bool| {
            if f <= 0.0 {
                return from;
            }
            if f >= 1.0 {
                return to;
            }
            let (v, dir) = if horizontal {
                // A vertical frame runs along the y axis, bottom to top.
                (plot_y(f), if start { -1.0 } else { 1.0 })
            } else {
                (plot_x(f), if start { 1.0 } else { -1.0 })
            };
            if other_framed {
                snap_in(other_width, v, dir)
            } else {
                v
            }
        };
        for (k, knot) in map.knots().iter().enumerate() {
            let (a, b) = visual((knot.start, knot.end));
            for (end, f) in [a, b].into_iter().enumerate() {
                // End 0 closes the piece before the gap (left or below),
                // end 1 opens the one after it.
                let closes = end == 0;
                let (u, v) = if horizontal {
                    let dir = if closes { -1.0 } else { 1.0 };
                    let x = snap_in(axis.line_width, plot_x(f), dir);
                    ((x, y0), (x, y1))
                } else {
                    let dir = if closes { 1.0 } else { -1.0 };
                    let y = snap_in(axis.line_width, plot_y(f), dir);
                    ((x0, y), (x1, y))
                };
                let Some(other) = &other else {
                    each(AxisStroke::Frame(k, end), u, v);
                    continue;
                };
                for piece in other.pieces() {
                    let (from, to) = other_visual(piece);
                    if horizontal {
                        // From the bottom (`y1`) to the top (`y0`).
                        let (lo, hi) = (along(from, y1, y0, true), along(to, y1, y0, false));
                        each(AxisStroke::Frame(k, end), (u.0, hi), (u.0, lo));
                    } else {
                        let (lo, hi) = (along(from, x0, x1, true), along(to, x0, x1, false));
                        each(AxisStroke::Frame(k, end), (lo, u.1), (hi, u.1));
                    }
                }
            }
        }
    }
    each_break_slash(
        axis,
        map,
        horizontal,
        plot,
        (p0, p1),
        scale,
        |k, end, u, v| each(AxisStroke::Break(k, end), u, v),
    );
}

/// Where a piece of a broken axis' line ends at visual fraction `f`
/// (#200): the line's own end at the plot's edges, else the gap edge on the
/// logical pixel grid of `scale` (a [`usable_scale`]) inside the plot.
/// `(p0, p1)` is the line's [`axis_anchor`]: `p0` the left end of a
/// horizontal line and the top end of a vertical one.
fn piece_end(
    horizontal: bool,
    plot: &RectF,
    (p0, p1): ((f32, f32), (f32, f32)),
    scale: f32,
    f: f64,
) -> (f32, f32) {
    let grid = |v: f32| (v / scale).round() * scale;
    let t = f as f32;
    if f <= 0.0 || f >= 1.0 {
        return if horizontal {
            (p0.0 + (p1.0 - p0.0) * t, p0.1)
        } else {
            (p0.0, p1.1 + (p0.1 - p1.1) * t)
        };
    }
    if horizontal {
        (grid(plot.x + plot.width * t), p0.1)
    } else {
        (p0.0, grid(plot.y + plot.height * (1.0 - t)))
    }
}

/// Every slash of the break marks of the broken `axis` (`map` its
/// [`crate::layout::BreakMap`]) along its line `(p0, p1)`: the slash of
/// gap `k` at the gap's start (`end` 0) or end (`end` 1), in the axis'
/// visual order, from `u` to `v`, before the detached-axis translation.
/// Nothing when the axis draws no mark. The stroke walk draws these
/// slashes and the tick labels keep clear of them (#272), so both read one
/// placement.
fn each_break_slash(
    axis: &AxisOptions,
    map: &crate::layout::BreakMap,
    horizontal: bool,
    plot: &RectF,
    (p0, p1): ((f32, f32), (f32, f32)),
    scale: f32,
    mut each: impl FnMut(usize, usize, (f32, f32), (f32, f32)),
) {
    let size = match axis.breaks.as_ref().map(|breaks| &breaks.mark) {
        Some(crate::config::BreakMark::Slash {
            size,
            color: _,
            width: _,
        }) => *size,
        Some(crate::config::BreakMark::None) | None => return,
    };
    // Each slash leans 60 degrees from the line: across it by `size`, along
    // it by half that.
    let (along, across) = (size * 0.25, size * 0.5);
    for (k, knot) in map.knots().iter().enumerate() {
        let (a, b) = if axis.inverted {
            (1.0 - knot.end, 1.0 - knot.start)
        } else {
            (knot.start, knot.end)
        };
        for (end, f) in [a, b].into_iter().enumerate() {
            // Centred on the piece's end, where the line stops.
            let (x, y) = piece_end(horizontal, plot, (p0, p1), scale, f);
            let (u, v) = if horizontal {
                ((x - along, y + across), (x + along, y - across))
            } else {
                ((x - across, y + along), (x + across, y - along))
            };
            each(k, end, u, v);
        }
    }
}

/// The box around every break-mark slash of `axis` on `side` of `plot`
/// (#272), as [`each_axis_stroke`] places them (`cross` and `scale` are
/// its), before the detached-axis translation: each slash's end points
/// grown by half its stroke width. Nothing when the axis line is hidden,
/// unbroken or draws no mark.
pub fn each_break_mark_box(
    axis: &AxisOptions,
    side: &Side,
    plot: &RectF,
    cross: Option<f32>,
    scale: f32,
    mut each: impl FnMut(RectF),
) {
    let width = match axis.breaks.as_ref().map(|breaks| &breaks.mark) {
        Some(crate::config::BreakMark::Slash {
            size: _,
            color: _,
            width,
        }) => *width,
        Some(crate::config::BreakMark::None) | None => return,
    };
    if !axis.line_visible {
        return;
    }
    let map = crate::layout::BreakMap::of(axis, axis_len(side, plot));
    if !map.is_broken() {
        return;
    }
    let horizontal = matches!(side, Side::Top | Side::Bottom);
    let line = axis_anchor(side.clone(), plot, cross);
    let half = width.max(0.0) * 0.5;
    each_break_slash(
        axis,
        &map,
        horizontal,
        plot,
        line,
        usable_scale(scale),
        |_, _, u, v| {
            each(RectF {
                x: u.0.min(v.0) - half,
                y: u.1.min(v.1) - half,
                width: (u.0 - v.0).abs() + 2.0 * half,
                height: (u.1 - v.1).abs() + 2.0 * half,
            })
        },
    );
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
        let pos = point_on_rect_side(axis_fraction(value, axis, &side, rect), &side, rect);
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
        let mut axis = crate::defaults::axis_options_y();
        axis.min = 0.0;
        axis.max = 100.0;
        axis.major_spacing = 10.0;
        axis.minor_count = 1;
        axis.line_visible = true;
        axis.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(15.0, 85.0)],
            gap_px: 20.0,
            mark,
            ..crate::defaults::axis_breaks()
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
        for (mark, slashes) in [(BreakMark::None, 0), (crate::defaults::break_mark(), 2)] {
            let axis = broken(mark);
            let majors = major_tick_values_for_axis(&axis, Side::Left, &PLOT, &NoMeasure);
            let (mut lines, mut marks) = (Vec::new(), Vec::new());
            each_axis_stroke(
                &axis,
                Side::Left,
                &PLOT,
                None,
                None,
                &NoMeasure,
                &majors,
                1.0,
                |stroke, a, b| match stroke {
                    AxisStroke::Line => lines.push((a, b)),
                    AxisStroke::Break(k, end) => marks.push((k, end, a, b)),
                    AxisStroke::Tick(_) | AxisStroke::Frame(..) => {}
                },
            );
            assert_eq!(lines.len(), 2, "one segment per piece");
            assert_eq!(marks.len(), slashes);
            // Neither segment crosses the gap: the lower piece ends where
            // the gap starts and the upper one begins where it ends, each
            // on the pixel grid where the data pieces end (#249).
            let map = crate::layout::BreakMap::of(&axis, PLOT.height);
            let knot = map.knots()[0];
            let y_at = |f: f64| (PLOT.y + PLOT.height * (1.0 - f as f32)).round();
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

    /// Crowded labels on a broken axis are thinned on the unbroken walk:
    /// every piece keeps round multiples of the kept step, not a cadence
    /// carried over from the piece below.
    #[test]
    fn each_piece_keeps_the_round_labels_of_the_unbroken_axis() {
        struct Tall;
        impl MeasureText for Tall {
            fn measure_rich(&self, _: &RichText) -> TextExtents {
                TextExtents {
                    width: 20.0,
                    ascent: 8.0,
                    descent: 2.0,
                }
            }
        }
        let mut axis = broken(BreakMark::None);
        axis.major_spacing = 0.2;
        if let Some(breaks) = axis.breaks.as_mut() {
            breaks.ranges = vec![AxisBreak::new(10.0, 90.0)];
        }
        let majors = major_tick_values_for_axis(&axis, Side::Left, &PLOT, &Tall);
        let labels =
            crate::chrome::visible_numeric_labels(&axis, Side::Left, &Tall, &majors, |v| {
                value_to_screen(v, &axis, Side::Left, &PLOT)
            });
        let values: Vec<f64> = labels.iter().map(|l| l.value).collect();
        assert!(values.len() >= 4, "{values:?}");
        let step = values[1] - values[0];
        assert!(step > 0.2, "the labels were thinned: {values:?}");
        assert!(values.iter().any(|&v| v > 90.0), "{values:?}");
        for v in &values {
            let k = v / step;
            assert!(
                (k - k.round()).abs() < 1e-6,
                "{v} is no multiple of {step}: {values:?}"
            );
        }
    }

    /// Grid lines across a broken axis are drawn per piece: none crosses a
    /// gap, and every line reaches both outer edges.
    #[test]
    fn grid_lines_leave_the_gaps_empty() {
        let mut config = crate::defaults::config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.bottom_x.major_spacing = 1.0;
        config.left_y = broken(BreakMark::None);
        config.grid.show_major_x = true;
        config.grid.show_major_y = true;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
        let map = crate::layout::BreakMap::of(&config.left_y, PLOT.height);
        let knot = map.knots()[0];
        let gap = (
            PLOT.y + PLOT.height * (1.0 - knot.end as f32),
            PLOT.y + PLOT.height * (1.0 - knot.start as f32),
        );
        let mut segments = Vec::new();
        crate::chrome::grid::emit_grid(
            &config,
            &PLOT,
            &NoMeasure,
            &mut crate::sink::Parts(&mut |part| {
                if let crate::sink::DrawnPart::Segment { a, b, .. } = part {
                    segments.push((a, b));
                }
            }),
            1.0,
        );
        let vertical: Vec<_> = segments.iter().filter(|(a, b)| a[0] == b[0]).collect();
        assert!(!vertical.is_empty());
        for (a, b) in &vertical {
            let (lo, hi) = (a[1].min(b[1]), a[1].max(b[1]));
            assert!(
                hi <= gap.0 + 1e-3 || lo >= gap.1 - 1e-3,
                "{a:?}-{b:?} crosses {gap:?}"
            );
        }
        // Each vertical line is two segments, one per piece.
        assert_eq!(vertical.len() % 2, 0);
        assert!(vertical.iter().any(|(a, _)| (a[1] - PLOT.y).abs() < 1e-3));
        assert!(
            vertical
                .iter()
                .any(|(_, b)| (b[1] - (PLOT.y + PLOT.height)).abs() < 1e-3)
        );
    }

    /// A top or right axis is drawn with its partner's cuts at the same
    /// raw fractions, in its own units and direction: one piece layout,
    /// no tick in a gap.
    #[test]
    fn a_top_or_right_axis_takes_its_partners_cuts() {
        let mut config = crate::defaults::config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 100.0;
        config.bottom_x.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(20.0, 70.0)],
            ..crate::defaults::axis_breaks()
        });
        // The top axis in other units (twice the bottom's), the right axis
        // mirroring an unbroken left axis.
        config.top_x.min = 0.0;
        config.top_x.max = 200.0;
        config.top_x.major_spacing = 20.0;
        for inverted in [false, true] {
            config.top_x.inverted = inverted;
            let top = crate::chrome::drawn_axis(&config, &Side::Top);
            let cuts = &top.breaks.as_ref().unwrap().ranges;
            let expected = if inverted {
                (60.0, 160.0)
            } else {
                (40.0, 140.0)
            };
            assert!(
                (cuts[0].from - expected.0).abs() < 1e-9 && (cuts[0].to - expected.1).abs() < 1e-9,
                "{cuts:?}"
            );
            let (a, b) = (
                crate::layout::BreakMap::of(&top, PLOT.width),
                crate::layout::BreakMap::of(&config.bottom_x, PLOT.width),
            );
            // The same gap on screen: mirrored back on an inverted axis.
            let (ka, kb) = (a.knots()[0], b.knots()[0]);
            let visual = if inverted {
                (1.0 - ka.end, 1.0 - ka.start)
            } else {
                (ka.start, ka.end)
            };
            assert!(
                (visual.0 - kb.start).abs() < 1e-9 && (visual.1 - kb.end).abs() < 1e-9,
                "one piece layout: {ka:?} vs {kb:?}"
            );
            let majors = major_tick_values_for_axis(&top, Side::Top, &PLOT, &NoMeasure);
            assert!(
                majors.iter().all(|&v| !(v > expected.0 && v < expected.1)),
                "{majors:?}"
            );
            assert!(!top.breaks.as_ref().unwrap().frame_each_panel);
        }
        let right = crate::chrome::drawn_axis(&config, &Side::Right);
        assert_eq!(right.breaks, None, "the left axis is unbroken");
        // An axis with cuts of its own keeps them.
        config.top_x.breaks = Some(crate::defaults::axis_breaks());
        assert_eq!(
            crate::chrome::drawn_axis(&config, &Side::Top).breaks,
            Some(crate::defaults::axis_breaks())
        );
    }

    /// The frames of the pieces along each gap, in either style when the
    /// axis asks for them: one across the plot at each edge of the gap.
    #[test]
    fn a_broken_axis_frames_each_piece_along_the_gap_when_asked() {
        use crate::config::AxisBreakStyle;
        for (style, frame, expected) in [
            (AxisBreakStyle::Panels, true, 2),
            (AxisBreakStyle::Panels, false, 0),
            (AxisBreakStyle::Continuous, true, 2),
            (AxisBreakStyle::Continuous, false, 0),
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
                    None,
                    &NoMeasure,
                    &[],
                    1.0,
                    |stroke, a, b| {
                        if let AxisStroke::Frame(k, end) = stroke {
                            frames.push((k, end, a, b));
                        }
                    },
                );
                // A frame stands on its piece's edge in the plot, snapped to
                // the pixel grid (#205) and inside its own piece (#249).
                let half = axis.line_width.max(1.0) * 0.5;
                assert_eq!(frames.len(), expected, "{style:?} frame {frame}");
                let len = axis_len(&side, &PLOT);
                let knot = crate::layout::BreakMap::of(&axis, len).knots()[0];
                let mut centres = [0.0f32; 2];
                for (k, end, a, b) in frames {
                    assert_eq!(k, 0);
                    let f = if end == 0 { knot.start } else { knot.end } as f32;
                    let into = if end == 0 { -1.0 } else { 1.0 };
                    if matches!(side, Side::Left) {
                        let y = (PLOT.y + PLOT.height * (1.0 - f)).round() - into * half;
                        centres[end] = y;
                        assert!((a.1 - y).abs() < 1e-3 && (b.1 - y).abs() < 1e-3);
                        let short = axis.line_width;
                        assert_eq!(
                            (a.0, b.0),
                            (PLOT.x + 0.5, PLOT.x + PLOT.width - 0.5 - short)
                        );
                    } else {
                        let x = (PLOT.x + PLOT.width * f).round() + into * half;
                        centres[end] = x;
                        assert!((a.0 - x).abs() < 1e-3 && (b.0 - x).abs() < 1e-3);
                        let short = axis.line_width;
                        assert_eq!(
                            (a.1, b.1),
                            (PLOT.y + 0.5 + short, PLOT.y + PLOT.height - 0.5)
                        );
                    }
                }
                // The clear space between the two frames is the gap's own
                // width in pixels, to the rounding of its two edges.
                if expected > 0 {
                    let clear = (centres[1] - centres[0]).abs() - 2.0 * half;
                    let gap_px = axis.breaks.as_ref().unwrap().gap_px;
                    assert!(
                        (clear - gap_px).abs() <= 1.0,
                        "{side:?}: {clear} vs {gap_px}"
                    );
                }
            }
        }
    }

    /// #249: when both axes are broken, each frame stops at the other
    /// axis' gap, on that axis' frame, so the gaps meet as a cross: one
    /// frame segment per piece of the other axis, none inside its gap.
    #[test]
    fn frames_of_two_broken_axes_meet_as_a_cross() {
        let y = broken(BreakMark::None);
        let mut x = broken(BreakMark::None);
        x.line_width = 1.0;
        let collect = |axis: &AxisOptions, side: Side, across: &AxisOptions| {
            let mut frames = Vec::new();
            each_axis_stroke(
                axis,
                side,
                &PLOT,
                None,
                Some(across),
                &NoMeasure,
                &[],
                1.0,
                |stroke, a, b| {
                    if let AxisStroke::Frame(k, end) = stroke {
                        frames.push((k, end, a, b));
                    }
                },
            );
            frames
        };
        let half = y.line_width.max(1.0) * 0.5;
        let knot = crate::layout::BreakMap::of(&x, PLOT.width).knots()[0];
        let y_knot = crate::layout::BreakMap::of(&y, PLOT.height).knots()[0];
        // The y axis' frames run along x: two frames, each cut at x's gap.
        let y_frames = collect(&y, Side::Left, &x);
        assert_eq!(y_frames.len(), 4, "{y_frames:?}");
        // x's frames, each inside its own piece (#249).
        let gap_x = (
            (PLOT.x + PLOT.width * knot.start as f32).round() - half,
            (PLOT.x + PLOT.width * knot.end as f32).round() + half,
        );
        for (_, _, a, b) in &y_frames {
            let (lo, hi) = (a.0.min(b.0), a.0.max(b.0));
            assert!(
                hi <= gap_x.0 + 1e-3 || lo >= gap_x.1 - 1e-3,
                "{a:?} {b:?} in {gap_x:?}"
            );
        }
        assert!(
            y_frames
                .iter()
                .any(|(_, _, _, b)| (b.0 - gap_x.0).abs() < 1e-3)
        );
        assert!(
            y_frames
                .iter()
                .any(|(_, _, a, _)| (a.0 - gap_x.1).abs() < 1e-3)
        );
        // The x axis' frames run along y and stop at y's gap likewise.
        let x_frames = collect(&x, Side::Bottom, &y);
        assert_eq!(x_frames.len(), 4, "{x_frames:?}");
        let gap_y = (
            (PLOT.y + PLOT.height * (1.0 - y_knot.end as f32)).round() - half,
            (PLOT.y + PLOT.height * (1.0 - y_knot.start as f32)).round() + half,
        );
        for (_, _, a, b) in &x_frames {
            let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
            assert!(
                hi <= gap_y.0 + 1e-3 || lo >= gap_y.1 - 1e-3,
                "{a:?} {b:?} in {gap_y:?}"
            );
        }
        // An unbroken axis across leaves each frame whole.
        let mut plain = crate::defaults::axis_options_x();
        plain.min = 0.0;
        plain.max = 100.0;
        assert_eq!(collect(&y, Side::Left, &plain).len(), 2);
    }
}

#[cfg(test)]
mod label_clearance_tests {
    //! The tick labels of a rectangular chart make way where they collide
    //! with the axes' break marks and with the labels of the axis across
    //! them where the two axes meet (#272), as each axis'
    //! [`LabelClearanceMode`] allows.
    use super::*;
    use crate::config::{AxisBreak, BreakMark, LabelClearanceMode};
    use crate::sink::{DrawnPart, Parts};
    use crate::text::{RichText, TextExtents};

    /// Glyphs 0.6 em wide, 0.8 em up and 0.2 em down.
    struct Wide;
    impl MeasureText for Wide {
        fn measure_plain(&self, text: &str, _: &str, size: f32) -> TextExtents {
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

    /// The boxes of the tick labels of `element` as the chart places them
    /// for drawing, hit testing, selection and auto layout.
    fn labels(config: &Config, side: Side) -> Vec<RectF> {
        let mut out = Vec::new();
        crate::select::emit_element(
            config,
            &Element::AxisLabel(side),
            &Wide,
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    out.push(r);
                }
            }),
        );
        out
    }

    /// The boxes of the tick labels on `side` as the raster draws them
    /// for `config` whose pixel values carry the display or export
    /// `scale` (a config scaled by it).
    fn labels_at(config: &Config, side: Side, scale: f32) -> Vec<RectF> {
        let mut out = Vec::new();
        crate::chrome::emit_part_of(
            config,
            &Element::AxisLabel(side),
            &Wide,
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    out.push(r);
                }
            }),
            scale,
        );
        out
    }

    fn axis_on<'a>(config: &'a Config, side: &Side) -> &'a AxisOptions {
        match side {
            Side::Left => &config.left_y,
            _ => &config.bottom_x,
        }
    }

    /// The boxes the axis on `side` would place standing alone, before the
    /// labels across it are taken into account.
    fn lone(config: &Config, side: Side) -> Vec<RectF> {
        let mut out = Vec::new();
        emit_axis_tick_labels(
            axis_on(config, &side),
            side,
            &data_area_rect(&config.data_area().unwrap()),
            &Wide,
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    out.push(r);
                }
            }),
        );
        out
    }

    /// The boxes the decision reads for the labels of [`lone`], in its
    /// order: each label at rest, its width by its digits' height
    /// ([`label_decision_height`]).
    fn rest(config: &Config, side: Side) -> Vec<RectF> {
        let axis = axis_on(config, &side);
        let plot = data_area_rect(&config.data_area().unwrap());
        let majors = major_tick_values_for_axis(axis, side.clone(), &plot, &Wide);
        let mut out = Vec::new();
        each_kept_axis_tick_label(axis, side, &plot, None, 1.0, &Wide, &majors, |_, _, r| {
            out.push(r)
        });
        out
    }

    /// The resting boxes of the labels of `side` that `kept` (drawn boxes)
    /// still holds.
    fn rest_of_kept(config: &Config, side: Side, kept: &[RectF]) -> Vec<RectF> {
        let lone = lone(config, side.clone());
        rest(config, side)
            .into_iter()
            .zip(&lone)
            .filter(|(_, drawn)| kept.contains(drawn))
            .map(|(r, _)| r)
            .collect()
    }

    /// Whether any box of `a` comes within the visual gap of any box of
    /// `b`, the boxes of `a` in an `fa` px font and those of `b` in an
    /// `fb` px one ([`boxes_touch`]).
    fn any_touch(a: &[RectF], fa: f32, b: &[RectF], fb: f32) -> bool {
        a.iter()
            .any(|a| b.iter().any(|b| boxes_touch((a, fa), (b, fb))))
    }

    /// How far apart a label in an `fa` px font and a box read with an
    /// `fb` px font must stand to both stay: the visual gap
    /// ([`visual_gap_for`]).
    fn clear(fa: f32, fb: f32) -> f32 {
        visual_gap_for(fa, fb)
    }

    /// [`clear`] at [`corner_chart`]'s corner: a 30 px y label against a
    /// 19 px x label.
    fn corner_clear() -> f32 {
        clear(30.0, 19.0)
    }

    /// A chart whose corner labels are as wide as `value_maps`' colormap
    /// (#272): `-10.0` under the left end of x and `-7.00` beside the
    /// bottom of y, the y labels in a larger font, so the lowest y label's
    /// digits reach into the first x label's.
    fn corner_chart() -> Config {
        let mut config = crate::defaults::config();
        for (axis, extent, spacing) in [
            (&mut config.bottom_x, 10.0, 5.0),
            (&mut config.left_y, 7.0, 3.5),
        ] {
            axis.min = -extent;
            axis.max = extent;
            axis.major_spacing = spacing;
            axis.label_style.format = crate::format::LabelFormat::Plain(
                crate::format::PlainNumberFormat::fixed(2, false),
            );
            axis.label_style.font_size = 19.0;
        }
        config.left_y.label_style.font_size = 30.0;
        config
    }

    /// How far the lowest y label's resting box stands above the first x
    /// label's (negative: they overlap); their spans along x overlap.
    fn corner_gap(config: &Config) -> f32 {
        let (y, x) = (rest(config, Side::Left), rest(config, Side::Bottom));
        let low = y.iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
        let first = x.iter().min_by(|a, b| a.x.total_cmp(&b.x)).unwrap();
        assert!(
            low.x < first.x + first.width && first.x < low.x + low.width,
            "the corner labels meet along x: {low:?} {first:?}"
        );
        first.y - (low.y + low.height)
    }

    /// [`corner_chart`] with the lowest y label raised until its resting
    /// box stands `gap` px above the first x label's (a negative `gap`
    /// overlaps by that much).
    fn corner_at(gap: f32) -> Config {
        let mut config = corner_chart();
        let raise = f64::from(gap - corner_gap(&config));
        assert!(raise >= 0.0, "the corner overlaps by more than {gap}");
        let h = f64::from(data_area_rect(&config.data_area().unwrap()).height);
        // -7 sits `raise` px above the bottom of [min, 7].
        config.left_y.min = -7.0 * (h + raise) / (h - raise);
        let got = corner_gap(&config);
        assert!((got - gap).abs() < 1e-2, "{got} for {gap}");
        assert_eq!(rest(&config, Side::Left).len(), 5, "-7.00 is still drawn");
        config
    }

    // #272: the lowest y label and the first x label overlapped where the
    // axes meet. The y label yields; every other label stays, and no kept
    // y label comes within the visual gap of an x label.
    #[test]
    fn the_y_label_at_the_corner_yields_to_the_x_label() {
        let config = corner_chart();
        let (x, y) = (labels(&config, Side::Bottom), labels(&config, Side::Left));
        let (lone_x, lone_y) = (lone(&config, Side::Bottom), lone(&config, Side::Left));
        assert!(corner_gap(&config) < 0.0, "the corner labels overlap");
        assert!(
            !any_touch(
                &rest_of_kept(&config, Side::Left, &y),
                30.0,
                &rest(&config, Side::Bottom),
                19.0
            ),
            "no kept y label comes within the visual gap of an x label"
        );
        assert_eq!(x, lone_x, "the x labels are all kept");
        assert_eq!(
            y.len() + 1,
            lone_y.len(),
            "only the corner y label is dropped"
        );
        let lowest = |r: &[RectF]| r.iter().map(|b| b.y).fold(f32::MIN, f32::max);
        assert!(
            lowest(&y) < lowest(&lone_y),
            "the dropped label is the lowest"
        );
        // The drawn chrome places the same labels as the hit and selection
        // walk above.
        let mut drawn = Vec::new();
        emit_chart_chrome(
            &config,
            &Wide,
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    drawn.push(r);
                }
            }),
            1.0,
        );
        for r in x.iter().chain(&y) {
            assert!(drawn.contains(r), "{r:?} is drawn");
        }
        assert!(!drawn.contains(lone_y.iter().find(|r| !y.contains(r)).unwrap()));
    }

    // `Auto` drops a corner label where its box comes within the visual
    // gap of the label across: 2 px in total at 18 px, here each box's
    // half read from its font, (30 + 19) / 18 = 2.72 px for the 30 px y
    // label against the 19 px x label. Overlapping by 1 px, 1 or 2 px
    // clear or just under the gap it goes; just over it, or well clear,
    // it stays.
    #[test]
    fn auto_drops_a_corner_label_within_the_visual_gap() {
        let c = corner_clear();
        assert!((c - 49.0 / 18.0).abs() < 1e-5, "{c}");
        for (gap, dropped) in [
            (-1.0, 1),
            (1.0, 1),
            (2.0, 1),
            (c - 0.05, 1),
            (c + 0.05, 0),
            (3.0, 0),
            (9.0, 0),
        ] {
            let config = corner_at(gap);
            assert_eq!(
                labels(&config, Side::Left).len() + dropped,
                lone(&config, Side::Left).len(),
                "{gap} px apart"
            );
            assert_eq!(
                labels(&config, Side::Bottom),
                lone(&config, Side::Bottom),
                "{gap} px apart: the x labels are all kept"
            );
            // `Keep` on the yielding axis keeps them all at any gap.
            let mut keep = config.clone();
            keep.left_y.label_style.clearance = LabelClearanceMode::Keep;
            assert_eq!(
                labels(&keep, Side::Left),
                lone(&keep, Side::Left),
                "{gap} px apart: Keep drops none"
            );
        }
    }

    // Two boxes touch to the eye when they come closer than the visual
    // gap both across and along the screen: 2 px in total between two
    // 18 px labels, each box's half read as that share of its own font, so
    // it doubles with a 2x config's fonts and is symmetric in the two
    // fonts. Closer than the gap they touch; exactly the gap apart, or
    // farther, they do not.
    #[test]
    fn boxes_touch_within_the_visual_gap_of_their_fonts() {
        assert_eq!(visual_gap_for(18.0, 18.0), LABEL_VISUAL_GAP);
        assert_eq!(LABEL_VISUAL_GAP, 2.0);
        assert_eq!(visual_gap_for(36.0, 36.0), 2.0 * LABEL_VISUAL_GAP);
        assert_eq!(visual_gap_for(18.0, 36.0), visual_gap_for(36.0, 18.0));
        assert_eq!(visual_gap_for(-3.0, -1.0), 0.0);
        let a = RectF {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let below = |gap: f32| RectF { y: 10.0 + gap, ..a };
        let beside = |gap: f32| RectF { x: 10.0 + gap, ..a };
        for (fa, fb, threshold) in [(18.0, 18.0, 2.0), (36.0, 36.0, 4.0), (18.0, 36.0, 3.0)] {
            assert_eq!(visual_gap_for(fa, fb), threshold);
            for gap in [-1.0, 0.0, 1.0, threshold - 0.01] {
                assert!(boxes_touch((&a, fa), (&below(gap), fb)), "{gap}: {fa} {fb}");
                assert!(
                    boxes_touch((&beside(gap), fb), (&a, fa)),
                    "{gap}: {fa} {fb}"
                );
            }
            for gap in [threshold, threshold + 0.01, threshold + 5.0] {
                assert!(
                    !boxes_touch((&a, fa), (&below(gap), fb)),
                    "{gap}: {fa} {fb}"
                );
                assert!(
                    !boxes_touch((&beside(gap), fb), (&a, fa)),
                    "{gap}: {fa} {fb}"
                );
            }
        }
        // Within the gap along one direction only: apart along the other,
        // they do not touch, as the hexbin's two diagonal "0" labels at a
        // 21 px font (gap 2.33 px) stand 2.32 px apart down the screen and
        // 3.83 px across it.
        let diagonal = RectF {
            x: 13.83,
            y: 12.32,
            ..a
        };
        assert!(visual_gap_for(21.0, 21.0) > 2.32);
        assert!(!boxes_touch((&a, 21.0), (&diagonal, 21.0)));
        assert!(boxes_touch(
            (&a, 21.0),
            (
                &RectF {
                    x: 12.0,
                    ..diagonal
                },
                21.0
            )
        ));
    }

    // Which labels yield is decided at rest: a drag of either label set
    // moves the drawn set as one, as it moves its box and hit region.
    #[test]
    fn a_drag_moves_the_kept_labels_as_one() {
        let config = corner_chart();
        let (x, y) = (labels(&config, Side::Bottom), labels(&config, Side::Left));
        assert!(
            y.len() < lone(&config, Side::Left).len(),
            "a y label yields"
        );
        for (dx, dy) in [(0.0, 40.0), (0.0, -40.0), (-12.0, 25.0)] {
            let mut dragged = config.clone();
            dragged.left_y.label_style.label_offset_x += dx;
            dragged.left_y.label_style.label_offset_y += dy;
            let moved: Vec<RectF> = y
                .iter()
                .map(|r| RectF {
                    x: r.x + dx,
                    y: r.y + dy,
                    width: r.width,
                    height: r.height,
                })
                .collect();
            let got = labels(&dragged, Side::Left);
            assert_eq!(got.len(), moved.len(), "y by ({dx}, {dy})");
            for (a, b) in got.iter().zip(&moved) {
                assert!(
                    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3,
                    "y by ({dx}, {dy}): {a:?} for {b:?}"
                );
            }
            assert_eq!(labels(&dragged, Side::Bottom), x, "x stays");
            let mut dragged = config.clone();
            dragged.bottom_x.label_style.label_offset_x += dx;
            assert_eq!(labels(&dragged, Side::Left), y, "y stays when x moves");
        }
    }

    // A 2x export keeps the labels the 1x one keeps, though the gap past
    // the ticks is not scaled with it: the decision reads that gap, and the
    // visual gap between the boxes, as shares of the font. That holds for
    // a corner that overlaps, one 1 px clear, and ones either side of the
    // visual gap, in the raster at 2x and in the hit walk of the 2x
    // config, which knows no display scale.
    #[test]
    fn a_2x_export_keeps_the_1x_labels() {
        let mut configs = Vec::new();
        for font in [12.0, 19.0, 30.0] {
            let mut config = corner_chart();
            for axis in [&mut config.bottom_x, &mut config.left_y] {
                axis.label_style.font_size = font;
            }
            configs.push((format!("{font} px"), config));
        }
        configs.push(("overlapping".into(), corner_chart()));
        configs.push(("1 px over".into(), corner_at(-1.0)));
        configs.push(("1 px clear".into(), corner_at(1.0)));
        configs.push(("under the gap".into(), corner_at(corner_clear() - 0.3)));
        configs.push(("past the gap".into(), corner_at(corner_clear() + 0.3)));
        for (name, config) in configs {
            let twice = config.scaled(2.0);
            for side in [Side::Bottom, Side::Left] {
                let one = labels(&config, side.clone());
                assert_eq!(
                    one,
                    labels_at(&config, side.clone(), 1.0),
                    "{name} {side:?}"
                );
                let two = labels_at(&twice, side.clone(), 2.0);
                // The hit and selection walk, which knows no display
                // scale, keeps the labels the 2x raster draws.
                assert_eq!(two, labels(&twice, side.clone()), "{name} {side:?}");
                assert_eq!(one.len(), two.len(), "{name} {side:?}");
                for (a, b) in one.iter().zip(&two) {
                    assert!(
                        (b.x - 2.0 * a.x).abs() < 9.0,
                        "{name} {side:?}: {a:?} {b:?}"
                    );
                }
            }
        }
    }

    // An axis the corner would leave with fewer than two labels keeps
    // them all, as a log axis' ladder never demotes below two.
    #[test]
    fn a_corner_never_takes_an_axis_below_two_labels() {
        let mut config = corner_chart();
        config.left_y.min = 0.0;
        config.left_y.max = 14.0;
        config.left_y.major_spacing = 14.0;
        let lone_y = lone(&config, Side::Left);
        assert_eq!(lone_y.len(), 2, "0.00 and 14.00");
        assert!(corner_gap(&config) < 0.0, "the corner labels overlap");
        assert_eq!(labels(&config, Side::Left), lone_y);
    }

    // A categorical y axis names every category, so the x label yields at
    // its corner instead; with both categorical neither does. A `Keep` x
    // axis then keeps its label.
    #[test]
    fn a_categorical_axis_never_yields_at_a_corner() {
        let mut config = corner_chart();
        config.left_y.categories = Some(
            (0..3)
                .map(|i| crate::config::Category::new(format!("c{i}"), format!("category {i}")))
                .collect(),
        );
        // The first category on the bottom edge, where the x labels start.
        config.left_y.category_window = Some([0.0, 2.5]);
        let (x, y) = (labels(&config, Side::Bottom), labels(&config, Side::Left));
        assert_eq!(y.len(), 3, "every category is named");
        let y_rest = rest(&config, Side::Left);
        assert!(
            any_touch(&rest(&config, Side::Bottom), 19.0, &y_rest, 30.0),
            "the corner labels overlap"
        );
        assert!(
            !any_touch(
                &rest_of_kept(&config, Side::Bottom, &x),
                19.0,
                &y_rest,
                30.0
            ),
            "no kept x label comes within the visual gap of a category"
        );
        assert!(
            x.len() < lone(&config, Side::Bottom).len(),
            "the x label yields"
        );
        let mut keep = config.clone();
        keep.bottom_x.label_style.clearance = LabelClearanceMode::Keep;
        assert_eq!(
            labels(&keep, Side::Bottom),
            lone(&config, Side::Bottom),
            "a Keep x axis keeps its corner label"
        );
        assert_eq!(labels(&keep, Side::Left).len(), 3);
        assert!(yields_corner(
            &config.bottom_x,
            &Side::Bottom,
            &config.left_y
        ));
        let mut both = config.left_y.clone();
        both.categories = Some(Vec::new());
        assert!(!yields_corner(
            &config.left_y,
            &Side::Left,
            &config.bottom_x
        ));
        assert!(!yields_corner(&both, &Side::Bottom, &config.left_y));
    }

    /// A broken x axis whose slashes are `size` px long, on the chart
    /// `broken_axes` draws.
    fn broken_chart(size: f32) -> Config {
        let mut config = crate::defaults::config();
        let x = &mut config.bottom_x;
        x.min = 500.0;
        x.max = 1450.0;
        x.major_spacing = 50.0;
        x.line_visible = true;
        x.label_style.font_size = 19.0;
        let mut breaks = crate::defaults::axis_breaks();
        breaks.ranges = vec![AxisBreak::new(760.0, 1080.0)];
        breaks.mark = BreakMark::Slash {
            size,
            color: None,
            width: 1.5,
        };
        x.breaks = Some(breaks);
        config
    }

    fn marks(config: &Config) -> Vec<RectF> {
        let plot = data_area_rect(&config.data_area().unwrap());
        let mut marks = Vec::new();
        each_break_mark_box(&config.bottom_x, &Side::Bottom, &plot, None, 1.0, |b| {
            marks.push(b)
        });
        marks
    }

    // #272: on `broken_axes` the slashes of the break mark ran into the
    // labels either side of the gap. A label whose box comes within the
    // visual gap of a slash is dropped; without a mark, or with
    // `Keep`, the same labels are drawn.
    #[test]
    fn tick_labels_make_way_for_the_break_mark() {
        let config = broken_chart(24.0);
        let marks = marks(&config);
        assert_eq!(marks.len(), 2, "one slash either side of the gap");
        let kept = labels(&config, Side::Bottom);
        assert!(!any_touch(
            &rest_of_kept(&config, Side::Bottom, &kept),
            19.0,
            &marks,
            19.0
        ));
        let mut unmarked = config.clone();
        if let Some(breaks) = unmarked.bottom_x.breaks.as_mut() {
            breaks.mark = BreakMark::None;
        }
        let all = labels(&unmarked, Side::Bottom);
        let overlapping = rest(&unmarked, Side::Bottom)
            .iter()
            .filter(|label| {
                marks
                    .iter()
                    .any(|mark| boxes_touch((label, 19.0), (mark, 19.0)))
            })
            .count();
        assert!(overlapping > 0, "a label runs into the mark");
        assert_eq!(
            kept.len() + overlapping,
            all.len(),
            "only those labels are dropped"
        );
        let mut keep = config.clone();
        keep.bottom_x.label_style.clearance = LabelClearanceMode::Keep;
        assert_eq!(labels(&keep, Side::Bottom), all, "Keep drops none");
    }

    // A label overlapping a slash by 1 px, 1 px clear of it or just under
    // the visual gap clear (2.11 px at the 19 px font) goes;
    // just over it, or well clear, it stays.
    #[test]
    fn a_label_within_the_visual_gap_of_the_break_mark_goes() {
        // How far apart two boxes stand: the larger of their distances
        // across and along the screen (negative where they overlap), the
        // one the visual gap must bridge for the boxes to touch.
        let apart = |a: &RectF, b: &RectF| {
            let x = (b.x - (a.x + a.width)).max(a.x - (b.x + b.width));
            let y = (b.y - (a.y + a.height)).max(a.y - (b.y + b.height));
            x.max(y)
        };
        // The nearest any label at rest stands to a slash of length `size`.
        let labels_at_rest = rest(&broken_chart(0.0), Side::Bottom);
        let reach = |size: f32| {
            marks(&broken_chart(size))
                .iter()
                .flat_map(|mark| labels_at_rest.iter().map(move |l| apart(l, mark)))
                .reduce(f32::min)
                .expect("a slash")
        };
        // A longer slash reaches nearer: find the length for each gap.
        let (short, long) = (1.0, 24.0);
        let c = clear(19.0, 19.0);
        assert!(reach(short) > c + 1.0 && reach(long) < -1.0);
        let length_for = |gap: f32| {
            let (mut lo, mut hi) = (short, long);
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                if reach(mid) > gap {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            hi
        };
        for (gap, dropped) in [
            (-1.0, true),
            (1.0, true),
            (c - 0.1, true),
            (c + 0.1, false),
            (c + 1.0, false),
        ] {
            let size = length_for(gap);
            assert!(
                (reach(size) - gap).abs() < 1e-2,
                "{} for {gap}",
                reach(size)
            );
            let config = broken_chart(size);
            let mut unmarked = config.clone();
            if let Some(breaks) = unmarked.bottom_x.breaks.as_mut() {
                breaks.mark = BreakMark::None;
            }
            let (kept, all) = (
                labels(&config, Side::Bottom),
                labels(&unmarked, Side::Bottom),
            );
            assert_eq!(kept.len() < all.len(), dropped, "{gap} px apart");
        }
    }

    // The setting is additive on the wire: `Auto` is omitted, so a
    // document written before it existed reads as `Auto` and writes back
    // byte for byte; `Keep` round-trips.
    #[cfg(feature = "serde")]
    #[test]
    fn the_clearance_mode_is_additive_on_the_wire() {
        let config = crate::defaults::config();
        let text = serde_json::to_string(&config).unwrap();
        assert!(!text.contains("clearance"), "Auto is omitted");
        let old: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(old.left_y.label_style.clearance, LabelClearanceMode::Auto);
        assert_eq!(serde_json::to_string(&old).unwrap(), text);
        let mut keep = config.clone();
        keep.left_y.label_style.clearance = LabelClearanceMode::Keep;
        let json = serde_json::to_value(&keep).unwrap();
        assert_eq!(json["left_y"]["label_style"]["clearance"], "Keep");
        assert!(json["bottom_x"]["label_style"].get("clearance").is_none());
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, keep);
        let mut explicit = serde_json::to_value(&config).unwrap();
        explicit["left_y"]["label_style"]["clearance"] = "Auto".into();
        assert_eq!(serde_json::from_value::<Config>(explicit).unwrap(), config);
        assert_eq!(
            keep.scaled(2.0).left_y.label_style.clearance,
            LabelClearanceMode::Keep,
            "a mode is not a pixel dimension"
        );
    }
}
