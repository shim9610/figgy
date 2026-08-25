use crate::config::{AxisOptions, BarAlign, ColorBarOptions, LegendCorner, TickVisibility};
use crate::text::TextExtents;

use super::{ChartArea, DataArea, RectF, Side};

/// Minimum gap between a tick end and its label, in pixels.
pub const LABEL_GAP: f32 = 4.0;

/// Inset between a legend box and its data-area corner, in pixels.
pub const LEGEND_INSET: f32 = 6.0;

/// Axis-line endpoints on pixel centers. Top/left use the first inside pixel;
/// bottom/right use the last inside pixel.
pub fn axis_anchor(side: Side, data_area: &DataArea) -> ((f32, f32), (f32, f32)) {
    let x0 = data_area.x as f32 + 0.5;
    let y0 = data_area.y as f32 + 0.5;
    let x1 = (data_area.x + data_area.width) as f32 - 0.5;
    let y1 = (data_area.y + data_area.height) as f32 - 0.5;
    match side {
        Side::Top => ((x0, y0), (x1, y0)),
        Side::Bottom => ((x0, y1), (x1, y1)),
        Side::Left => ((x0, y0), (x0, y1)),
        Side::Right => ((x1, y0), (x1, y1)),
    }
}

/// Perpendicular screen translation for a detached axis.
pub fn axis_offset(side: Side, line_offset: f32) -> (f32, f32) {
    match side {
        Side::Left | Side::Right => (line_offset, 0.0),
        Side::Top | Side::Bottom => (0.0, line_offset),
    }
}

/// Visible reach on the inward and outward sides of an axis line.
pub fn axis_visibility_extent(axis: &AxisOptions) -> (f32, f32) {
    let tick = axis.major_tick_length;
    let (inward, outward) = match axis.tick {
        TickVisibility::None => (0.0, 0.0),
        TickVisibility::Outside => (0.0, tick),
        TickVisibility::Inside => (tick, 0.0),
        TickVisibility::Both => (tick, tick),
    };
    let half_line = if axis.line_visible {
        axis.line_width.max(1.0) * 0.5
    } else {
        0.0
    };
    (inward.max(half_line), outward.max(half_line))
}

/// Axis line/tick interaction bounds, including detached-axis translation.
pub fn axis_visibility_rect(side: Side, data_area: &DataArea, axis: &AxisOptions) -> RectF {
    let (inward, outward) = axis_visibility_extent(axis);
    let rect = match side.clone() {
        Side::Top => RectF {
            x: data_area.x as f32,
            y: data_area.y as f32 - outward,
            width: data_area.width as f32,
            height: inward + outward,
        },
        Side::Bottom => RectF {
            x: data_area.x as f32,
            y: (data_area.y + data_area.height) as f32 - inward,
            width: data_area.width as f32,
            height: inward + outward,
        },
        Side::Left => RectF {
            x: data_area.x as f32 - outward,
            y: data_area.y as f32,
            width: inward + outward,
            height: data_area.height as f32,
        },
        Side::Right => RectF {
            x: (data_area.x + data_area.width) as f32 - inward,
            y: data_area.y as f32,
            width: inward + outward,
            height: data_area.height as f32,
        },
    };
    let (dx, dy) = axis_offset(side, axis.line_offset);
    rect.translated(dx, dy)
}

/// Axis line/tick interaction bounds when the axis runs on one edge of an
/// arbitrary rectangle. The colourbar uses this with its *painted strip*
/// rectangle, so a shortened, aligned, or freely offset strip and its axis
/// chrome cannot acquire different interaction geometry.
pub fn rect_axis_visibility_rect(side: Side, rect: &RectF, axis: &AxisOptions) -> RectF {
    let (inward, outward) = axis_visibility_extent(axis);
    let bounds = match side.clone() {
        Side::Top => RectF {
            x: rect.x,
            y: rect.y - outward,
            width: rect.width,
            height: inward + outward,
        },
        Side::Bottom => RectF {
            x: rect.x,
            y: rect.y + rect.height - inward,
            width: rect.width,
            height: inward + outward,
        },
        Side::Left => RectF {
            x: rect.x - outward,
            y: rect.y,
            width: inward + outward,
            height: rect.height,
        },
        Side::Right => RectF {
            x: rect.x + rect.width - inward,
            y: rect.y,
            width: inward + outward,
            height: rect.height,
        },
    };
    let (dx, dy) = axis_offset(side, axis.line_offset);
    bounds.translated(dx, dy)
}

/// Place a fraction on one side of a rectangle. Horizontal sides run
/// left-to-right; vertical sides run bottom-to-top, matching numeric axes.
/// The colourbar renderer, label hit geometry, and tests all use this one
/// mapping rather than independently deciding which screen end is low.
pub fn point_on_rect_side(t: f32, side: &Side, rect: &RectF) -> (f32, f32) {
    match side {
        Side::Top => (rect.x + t * rect.width, rect.y),
        Side::Bottom => (rect.x + t * rect.width, rect.y + rect.height),
        Side::Left => (rect.x, rect.y + rect.height - t * rect.height),
        Side::Right => (rect.x + rect.width, rect.y + rect.height - t * rect.height),
    }
}

/// Baseline origin for one measured tick label.
pub fn label_origin(
    side: Side,
    tick_position: (f32, f32),
    outward: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> (f32, f32) {
    let (x, y) = match side {
        Side::Top => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 - outward - LABEL_GAP - extents.descent,
        ),
        Side::Bottom => (
            tick_position.0 - extents.width * 0.5,
            tick_position.1 + outward + LABEL_GAP + extents.ascent,
        ),
        Side::Left => (
            tick_position.0 - outward - LABEL_GAP - extents.width,
            tick_position.1 + (extents.ascent - extents.descent) * 0.5,
        ),
        Side::Right => (
            tick_position.0 + outward + LABEL_GAP,
            tick_position.1 + (extents.ascent - extents.descent) * 0.5,
        ),
    };
    (x + offset.0, y + offset.1)
}

/// Pixel rectangle occupied by a label drawn at a baseline origin.
pub fn label_rect(origin: (f32, f32), extents: TextExtents) -> RectF {
    RectF {
        x: origin.0,
        y: origin.1 - extents.ascent,
        width: extents.width,
        height: extents.height(),
    }
}

/// Draw origin and optional rotation for a chart or axis title.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitlePlacement {
    pub origin: (f32, f32),
    pub rotation_center: (f32, f32),
    pub rotation_degrees: f32,
}

impl TitlePlacement {
    /// Axis-aligned screen rectangle of the placed text envelope.
    pub fn rect(self, extents: TextExtents) -> RectF {
        let local = label_rect(self.origin, extents);
        match self.rotation_degrees {
            -90.0 => RectF {
                x: self.rotation_center.0 + (local.y - self.rotation_center.1),
                y: self.rotation_center.1 - (local.x + local.width - self.rotation_center.0),
                width: local.height,
                height: local.width,
            },
            90.0 => RectF {
                x: self.rotation_center.0 - (local.y + local.height - self.rotation_center.1),
                y: self.rotation_center.1 + (local.x - self.rotation_center.0),
                width: local.height,
                height: local.width,
            },
            _ => local,
        }
    }
}

pub fn chart_title_placement(
    chart_area: &ChartArea,
    top_margin: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let origin = (
        chart_area.x as f32 + chart_area.width as f32 * 0.5 - extents.width * 0.5 + offset.0,
        chart_area.y as f32 + (top_margin - extents.height()) * 0.5 + extents.ascent + offset.1,
    );
    TitlePlacement {
        origin,
        rotation_center: origin,
        rotation_degrees: 0.0,
    }
}

/// The margin geometry an axis title is placed inside.
///
/// A struct rather than three loose `f32` parameters: they are all pixel
/// lengths measured along the same direction, and a positional swap between
/// them would move a title without failing to compile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBand {
    /// The axis' own outer margin — the band its title and tick labels share.
    pub out_margin: f32,
    /// The chart title band at the top of the chart area. Only `Side::Top` sits
    /// below it; the other sides ignore this.
    pub chart_title_margin: f32,
    /// How far this side's axis band is pushed in from the chart-area edge by
    /// whatever occupies the outside of that margin — today the colourbar band
    /// ([`Config::colorbar_band`](crate::config::Config::colorbar_band)), zero on
    /// the other three sides. Without it the placement assumes the axis band
    /// touches the chart edge, and a title on the colourbar's side would be
    /// drawn on top of the bar.
    pub edge_inset: f32,
}

/// Where an axis title sits inside its side's margin.
pub fn axis_title_placement(
    side: Side,
    chart_area: &ChartArea,
    data_area: &DataArea,
    band: TitleBand,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let TitleBand {
        out_margin,
        chart_title_margin,
        edge_inset,
    } = band;
    match side {
        Side::Top | Side::Bottom => {
            let band_top = if matches!(side, Side::Top) {
                chart_area.y as f32 + chart_title_margin + edge_inset
            } else {
                (chart_area.y + chart_area.height) as f32 - out_margin - edge_inset
            };
            let origin = (
                data_area.x as f32 + data_area.width as f32 * 0.5 - extents.width * 0.5 + offset.0,
                band_top + (out_margin - extents.height()) * 0.5 + extents.ascent + offset.1,
            );
            TitlePlacement {
                origin,
                rotation_center: origin,
                rotation_degrees: 0.0,
            }
        }
        Side::Left | Side::Right => {
            let center = (
                if matches!(side, Side::Left) {
                    chart_area.x as f32 + edge_inset + out_margin * 0.5
                } else {
                    (chart_area.x + chart_area.width) as f32 - edge_inset - out_margin * 0.5
                },
                data_area.y as f32 + data_area.height as f32 * 0.5,
            );
            TitlePlacement {
                origin: (
                    center.0 - extents.width * 0.5 + offset.0,
                    center.1 + (extents.ascent - extents.descent) * 0.5 + offset.1,
                ),
                rotation_center: center,
                rotation_degrees: if matches!(side, Side::Left) {
                    -90.0
                } else {
                    90.0
                },
            }
        }
    }
}

/// Place a colourbar title from the actual painted strip rectangle.
///
/// The title sits in `axis.out_margin`, immediately beyond the strip's major
/// tick reach, and is centred along the strip's own long dimension. Therefore
/// `length_frac`, `align`, `offset_{x,y}`, and resize all move the title through
/// the same rectangle that the renderer paints and the hit map selects.
pub fn colorbar_title_placement(
    side: Side,
    strip: &RectF,
    axis: &AxisOptions,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    let tick = axis.major_tick_length.max(0.0);
    let out_margin = axis.out_margin.max(0.0);
    match side {
        Side::Top | Side::Bottom => {
            let band_top = if matches!(side, Side::Top) {
                strip.y - tick - out_margin
            } else {
                strip.y + strip.height + tick
            };
            let origin = (
                strip.x + strip.width * 0.5 - extents.width * 0.5 + offset.0,
                band_top + (out_margin - extents.height()) * 0.5 + extents.ascent + offset.1,
            );
            TitlePlacement {
                origin,
                rotation_center: origin,
                rotation_degrees: 0.0,
            }
        }
        Side::Left | Side::Right => {
            let center = (
                if matches!(side, Side::Left) {
                    strip.x - tick - out_margin * 0.5
                } else {
                    strip.x + strip.width + tick + out_margin * 0.5
                },
                strip.y + strip.height * 0.5,
            );
            TitlePlacement {
                origin: (
                    center.0 - extents.width * 0.5 + offset.0,
                    center.1 + (extents.ascent - extents.descent) * 0.5 + offset.1,
                ),
                rotation_center: center,
                rotation_degrees: if matches!(side, Side::Left) {
                    -90.0
                } else {
                    90.0
                },
            }
        }
    }
}

/// Convert an axis-title local offset to its rotated screen-space offset.
pub fn axis_title_offset_to_screen(side: Side, offset: (f32, f32)) -> (f32, f32) {
    match side {
        Side::Left => (offset.1, -offset.0),
        Side::Right => (-offset.1, offset.0),
        Side::Top | Side::Bottom => offset,
    }
}

/// Convert a screen-space drag delta to an axis title's local offset frame.
pub fn screen_offset_to_axis_title(side: Side, offset: (f32, f32)) -> (f32, f32) {
    match side {
        Side::Left => (-offset.1, offset.0),
        Side::Right => (offset.1, -offset.0),
        Side::Top | Side::Bottom => offset,
    }
}

pub fn legend_rect(
    data_area: &DataArea,
    corner: LegendCorner,
    padding: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> RectF {
    let width = extents.width + padding * 2.0;
    let height = extents.height() + padding * 2.0;
    let x = match corner {
        LegendCorner::TopLeft | LegendCorner::BottomLeft => data_area.x as f32 + LEGEND_INSET,
        LegendCorner::TopRight | LegendCorner::BottomRight => {
            (data_area.x + data_area.width) as f32 - width - LEGEND_INSET
        }
    };
    let y = match corner {
        LegendCorner::TopLeft | LegendCorner::TopRight => data_area.y as f32 + LEGEND_INSET,
        LegendCorner::BottomLeft | LegendCorner::BottomRight => {
            (data_area.y + data_area.height) as f32 - height - LEGEND_INSET
        }
    };
    RectF {
        x: x + offset.0,
        y: y + offset.1,
        width,
        height,
    }
}

/// The colourbar strip's rectangle.
///
/// `side` alone decides the orientation: `Left` / `Right` give a vertical strip
/// `thickness_px` wide, `Top` / `Bottom` a horizontal one `thickness_px` tall.
/// The long dimension is `length_frac` of the data area's matching dimension,
/// placed by `align`, so the bar lines up with the plot it describes. The
/// strip's axis runs along its outer long edge — which is why its ends carry
/// `axis.min` / `axis.max` rather than the data area's corners.
///
/// **The short dimension is measured inward from the chart-area edge, not
/// outward from the data area.** The colourbar band is the *outermost* part of
/// its side's margin: from the chart edge inward it is the strip's own label
/// margin and tick, then the strip, then `gap_px`, and only then the chart
/// axis' band next to the data area. Placing the strip immediately outside the
/// data area instead would sit it on top of that axis' tick labels whenever
/// they are visible on the same side — and axis labels are drawn from the data
/// area outward, so they cannot move out of the way.
///
/// The two agree by construction: `gap_px` is what is left between the axis band
/// and the strip precisely because `colorbar_parts` charged the same terms to
/// that side's margin.
///
/// `offset_x` / `offset_y` shift the result and nothing else: they are
/// margin-noncontributing, exactly like the legend's and the titles' offsets, so
/// dragging the bar does not reflow the data area out from under the pointer.
///
/// Callers get a rect for a hidden bar too — the visibility decision belongs to
/// the drawing code and to `colorbar_contribution`, not here.
pub fn colorbar_rect(
    chart_area: &ChartArea,
    data_area: &DataArea,
    chart_title_margin: f32,
    bar: &ColorBarOptions,
) -> RectF {
    let frac = if bar.length_frac.is_finite() {
        bar.length_frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let thickness = bar.thickness_px.max(0.0);
    // The strip's distance from the chart edge: its labels, then its ticks, then
    // the strip itself.
    let outer = bar.axis.out_margin.max(0.0) + bar.axis.major_tick_length.max(0.0) + thickness;
    let (dx, dy, dw, dh) = (
        data_area.x as f32,
        data_area.y as f32,
        data_area.width as f32,
        data_area.height as f32,
    );
    let (cx, cy, cw, ch) = (
        chart_area.x as f32,
        chart_area.y as f32,
        chart_area.width as f32,
        chart_area.height as f32,
    );

    // Written as one match over the four sides so a nested wildcard never stands
    // in for a side.
    let vertical_length = dh * frac;
    let horizontal_length = dw * frac;
    let anchored = match bar.side {
        Side::Left => RectF {
            x: cx + outer - thickness,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        Side::Right => RectF {
            x: cx + cw - outer,
            y: dy + align_offset(dh, vertical_length, &bar.align),
            width: thickness,
            height: vertical_length,
        },
        // The chart title owns the very top of the chart area; the colourbar
        // band is the outermost part of what is left, exactly as
        // `top_total` sums them.
        Side::Top => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + chart_title_margin + outer - thickness,
            width: horizontal_length,
            height: thickness,
        },
        Side::Bottom => RectF {
            x: dx + align_offset(dw, horizontal_length, &bar.align),
            y: cy + ch - outer,
            width: horizontal_length,
            height: thickness,
        },
    };
    anchored.translated(bar.offset_x, bar.offset_y)
}

/// Where a strip of `length` starts within `available`. `Start` is top / left,
/// matching the screen direction the sides are named in.
fn align_offset(available: f32, length: f32, align: &BarAlign) -> f32 {
    match align {
        BarAlign::Start => 0.0,
        BarAlign::Center => (available - length) * 0.5,
        BarAlign::End => available - length,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::Rect;

    fn areas() -> (ChartArea, DataArea) {
        (
            ChartArea(Rect {
                x: 10,
                y: 20,
                width: 300,
                height: 200,
            }),
            DataArea(Rect {
                x: 50,
                y: 60,
                width: 200,
                height: 100,
            }),
        )
    }

    fn band(out_margin: f32, chart_title_margin: f32, edge_inset: f32) -> TitleBand {
        TitleBand {
            out_margin,
            chart_title_margin,
            edge_inset,
        }
    }

    fn extents() -> TextExtents {
        TextExtents {
            width: 20.0,
            ascent: 7.0,
            descent: 3.0,
        }
    }

    #[test]
    fn axis_anchors_and_visibility_preserve_half_pixels() {
        let (_, data) = areas();
        assert_eq!(axis_anchor(Side::Top, &data), ((50.5, 60.5), (249.5, 60.5)));
        assert_eq!(
            axis_anchor(Side::Right, &data),
            ((249.5, 60.5), (249.5, 159.5))
        );

        let mut axis = default_config().left_y;
        axis.tick = TickVisibility::Both;
        axis.major_tick_length = 5.0;
        axis.line_width = 1.0;
        axis.line_offset = 2.0;
        assert_eq!(axis_visibility_extent(&axis), (5.0, 5.0));
        assert_eq!(
            axis_visibility_rect(Side::Left, &data, &axis),
            RectF {
                x: 47.0,
                y: 60.0,
                width: 10.0,
                height: 100.0,
            }
        );
    }

    #[test]
    fn label_origin_and_rect_cover_all_sides_and_offsets() {
        let tick = (100.0, 200.0);
        let offset = (2.0, 4.0);
        let m = extents();
        let expected = [
            (Side::Top, (92.0, 192.0)),
            (Side::Bottom, (92.0, 220.0)),
            (Side::Left, (73.0, 206.0)),
            (Side::Right, (111.0, 206.0)),
        ];
        for (side, origin) in expected {
            let actual = label_origin(side, tick, 5.0, offset, m);
            assert_eq!(actual, origin);
            assert_eq!(label_rect(actual, m).height, 10.0);
        }
    }

    #[test]
    fn title_placement_keeps_rotation_signs_and_offset_frames() {
        let (chart, data) = areas();
        let m = extents();
        let left = axis_title_placement(
            Side::Left,
            &chart,
            &data,
            band(40.0, 32.0, 0.0),
            (2.0, 3.0),
            m,
        );
        let right = axis_title_placement(
            Side::Right,
            &chart,
            &data,
            band(40.0, 32.0, 0.0),
            (2.0, 3.0),
            m,
        );
        assert_eq!(left.rotation_degrees, -90.0);
        assert_eq!(right.rotation_degrees, 90.0);
        assert_eq!(
            left.rect(m),
            RectF {
                x: 28.0,
                y: 98.0,
                width: 10.0,
                height: 20.0
            }
        );
        assert_eq!(
            right.rect(m),
            RectF {
                x: 282.0,
                y: 102.0,
                width: 10.0,
                height: 20.0
            }
        );

        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let local = (7.0, -4.0);
            assert_eq!(
                screen_offset_to_axis_title(side.clone(), axis_title_offset_to_screen(side, local)),
                local
            );
        }
    }

    #[test]
    fn colorbar_title_tracks_the_actual_strip_on_every_side() {
        let strip = RectF {
            x: 70.0,
            y: 80.0,
            width: 40.0,
            height: 100.0,
        };
        let moved = strip.translated(13.0, -9.0);
        let axis = crate::default::default_axis_options_colorbar();
        let m = extents();

        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let before =
                colorbar_title_placement(side.clone(), &strip, &axis, (0.0, 0.0), m).rect(m);
            let after = colorbar_title_placement(side, &moved, &axis, (0.0, 0.0), m).rect(m);
            assert_eq!(after, before.translated(13.0, -9.0));
        }

        let right = colorbar_title_placement(Side::Right, &strip, &axis, (0.0, 0.0), m).rect(m);
        assert_eq!(right.y + right.height * 0.5, strip.y + strip.height * 0.5);

        let top = colorbar_title_placement(Side::Top, &strip, &axis, (0.0, 0.0), m).rect(m);
        assert_eq!(top.x + top.width * 0.5, strip.x + strip.width * 0.5);
    }

    #[test]
    fn rect_axis_visibility_uses_tick_direction_and_line_offset() {
        let strip = RectF {
            x: 10.0,
            y: 20.0,
            width: 40.0,
            height: 100.0,
        };
        let mut axis = crate::default::default_axis_options_colorbar();
        axis.line_visible = false;
        axis.major_tick_length = 5.0;
        axis.line_offset = 3.0;

        axis.tick = TickVisibility::Outside;
        assert_eq!(
            rect_axis_visibility_rect(Side::Right, &strip, &axis),
            RectF {
                x: 53.0,
                y: 20.0,
                width: 5.0,
                height: 100.0,
            }
        );

        axis.tick = TickVisibility::Inside;
        assert_eq!(
            rect_axis_visibility_rect(Side::Right, &strip, &axis),
            RectF {
                x: 48.0,
                y: 20.0,
                width: 5.0,
                height: 100.0,
            }
        );
    }

    #[test]
    fn legend_rect_covers_all_corners_and_offsets() {
        let (_, data) = areas();
        let m = extents();
        let expected = [
            (LegendCorner::TopLeft, (57.0, 68.0)),
            (LegendCorner::TopRight, (215.0, 68.0)),
            (LegendCorner::BottomLeft, (57.0, 136.0)),
            (LegendCorner::BottomRight, (215.0, 136.0)),
        ];
        for (corner, (x, y)) in expected {
            let rect = legend_rect(&data, corner, 5.0, (1.0, 2.0), m);
            assert_eq!(
                rect,
                RectF {
                    x,
                    y,
                    width: 30.0,
                    height: 20.0
                }
            );
        }
    }

    /// A config whose data area really is the chart area minus its margins, so
    /// the band relationships the strip is placed by actually hold. The
    /// synthetic `areas()` fixture above cannot: its data area is not derived
    /// from any margin.
    fn colorbar_config(side: Side) -> (crate::config::Config, DataArea) {
        let mut cfg = crate::default::default_config();
        let mut bar = crate::default::default_colorbar_options();
        bar.side = side;
        bar.thickness_px = 20.0;
        bar.gap_px = 10.0;
        bar.length_frac = 0.5;
        bar.align = BarAlign::Center;
        cfg.colorbar = Some(bar);
        let da = cfg.data_area().expect("data area");
        (cfg, da)
    }

    fn rect_of(cfg: &crate::config::Config, da: &DataArea) -> RectF {
        colorbar_rect(
            &cfg.chart_area,
            da,
            cfg.chart_title.top_margin,
            cfg.colorbar.as_ref().expect("colourbar"),
        )
    }

    /// The band the strip sits in, taken apart. Every side must show the same
    /// three-part structure outward from the data area: the chart axis' band,
    /// then `gap_px`, then the strip — with the strip's own tick and label
    /// margin between it and the chart-area edge.
    #[test]
    fn the_colorbar_strip_sits_outside_the_axis_band_not_on_top_of_it() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let (cfg, da) = colorbar_config(side.clone());
            let bar = cfg.colorbar.as_ref().expect("colourbar");
            let rect = rect_of(&cfg, &da);
            let axis = crate::layout::axis_ref(&cfg, &side);
            let axis_band = axis.out_margin + axis.major_tick_length;
            let outer_band = bar.axis.out_margin + bar.axis.major_tick_length;
            let ca = &cfg.chart_area;

            let (from_data_area, to_chart_edge) = match side {
                Side::Left => (da.x as f32 - (rect.x + rect.width), rect.x - ca.x as f32),
                Side::Right => (
                    rect.x - (da.x + da.width) as f32,
                    (ca.x + ca.width) as f32 - (rect.x + rect.width),
                ),
                Side::Top => (
                    da.y as f32 - (rect.y + rect.height),
                    rect.y - (ca.y as f32 + cfg.chart_title.top_margin),
                ),
                Side::Bottom => (
                    rect.y - (da.y + da.height) as f32,
                    (ca.y + ca.height) as f32 - (rect.y + rect.height),
                ),
            };
            // Between the data area and the strip: the axis' own band, then the
            // gap. Nothing of the strip reaches into the axis' label space.
            assert!(
                (from_data_area - (axis_band + bar.gap_px)).abs() <= 1.0,
                "{side:?}: {from_data_area} px from the data area, expected axis band \
                 {axis_band} + gap {}",
                bar.gap_px
            );
            // Between the strip and the chart edge: the strip's tick and labels.
            assert!(
                (to_chart_edge - outer_band).abs() <= 1.0,
                "{side:?}: {to_chart_edge} px to the chart edge, expected {outer_band}"
            );
        }
    }

    // Orientation, length, and alignment along the side.
    #[test]
    fn colorbar_orientation_and_length_follow_side_and_length_frac() {
        let (cfg, da) = colorbar_config(Side::Right);
        let vertical = rect_of(&cfg, &da);
        assert_eq!(vertical.width, 20.0);
        assert_eq!(vertical.height, da.height as f32 * 0.5);
        // Centred along the data area.
        assert_eq!(vertical.y, da.y as f32 + da.height as f32 * 0.25);

        let (cfg, da) = colorbar_config(Side::Bottom);
        let horizontal = rect_of(&cfg, &da);
        assert_eq!(horizontal.height, 20.0);
        assert_eq!(horizontal.width, da.width as f32 * 0.5);
        assert_eq!(horizontal.x, da.x as f32 + da.width as f32 * 0.25);
    }

    // `align` places the strip along its side; `Start` is top / left in screen
    // direction, matching how the sides are named.
    #[test]
    fn colorbar_align_places_the_strip_along_its_side() {
        let (mut cfg, da) = colorbar_config(Side::Right);
        let quarter = da.height as f32 * 0.5;

        cfg.colorbar.as_mut().unwrap().align = BarAlign::Start;
        assert_eq!(rect_of(&cfg, &da).y, da.y as f32);
        cfg.colorbar.as_mut().unwrap().align = BarAlign::End;
        assert_eq!(rect_of(&cfg, &da).y, (da.y + da.height) as f32 - quarter);

        let (mut cfg, da) = colorbar_config(Side::Bottom);
        let half = da.width as f32 * 0.5;
        cfg.colorbar.as_mut().unwrap().align = BarAlign::Start;
        assert_eq!(rect_of(&cfg, &da).x, da.x as f32);
        cfg.colorbar.as_mut().unwrap().align = BarAlign::End;
        assert_eq!(rect_of(&cfg, &da).x, (da.x + da.width) as f32 - half);
    }

    // A full-length bar spans its side exactly, whatever the alignment.
    #[test]
    fn a_full_length_colorbar_spans_its_side() {
        for align in [BarAlign::Start, BarAlign::Center, BarAlign::End] {
            let (mut cfg, da) = colorbar_config(Side::Right);
            {
                let bar = cfg.colorbar.as_mut().unwrap();
                bar.length_frac = 1.0;
                bar.align = align.clone();
            }
            let rect = rect_of(&cfg, &da);
            assert_eq!(rect.y, da.y as f32, "{align:?}");
            assert_eq!(rect.height, da.height as f32, "{align:?}");
        }
    }

    // Nonsense dims produce a degenerate rect, not a panic and not a rect that
    // reaches back over the data area. `Config::validate` is what rejects them.
    #[test]
    fn colorbar_rect_survives_values_validate_would_reject() {
        let (mut cfg, da) = colorbar_config(Side::Right);
        for frac in [0.0, -1.0, f32::NAN, 2.0] {
            cfg.colorbar.as_mut().unwrap().length_frac = frac;
            let rect = rect_of(&cfg, &da);
            assert!(
                rect.height >= 0.0,
                "frac {frac} gave height {}",
                rect.height
            );
            assert!(
                rect.height <= da.height as f32,
                "frac {frac} overran its side"
            );
        }
        cfg.colorbar.as_mut().unwrap().length_frac = 0.5;
        cfg.colorbar.as_mut().unwrap().thickness_px = -5.0;
        let rect = rect_of(&cfg, &da);
        assert_eq!(rect.width, 0.0);
    }

    // The axis title band is pushed in by whatever occupies the outside of its
    // side's margin. Without the inset a title on the colourbar's side would be
    // centred on top of the bar.
    #[test]
    fn axis_title_placement_is_pushed_in_by_the_colorbar_band() {
        let (cfg, da) = colorbar_config(Side::Right);
        let colorbar_band = cfg.colorbar_band(&Side::Right);
        assert!(colorbar_band > 0.0);

        let placement = |inset: f32| {
            axis_title_placement(
                Side::Right,
                &cfg.chart_area,
                &da,
                band(cfg.right_y.out_margin, cfg.chart_title.top_margin, inset),
                (0.0, 0.0),
                extents(),
            )
        };
        let with_band = placement(colorbar_band);
        let without = placement(0.0);
        assert!(
            (without.rotation_center.0 - with_band.rotation_center.0 - colorbar_band).abs() < 1e-3,
            "the inset must move the title inward by exactly the band"
        );

        // And the sides the bar is not on are unaffected, because their band is 0.
        for side in [Side::Top, Side::Bottom, Side::Left] {
            assert_eq!(cfg.colorbar_band(&side), 0.0);
        }
    }
}
