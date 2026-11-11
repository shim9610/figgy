//! The grid and axes of a 3D chart (`Config.view_3d`): grid lines of each
//! axis on the box's three back panes for the current camera, and the
//! three axes on the box edges `Config::box_axis_edge` chooses (x and y on
//! the floor edges nearest the eye, z on the left-most vertical edge), each
//! with its ticks and upright labels outside the box. Every point is
//! projected through `Config::project_3d`, the camera the GPU draws with,
//! and every part goes through the sink, so drawing, hit regions, selection
//! outlines and the vector exports share one placement.

use super::*;
use crate::sink::{Sink, Slot, StrokeStyle};
use crate::view3d::box_axis;

pub use crate::ternary::side_component;

/// The axis fraction of `value` on `axis`, `0..=1` inside its range, read
/// as the GPU reads it: logarithmic and inverted axes included.
fn fraction(axis: &AxisOptions, value: f64) -> Option<f64> {
    let f = match axis.scale {
        AxisScale::Logarithmic => {
            let (lo, hi) = crate::layout::guarded_log_range(axis.min, axis.max);
            crate::layout::span_fraction(value, lo, hi, &AxisScale::Logarithmic)
        }
        // A 3D chart refuses a probability axis (#209).
        AxisScale::Probability(_) => None,
        AxisScale::Linear => {
            let (lo, hi) = axis.data_range();
            crate::layout::span_fraction(value, lo, hi, &AxisScale::Linear)
        }
    }?;
    let f = if axis.inverted { 1.0 - f } else { f };
    (f.is_finite() && (-1e-9..=1.0 + 1e-9).contains(&f)).then_some(f.clamp(0.0, 1.0))
}

/// The box grid below the data: on each back pane, a line of constant
/// value at each major tick of the pane's two axes, from edge to edge,
/// pane borders included. The x family takes the grid's x styles, y and z
/// its y styles.
pub fn emit_box_grid(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(panes) = config.box_back_panes() else {
        return;
    };
    let grid = &config.grid;
    let plot = data_area_rect(da);
    for k in 0..3 {
        let (show, color, width, style) = if k == 0 {
            (
                grid.show_major_x,
                grid.major_x_color,
                grid.major_x_width,
                grid.major_x_style,
            )
        } else {
            (
                grid.show_major_y,
                grid.major_y_color,
                grid.major_y_width,
                grid.major_y_style,
            )
        };
        if !show {
            continue;
        }
        let axis = box_axis(config, k);
        let values = major_tick_values_for_axis(axis, box_side(k), &plot, measure);
        for (i, value) in values.into_iter().enumerate() {
            let Some(f) = fraction(axis, value) else {
                continue;
            };
            // The line of constant `k` runs across each back pane that
            // contains direction `k`: the panes across the two others.
            for across in (0..3).filter(|j| *j != k) {
                let along = 3 - k - across;
                let mut a = [0.0f64; 3];
                a[k] = f;
                a[across] = panes[across];
                let mut b = a;
                a[along] = 0.0;
                b[along] = 1.0;
                let (Some(pa), Some(pb)) = (config.project_3d(a), config.project_3d(b)) else {
                    continue;
                };
                let family = ["x", "y", "z"][k];
                let tag = format!("grid_major_{family}_{i}_{across}");
                sink.stroke(
                    Slot::Below,
                    [pa[0] as f32, pa[1] as f32],
                    [pb[0] as f32, pb[1] as f32],
                    &StrokeStyle {
                        width,
                        color,
                        dash: style.pattern(),
                        tag: &tag,
                    },
                );
            }
        }
    }
}

/// The axis side of box direction `k`: x `Bottom`, y `Left`, z `Right`.
fn box_side(k: usize) -> Side {
    crate::ternary::component_side(k)
}

/// The box chrome above the data: each axis edge with its ticks, then each
/// axis' tick labels.
pub fn emit_box_axes(
    config: &Config,
    da: &DataArea,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    for k in 0..3 {
        emit_box_axis_strokes(config, da, k, measure, sink);
    }
    for k in 0..3 {
        emit_box_labels(config, da, k, measure, sink);
    }
}

/// Axis `k`'s edge and ticks (`Element::Axis` of its side): the edge moved
/// out by `line_offset` along its normal, the ticks along the normal.
pub fn emit_box_axis_strokes(
    config: &Config,
    da: &DataArea,
    k: usize,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let Some(edge) = config.box_axis_edge(k) else {
        return;
    };
    let axis = box_axis(config, k);
    super::emit_edge_axis_strokes(
        axis,
        box_side(k),
        da,
        measure,
        sink,
        (edge.a, edge.b),
        |value| fraction(axis, value).map(|f| f as f32),
        |f, offset| edge.point(f, offset),
    );
}

/// How far axis `k`'s tick labels reach past its edge along the edge's
/// normal, as [`emit_box_labels`] places them: the tick gap plus the
/// largest extent along the normal of a label of any major (the drawn
/// labels are those or a pruned subset), and the label offset along it. `None` when the labels are hidden or the chart is not 3D. The title
/// sits past this, so a wide label on a near-vertical edge (the z axis's
/// `3.00`) does not run into it.
pub(crate) fn box_label_reach(config: &Config, k: usize, measure: &dyn MeasureText) -> Option<f32> {
    let edge = config.box_axis_edge(k)?;
    let axis = box_axis(config, k);
    let ls = &axis.label_style;
    if !ls.visible || !ls.label_visible {
        return None;
    }
    let n = edge.normal;
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    // Every major the axis walks: the labels drawn are these or a pruned
    // subset, so the widest of them bounds the band without a list.
    crate::tick::major_ticks(axis)
        .filter(|value| fraction(axis, *value).is_some())
        .map(|value| {
            let (_, m) = super::polar::tick_text(axis, value, measure);
            n[0].abs() * m.width + n[1].abs() * (m.ascent + m.descent)
        })
        .fold(None, |widest: Option<f32>, extent| {
            Some(widest.map_or(extent, |w| w.max(extent)))
        })
        .map(|widest| gap + widest + ls.label_offset_y.max(0.0))
}

/// Axis `k`'s tick labels (`Element::AxisLabel` of its side): centred past
/// the tick ends along the edge's normal, upright, moved by the label
/// offsets read along the edge (`label_offset_x`) and along the normal
/// (`label_offset_y`).
pub fn emit_box_labels(
    config: &Config,
    da: &DataArea,
    k: usize,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let ls = &box_axis(config, k).label_style;
    each_box_label(config, da, k, measure, &mut |text, center, m, _| {
        super::polar::place_centered(sink, text.as_tick_text(), center, m, ls);
    });
}

/// The box a label of extents `m` and font size `font` centred on
/// `center` is read with where it decides what it keeps clear of (#272):
/// its drawn width and the height of its digits
/// ([`super::line::label_decision_height`]), the same at every export
/// scale.
fn centred_box(center: (f32, f32), m: TextExtents, font: f32) -> RectF {
    let height = super::line::label_decision_height(font);
    RectF {
        x: center.0 - m.width * 0.5,
        y: center.1 - height * 0.5,
        width: m.width,
        height,
    }
}

/// Every tick label [`emit_box_labels`] draws for axis `k`: its text, its
/// centre, its extents and its centre at rest (before the label offsets a
/// drag writes). Where the z edge meets the floor, a z label
/// whose box overlaps a label of the x or y axis yields to it (#272) while
/// the z axis keeps at least two labels, as a
/// Cartesian y label yields to the x label at the corner
/// ([`super::yields_corner`]): the floor's labels name the floor corner.
/// Two boxes overlap there when they come within a small visual gap of
/// each other ([`super::line::boxes_touch`]): 2 px in total at the default
/// 18 px tick label font, each box's half read as a share of its own font,
/// so the gap scales with the display or export and a 2x export keeps the
/// 1x labels; boxes the gap apart or more stay. With the z axis'
/// [`LabelClearanceMode::Keep`] none yields; the floor axes' settings do
/// not matter there.
fn each_box_label(
    config: &Config,
    da: &DataArea,
    k: usize,
    measure: &dyn MeasureText,
    each: &mut dyn FnMut(NumericTickText, (f32, f32), TextExtents, (f32, f32)),
) {
    let Some(edge) = config.box_axis_edge(k) else {
        return;
    };
    let axis = box_axis(config, k);
    let ls = &axis.label_style;
    if !ls.visible || !ls.label_visible {
        return;
    }
    let majors = major_tick_values_for_axis(axis, box_side(k), &data_area_rect(da), measure);
    let n = edge.normal;
    let along = edge.direction();
    let gap = super::polar::outward_tick(axis) + ls.font_size.max(0.0) / 3.0;
    // The x and y edges meet at the floor corner nearest the eye; the x
    // label there keeps it, as a ternary corner shows one label.
    let shared = (k == 1)
        .then(|| config.box_axis_edge(0))
        .flatten()
        .map(|x_edge| x_edge.t0[1]);
    let place = |value: f64| {
        let f = fraction(axis, value)
            .filter(|f| shared.is_none_or(|corner| (f - corner).abs() > 1e-6))?;
        let (text, m) = super::polar::tick_text(axis, value, measure);
        let half = n[0].abs() * m.width * 0.5 + n[1].abs() * (m.ascent + m.descent) * 0.5;
        let at = edge.point(f as f32, gap + half);
        let center = (
            at[0] + along[0] * ls.label_offset_x + n[0] * ls.label_offset_y,
            at[1] + along[1] * ls.label_offset_x + n[1] * ls.label_offset_y,
        );
        Some((text, center, m, (at[0], at[1])))
    };
    // Whether a z label at rest touches a label of the floor's axes at
    // rest: a drag moves a label set as one.
    let touching = |rest: (f32, f32), m: TextExtents| {
        let own = centred_box(rest, m, ls.font_size);
        let mut touching = false;
        for floor in 0..2 {
            let font = box_axis(config, floor).label_style.font_size;
            each_box_label(config, da, floor, measure, &mut |_, _, e, r| {
                touching |= super::line::boxes_touch(
                    (&own, ls.font_size),
                    (&centred_box(r, e, font), font),
                );
            });
        }
        touching
    };
    // The corner never takes the z axis below two labels, as a Cartesian
    // corner never does.
    let yields = k == 2 && ls.clearance == LabelClearanceMode::Auto && {
        let (mut count, mut yielding) = (0usize, 0usize);
        for value in &majors {
            if let Some((_, _, m, rest)) = place(*value) {
                count += 1;
                yielding += usize::from(touching(rest, m));
            }
        }
        yielding > 0 && count - yielding >= 2
    };
    for value in majors {
        let Some((text, center, m, rest)) = place(value) else {
            continue;
        };
        if yields && touching(rest, m) {
            continue;
        }
        each(text, center, m, rest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::{DrawnPart, Parts};
    use crate::text::{RichText, TextExtents};

    /// Glyphs 0.6 em wide and 0.7 em up, as digits are.
    struct Wide;
    impl MeasureText for Wide {
        fn measure_plain(&self, text: &str, _: &str, size: f32) -> TextExtents {
            TextExtents {
                width: text.chars().count() as f32 * size * 0.6,
                ascent: size * 0.7,
                descent: 0.0,
            }
        }
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            self.measure_plain("", "", rt.font_size)
        }
    }

    fn label_boxes(config: &Config, side: Side) -> Vec<RectF> {
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

    /// The `trisurf` view (#272) with the z labels in a `z_font` px font.
    fn corner_chart(z_font: f32) -> Config {
        let mut config = crate::defaults::config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 0,
            width: 1200,
            height: 800,
        });
        let mut view = crate::defaults::view3d();
        view.azimuth_degrees = -55.;
        view.elevation_degrees = 32.;
        view.distance = 2.4;
        view.box_aspect = Some([1., 1., 0.6]);
        config.set_view_3d(Some(view));
        for (axis, min, max, spacing) in [
            (&mut config.bottom_x, -1.0, 1.0, 0.5),
            (&mut config.left_y, -1.0, 1.0, 0.5),
            (&mut config.right_y, -2.0, 1.0, 1.0),
        ] {
            axis.min = min;
            axis.max = max;
            axis.major_spacing = spacing;
            axis.label_style.font_size = 19.0;
            axis.label_style.significant_digits = 1;
            axis.label_style.visible = true;
            axis.label_style.label_visible = true;
        }
        config.right_y.label_style.font_size = z_font;
        config
    }

    /// The labels of the three axes of `config`.
    fn all_labels(config: &Config) -> [Vec<RectF>; 3] {
        [Side::Bottom, Side::Left, Side::Right].map(|side| label_boxes(config, side))
    }

    /// The `z_font` chart with every z label kept, for comparison.
    fn keeping(z_font: f32) -> Config {
        let mut config = corner_chart(z_font);
        config.right_y.label_style.clearance = crate::config::LabelClearanceMode::Keep;
        config
    }

    /// The box the decision reads for a label drawn in `b` in a `font` px
    /// font: the same centre and width, the decision's height
    /// ([`super::super::line::label_decision_height`]).
    fn decided(b: &RectF, font: f32) -> RectF {
        let height = super::super::line::label_decision_height(font);
        RectF {
            y: b.y + (b.height - height) * 0.5,
            height,
            ..*b
        }
    }

    /// Whether the decision boxes of two labels overlap.
    fn overlap((a, fa): (&RectF, f32), (b, fb): (&RectF, f32)) -> bool {
        super::super::line::boxes_within(&decided(a, fa), &decided(b, fb), 0.0)
    }

    /// Whether the decision boxes of two labels come within the visual gap
    /// of each other.
    fn touch((a, fa): (&RectF, f32), (b, fb): (&RectF, f32)) -> bool {
        super::super::line::boxes_touch((&decided(a, fa), fa), (&decided(b, fb), fb))
    }

    /// How far apart a z label in a `font` px font and a 19 px floor label
    /// must stand to both stay: the visual gap between them.
    fn clear(font: f32) -> f32 {
        super::super::line::visual_gap_for(font, 19.0)
    }

    /// How far the lowest z label's decision box stands above the nearest
    /// x label's below it, with every z label kept, at a `font` px z font.
    fn corner_gap(font: f32) -> f32 {
        let [x, _, z] = all_labels(&keeping(font));
        let low = decided(z.iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap(), font);
        x.iter()
            .map(|b| decided(b, 19.0))
            .filter(|b| b.x < low.x + low.width && low.x < b.x + b.width)
            .map(|b| b.y - (low.y + low.height))
            .reduce(f32::min)
            .expect("an x label under the lowest z label")
    }

    /// The labels of axis `side` as the raster draws them for `config`
    /// whose pixel values carry the display or export `scale`.
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

    // #272: on `trisurf` the lowest z label ("-2") sat on the first x label
    // ("-1.0") where the z edge meets the floor. Where their boxes
    // overlap, the z label yields; the floor's labels are all kept.
    #[test]
    fn a_z_label_yields_to_the_floor_labels_at_the_corner() {
        // A 40 px z font: the lowest z label overlaps the first x label.
        let [all_x, all_y, all_z] = all_labels(&keeping(40.0));
        assert_eq!(all_z.len(), 4);
        assert!(
            all_z
                .iter()
                .any(|a| all_x.iter().any(|b| overlap((a, 40.0), (b, 19.0)))),
            "the corner labels overlap: {all_z:?} {all_x:?}"
        );
        let [x, y, z] = all_labels(&corner_chart(40.0));
        for a in &z {
            assert!(
                !x.iter().chain(&y).any(|b| touch((a, 40.0), (b, 19.0))),
                "no kept z label comes within the visual gap of a floor label: {a:?}"
            );
        }
        assert_eq!((x, y), (all_x, all_y), "every floor label is kept");
        assert_eq!(z.len(), 3, "the corner z label yields: {z:?}");
    }

    // `Auto` drops a z label where its box comes within the visual gap of
    // a floor label's: 2 px in total at 18 px, each box's half read from
    // its own font, (font + 19) / 18 px here. The lowest z label's box
    // stands 4.17 px clear of the first x label's at a 24 px font and
    // 0.35 px closer per px of font, while the gap grows by 0.06 px per
    // px: they meet near 28.4 px. Well clear (20 px, 24 px) and just over
    // the gap clear (28 px) it stays; just under it (28.5 px) and under a
    // pixel clear (34 px) it goes.
    #[test]
    fn a_z_label_within_the_visual_gap_of_the_floor_labels_goes() {
        for (font, kept, near) in [
            (20.0, 4, false),
            (24.0, 4, false),
            (28.0, 4, true),
            (28.5, 3, true),
            (34.0, 3, false),
        ] {
            let (gap, c) = (corner_gap(font), clear(font));
            assert_eq!(gap < c, kept == 3, "{font} px: {gap} px clear of {c}");
            if near {
                assert!((gap - c).abs() < 0.2, "{font} px: {gap} px clear of {c}");
            }
            let [x, y, z] = all_labels(&corner_chart(font));
            assert_eq!(z.len(), kept, "{font} px: {gap} px clear of {c}");
            assert_eq!(
                [x, y],
                [Side::Bottom, Side::Left].map(|side| label_boxes(&keeping(font), side)),
                "{font} px: every floor label is kept"
            );
            // `Keep` keeps them all at any gap.
            assert_eq!(all_labels(&keeping(font))[2].len(), 4, "{font} px: Keep");
        }
        assert!((0.0..1.0).contains(&corner_gap(34.0)));
    }

    // A 2x export keeps the z labels the 1x one keeps: the visual gap is
    // read from the fonts, which scale with it, either side of the gap; the
    // hit walk of the 2x config, which knows no display scale, keeps the
    // labels the 2x raster draws.
    #[test]
    fn a_2x_export_keeps_the_1x_z_labels() {
        for font in [20.0, 24.0, 28.0, 28.5, 30.0, 40.0] {
            let config = corner_chart(font);
            let twice = config.scaled(2.0);
            for side in [Side::Bottom, Side::Left, Side::Right] {
                let one = label_boxes(&config, side.clone());
                assert_eq!(one, labels_at(&config, side.clone(), 1.0), "{font} px");
                let two = labels_at(&twice, side.clone(), 2.0);
                assert_eq!(two, label_boxes(&twice, side.clone()), "{font} px {side:?}");
                assert_eq!(one.len(), two.len(), "{font} px {side:?}");
                for (a, b) in one.iter().zip(&two) {
                    assert!(
                        (b.x - 2.0 * a.x).abs() < 2.0 && (b.y - 2.0 * a.y).abs() < 2.0,
                        "{font} px {side:?}: {a:?} {b:?}"
                    );
                }
            }
        }
    }

    // A `Keep` z axis keeps the label that overlaps; the floor axes'
    // settings do not matter at the z corner.
    #[test]
    fn a_keep_z_axis_keeps_its_corner_label() {
        let [x, y, z] = all_labels(&keeping(40.0));
        assert_eq!(z.len(), 4, "Keep drops none");
        let mut floor_keep = corner_chart(40.0);
        for axis in [&mut floor_keep.bottom_x, &mut floor_keep.left_y] {
            axis.label_style.clearance = crate::config::LabelClearanceMode::Keep;
        }
        let [fx, fy, fz] = all_labels(&floor_keep);
        assert_eq!((fx, fy), (x, y));
        assert_eq!(fz.len(), 3, "the z label still yields to Keep floor axes");
    }
}
