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
    // Category bands first, under every grid line (#357).
    emit_category_bands(&config.bottom_x, Side::Bottom, plot, sink);
    emit_category_bands(&config.left_y, Side::Left, plot, sink);

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

/// The bands of a categorical `axis` on `side` (#357,
/// [`crate::config::AxisOptions::category_bands`]): one rectangle across
/// the plot over each slot its pattern fills, from half a category before
/// the category to half after, clipped to the plot. A value axis, or an
/// axis without bands, emits nothing. Every category is visited: the
/// count is the SSOT's, with no limit.
fn emit_category_bands(axis: &AxisOptions, side: Side, plot: &RectF, sink: &mut dyn Sink) {
    let (Some(bands), Some(categories)) = (axis.category_bands, axis.categories.as_ref()) else {
        return;
    };
    let vertical = matches!(side, Side::Left | Side::Right);
    let (lo, hi) = if vertical {
        (plot.y, plot.y + plot.height)
    } else {
        (plot.x, plot.x + plot.width)
    };
    for index in (0..categories.len()).filter(|&i| bands.every.fills(i)) {
        let at = |v: f64| {
            let p = value_to_screen(v, axis, side.clone(), plot);
            if vertical { p.1 } else { p.0 }
        };
        let (a, b) = (at(index as f64 - 0.5), at(index as f64 + 0.5));
        let (a, b) = (a.min(b).max(lo), a.max(b).min(hi));
        if !(b > a) {
            continue;
        }
        let rect = if vertical {
            RectF {
                x: plot.x,
                y: a,
                width: plot.width,
                height: b - a,
            }
        } else {
            RectF {
                x: a,
                y: plot.y,
                width: b - a,
                height: plot.height,
            }
        };
        sink.rect(Slot::Below, rect, Some(bands.fill), None);
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

#[cfg(test)]
mod tests {
    //! Category bands (#357): a rectangle across the plot over every other
    //! category slot, under the grid, for any number of categories.
    use super::*;
    use crate::config::{Category, CategoryBandEvery, CategoryBands};
    use crate::sink::{DrawnPart, Parts};
    use crate::text::{RichText, TextExtents};

    struct NoMeasure;
    impl MeasureText for NoMeasure {
        fn measure_rich(&self, _: &RichText) -> TextExtents {
            TextExtents {
                width: 0.0,
                ascent: 0.0,
                descent: 0.0,
            }
        }
    }

    const PLOT: RectF = RectF {
        x: 50.0,
        y: 20.0,
        width: 300.0,
        height: 200.0,
    };

    fn rects(config: &Config) -> Vec<RectF> {
        let mut out = Vec::new();
        emit_grid(
            config,
            &PLOT,
            &NoMeasure,
            &mut Parts(&mut |part| {
                if let DrawnPart::Rect(r) = part {
                    out.push(r);
                }
            }),
            1.0,
        );
        out
    }

    fn rows(n: usize) -> Config {
        let mut config = crate::defaults::config();
        config.left_y.categories = Some(
            (0..n)
                .map(|i| Category::new(format!("r{i}"), format!("Row {i}")))
                .collect(),
        );
        config.left_y.category_bands = Some(CategoryBands {
            fill: crate::Color::from_rgb8(240, 240, 240),
            every: CategoryBandEvery::Alternate,
        });
        config
    }

    #[test]
    fn every_other_row_gets_a_band_across_the_plot() {
        let config = rows(5);
        let bands = rects(&config);
        // Rows 1 and 3, each one row of 200 / 5 px high, full width.
        assert_eq!(bands.len(), 2);
        for band in &bands {
            assert_eq!((band.x, band.width), (PLOT.x, PLOT.width));
            assert!((band.height - 40.0).abs() < 1e-3, "{band:?}");
        }
        // The first row stays clear whichever way the axis runs: inverted,
        // row 0 is at the top and row 1's band starts one row below it.
        let mut inverted = config.clone();
        inverted.left_y.inverted = true;
        let top = rects(&inverted)
            .iter()
            .map(|r| r.y)
            .fold(f32::INFINITY, f32::min);
        assert!((top - (PLOT.y + 40.0)).abs() < 1e-3, "{top}");
        // No bands without the option, or on a value axis.
        let mut plain = config.clone();
        plain.left_y.category_bands = None;
        assert!(rects(&plain).is_empty());
        let mut value = crate::defaults::config();
        value.left_y.category_bands = config.left_y.category_bands;
        assert!(rects(&value).is_empty());
    }

    /// Rule 10: the bands follow every category, however many.
    #[test]
    fn bands_cover_every_category_with_no_limit() {
        let n = 5001;
        assert_eq!(rects(&rows(n)).len(), n / 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn bands_round_trip_and_older_documents_have_none() {
        let config = rows(3);
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("category_bands"));
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
        let plain = serde_json::to_string(&crate::defaults::config()).unwrap();
        assert!(!plain.contains("category_bands"));
    }

    #[test]
    fn a_band_fill_outside_the_colour_range_is_refused() {
        let mut config = rows(3);
        if let Some(bands) = config.left_y.category_bands.as_mut() {
            bands.fill.a = 1.5;
        }
        let err = config.validate_chart().unwrap_err();
        assert_eq!(err.kind, crate::RuleKind::CategoryBandsFill);
    }
}
