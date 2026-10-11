//! Shared bounded-axis decorations. Values/category planning stays with each
//! data model; drawing and measured interaction geometry are produced together.
use super::*;
use crate::decoration::DecorationPainter;
use model::{Category, DecorationId};
pub(crate) struct BoundedAxisDecorations<'a> {
    pub size: (u32, u32),
    pub plot: [f32; 4],
    pub horizontal: bool,
    pub zero: Option<f32>,
    pub ticks: &'a [(f32, String)],
    pub categories: &'a [Category],
    pub title: &'a str,
    pub value_title: &'a str,
    pub category_title: &'a str,
    pub font_family: &'a str,
    pub font_size: f32,
    pub color: Color,
    pub grid: bool,
    pub legend: Vec<(usize, f32, f32, &'a str, Color)>,
    pub legend_height: f32,
}
pub(crate) fn draw_bounded_decorations(
    p: &BoundedAxisDecorations<'_>,
    d: &mut DecorationPainter<'_>,
    back: &mut Canvas,
    front: &mut Canvas,
) -> Result<(), &'static str> {
    let [left, top, right, bottom] = p.plot;
    let (w, h) = (p.size.0 as f32, p.size.1 as f32);
    let fs = p.font_size;
    let measure = |text: &str, size: f32| {
        measure_plain_text(
            text,
            p.font_family,
            size,
            false,
            false,
            FontPolicy::Standard,
        )
        .width
    };
    if !p.title.is_empty() {
        let tw = measure(p.title, fs * 1.5);
        if tw > w - 32. {
            return Err("title does not fit");
        }
        d.text(
            DecorationId::ChartTitle,
            front,
            p.font_family,
            p.title,
            (w - tw) * 0.5,
            fs * 1.5 + 12.,
            fs * 1.5,
            p.color,
        );
    }
    for (t, label) in p.ticks {
        let (x, y, side) = if p.horizontal {
            let x = left + t * (right - left);
            if p.grid {
                back.draw_line(
                    (x * d.scale, top * d.scale),
                    (x * d.scale, bottom * d.scale),
                    &Paint::stroke(&Color::from_rgb8(224, 229, 235), d.scale),
                );
            }
            (x - measure(label, fs) * 0.5, bottom + fs + 8., Side::Bottom)
        } else {
            let y = bottom - t * (bottom - top);
            if p.grid {
                back.draw_line(
                    (left * d.scale, y * d.scale),
                    (right * d.scale, y * d.scale),
                    &Paint::stroke(&Color::from_rgb8(224, 229, 235), d.scale),
                );
            }
            (left - measure(label, fs) - 10., y + fs * 0.32, Side::Left)
        };
        d.text(
            DecorationId::AxisLabel(side),
            front,
            p.font_family,
            label,
            x,
            y,
            fs,
            p.color,
        );
    }
    let (ax, ay) = if let Some(zero) = p.zero {
        if p.horizontal {
            (zero, bottom)
        } else {
            (left, zero)
        }
    } else {
        (left, bottom)
    };
    d.axis(
        DecorationId::Axis(Side::Bottom),
        back,
        (left, ay),
        (right, ay),
        p.color,
    );
    d.axis(
        DecorationId::Axis(Side::Left),
        back,
        (ax, top),
        (ax, bottom),
        p.color,
    );
    let slot = if p.horizontal {
        (bottom - top) / p.categories.len() as f32
    } else {
        (right - left) / p.categories.len() as f32
    };
    for (i, cat) in p.categories.iter().enumerate() {
        let center = if p.horizontal {
            top + (i as f32 + 0.5) * slot
        } else {
            left + (i as f32 + 0.5) * slot
        };
        let (x, y, side) = if p.horizontal {
            (
                left - measure(&cat.label, fs) - 10.,
                center + fs * 0.32,
                Side::Left,
            )
        } else {
            (
                center - measure(&cat.label, fs) * 0.5,
                bottom + fs + 8.,
                Side::Bottom,
            )
        };
        d.text(
            DecorationId::AxisLabel(side),
            front,
            p.font_family,
            &cat.label,
            x,
            y,
            fs,
            p.color,
        );
    }
    let (axis_title, side_title) = if p.horizontal {
        (p.value_title, p.category_title)
    } else {
        (p.category_title, p.value_title)
    };
    if !axis_title.is_empty() {
        let tw = measure(axis_title, fs);
        if tw > right - left {
            return Err("axis title does not fit");
        }
        d.text(
            DecorationId::AxisTitle(Side::Bottom),
            front,
            p.font_family,
            axis_title,
            (left + right - tw) * 0.5,
            bottom + 2. * fs + 22.,
            fs,
            p.color,
        );
    }
    if !side_title.is_empty() {
        if measure(side_title, fs) > bottom - top {
            return Err("axis title does not fit");
        }
        d.rotated_text(
            DecorationId::AxisTitle(Side::Left),
            front,
            p.font_family,
            side_title,
            fs + 4.,
            (top + bottom) * 0.5,
            fs,
            p.color,
        );
    }
    for (row, x, _, label, color) in &p.legend {
        let total = p
            .legend
            .iter()
            .filter(|(r, _, _, _, _)| r == row)
            .map(|(_, _, v, _, _)| v)
            .sum::<f32>();
        let lx = (w - total) * 0.5 + x;
        let ly = h - p.legend_height + 12. + *row as f32 * (fs + 12.);
        d.rect(
            DecorationId::Legend,
            back,
            RectF {
                x: lx,
                y: ly - fs * 0.65,
                width: fs,
                height: fs * 0.65,
            },
            *color,
        );
        d.text(
            DecorationId::Legend,
            front,
            p.font_family,
            label,
            lx + fs + 7.,
            ly,
            fs,
            p.color,
        );
    }
    Ok(())
}
