use crate::config::{AxisOptions, ChartTitleOptions, Config};

mod fit;
mod nudge;
mod rect;

pub use fit::FitStrategy;
pub use nudge::{Element, NudgeReject, NudgeResult};
pub use rect::{ChartArea, DataArea, Rect};

// Support types.

#[derive(Debug, Clone, PartialEq)]
pub struct Margins {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    OverflowHorizontal { required: f32, available: u32 },
    OverflowVertical { required: f32, available: u32 },
    EmptyDataArea,
    TargetOutOfChartArea,
    Infeasible,
}

// Margin contribution helpers (used by sub-modules).
//
// Every contribution is counted regardless of visibility. `label_offset_{x,y}`
// is a **visual-only offset** that never moves margins (so axis-label nudges
// shift the label without disturbing layout); the space to hold the label is
// always reserved by `out_margin`. `major_tick_length` contributes and is
// **fixed** across fit/resize so tick marks don't grow with the window.

pub(super) fn tick_contribution(axis: &AxisOptions) -> f32 {
    axis.major_tick_length
}

pub(super) fn chart_title_contribution(title: &ChartTitleOptions) -> f32 {
    title.top_margin
}

fn top_total(cfg: &Config) -> f32 {
    chart_title_contribution(&cfg.chart_title)
        + cfg.top_x.out_margin
        + tick_contribution(&cfg.top_x)
}

fn bottom_total(cfg: &Config) -> f32 {
    cfg.bottom_x.out_margin + tick_contribution(&cfg.bottom_x)
}

fn left_total(cfg: &Config) -> f32 {
    cfg.left_y.out_margin + tick_contribution(&cfg.left_y)
}

fn right_total(cfg: &Config) -> f32 {
    cfg.right_y.out_margin + tick_contribution(&cfg.right_y)
}

// Side → axis ref / mut accessors.
pub(super) fn axis_ref<'a>(cfg: &'a Config, side: &Side) -> &'a AxisOptions {
    match side {
        Side::Top => &cfg.top_x,
        Side::Bottom => &cfg.bottom_x,
        Side::Left => &cfg.left_y,
        Side::Right => &cfg.right_y,
    }
}

pub(super) fn axis_mut<'a>(cfg: &'a mut Config, side: &Side) -> &'a mut AxisOptions {
    match side {
        Side::Top => &mut cfg.top_x,
        Side::Bottom => &mut cfg.bottom_x,
        Side::Left => &mut cfg.left_y,
        Side::Right => &mut cfg.right_y,
    }
}

// Lookup / validation.

impl Config {
    pub fn margins(&self) -> Margins {
        Margins {
            top: top_total(self),
            bottom: bottom_total(self),
            left: left_total(self),
            right: right_total(self),
        }
    }

    pub fn data_area(&self) -> Result<DataArea, LayoutError> {
        let ca = &self.chart_area;
        let m = self.margins();

        let h_need = m.left + m.right;
        if h_need > ca.width as f32 {
            return Err(LayoutError::OverflowHorizontal {
                required: h_need,
                available: ca.width,
            });
        }
        let v_need = m.top + m.bottom;
        if v_need > ca.height as f32 {
            return Err(LayoutError::OverflowVertical {
                required: v_need,
                available: ca.height,
            });
        }

        let x = ca.x as f32 + m.left;
        let y = ca.y as f32 + m.top;
        let w = ca.width as f32 - h_need;
        let h = ca.height as f32 - v_need;

        if w <= 0.0 || h <= 0.0 {
            return Err(LayoutError::EmptyDataArea);
        }

        Ok(DataArea(Rect {
            x: x.floor() as u32,
            y: y.floor() as u32,
            width: w.floor() as u32,
            height: h.floor() as u32,
        }))
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        let _ = self.data_area()?;
        for axis in [&self.top_x, &self.bottom_x, &self.left_y, &self.right_y] {
            if axis.max <= axis.min {
                return Err(LayoutError::Infeasible);
            }
            if axis.out_margin < 0.0
                || axis.major_tick_length < 0.0
                || axis.minor_tick_length < 0.0
                || axis.line_width < 0.0
            {
                return Err(LayoutError::Infeasible);
            }
        }
        Ok(())
    }
}

// Invariant tests (data_area / margins / validate).

#[cfg(test)]
mod tests {
    use crate::default::default_config;

    // data_area() must be fully contained inside chart_area on success.
    #[test]
    fn data_area_contained_in_chart_area() {
        let cfg = default_config();
        let da = cfg.data_area().unwrap();
        let ca = &cfg.chart_area;
        assert!(da.x >= ca.x);
        assert!(da.y >= ca.y);
        assert!(da.x + da.width <= ca.x + ca.width);
        assert!(da.y + da.height <= ca.y + ca.height);
    }

    // All margin-contribution fields must be non-negative.
    #[test]
    fn margins_non_negative() {
        let cfg = default_config();
        let m = cfg.margins();
        assert!(m.top >= 0.0);
        assert!(m.bottom >= 0.0);
        assert!(m.left >= 0.0);
        assert!(m.right >= 0.0);
    }
}
