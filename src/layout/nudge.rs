use super::{axis_mut, axis_ref, Config, Side};

// Nudge types.

#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    ChartTitle,
    AxisTitle(Side),
    AxisLabel(Side),
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
    }
}

// Nudge methods.

impl Config {
    pub fn nudge(&mut self, element: Element, dx: f32, dy: f32) -> NudgeResult {
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
}
