//! Axis titles, the rotated-title draw they share with the colourbar and the
//! chart title, and the side tags of the decoration stroker.

use super::*;

// Decoration stroke plumbing.
//
// Each layer entry (`draw_grid` / `draw_decoration_layer`) derives one
// [`DecoStroker`] from `config.draw_style` and threads it down by reference.
// `Precise` executes the exact pre-stroker code path (plain `draw_line` /
// `draw_rect`); the sketch arm mixes stable kind+index element tags
// ("axis_left", "tick_left_3", "grid_major_x_2", "legend_box") into the
// global seed so every element wobbles differently but identically across
// re-rasters. The selection overlay and all text stay off the stroker —
// unconditionally precise.

/// Stable side name used in stroker element tags.
pub(crate) fn side_tag(side: &Side) -> &'static str {
    match side {
        Side::Top => "top",
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    }
}

// Axis title (RichText).

pub(crate) fn draw_axis_title(
    canvas: &mut Canvas,
    config: &Config,
    da: &DataArea,
    side: Side,
    fp: FontPolicy,
) {
    let axis = match side {
        Side::Top => &config.top_x,
        Side::Bottom => &config.bottom_x,
        Side::Left => &config.left_y,
        Side::Right => &config.right_y,
    };
    let to = &axis.title_option;
    if !to.visible {
        return;
    }

    let m = measure_rich_text(&to.text, fp);
    let placement = axis_title_placement(
        side.clone(),
        &config.chart_area,
        da,
        TitleBand {
            out_margin: axis.out_margin,
            chart_title_margin: config.chart_title.top_margin,
            edge_inset: config.colorbar_band(&side),
        },
        (to.offset_x, to.offset_y),
        m,
    );
    draw_placed_title(canvas, &to.text, placement, fp);
}

pub(crate) fn draw_placed_title(
    canvas: &mut Canvas,
    rt: &crate::text::RichText,
    placement: TitlePlacement,
    fp: FontPolicy,
) {
    if placement.rotation_degrees == 0.0 {
        draw_rich_text(canvas, rt, placement.origin, fp);
    } else {
        canvas.save();
        canvas.rotate_at(
            placement.rotation_degrees,
            placement.rotation_center.0,
            placement.rotation_center.1,
        );
        draw_rich_text(canvas, rt, placement.origin, fp);
        canvas.restore();
    }
}
