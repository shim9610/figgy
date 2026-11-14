//! Grid lines, drawn at the tick walkers' values.

use super::*;
use crate::color::Color;
use crate::line::LineStylePreset;

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

/// The grid, below the data: minor lines first so the majors paint over
/// them, like the rectangular grid always did.
///
/// `scale` is the display or export scale `config`'s pixel values carry, the
/// grid the axis frames snap to (#249).
pub fn emit_grid(
    config: &Config,
    plot: &RectF,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
    scale: f32,
) {
    let g = &config.grid;
    // A grid line across a broken axis is drawn per piece and leaves its
    // gaps empty, like the data (#200): vertical lines over `left_y`'s
    // pieces, horizontal ones over `bottom_x`'s.
    let (ys, ny) = piece_spans(&config.left_y, plot, true);
    let (xs, nx) = piece_spans(&config.bottom_x, plot, false);

    let mut line = |a: (f32, f32),
                    b: (f32, f32),
                    color: &Color,
                    width: f32,
                    style: &LineStylePreset,
                    tag: &str| {
        let vertical = a.0 == b.0;
        let spans = if vertical { &ys[..ny] } else { &xs[..nx] };
        // A line on a piece's edge at a gap stays inside its piece, so no
        // half of it shows in the gap (#249); the plot's own edges are
        // left as they are.
        let (a, b) = if vertical {
            let x = inside_piece(a.0, &xs[..nx], width, scale);
            ((x, a.1), (x, b.1))
        } else {
            let y = inside_piece(a.1, &ys[..ny], width, scale);
            ((a.0, y), (b.0, y))
        };
        for &(lo, hi) in spans {
            let (p, q) = if vertical {
                ((a.0, lo), (b.0, hi))
            } else {
                ((lo, a.1), (hi, b.1))
            };
            sink.stroke(
                Slot::Below,
                [p.0, p.1],
                [q.0, q.1],
                &StrokeStyle {
                    width,
                    color: *color,
                    dash: style.pattern(),
                    tag,
                },
            );
        }
    };
    let (x_top, x_bot) = (plot.y, plot.y + plot.height);
    let (y_left, y_right) = (plot.x, plot.x + plot.width);

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        for (i, v) in minor_tick_values_for_axis(&config.bottom_x, Side::Bottom, plot, measure)
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
        for (i, v) in minor_tick_values_for_axis(&config.left_y, Side::Left, plot, measure)
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
        for (i, v) in major_tick_values_for_axis(&config.bottom_x, Side::Bottom, plot, measure)
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
        for (i, v) in major_tick_values_for_axis(&config.left_y, Side::Left, plot, measure)
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

/// `v` moved inside the piece span of `spans` it falls in by half of a
/// line `width`, at the edges that face a gap only, each edge on the
/// logical pixel grid of `scale` where the axis frames stand
/// (`chrome::line`).
fn inside_piece(v: f32, spans: &[(f32, f32)], width: f32, scale: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let half = width.max(scale) * 0.5;
    let grid = |e: f32| (e / scale).round() * scale;
    let last = spans.len().saturating_sub(1);
    for (k, &(lo, hi)) in spans.iter().enumerate() {
        if v < lo - half || v > hi + half {
            continue;
        }
        let lo = if k > 0 {
            grid(lo) + half
        } else {
            f32::NEG_INFINITY
        };
        let hi = if k < last {
            grid(hi) - half
        } else {
            f32::INFINITY
        };
        return if lo <= hi { v.clamp(lo, hi) } else { v };
    }
    v
}

/// The screen spans of `axis`' pieces along the plot (#200), ascending: `y`
/// spans of a vertical axis, `x` spans of a horizontal one; the whole edge
/// when the axis is unbroken.
fn piece_spans(
    axis: &AxisOptions,
    plot: &RectF,
    vertical: bool,
) -> ([(f32, f32); crate::config::MAX_AXIS_BREAKS + 1], usize) {
    let full = if vertical {
        (plot.y, plot.y + plot.height)
    } else {
        (plot.x, plot.x + plot.width)
    };
    let mut out = [full; crate::config::MAX_AXIS_BREAKS + 1];
    let map = crate::layout::BreakMap::of(axis, full.1 - full.0);
    if !map.is_broken() {
        return (out, 1);
    }
    let mut count = 0;
    for ((a, b), slot) in map.pieces().zip(out.iter_mut()) {
        let (a, b) = if axis.inverted {
            (1.0 - b, 1.0 - a)
        } else {
            (a, b)
        };
        let (a, b) = (a as f32, b as f32);
        *slot = if vertical {
            (
                plot.y + plot.height * (1.0 - b),
                plot.y + plot.height * (1.0 - a),
            )
        } else {
            (plot.x + plot.width * a, plot.x + plot.width * b)
        };
        count += 1;
    }
    out[..count].sort_by(|p, q| p.0.total_cmp(&q.0));
    (out, count)
}
