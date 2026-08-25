use crate::text::TextExtents;

use super::{
    Config, LABEL_GAP, Side, TitleBand, axis_mut, axis_offset, axis_ref,
    axis_title_offset_to_screen, axis_title_placement, chart_title_placement, colorbar_rect,
    colorbar_title_placement, legend_rect, point_on_rect_side, screen_offset_to_axis_title,
};

// Nudge types.

#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    ChartTitle,
    AxisTitle(Side),
    AxisLabel(Side),
    /// The axis line itself — a *detached* move. Shifts only perpendicular to
    /// its direction (y-axes horizontally, x-axes vertically) via
    /// `AxisOptions::line_offset`; the data area, grid, and data transform
    /// stay put, so tick positions along the axis stay aligned with the data.
    Axis(Side),
    /// One edge of the data area — moves by adjusting that side's
    /// `out_margin`, dragging the whole data area boundary with it. Used by
    /// the data-area resize handles.
    DataAreaEdge(Side),
    /// The whole data area — translates without resizing by shifting opposite
    /// margins in tandem (left grows as right shrinks, etc.). Ticks, grid,
    /// and the data transform all derive from the data area, so everything
    /// moves as one.
    DataArea,
    /// The legend box — moves freely via `legend.offset_{x,y}` relative to
    /// its corner anchor.
    Legend,
    /// The colourbar strip — moves freely via `colorbar.offset_{x,y}` relative
    /// to its `side` + `align` anchor, the same rule as [`Self::Legend`].
    ColorBar,
    /// The colourbar axis line and ticks. Like a chart axis, a drag detaches it
    /// only perpendicular to its direction through `colorbar.axis.line_offset`.
    ColorBarAxis,
    /// The colourbar tick-label band. A drag updates the z axis' label offsets.
    ColorBarLabel,
    /// The colourbar title. A drag updates the z axis' title offsets in the
    /// title's rotated local frame.
    ColorBarTitle,
    /// One edge of the colourbar strip, named by the **screen** side of the
    /// strip it is on — the resize-handle target, mirroring
    /// [`Self::DataAreaEdge`].
    ///
    /// Which dimension it changes follows the bar's orientation, not the
    /// element: on a vertical bar the left/right edges are `thickness_px` and
    /// the top/bottom edges are `length_frac`; on a horizontal bar it is the
    /// other way round. The element stays orientation-free because the handle
    /// that produced it only knows screen directions.
    ColorBarEdge(Side),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NudgeResult {
    Moved,
    Rejected(NudgeReject),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NudgeReject {
    OutOfBounds,
}

// Per-element anchor and current offset.

/// Measurement-free label extent estimate used only for nudge containment.
const LABEL_EXTENT_EST: f32 = 10.0;
const ZERO_EXTENTS: TextExtents = TextExtents {
    width: 0.0,
    ascent: 0.0,
    descent: 0.0,
};

/// Measurement-free representative screen anchor used by nudge containment.
/// Exact measured bounds remain the `Selectable` policy.
fn element_anchor(cfg: &Config, element: &Element) -> Option<(f32, f32)> {
    let ca = &cfg.chart_area;
    let da = cfg.data_area().ok()?;

    let anchor = match element {
        Element::ChartTitle => {
            chart_title_placement(ca, cfg.chart_title.top_margin, (0.0, 0.0), ZERO_EXTENTS).origin
        }
        Element::AxisTitle(side) => {
            let axis = axis_ref(cfg, side);
            axis_title_placement(
                side.clone(),
                ca,
                &da,
                TitleBand {
                    out_margin: axis.out_margin,
                    chart_title_margin: cfg.chart_title.top_margin,
                    edge_inset: cfg.colorbar_band(side),
                },
                (0.0, 0.0),
                ZERO_EXTENTS,
            )
            .origin
        }
        // Axis label: just outside the tick end (with approximate label extent).
        Element::AxisLabel(Side::Top) => (
            da.x as f32 + da.width as f32 * 0.5,
            da.y as f32 - cfg.top_x.major_tick_length - LABEL_GAP - LABEL_EXTENT_EST,
        ),
        Element::AxisLabel(Side::Bottom) => (
            da.x as f32 + da.width as f32 * 0.5,
            (da.y + da.height) as f32
                + cfg.bottom_x.major_tick_length
                + LABEL_GAP
                + LABEL_EXTENT_EST,
        ),
        Element::AxisLabel(Side::Left) => (
            da.x as f32 - cfg.left_y.major_tick_length - LABEL_GAP - LABEL_EXTENT_EST,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        Element::AxisLabel(Side::Right) => (
            (da.x + da.width) as f32 + cfg.right_y.major_tick_length + LABEL_GAP + LABEL_EXTENT_EST,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        // Legend: corner anchor point (inset corner of the data area). The
        // box extent isn't known here (it needs text measurement), but the
        // chart_area containment check only needs a representative point.
        Element::Legend => {
            let rect = legend_rect(&da, cfg.legend.corner, 0.0, (0.0, 0.0), ZERO_EXTENTS);
            (rect.x, rect.y)
        }
        // Colourbar: the un-offset strip corner. Same shape as the legend —
        // a representative point is all the chart_area containment check needs.
        Element::ColorBar => {
            let bar = cfg.colorbar.as_ref()?;
            let mut anchored = bar.clone();
            anchored.offset_x = 0.0;
            anchored.offset_y = 0.0;
            let rect = colorbar_rect(ca, &da, cfg.chart_title.top_margin, &anchored);
            (rect.x, rect.y)
        }
        Element::ColorBarLabel => {
            let bar = cfg.colorbar.as_ref()?;
            let rect = colorbar_rect(ca, &da, cfg.chart_title.top_margin, bar);
            let pos = point_on_rect_side(0.5, &bar.side, &rect);
            let (axis_dx, axis_dy) = axis_offset(bar.side.clone(), bar.axis.line_offset);
            let reach = bar.axis.major_tick_length + LABEL_GAP + LABEL_EXTENT_EST;
            let (out_x, out_y) = match bar.side {
                Side::Top => (0.0, -reach),
                Side::Bottom => (0.0, reach),
                Side::Left => (-reach, 0.0),
                Side::Right => (reach, 0.0),
            };
            (pos.0 + axis_dx + out_x, pos.1 + axis_dy + out_y)
        }
        Element::ColorBarTitle => {
            let bar = cfg.colorbar.as_ref()?;
            let rect = colorbar_rect(ca, &da, cfg.chart_title.top_margin, bar);
            colorbar_title_placement(bar.side.clone(), &rect, &bar.axis, (0.0, 0.0), ZERO_EXTENTS)
                .origin
        }
        // Axis / data-area / colourbar-edge moves have their own rules —
        // dispatched before this function is reached.
        Element::Axis(_)
        | Element::ColorBarAxis
        | Element::DataAreaEdge(_)
        | Element::DataArea
        | Element::ColorBarEdge(_) => return None,
    };

    Some(anchor)
}

fn current_offset(cfg: &Config, element: &Element) -> (f32, f32) {
    match element {
        Element::ChartTitle => (cfg.chart_title.offset_x, cfg.chart_title.offset_y),
        Element::AxisTitle(side) => {
            let a = axis_ref(cfg, side);
            (a.title_option.offset_x, a.title_option.offset_y)
        }
        Element::AxisLabel(side) => {
            let a = axis_ref(cfg, side);
            (a.label_style.label_offset_x, a.label_style.label_offset_y)
        }
        Element::Legend => (cfg.legend.offset_x, cfg.legend.offset_y),
        Element::ColorBar => cfg
            .colorbar
            .as_ref()
            .map_or((0.0, 0.0), |bar| (bar.offset_x, bar.offset_y)),
        Element::ColorBarLabel => cfg.colorbar.as_ref().map_or((0.0, 0.0), |bar| {
            (
                bar.axis.label_style.label_offset_x,
                bar.axis.label_style.label_offset_y,
            )
        }),
        Element::ColorBarTitle => cfg.colorbar.as_ref().map_or((0.0, 0.0), |bar| {
            (
                bar.axis.title_option.offset_x,
                bar.axis.title_option.offset_y,
            )
        }),
        // Dispatched before the offset path.
        Element::Axis(_)
        | Element::ColorBarAxis
        | Element::DataAreaEdge(_)
        | Element::DataArea
        | Element::ColorBarEdge(_) => (0.0, 0.0),
    }
}

// Nudge methods.

impl Config {
    /// Apply a measurement-free approximate containment policy. Text-bearing
    /// elements use representative anchors here; exact glyph bounds are kept
    /// in the `Selectable` path and no text measurer is required by nudge.
    pub fn nudge(&mut self, element: Element, dx: f32, dy: f32) -> NudgeResult {
        // Axis / data-area-edge moves have their own rules (perpendicular
        // only); everything else moves via stored offsets below.
        if let Element::Axis(side) = element {
            return self.nudge_axis(side, dx, dy);
        }
        if let Element::ColorBarAxis = element {
            return self.nudge_colorbar_axis(dx, dy);
        }
        if let Element::DataAreaEdge(side) = element {
            return self.nudge_data_area_edge(side, dx, dy);
        }
        if let Element::DataArea = element {
            return self.nudge_data_area(dx, dy);
        }
        if let Element::ColorBarEdge(side) = element {
            return self.nudge_colorbar_edge(side, dx, dy);
        }
        let anchor = match element_anchor(self, &element) {
            Some(a) => a,
            None => return NudgeResult::Rejected(NudgeReject::OutOfBounds),
        };
        // Compute the current screen position by converting the stored local
        // offset to screen frame (identity except for rotated axis titles).
        let (ox, oy) = current_offset(self, &element);
        let (screen_ox, screen_oy) = match &element {
            Element::AxisTitle(side) => axis_title_offset_to_screen(side.clone(), (ox, oy)),
            Element::ColorBarTitle => {
                let Some(side) = self.colorbar.as_ref().map(|bar| bar.side.clone()) else {
                    return NudgeResult::Rejected(NudgeReject::OutOfBounds);
                };
                axis_title_offset_to_screen(side, (ox, oy))
            }
            _ => (ox, oy),
        };
        let new_x = anchor.0 + screen_ox + dx;
        let new_y = anchor.1 + screen_oy + dy;

        let ca = &self.chart_area;
        let x_min = ca.x as f32;
        let y_min = ca.y as f32;
        let x_max = (ca.x + ca.width) as f32;
        let y_max = (ca.y + ca.height) as f32;

        if new_x < x_min || new_x > x_max || new_y < y_min || new_y > y_max {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }

        match element {
            Element::ChartTitle => {
                self.chart_title.offset_x += dx;
                self.chart_title.offset_y += dy;
            }
            Element::AxisTitle(side) => {
                // Left/Right axis titles are drawn with a canvas rotate, so we
                // map screen dx/dy back into the rotated local frame so the
                // title moves in the direction the user expects on screen.
                let (local_dx, local_dy) = screen_offset_to_axis_title(side.clone(), (dx, dy));
                let a = axis_mut(self, &side);
                a.title_option.offset_x += local_dx;
                a.title_option.offset_y += local_dy;
            }
            Element::AxisLabel(side) => {
                // label_offset_{x,y} is a margin-noncontributing visual offset
                // in screen coordinates; add directly.
                let a = axis_mut(self, &side);
                a.label_style.label_offset_x += dx;
                a.label_style.label_offset_y += dy;
            }
            Element::Legend => {
                self.legend.offset_x += dx;
                self.legend.offset_y += dy;
            }
            Element::ColorBar => {
                // `element_anchor` already returned `None` for a missing bar, so
                // reaching here means there is one.
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.offset_x += dx;
                    bar.offset_y += dy;
                }
            }
            Element::ColorBarLabel => {
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.axis.label_style.label_offset_x += dx;
                    bar.axis.label_style.label_offset_y += dy;
                }
            }
            Element::ColorBarTitle => {
                let Some(side) = self.colorbar.as_ref().map(|bar| bar.side.clone()) else {
                    return NudgeResult::Rejected(NudgeReject::OutOfBounds);
                };
                let (local_dx, local_dy) = screen_offset_to_axis_title(side, (dx, dy));
                if let Some(bar) = self.colorbar.as_mut() {
                    bar.axis.title_option.offset_x += local_dx;
                    bar.axis.title_option.offset_y += local_dy;
                }
            }
            // Dispatched at the top of `nudge`.
            Element::Axis(_)
            | Element::ColorBarAxis
            | Element::DataAreaEdge(_)
            | Element::DataArea
            | Element::ColorBarEdge(_) => {}
        }
        NudgeResult::Moved
    }

    /// Whole-data-area rule (drag-to-move): translate without resizing by
    /// shifting opposite margins in tandem. Each axis component applies
    /// independently and clamps at the chart edge (a blocked horizontal move
    /// doesn't kill a valid vertical one), so the area slides along the
    /// boundary like any draggable box.
    fn nudge_data_area(&mut self, dx: f32, dy: f32) -> NudgeResult {
        let mut moved = false;

        if dx != 0.0 {
            let new_left = self.left_y.out_margin + dx;
            let new_right = self.right_y.out_margin - dx;
            if new_left >= 0.0 && new_right >= 0.0 {
                self.left_y.out_margin = new_left;
                self.right_y.out_margin = new_right;
                moved = true;
            }
        }
        if dy != 0.0 {
            let new_top = self.top_x.out_margin + dy;
            let new_bottom = self.bottom_x.out_margin - dy;
            if new_top >= 0.0 && new_bottom >= 0.0 {
                self.top_x.out_margin = new_top;
                self.bottom_x.out_margin = new_bottom;
                moved = true;
            }
        }

        if moved {
            NudgeResult::Moved
        } else {
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        }
    }

    /// Detached-axis drag rule: the axis line (with its ticks and labels)
    /// moves only **perpendicular to itself** — y-axes horizontally, x-axes
    /// vertically — via `line_offset`. The parallel component is discarded.
    /// The data area / grid / data transform are untouched, so tick positions
    /// along the axis stay aligned with the data; the constraint is only that
    /// the axis line stays inside the chart area (it may cross into the data
    /// area, e.g. a y-axis at x = 0).
    fn nudge_axis(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        let d = match side {
            Side::Left | Side::Right => dx,
            Side::Top | Side::Bottom => dy,
        };
        if d == 0.0 {
            // Pure parallel drag — nothing to move, but not an error.
            return NudgeResult::Moved;
        }
        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let new_offset = axis_ref(self, &side).line_offset + d;

        // The shifted axis line must stay inside the chart area.
        let line_pos = match side {
            Side::Left => da.x as f32 + new_offset,
            Side::Right => (da.x + da.width) as f32 + new_offset,
            Side::Top => da.y as f32 + new_offset,
            Side::Bottom => (da.y + da.height) as f32 + new_offset,
        };
        let ca = &self.chart_area;
        let (lo, hi) = match side {
            Side::Left | Side::Right => (ca.x as f32, (ca.x + ca.width) as f32),
            Side::Top | Side::Bottom => (ca.y as f32, (ca.y + ca.height) as f32),
        };
        if line_pos < lo || line_pos > hi {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        axis_mut(self, &side).line_offset = new_offset;
        NudgeResult::Moved
    }

    /// Colourbar-axis drag rule: detach the z-axis chrome perpendicular to the
    /// strip without moving the strip, its title, the data area, or the z
    /// transform. Labels follow the line/ticks because they share
    /// `colorbar.axis.line_offset` in the renderer.
    fn nudge_colorbar_axis(&mut self, dx: f32, dy: f32) -> NudgeResult {
        let Some(bar) = self.colorbar.as_ref() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let side = bar.side.clone();
        let d = match side {
            Side::Left | Side::Right => dx,
            Side::Top | Side::Bottom => dy,
        };
        if d == 0.0 {
            return NudgeResult::Moved;
        }
        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let rect = colorbar_rect(&self.chart_area, &da, self.chart_title.top_margin, bar);
        let new_offset = bar.axis.line_offset + d;
        let line_pos = match side {
            Side::Left => rect.x + new_offset,
            Side::Right => rect.x + rect.width + new_offset,
            Side::Top => rect.y + new_offset,
            Side::Bottom => rect.y + rect.height + new_offset,
        };
        let ca = &self.chart_area;
        let (lo, hi) = match side {
            Side::Left | Side::Right => (ca.x as f32, (ca.x + ca.width) as f32),
            Side::Top | Side::Bottom => (ca.y as f32, (ca.y + ca.height) as f32),
        };
        if line_pos < lo || line_pos > hi {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        self.colorbar.as_mut().expect("colourbar").axis.line_offset = new_offset;
        NudgeResult::Moved
    }

    /// Colourbar-edge rule (resize handles): the dragged edge grows the
    /// dimension it belongs to.
    ///
    /// Which dimension that is comes from the bar's orientation, not the handle:
    /// on a vertical bar the left/right edges are `thickness_px` and the
    /// top/bottom edges are `length_frac`. The sign is "away from the strip's
    /// centre grows it", the same shape as `nudge_data_area_edge`'s per-side sign.
    ///
    /// The anchor decides how a size change moves the edge under the pointer, so
    /// the delta is scaled to keep the two together: with `BarAlign::Center` both
    /// ends move by half, so the size has to change by twice the drag. With
    /// `Start` / `End` one end is pinned and the free end carries the whole
    /// change — dragging the *pinned* end's handle still resizes, from the free
    /// end, which is what an anchored box does.
    ///
    /// `thickness_px` needs no such factor. The band anchor pins the edge named
    /// by `bar.side`, so changing thickness alone already makes the opposite,
    /// inner edge follow its handle. When the pinned outer edge itself is
    /// dragged, the strip is translated by the same pointer delta as the size
    /// change. That keeps the opposite edge fixed and puts the grabbed edge
    /// under the pointer instead of leaving it behind at the band anchor.
    fn nudge_colorbar_edge(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        let Some(bar) = self.colorbar.as_ref() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        // Growth is the drag projected onto the edge's outward normal.
        let growth = match side {
            Side::Left => -dx,
            Side::Right => dx,
            Side::Top => -dy,
            Side::Bottom => dy,
        };
        if growth == 0.0 {
            // Motion parallel to this edge — nothing to resize, not an error.
            return NudgeResult::Moved;
        }
        let bar_vertical = matches!(bar.side, Side::Left | Side::Right);
        let edge_vertical = matches!(side, Side::Left | Side::Right);
        // A vertical bar's left/right edges are its thickness; so are a
        // horizontal bar's top/bottom edges.
        let edge_is_thickness = bar_vertical == edge_vertical;

        if edge_is_thickness {
            let old_thickness = bar.thickness_px;
            let old_offset_x = bar.offset_x;
            let old_offset_y = bar.offset_y;
            let new = old_thickness + growth;
            if new < 0.0 {
                return NudgeResult::Rejected(NudgeReject::OutOfBounds);
            }
            let dragged_outer_edge = bar.side == side;
            let new_offset_x = if dragged_outer_edge && bar_vertical {
                old_offset_x + dx
            } else {
                old_offset_x
            };
            let new_offset_y = if dragged_outer_edge && !bar_vertical {
                old_offset_y + dy
            } else {
                old_offset_y
            };
            // Growing the strip grows its band, which shrinks the data area —
            // so the same check `nudge_data_area_edge` makes applies, by the
            // same write-then-revert (no Config clone on a pointer-move path).
            // Thickness and the outer-handle offset are one transaction: a
            // rejected resize must not leave the strip translated.
            {
                let bar = self.colorbar.as_mut().expect("colourbar");
                bar.thickness_px = new;
                bar.offset_x = new_offset_x;
                bar.offset_y = new_offset_y;
            }
            if self.data_area().is_err() {
                let bar = self.colorbar.as_mut().expect("colourbar");
                bar.thickness_px = old_thickness;
                bar.offset_x = old_offset_x;
                bar.offset_y = old_offset_y;
                return NudgeResult::Rejected(NudgeReject::OutOfBounds);
            }
            return NudgeResult::Moved;
        }

        let Ok(da) = self.data_area() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let span = if bar_vertical {
            da.height as f32
        } else {
            da.width as f32
        };
        if span <= 0.0 {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let factor = match bar.align {
            crate::config::BarAlign::Center => 2.0,
            crate::config::BarAlign::Start | crate::config::BarAlign::End => 1.0,
        };
        let new = bar.length_frac + factor * growth / span;
        // `Config::validate`'s range: a strip of zero length has no pixel to
        // hold a colour, and one longer than its side runs past the axis it is
        // labelled against.
        if !(new > 0.0 && new <= 1.0) {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        self.colorbar.as_mut().expect("colourbar").length_frac = new;
        NudgeResult::Moved
    }

    /// Data-area-edge rule (resize handles): one boundary moves by adjusting
    /// that side's `out_margin`, perpendicular only. The data area — and with
    /// it both the tick raster and the GPU data transform — derives from the
    /// margins, so data and ticks move as one.
    fn nudge_data_area_edge(&mut self, side: Side, dx: f32, dy: f32) -> NudgeResult {
        // Screen delta → out_margin delta. Moving an edge toward the chart
        // center grows its margin; toward the chart edge shrinks it.
        let dm = match side {
            Side::Left => dx,
            Side::Right => -dx,
            Side::Top => dy,
            Side::Bottom => -dy,
        };
        if dm == 0.0 {
            return NudgeResult::Moved;
        }
        let old = axis_ref(self, &side).out_margin;
        let new = old + dm;
        if new < 0.0 {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        axis_mut(self, &side).out_margin = new;
        // The shifted margin must still leave a valid data area.
        if self.data_area().is_err() {
            axis_mut(self, &side).out_margin = old;
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        NudgeResult::Moved
    }

    pub fn nudge_x(&mut self, element: Element, dx: f32) -> NudgeResult {
        self.nudge(element, dx, 0.0)
    }

    pub fn nudge_y(&mut self, element: Element, dy: f32) -> NudgeResult {
        self.nudge(element, 0.0, dy)
    }
}

// Invariant tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

    // data_area() must be identical before and after nudge().
    #[test]
    fn nudge_preserves_data_area() {
        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let r = cfg.nudge(Element::ChartTitle, 5.0, 3.0);
        assert_eq!(r, NudgeResult::Moved);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_before, da_after);
    }

    // On Rejected, Config must remain unchanged.
    #[test]
    fn nudge_rejected_preserves_state() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::ChartTitle, -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    // On Moved, only the target offset must change.
    #[test]
    fn nudge_moved_changes_only_target_offset() {
        let mut cfg = default_config();
        let before = cfg.clone();
        cfg.nudge(Element::AxisLabel(Side::Left), 1.0, 0.0);
        let mut expected = before.clone();
        expected.left_y.label_style.label_offset_x += 1.0;
        assert_eq!(cfg, expected);
    }

    // A y-axis moves only horizontally, *detached*: the parallel (dy)
    // component is discarded, the motion lands in line_offset, and the data
    // area does NOT move — the axis floats away while tick positions along
    // the axis stay aligned with the data.
    #[test]
    fn axis_nudge_detaches_axis_and_keeps_data_area() {
        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let margin_before = cfg.left_y.out_margin;

        let r = cfg.nudge(Element::Axis(Side::Left), 10.0, 999.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.line_offset, 10.0);
        // Layout untouched — margin and data area identical.
        assert_eq!(cfg.left_y.out_margin, margin_before);
        assert_eq!(cfg.data_area().unwrap(), da_before);
    }

    // An x-axis moves only vertically (the dx component is discarded).
    #[test]
    fn bottom_axis_nudge_uses_dy_only() {
        let mut cfg = default_config();
        let r = cfg.nudge(Element::Axis(Side::Bottom), 999.0, -8.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.line_offset, -8.0);
        assert_eq!(
            cfg.bottom_x.out_margin,
            default_config().bottom_x.out_margin
        );
    }

    // The detached axis may cross INTO the data area (e.g. y-axis at x = 0)
    // but never leave the chart area.
    #[test]
    fn axis_nudge_allows_crossing_into_data_area() {
        let mut cfg = default_config();
        let r = cfg.nudge(Element::Axis(Side::Left), 50.0, 0.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.line_offset, 50.0);
    }

    #[test]
    fn axis_nudge_rejects_leaving_chart_area() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::Axis(Side::Left), -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    // Data-area edges (resize path) still move via margins.
    #[test]
    fn data_area_edge_nudge_moves_margin_and_data_area() {
        let mut cfg = default_config();
        let da_before = cfg.data_area().unwrap();
        let margin_before = cfg.left_y.out_margin;

        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), 10.0, 999.0);
        assert_eq!(r, NudgeResult::Moved);
        assert_eq!(cfg.left_y.out_margin, margin_before + 10.0);
        let da_after = cfg.data_area().unwrap();
        assert_eq!(da_after.x, da_before.x + 10);
        assert_eq!(da_after.width, da_before.width - 10);
    }

    #[test]
    fn data_area_edge_rejects_negative_margin() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), -1e6, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }

    #[test]
    fn data_area_edge_rejects_margin_overflow() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let w = cfg.chart_area.width as f32;
        let r = cfg.nudge(Element::DataAreaEdge(Side::Left), w * 2.0, 0.0);
        assert_eq!(r, NudgeResult::Rejected(NudgeReject::OutOfBounds));
        assert_eq!(cfg, before);
    }
}
