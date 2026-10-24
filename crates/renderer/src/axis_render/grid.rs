//! Grid lines, drawn at the tick walkers' values.

use super::*;

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

/// The grid, below the data: minor lines first so the majors paint over
/// them, like the rectangular grid always did.
pub(super) fn emit_grid(config: &Config, plot: &RectF, fp: FontPolicy, sink: &mut dyn Sink) {
    let g = &config.grid;
    let x_top = plot.y;
    let x_bot = plot.y + plot.height;
    let y_left = plot.x;
    let y_right = plot.x + plot.width;

    let mut line = |a: (f32, f32),
                    b: (f32, f32),
                    color: &Color,
                    width: f32,
                    style: &LineStylePreset,
                    tag: &str| {
        sink.stroke(
            Slot::Below,
            [a.0, a.1],
            [b.0, b.1],
            &StrokeStyle {
                width,
                color: *color,
                dash: style.pattern(),
                tag,
            },
        );
    };

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        for (i, v) in minor_tick_values_for_axis(&config.bottom_x, Side::Bottom, plot, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, plot);
            let tag = format!("grid_minor_x_{i}");
            line(
                (pos.0, x_top),
                (pos.0, x_bot),
                &g.minor_x_color,
                g.minor_x_width,
                &g.minor_x_style,
                &tag,
            );
        }
    }
    if g.show_minor_y {
        for (i, v) in minor_tick_values_for_axis(&config.left_y, Side::Left, plot, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.left_y, Side::Left, plot);
            let tag = format!("grid_minor_y_{i}");
            line(
                (y_left, pos.1),
                (y_right, pos.1),
                &g.minor_y_color,
                g.minor_y_width,
                &g.minor_y_style,
                &tag,
            );
        }
    }

    if g.show_major_x {
        for (i, v) in major_tick_values_for_axis(&config.bottom_x, Side::Bottom, plot, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, plot);
            let tag = format!("grid_major_x_{i}");
            line(
                (pos.0, x_top),
                (pos.0, x_bot),
                &g.major_x_color,
                g.major_x_width,
                &g.major_x_style,
                &tag,
            );
        }
    }
    if g.show_major_y {
        for (i, v) in major_tick_values_for_axis(&config.left_y, Side::Left, plot, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.left_y, Side::Left, plot);
            let tag = format!("grid_major_y_{i}");
            line(
                (y_left, pos.1),
                (y_right, pos.1),
                &g.major_y_color,
                g.major_y_width,
                &g.major_y_style,
                &tag,
            );
        }
    }
}
