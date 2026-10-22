//! Selection highlight boxes and their resize handles.

use super::*;

/// Draw selection highlight boxes — the raster realization of the model's
/// `Selectable::selection_box` policy. Box coordinates are taken as-is in the
/// canvas frame; the raster entry above handles the chart_area shift.
/// Resize handles (when present) are drawn as white squares with the
/// selection color as border, slide-editor style.
pub fn draw_selection_boxes(canvas: &mut Canvas, boxes: &[SelectionBox]) {
    for b in boxes {
        let outline = Paint::stroke(&b.color, b.stroke_width);
        canvas.draw_rect(b.rect.x, b.rect.y, b.rect.width, b.rect.height, &outline);

        if b.handles.is_empty() {
            continue;
        }
        let fill = Paint::fill(&Color::WHITE);
        for h in &b.handles {
            canvas.draw_rect(h.x, h.y, h.width, h.height, &fill);
            canvas.draw_rect(h.x, h.y, h.width, h.height, &outline);
        }
    }
}
