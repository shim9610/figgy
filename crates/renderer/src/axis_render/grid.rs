//! Grid lines and the linear grid walk the tick walkers share.

use super::*;

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

/// The grid, below the data: minor lines first so the majors paint over
/// them, like the rectangular grid always did.
pub(super) fn emit_grid(config: &Config, da: &DataArea, fp: FontPolicy, sink: &mut dyn Sink) {
    if config.polar.is_some() {
        polar::emit_polar_grid(config, da, fp, sink);
        return;
    }
    let g = &config.grid;
    let x_top = da.y as f32;
    let x_bot = (da.y + da.height) as f32;
    let y_left = da.x as f32;
    let y_right = (da.x + da.width) as f32;

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
        for (i, v) in minor_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
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
        for (i, v) in minor_tick_values_for_axis(&config.left_y, Side::Left, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
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
        for (i, v) in major_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
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
        for (i, v) in major_tick_values_for_axis(&config.left_y, Side::Left, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
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

pub(super) const GRID_EPSILON: f64 = 1.0e-9;

/// Range a tick walker may iterate: finite ends, positive span. NaN/±inf
/// ends otherwise slip past plain `max <= min` comparisons and saturate the
/// integer step bounds (`inf as i64` = i64::MAX — an unbounded walk).
pub(super) fn walkable_range(axis: &AxisOptions) -> bool {
    axis.min.is_finite() && axis.max.is_finite() && axis.max > axis.min
}

pub(super) fn linear_grid_values(
    axis: &AxisOptions,
    spacing: f64,
    max_intervals: usize,
) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if !walkable_range(axis)
        || !spacing.is_finite()
        || spacing <= 0.0
        || !spacing_advances_range(axis, spacing)
    {
        return out;
    }
    let first = ((axis.min / spacing) - GRID_EPSILON).ceil();
    let last = ((axis.max / spacing) + GRID_EPSILON).floor();
    if !first.is_finite() || !last.is_finite() || last < first {
        return out;
    }

    for offset in 0..=max_intervals {
        let index = first + offset as f64;
        if !index.is_finite() || index > last {
            break;
        }
        let value = index * spacing;
        if !value.is_finite() {
            continue;
        }
        if out
            .last()
            .is_some_and(|(_, previous): &(f64, f64)| *previous == value)
        {
            continue;
        }
        out.push((index, value));
    }
    out
}
