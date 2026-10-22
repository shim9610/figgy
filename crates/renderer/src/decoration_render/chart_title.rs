//! The chart title.

use super::*;

// Chart title (RichText).

pub(crate) fn draw_chart_title(canvas: &mut Canvas, config: &Config, fp: FontPolicy) {
    let ct = &config.chart_title;
    let m = measure_rich_text(&ct.text, fp);
    let placement = chart_title_placement(
        &config.chart_area,
        ct.top_margin,
        (ct.offset_x, ct.offset_y),
        m,
    );
    draw_placed_title(canvas, &ct.text, placement, fp);
}
