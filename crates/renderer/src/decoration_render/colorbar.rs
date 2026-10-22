//! The colourbar: its strip, border, axis chrome and title.

use super::*;
use crate::axis_render::*;

// Colourbar.
//
// No GPU pipeline: the strip is a CPU ramp and its ticks, labels, and title go
// through the *same* helpers the four chart axes use (`draw_tick`,
// `draw_tick_label`, `colorbar_title_placement`, `major_tick_values`,
// `format_tick_value`, `format_tick_power`). That reuse is the point — the bar's
// axis is an `AxisOptions`, so a log colourbar gets decade ticks and 10ⁿ labels
// from the code that already does it for a log axis, instead of a second
// implementation that drifts.

/// Draw the colourbar: strip, border, axis line, ticks, tick labels, title.
///
/// A no-op when there is no colourbar or it is hidden — and `colorbar_parts` has
/// then reserved no band for it, so there is nothing to leave blank.
pub(crate) fn draw_colorbar(
    canvas: &mut Canvas,
    config: &Config,
    da: &DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    let Some((bar, rect)) = drawn_colorbar(config, da) else {
        return;
    };

    draw_colorbar_strip(canvas, bar, &rect);

    // Border. Precise fill, wobbled outline — the same split as the legend box,
    // for the same reason: perturbing the fill too would visibly disagree with
    // the independently wobbled border.
    if bar.border_width > 0.0 {
        let paint = Paint::stroke(&bar.border_color, bar.border_width);
        stroker.stroke_rect_outline(
            canvas,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            &paint,
            "colorbar_strip",
        );
    }

    let axis = &bar.axis;
    let side = bar.side.clone();

    // Like a chart axis, the colourbar axis chrome may detach perpendicular to
    // its direction. The strip and title stay put; line, ticks, and tick labels
    // share this one transform so they cannot drift from one another.
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let detached = off_x != 0.0 || off_y != 0.0;
    if detached {
        canvas.save();
        canvas.translate(off_x, off_y);
    }

    // Line and ticks, from the walk `drawn_parts` reports.
    let majors = major_tick_values(axis);
    let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
    each_colorbar_axis_stroke(axis, side.clone(), &rect, &majors, |stroke, a, b| {
        let tag = match stroke {
            AxisStroke::Line => "axis_colorbar".to_string(),
            AxisStroke::Tick(i) => format!("tick_colorbar_{i}"),
        };
        stroker.stroke_segment(canvas, a, b, &paint, &tag);
    });

    // Tick labels, through the walk `drawn_parts` reports.
    each_colorbar_tick_label(axis, side.clone(), &rect, fp, &majors, |text, pos| {
        draw_tick_text(canvas, &text, pos, side.clone(), axis, fp);
    });

    if detached {
        canvas.restore();
    }

    // Title. Its anchor is the painted strip rather than the full data area,
    // so shortening, aligning, dragging, or resizing the bar moves the title
    // with it. The title keeps its own offset and does not follow a separately
    // detached axis line.
    let to = &axis.title_option;
    if to.visible {
        let m = measure_rich_text(&to.text, fp);
        let placement = colorbar_title_placement(side, &rect, axis, (to.offset_x, to.offset_y), m);
        draw_placed_title(canvas, &to.text, placement, fp);
    }
}

/// The colourbar and its strip rect when [`draw_colorbar`] draws one: present,
/// visible and not degenerate. [`tick_warnings`] reports the bar's axis only
/// then.
pub(crate) fn drawn_colorbar<'a>(
    config: &'a Config,
    da: &DataArea,
) -> Option<(&'a crate::config::ColorBarOptions, RectF)> {
    let bar = config.colorbar.as_ref()?;
    if !bar.visible {
        return None;
    }
    let rect = colorbar_rect(&config.chart_area, da, config.chart_title.top_margin, bar);
    (rect.width > 0.0 && rect.height > 0.0).then_some((bar, rect))
}

/// Paint the ramp into the strip rect.
///
/// One band per device pixel along the bar, coloured by `ColorMap::sample` at
/// the band's centre — the ramp is a continuous function, so there is no reason
/// for the strip to show coarser steps than the display can.
///
/// `field_columnar.wgsl` reimplements `ColorMap::sample` over the same stops and
/// takes its `t` from the same `ColorBarOptions` range, so a field cell and the
/// strip position naming its value come out the *same colour* rather than nearly
/// so. `heatmap_render.rs` asserts that against `color_for_z` directly.
///
/// `t` runs from the `axis.min` end of the bar. `axis.inverted` swaps which
/// screen end that is — it does not reverse the ramp, because inverting an axis
/// changes where a value is drawn and not what colour it has.
pub(super) fn draw_colorbar_strip(
    canvas: &mut Canvas,
    bar: &crate::config::ColorBarOptions,
    rect: &RectF,
) {
    let vertical = matches!(bar.side, Side::Left | Side::Right);
    let length = if vertical { rect.height } else { rect.width };
    // One band per device pixel, sized so no band is *thinner* than a pixel:
    // `floor`, not `ceil`, spreads the remainder over the bands instead of
    // leaving sub-pixel slivers to be anti-aliased into their neighbours. At
    // least one band, so a sub-pixel strip still shows a colour.
    let bands = (length.floor() as usize).max(1);
    let step = length / bands as f32;

    // Only the bands that land on the canvas are drawn. The strip follows the
    // data area, which may run far past the chart edges; without this the work
    // grows with the strip's length rather than the canvas (issue #6: a strip
    // billions of pixels long). Band colours still come from the full length.
    let Some((vx0, vy0, vx1, vy1)) = canvas.visible_bounds() else {
        return;
    };
    let (cross_lo, cross_hi, along_lo, along_hi, start) = if vertical {
        (rect.x, rect.x + rect.width, vy0, vy1, rect.y)
    } else {
        (rect.y, rect.y + rect.height, vx0, vx1, rect.x)
    };
    let (cross_vis_lo, cross_vis_hi) = if vertical { (vx0, vx1) } else { (vy0, vy1) };
    if cross_hi <= cross_vis_lo || cross_lo >= cross_vis_hi {
        return;
    }
    let band_at = |pos: f32| ((f64::from(pos) - f64::from(start)) / f64::from(step)).max(0.0);
    let first = (band_at(along_lo).floor() as usize).min(bands);
    let last = (band_at(along_hi).ceil() as usize).min(bands);

    // Interior band edges snap to device pixels — the pixmap grid, read from
    // the visible bounds so a translated canvas snaps too. Left at fractional
    // positions, a pixel on a shared edge takes two partial anti-aliased
    // covers, which composite to alpha c + (1 - c)² rather than the band's
    // own alpha: translucent rows whose drifting phase showed as plateaus and
    // jumps along the strip. `step >= 1` keeps every snapped band non-empty.
    // Only the strip's two outer ends stay fractional, anti-aliased against
    // the background like any other edge.
    let grid = f64::from(if vertical { vy0 } else { vx0 });
    let edge = |k: usize| -> f32 {
        if k == 0 {
            start
        } else if k >= bands {
            start + length
        } else {
            let pos = f64::from(start) + k as f64 * f64::from(step);
            ((pos - grid).round() + grid) as f32
        }
    };

    for i in first..last {
        // Screen fraction of this band's centre, measured from the low end of
        // the axis (bottom for a vertical bar, left for a horizontal one).
        let along = (i as f32 + 0.5) / bands as f32;
        let screen_fraction = if vertical { 1.0 - along } else { along };
        let value_fraction = if bar.axis.inverted {
            1.0 - screen_fraction
        } else {
            screen_fraction
        };
        let paint = Paint::fill(&bar.colormap.sample(value_fraction));
        let (lo, hi) = (edge(i), edge(i + 1));
        if vertical {
            canvas.draw_rect(rect.x, lo, rect.width, hi - lo, &paint);
        } else {
            canvas.draw_rect(lo, rect.y, hi - lo, rect.height, &paint);
        }
    }
}
