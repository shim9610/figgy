use crate::config::{AxisOptions, LegendCorner, TickVisibility};
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

pub fn axis_title_placement(
    side: Side,
    chart_area: &ChartArea,
    data_area: &DataArea,
    chart_title_margin: f32,
    out_margin: f32,
    offset: (f32, f32),
    extents: TextExtents,
) -> TitlePlacement {
    match side {
        Side::Top | Side::Bottom => {
            let band_top = if matches!(side, Side::Top) {
                chart_area.y as f32 + chart_title_margin
            } else {
                (chart_area.y + chart_area.height) as f32 - out_margin
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
                    chart_area.x as f32 + out_margin * 0.5
                } else {
                    (chart_area.x + chart_area.width) as f32 - out_margin * 0.5
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
        let left = axis_title_placement(Side::Left, &chart, &data, 32.0, 40.0, (2.0, 3.0), m);
        let right = axis_title_placement(Side::Right, &chart, &data, 32.0, 40.0, (2.0, 3.0), m);
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
}
