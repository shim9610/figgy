use crate::config::{AxisOptions, ChartTitleOptions, Config};

mod fit;
mod geometry;
mod nudge;
mod rect;

pub use fit::FitStrategy;
pub use geometry::{
    LABEL_GAP, LEGEND_INSET, TitleBand, TitlePlacement, axis_anchor, axis_offset,
    axis_title_offset_to_screen, axis_title_placement, axis_visibility_extent,
    axis_visibility_rect, chart_title_placement, colorbar_rect, colorbar_title_placement,
    label_origin, label_rect, legend_rect, point_on_rect_side, rect_axis_visibility_rect,
    screen_offset_to_axis_title,
};
pub use nudge::{Element, NudgeReject, NudgeResult};
pub use rect::{ChartArea, DataArea, Rect, RectF};

// Support types.

#[derive(Debug, Clone, PartialEq)]
pub struct Margins {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// A chart edge. Serializable because `ColorBarOptions::side` is part of
/// `Config`; the layout-internal uses (`FitStrategy::Absorb`, nudge) predate
/// that and are unaffected.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

/// The colourbar band on one side, split into `(flex, fixed)`; `None` on the
/// three sides the bar is not on, and when it is hidden.
///
/// `flex` is the strip's axis `out_margin` — label space, scaled by fit/resize
/// exactly as a chart axis' is. `fixed` is the gap, the strip, and the strip's
/// tick length: a colourbar that got thinner with the window would misreport
/// the extent of its own colours.
///
/// The hidden case is the one place this departs from "contributions count
/// regardless of visibility": an axis' `visible` flags hide a *part* (line,
/// ticks, labels) whose space the rest still needs, while
/// `ColorBarOptions::visible = false` means nothing is drawn there at all, and a
/// reserved band with nothing in it is just a hole in the chart.
///
/// This is the **only** place that decides which side carries a band. Both
/// `colorbar_contribution` (what `margins()` charges) and `fit`'s per-side
/// components read it, so the two cannot disagree about a side and leave the
/// data area in two places at once.
pub(super) fn colorbar_parts(cfg: &Config, side: &Side) -> Option<(f32, f32)> {
    match cfg.colorbar.as_ref() {
        Some(bar) if bar.visible && bar.side == *side => Some((
            bar.axis.out_margin,
            bar.gap_px + bar.thickness_px + tick_contribution(&bar.axis),
        )),
        _ => None,
    }
}

/// The colourbar band's total contribution to one side's margin.
pub(super) fn colorbar_contribution(cfg: &Config, side: &Side) -> f32 {
    colorbar_parts(cfg, side).map_or(0.0, |(flex, fixed)| flex + fixed)
}

fn top_total(cfg: &Config) -> f32 {
    chart_title_contribution(&cfg.chart_title)
        + cfg.top_x.out_margin
        + tick_contribution(&cfg.top_x)
        + colorbar_contribution(cfg, &Side::Top)
}

fn bottom_total(cfg: &Config) -> f32 {
    cfg.bottom_x.out_margin
        + tick_contribution(&cfg.bottom_x)
        + colorbar_contribution(cfg, &Side::Bottom)
}

fn left_total(cfg: &Config) -> f32 {
    cfg.left_y.out_margin + tick_contribution(&cfg.left_y) + colorbar_contribution(cfg, &Side::Left)
}

fn right_total(cfg: &Config) -> f32 {
    cfg.right_y.out_margin
        + tick_contribution(&cfg.right_y)
        + colorbar_contribution(cfg, &Side::Right)
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

    /// How far this side's axis band is pushed in from the chart-area edge by
    /// the colourbar band. Zero on the three sides the bar is not on, and when
    /// it is hidden.
    ///
    /// Public because everything that positions chrome against the chart edge —
    /// the axis title's placement, its hit box, its nudge anchor — needs the
    /// same number, and computing it separately in each is how they drift apart.
    pub fn colorbar_band(&self, side: &Side) -> f32 {
        colorbar_contribution(self, side)
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        let _ = self.data_area()?;
        for axis in [&self.top_x, &self.bottom_x, &self.left_y, &self.right_y] {
            axis_dims_feasible(axis)?;
        }
        if let Some(bar) = self.colorbar.as_ref() {
            // The z axis is held to the same dimensional rules as the four
            // chart axes — it contributes to a margin the same way.
            axis_dims_feasible(&bar.axis)?;
            if bar.thickness_px < 0.0 || bar.gap_px < 0.0 || bar.border_width < 0.0 {
                return Err(LayoutError::Infeasible);
            }
            // A strip of zero length has no pixel to hold a colour, and one
            // longer than its side runs past the axis it is labelled against.
            if !(bar.length_frac > 0.0 && bar.length_frac <= 1.0) {
                return Err(LayoutError::Infeasible);
            }
        }
        Ok(())
    }
}

/// The dimensional rules every `AxisOptions` must satisfy, chart axis or
/// colourbar. Range direction is checked here; whether a *logarithmic* range is
/// positive is a renderer-side rule (`validate_renderer_config`), which is
/// where the reason string lives.
fn axis_dims_feasible(axis: &AxisOptions) -> Result<(), LayoutError> {
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
    Ok(())
}

// Invariant tests (data_area / margins / validate).

#[cfg(test)]
mod tests {
    use super::{Side, colorbar_contribution};
    use crate::config::Config;
    use crate::default::{default_colorbar_options, default_config};
    use crate::layout::LayoutError;

    fn config_with_colorbar(side: Side) -> Config {
        let mut cfg = default_config();
        let mut bar = default_colorbar_options();
        bar.side = side;
        cfg.colorbar = Some(bar);
        cfg
    }

    fn band(cfg: &Config) -> f32 {
        let bar = cfg.colorbar.as_ref().expect("colourbar");
        bar.gap_px + bar.thickness_px + bar.axis.out_margin + bar.axis.major_tick_length
    }

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

    // The band lands on the bar's side and nowhere else: the data area loses
    // exactly that much width, and the other three margins do not move.
    #[test]
    fn the_colorbar_band_is_charged_to_its_own_side_only() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let base = default_config();
            let cfg = config_with_colorbar(side.clone());
            let (before, after) = (base.margins(), cfg.margins());
            let expected = band(&cfg);

            let (grew, unchanged) = match side {
                Side::Top => (
                    after.top - before.top,
                    [
                        after.bottom - before.bottom,
                        after.left - before.left,
                        after.right - before.right,
                    ],
                ),
                Side::Bottom => (
                    after.bottom - before.bottom,
                    [
                        after.top - before.top,
                        after.left - before.left,
                        after.right - before.right,
                    ],
                ),
                Side::Left => (
                    after.left - before.left,
                    [
                        after.top - before.top,
                        after.bottom - before.bottom,
                        after.right - before.right,
                    ],
                ),
                Side::Right => (
                    after.right - before.right,
                    [
                        after.top - before.top,
                        after.bottom - before.bottom,
                        after.left - before.left,
                    ],
                ),
            };
            assert!(
                (grew - expected).abs() < 1e-3,
                "{side:?}: margin grew by {grew}, band is {expected}"
            );
            for delta in unchanged {
                assert!(
                    delta.abs() < 1e-6,
                    "{side:?}: another side moved by {delta}"
                );
            }

            let da = cfg.data_area().expect("data area");
            let base_da = base.data_area().expect("data area");
            match side {
                Side::Left | Side::Right => {
                    assert!(da.width < base_da.width);
                    assert_eq!(da.height, base_da.height);
                }
                Side::Top | Side::Bottom => {
                    assert!(da.height < base_da.height);
                    assert_eq!(da.width, base_da.width);
                }
            }
        }
    }

    // `visible: false` gives the band back. Unlike an axis' visibility flags —
    // which hide a part whose space the rest still needs — there is nothing
    // left to reserve space for.
    #[test]
    fn a_hidden_colorbar_reserves_nothing() {
        let mut cfg = config_with_colorbar(Side::Right);
        cfg.colorbar.as_mut().expect("colourbar").visible = false;
        assert_eq!(cfg.margins(), default_config().margins());
        assert_eq!(colorbar_contribution(&cfg, &Side::Right), 0.0);
    }

    // Nothing about the colourbar may make a chart with no z dimension differ
    // from one written before the field existed.
    #[test]
    fn a_config_without_a_colorbar_is_unaffected() {
        let cfg = default_config();
        assert!(cfg.colorbar.is_none());
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            assert_eq!(colorbar_contribution(&cfg, &side), 0.0);
        }
    }

    // The z axis is held to the same dimensional rules as the four chart axes,
    // and the strip's own dims have to be drawable.
    #[test]
    fn validate_rejects_a_degenerate_colorbar() {
        let good = config_with_colorbar(Side::Right);
        assert!(good.validate().is_ok());

        let mut reversed = good.clone();
        reversed.colorbar.as_mut().unwrap().axis.max = reversed.colorbar.as_ref().unwrap().axis.min;
        assert_eq!(reversed.validate(), Err(LayoutError::Infeasible));

        let mut negative_margin = good.clone();
        negative_margin.colorbar.as_mut().unwrap().axis.out_margin = -1.0;
        assert_eq!(negative_margin.validate(), Err(LayoutError::Infeasible));

        for frac in [0.0, -0.5, 1.5, f32::NAN] {
            let mut bad = good.clone();
            bad.colorbar.as_mut().unwrap().length_frac = frac;
            assert_eq!(
                bad.validate(),
                Err(LayoutError::Infeasible),
                "length_frac {frac} must be rejected"
            );
        }
    }
}
