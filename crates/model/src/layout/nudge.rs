use super::{Config, Side, axis_mut, axis_ref};

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

/// Approximate distance from axis line to label center (matches LABEL_GAP in
/// axis_render.rs) plus a rough label extent.
const LABEL_ANCHOR_EST: f32 = 14.0;

/// Actual screen-space render center of `element` when its offset is zero.
/// The final drawn position is `anchor + offset`.
fn element_anchor(cfg: &Config, element: &Element) -> Option<(f32, f32)> {
    let ca = &cfg.chart_area;
    let da = cfg.data_area().ok()?;

    let anchor = match element {
        // Center of the chart title band (ca.y .. ca.y + chart_title.top_margin).
        Element::ChartTitle => (
            ca.x as f32 + ca.width as f32 * 0.5,
            ca.y as f32 + cfg.chart_title.top_margin * 0.5,
        ),
        // Axis title: center of that side's out_margin band.
        Element::AxisTitle(Side::Top) => (
            da.x as f32 + da.width as f32 * 0.5,
            ca.y as f32 + cfg.chart_title.top_margin + cfg.top_x.out_margin * 0.5,
        ),
        Element::AxisTitle(Side::Bottom) => (
            da.x as f32 + da.width as f32 * 0.5,
            (ca.y + ca.height) as f32 - cfg.bottom_x.out_margin * 0.5,
        ),
        Element::AxisTitle(Side::Left) => (
            ca.x as f32 + cfg.left_y.out_margin * 0.5,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        Element::AxisTitle(Side::Right) => (
            (ca.x + ca.width) as f32 - cfg.right_y.out_margin * 0.5,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        // Axis label: just outside the tick end (with approximate label extent).
        Element::AxisLabel(Side::Top) => (
            da.x as f32 + da.width as f32 * 0.5,
            da.y as f32 - cfg.top_x.major_tick_length - LABEL_ANCHOR_EST,
        ),
        Element::AxisLabel(Side::Bottom) => (
            da.x as f32 + da.width as f32 * 0.5,
            (da.y + da.height) as f32 + cfg.bottom_x.major_tick_length + LABEL_ANCHOR_EST,
        ),
        Element::AxisLabel(Side::Left) => (
            da.x as f32 - cfg.left_y.major_tick_length - LABEL_ANCHOR_EST,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        Element::AxisLabel(Side::Right) => (
            (da.x + da.width) as f32 + cfg.right_y.major_tick_length + LABEL_ANCHOR_EST,
            da.y as f32 + da.height as f32 * 0.5,
        ),
        // Legend: corner anchor point (inset corner of the data area). The
        // box extent isn't known here (it needs text measurement), but the
        // chart_area containment check only needs a representative point.
        Element::Legend => {
            let inset = 6.0;
            match cfg.legend.corner {
                crate::config::LegendCorner::TopLeft => (da.x as f32 + inset, da.y as f32 + inset),
                crate::config::LegendCorner::TopRight => {
                    ((da.x + da.width) as f32 - inset, da.y as f32 + inset)
                }
                crate::config::LegendCorner::BottomLeft => {
                    (da.x as f32 + inset, (da.y + da.height) as f32 - inset)
                }
                crate::config::LegendCorner::BottomRight => (
                    (da.x + da.width) as f32 - inset,
                    (da.y + da.height) as f32 - inset,
                ),
            }
        }
        // Axis / data-area moves have their own rules — dispatched before
        // this function is reached.
        Element::Axis(_) | Element::DataAreaEdge(_) | Element::DataArea => return None,
    };

    Some(anchor)
}

/// Convert a local-frame offset (in the rotated coordinate system) to a
/// screen-frame offset. Only Left/Right axis titles are rotated; everything
/// else is identity.
fn local_to_screen_offset(element: &Element, ox: f32, oy: f32) -> (f32, f32) {
    match element {
        // Left (-90° CCW): local (ox, oy) → screen (oy, -ox).
        Element::AxisTitle(Side::Left) => (oy, -ox),
        // Right (+90° CW): local (ox, oy) → screen (-oy, ox).
        Element::AxisTitle(Side::Right) => (-oy, ox),
        _ => (ox, oy),
    }
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
        // Dispatched before the offset path.
        Element::Axis(_) | Element::DataAreaEdge(_) | Element::DataArea => (0.0, 0.0),
    }
}

// Nudge methods.

impl Config {
    pub fn nudge(&mut self, element: Element, dx: f32, dy: f32) -> NudgeResult {
        // Axis / data-area-edge moves have their own rules (perpendicular
        // only); everything else moves via stored offsets below.
        if let Element::Axis(side) = element {
            return self.nudge_axis(side, dx, dy);
        }
        if let Element::DataAreaEdge(side) = element {
            return self.nudge_data_area_edge(side, dx, dy);
        }
        if let Element::DataArea = element {
            return self.nudge_data_area(dx, dy);
        }
        let anchor = match element_anchor(self, &element) {
            Some(a) => a,
            None => return NudgeResult::Rejected(NudgeReject::OutOfBounds),
        };
        // Compute the current screen position by converting the stored local
        // offset to screen frame (identity except for rotated axis titles).
        let (ox, oy) = current_offset(self, &element);
        let (screen_ox, screen_oy) = local_to_screen_offset(&element, ox, oy);
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
                let (local_dx, local_dy) = match side {
                    Side::Top | Side::Bottom => (dx, dy),
                    Side::Left => (-dy, dx),  // inverse of -90° rotation
                    Side::Right => (dy, -dx), // inverse of +90° rotation
                };
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
            // Dispatched at the top of `nudge`.
            Element::Axis(_) | Element::DataAreaEdge(_) | Element::DataArea => {}
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
