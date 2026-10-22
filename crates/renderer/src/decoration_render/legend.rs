//! The legend box.

use super::*;

/// Draw the legend box in one corner of the data area.
///
/// The whole legend is **one rich document** (`legend.content`): `'\n'`
/// segments break lines, symbols are inline segments with per-segment color
/// overrides, and the document's font/font_size apply at draw time. The box
/// is the measured envelope plus `padding`, with the same formulas as the
/// model's `LegendElement` bounds (`model::select`) — change them together.
pub(crate) fn draw_legend(
    canvas: &mut Canvas,
    config: &Config,
    da: &crate::layout::DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    let lg = &config.legend;
    if !lg.visible || lg.content.segments.is_empty() {
        return;
    }

    let m = measure_rich_text(&lg.content, fp);
    let rect = legend_rect(da, lg.corner, lg.padding, (lg.offset_x, lg.offset_y), m);
    let (box_x, box_y, box_w, box_h) = (rect.x, rect.y, rect.width, rect.height);

    // Box background + border. The sketch stroker wobbles only the border
    // outline; the fill stays a precise rect — perturbing the fill too
    // would visibly disagree with the independently wobbled border.
    canvas.draw_rect(box_x, box_y, box_w, box_h, &Paint::fill(&lg.bg_color));
    let border = Paint::stroke(&lg.border_color, crate::layout::LEGEND_BORDER_WIDTH);
    stroker.stroke_rect_outline(canvas, box_x, box_y, box_w, box_h, &border, "legend_box");

    // Content: first baseline sits `ascent` below the padded top-left corner.
    draw_rich_text(
        canvas,
        &lg.content,
        (box_x + lg.padding, box_y + lg.padding + m.ascent),
        fp,
    );
}
