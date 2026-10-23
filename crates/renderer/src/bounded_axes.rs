//! Shared bounded-axis decorations. Values/category planning stays with each
//! data model; drawing and measured interaction geometry are produced together.
use super::*;
use crate::decoration::{DecorationPainter, LabelPlacer};
use model::{Category, DecorationId};

/// The grid line colour of every bounded value axis.
const GRID_COLOR: Color = Color::from_rgb8(224, 229, 235);
pub(crate) struct BoundedAxisDecorations<'a> {
    pub size: (u32, u32),
    pub plot: [f32; 4],
    pub left_title_x: f32,
    pub horizontal: bool,
    pub zero: Option<f32>,
    pub ticks: &'a [(f32, String)],
    pub categories: &'a [Category],
    pub title: &'a crate::bounded_title::TitleLayout,
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
    let mut bottom_labels = LabelPlacer::new(
        RectF {
            x: 4.,
            y: bottom + 3.,
            width: w - 8.,
            height: (h - bottom - 7.).max(0.),
        },
        [false, true],
    );
    let mut left_labels = LabelPlacer::new(
        RectF {
            x: 4.,
            y: 4.,
            width: (left - 8.).max(0.),
            height: h - 8.,
        },
        [true, false],
    );
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
    p.title.draw(d, front, w)?;
    for (t, label) in p.ticks {
        let (x, y, side) = if p.horizontal {
            let x = left + t * (right - left);
            if p.grid {
                d.chrome(back).line([x, top], [x, bottom], GRID_COLOR);
            }
            (x - measure(label, fs) * 0.5, bottom + fs + 8., Side::Bottom)
        } else {
            let y = bottom - t * (bottom - top);
            if p.grid {
                d.chrome(back).line([left, y], [right, y], GRID_COLOR);
            }
            (left - measure(label, fs) - 10., y + fs * 0.32, Side::Left)
        };
        let (x, y) = d.place_text(
            if side == Side::Bottom {
                &mut bottom_labels
            } else {
                &mut left_labels
            },
            p.font_family,
            label,
            x,
            y,
            fs,
        );
        d.on(DecorationId::AxisLabel(side), front).plain_text(
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
    d.on(DecorationId::Axis(Side::Bottom), back)
        .line([left, ay], [right, ay], p.color);
    d.on(DecorationId::Axis(Side::Left), back)
        .line([ax, top], [ax, bottom], p.color);
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
        let (x, y) = d.place_text(
            if side == Side::Bottom {
                &mut bottom_labels
            } else {
                &mut left_labels
            },
            p.font_family,
            &cat.label,
            x,
            y,
            fs,
        );
        d.on(DecorationId::AxisLabel(side), front).plain_text(
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
        d.on(DecorationId::AxisTitle(Side::Bottom), front)
            .plain_text(
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
        d.on(DecorationId::AxisTitle(Side::Left), front)
            .rotated_text(
                p.font_family,
                side_title,
                p.left_title_x,
                (top + bottom) * 0.5,
                fs,
                p.color,
            );
    }
    draw_bounded_legend(
        &BoundedLegend {
            size: p.size,
            entries: &p.legend,
            height: p.legend_height,
            font_family: p.font_family,
            font_size: fs,
            color: p.color,
        },
        d,
        back,
        front,
    );
    Ok(())
}

/// The legend rows every bounded family lays out the same way: centred
/// rows at the bottom of the canvas, a colour swatch and the label per
/// entry. Entries are `(row, x within the row, width, label, colour)`.
/// A value as a bounded chart's tick or data label: plain decimals, trailing
/// zeros trimmed, exponent form past six digits either way.
pub(crate) fn plain_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.abs() >= 1e6 || v.abs() < 0.001 {
        format!("{v:.2e}")
    } else {
        format!("{v:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}
pub(crate) struct BoundedLegend<'a> {
    pub size: (u32, u32),
    pub entries: &'a [(usize, f32, f32, &'a str, Color)],
    pub height: f32,
    pub font_family: &'a str,
    pub font_size: f32,
    pub color: Color,
}
/// Measure the legend rows of `labels` for a canvas `width` wide: each
/// entry's row, x within the row and width, plus the total legend height.
/// A label wider than the canvas is an error.
pub(crate) fn plan_bounded_legend<'a>(
    labels: impl Iterator<Item = &'a str>,
    width: f32,
    font_family: &str,
    font_size: f32,
) -> Result<(Vec<(usize, f32, f32)>, f32), &'static str> {
    let mut rows = Vec::new();
    let mut row_width = 0.0;
    let mut row = 0usize;
    for label in labels {
        let entry = measure_plain_text(
            label,
            font_family,
            font_size,
            false,
            false,
            FontPolicy::Standard,
        )
        .width
            + 34.0;
        if entry > width - 40.0 {
            return Err("legend label does not fit");
        }
        if row_width + entry > width - 40.0 && row_width > 0.0 {
            row += 1;
            row_width = 0.0;
        }
        rows.push((row, row_width, entry));
        row_width += entry;
    }
    let height = if rows.is_empty() {
        0.0
    } else {
        (row + 1) as f32 * (font_size + 12.0) + 12.0
    };
    Ok((rows, height))
}
pub(crate) fn draw_bounded_legend(
    p: &BoundedLegend<'_>,
    d: &mut DecorationPainter<'_>,
    back: &mut Canvas,
    front: &mut Canvas,
) {
    let (w, h) = (p.size.0 as f32, p.size.1 as f32);
    let fs = p.font_size;
    for (row, x, _, label, color) in p.entries {
        let total = p
            .entries
            .iter()
            .filter(|(r, _, _, _, _)| r == row)
            .map(|(_, _, v, _, _)| v)
            .sum::<f32>();
        let lx = (w - total) * 0.5 + x;
        let ly = h - p.height + 12. + *row as f32 * (fs + 12.);
        d.on(DecorationId::Legend, back).fill(
            RectF {
                x: lx,
                y: ly - fs * 0.65,
                width: fs,
                height: fs * 0.65,
            },
            *color,
        );
        d.on(DecorationId::Legend, front).plain_text(
            p.font_family,
            label,
            lx + fs + 7.,
            ly,
            fs,
            p.color,
        );
    }
}
