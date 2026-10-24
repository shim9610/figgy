//! The bounded charts' shared legend rows (the radar chart's); drawing and
//! measured interaction geometry are produced together.
use super::*;
use crate::decoration::DecorationPainter;
use model::DecorationId;

/// The legend rows every bounded family lays out the same way: centred
/// rows at the bottom of the canvas, a colour swatch and the label per
/// entry. Entries are `(row, x within the row, width, label, colour)`.
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
