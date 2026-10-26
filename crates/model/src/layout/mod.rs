use crate::config::{AxisOptions, ChartTitleOptions, Config};

mod fit;
mod geometry;
mod nudge;
mod rect;

pub use fit::FitStrategy;
pub use geometry::{
    LABEL_GAP, LEGEND_BORDER_WIDTH, LEGEND_INSET, LOG_RANGE_FALLBACK_MIN, TitleBand,
    TitlePlacement, auto_layout, axis_anchor, axis_cross, axis_fraction, axis_offset,
    axis_title_offset_to_screen, axis_title_placement, chart_title_placement,
    chart_title_placement_in, colorbar_rect, colorbar_title_placement, fit_chart_title_margin,
    guarded_log_range, label_origin, label_rect, legend_rect, point_on_rect_side,
    screen_offset_to_axis_title, span_fraction, visible_colorbar_rect,
};
pub use nudge::{Element, NudgeReject, NudgeResult};
pub use rect::{ChartArea, DataArea, Rect, RectF};

pub use crate::sink::DrawnPart;

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

/// How much of the data area, in pixels along each axis, must stay inside the
/// chart area when margins push it past the chart edges, so its selection box
/// and resize handles remain reachable.
pub const MIN_VISIBLE_DATA_AREA_PX: u32 = 8;

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
    right_out_margin_effective(cfg)
        + tick_contribution(&cfg.right_y)
        + colorbar_contribution(cfg, &Side::Right)
}

/// The right band every overlay layer shares, expressed as the parent
/// `right_y.out_margin` it is equivalent to.
///
/// Overlay Y axes are drawn on the right side of the one data area, so the
/// right margin is group geometry: the widest band of the parent axis and
/// every overlay axis (`out_margin + major_tick_length`, plus the colourbar
/// band, which is the same for all of them). Expressing the group as an
/// effective parent `out_margin` keeps `right_total` the same expression as
/// the three other sides, so a document without overlays computes exactly the
/// margins it did before the field existed, and lets the fit and nudge paths
/// write the group band back through the parent axis.
pub(super) fn right_out_margin_effective(cfg: &Config) -> f32 {
    let fixed = tick_contribution(&cfg.right_y) + colorbar_contribution(cfg, &Side::Right);
    cfg.overlays
        .iter()
        .fold(cfg.right_y.out_margin, |widest, overlay| {
            let band = overlay.y_axis.out_margin
                + tick_contribution(&overlay.y_axis)
                + colorbar_contribution(cfg, &Side::Right)
                - fixed;
            if band > widest { band } else { widest }
        })
}

/// After a write of the parent's right margin: make every overlay band fit
/// inside the parent's, so the group right margin is exactly the parent's.
/// A data-area edge drag or a fit sets group geometry, and the group is what
/// the host sees move. Nothing changes for a document without overlays.
pub(super) fn sync_overlay_right_margins(cfg: &mut Config) {
    if cfg.overlays.is_empty() {
        return;
    }
    let own = cfg.right_y.out_margin + tick_contribution(&cfg.right_y);
    for overlay in &mut cfg.overlays {
        let out = own - tick_contribution(&overlay.y_axis);
        overlay.y_axis.out_margin = if out > 0.0 { out } else { 0.0 };
    }
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

/// Crate-visible side accessor for the expansion module.
pub(crate) fn axis_options_mut<'a>(cfg: &'a mut Config, side: &Side) -> &'a mut AxisOptions {
    axis_mut(cfg, side)
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

        // A negative margin is just a coordinate: it moves that edge of the data
        // area out past the chart edge, which is allowed. A NaN or infinite one
        // is not a coordinate at all.
        if [m.top, m.bottom, m.left, m.right]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err(LayoutError::Infeasible);
        }

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

        // `Rect` is unsigned, so the data area cannot start above or left of the
        // canvas origin, nor end past `u32::MAX`: the cast would saturate and
        // silently move the opposite edge instead.
        let max = f64::from(u32::MAX);
        if x < 0.0
            || y < 0.0
            || f64::from(x) + f64::from(w) > max
            || f64::from(y) + f64::from(h) > max
        {
            return Err(LayoutError::TargetOutOfChartArea);
        }
        let rect = Rect {
            x: x.floor() as u32,
            y: y.floor() as u32,
            width: w.floor() as u32,
            height: h.floor() as u32,
        };
        // The data area may run past the chart edges, but it must keep a
        // grabbable strip on screen: once it is entirely outside, it can no
        // longer be selected or dragged back. A data area smaller than the
        // strip only has to stay wholly inside.
        let keeps_visible_edge = |start: u32, len: u32, ca_start: u32, ca_len: u32| {
            let lo = i64::from(start.max(ca_start));
            let hi =
                (i64::from(start) + i64::from(len)).min(i64::from(ca_start) + i64::from(ca_len));
            hi - lo >= i64::from(MIN_VISIBLE_DATA_AREA_PX.min(len).min(ca_len))
        };
        if !keeps_visible_edge(rect.x, rect.width, ca.x, ca.width)
            || !keeps_visible_edge(rect.y, rect.height, ca.y, ca.height)
        {
            return Err(LayoutError::TargetOutOfChartArea);
        }
        Ok(DataArea(rect))
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
        // Overlay axes contribute to the shared right band, so they are held
        // to the chart axes' dimensional rules. A non-finite band would hide
        // inside the group maximum, which is why finiteness is explicit here.
        for overlay in &self.overlays {
            axis_dims_feasible(&overlay.y_axis)?;
            if !overlay.y_axis.out_margin.is_finite()
                || !overlay.y_axis.major_tick_length.is_finite()
            {
                return Err(LayoutError::Infeasible);
            }
        }
        if self.validate_overlays().is_err() {
            return Err(LayoutError::Infeasible);
        }
        // Every subplot is a chart of its own and is held to the same rules,
        // with its links applied.
        if self.validate_subplots().is_err() {
            return Err(LayoutError::Infeasible);
        }
        for index in 0..self.subplots.len() {
            self.subplot_config(index)
                .ok_or(LayoutError::Infeasible)?
                .validate()?;
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
    let (min, max) = axis.data_range();
    if max <= min {
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
    use crate::layout::{ChartArea, LayoutError, Rect};
    use crate::select::tests::LaysOut;

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

    // Issue #6: a negative title margin is a coordinate, not an error. It may
    // push the data area past the chart edge, as long as the unsigned rect can
    // hold it and a grabbable strip stays on screen.
    fn small_chart(y: u32, top_margin: f32) -> Config {
        let mut cfg = config_with_colorbar(Side::Right);
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y,
            width: 320,
            height: 240,
        });
        cfg.chart_title.top_margin = top_margin;
        cfg
    }

    fn bottom_edge(cfg: &Config) -> f32 {
        cfg.chart_area.y as f32 + cfg.chart_area.height as f32 - cfg.margins().bottom
    }

    #[test]
    fn a_negative_title_margin_is_a_coordinate() {
        for (y, margin) in [(0, -5.0), (100, -60.0), (100, -90.0)] {
            let cfg = small_chart(y, margin);
            assert!(cfg.validate().is_ok(), "y {y}, title margin {margin}");
            let da = cfg.data_area().expect("data area").0;
            // The top edge follows the margin; the bottom edge does not move.
            assert_eq!(da.y as f32, (y as f32 + cfg.margins().top).floor());
            assert_eq!((da.y + da.height) as f32, bottom_edge(&cfg).floor());
        }
        // Running past the chart's top edge is allowed when the canvas has room.
        assert!(small_chart(100, -90.0).data_area().unwrap().0.y < 100);
    }

    // `Rect` is unsigned: a data area starting above the canvas origin used to
    // saturate to y = 0 and keep its height, moving the bottom edge down.
    #[test]
    fn a_data_area_above_the_canvas_origin_is_refused_not_shifted() {
        let cfg = small_chart(0, -60.0);
        assert!(cfg.margins().top < 0.0);
        assert_eq!(cfg.data_area(), Err(LayoutError::TargetOutOfChartArea));
        assert!(cfg.validate().is_err());
        // The issue's reproduction.
        assert_eq!(
            small_chart(0, -1e38).data_area(),
            Err(LayoutError::TargetOutOfChartArea)
        );
    }

    #[test]
    fn a_non_finite_margin_is_rejected() {
        for margin in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let cfg = small_chart(100, margin);
            assert_eq!(cfg.data_area(), Err(LayoutError::Infeasible), "{margin}");
            assert!(cfg.validate().is_err());
        }
    }

    // Past `u32::MAX` the unsigned rect cannot hold the far edge.
    #[test]
    fn a_data_area_past_u32_max_is_refused() {
        let mut cfg = small_chart(0, 32.0);
        cfg.bottom_x.out_margin = -1e38;
        assert_eq!(cfg.data_area(), Err(LayoutError::TargetOutOfChartArea));
    }

    // The data area must keep MIN_VISIBLE_DATA_AREA_PX inside the chart area,
    // so it can still be selected and dragged back.
    #[test]
    fn the_data_area_keeps_a_visible_strip() {
        use super::MIN_VISIBLE_DATA_AREA_PX as MIN;
        let base = small_chart(100, 32.0);
        let ca_bottom = (base.chart_area.y + base.chart_area.height) as f32;
        let top = base.margins().top;
        // Choose the title margin so the data area starts `inside` px above the
        // chart's bottom edge.
        let with_strip = |inside: f32| {
            let mut cfg = base.clone();
            let shift = ca_bottom - inside - (cfg.chart_area.y as f32 + top);
            cfg.chart_title.top_margin += shift;
            cfg.bottom_x.out_margin -= shift;
            cfg
        };
        assert!(with_strip(MIN as f32).data_area().is_ok());
        assert_eq!(
            with_strip(MIN as f32 - 1.0).data_area(),
            Err(LayoutError::TargetOutOfChartArea)
        );
        assert_eq!(
            with_strip(-20.0).data_area(),
            Err(LayoutError::TargetOutOfChartArea)
        );
        // A data area smaller than the strip only has to stay inside.
        let mut tiny = base.clone();
        let m = tiny.margins();
        tiny.bottom_x.out_margin += tiny.chart_area.height as f32 - m.top - m.bottom - 3.0;
        let da = tiny.data_area().expect("tiny data area").0;
        assert!(da.height < MIN);
    }

    // The right margin is group geometry: the widest band of the parent axis
    // and every overlay axis. The other three sides and a document without
    // overlays are untouched.
    #[test]
    fn overlay_axes_widen_only_the_shared_right_margin() {
        use crate::layer::OverlayLayer;
        let base = default_config();
        let own = base.margins();
        let mut cfg = base.clone();
        cfg.overlays.push(OverlayLayer::default());
        let band = |cfg: &Config, i: usize| {
            cfg.overlays[i].y_axis.out_margin + cfg.overlays[i].y_axis.major_tick_length
        };
        assert!(
            band(&cfg, 0) > own.right,
            "the default overlay band is wider"
        );
        let m = cfg.margins();
        assert_eq!(m.right, band(&cfg, 0));
        assert_eq!((m.top, m.bottom, m.left), (own.top, own.bottom, own.left));
        assert!(cfg.data_area().unwrap().width < base.data_area().unwrap().width);

        // A narrower overlay band hides inside the parent's.
        cfg.overlays[0].y_axis.out_margin = 0.0;
        cfg.overlays[0].y_axis.major_tick_length = 0.0;
        assert_eq!(cfg.margins(), own);

        // The widest of several overlays wins.
        cfg.overlays.push(OverlayLayer::default());
        cfg.overlays[1].y_axis.out_margin = 70.0;
        assert_eq!(cfg.margins().right, band(&cfg, 1));
    }

    // Writing the data area through the parent (fit, edge drag) writes the
    // group band: every overlay band is pulled inside the parent's.
    #[test]
    fn data_area_writes_set_the_group_right_band() {
        use crate::layer::OverlayLayer;
        use crate::layout::{Element, FitStrategy};
        let mut cfg = default_config();
        cfg.overlays.push(OverlayLayer::default());
        let before = cfg.data_area().unwrap().0;
        assert_eq!(
            cfg.nudge(Element::DataAreaEdge(Side::Right), &LaysOut, 20.0, 0.0),
            crate::layout::NudgeResult::Moved
        );
        let after = cfg.data_area().unwrap().0;
        assert_eq!(after.width, before.width + 20);
        assert!(
            cfg.overlays[0].y_axis.out_margin + cfg.overlays[0].y_axis.major_tick_length
                <= cfg.right_y.out_margin + cfg.right_y.major_tick_length + 1e-3
        );

        let m = cfg.margins();
        cfg.set_margins(
            super::Margins { right: 40.0, ..m },
            FitStrategy::PreserveTitles,
        )
        .unwrap();
        assert!((cfg.margins().right - 40.0).abs() < 1e-3);
        let target = cfg.data_area().unwrap();
        cfg.fit_to_data_area(target.clone(), FitStrategy::Absorb(Side::Left))
            .unwrap();
        assert_eq!(cfg.data_area().unwrap(), target);
    }

    #[test]
    fn validate_rejects_a_degenerate_overlay_axis() {
        use crate::layer::OverlayLayer;
        let mut cfg = default_config();
        cfg.overlays.push(OverlayLayer::default());
        assert!(cfg.validate().is_ok());
        let mut reversed = cfg.clone();
        reversed.overlays[0].y_axis.max = reversed.overlays[0].y_axis.min;
        assert_eq!(reversed.validate(), Err(LayoutError::Infeasible));
        let mut nan = cfg.clone();
        nan.overlays[0].y_axis.out_margin = f32::NAN;
        assert_eq!(nan.validate(), Err(LayoutError::Infeasible));
        let mut shared = cfg.clone();
        shared.overlays[0].series = vec!["a".into()];
        shared.overlays.push(OverlayLayer {
            series: vec!["a".into()],
            ..Default::default()
        });
        assert_eq!(shared.validate(), Err(LayoutError::Infeasible));
    }

    // The selection box of every chrome element is derived from `data_area()`,
    // so an interaction that left it erroring would drop the selection. Drive
    // random edge drags, data-area moves, margin edits and resizes from random
    // valid layouts: the layout must stay valid after each step, whether the
    // step was applied or refused, and the display copy must never trip the
    // origin or visible-strip guard.
    #[test]
    fn interactions_from_a_valid_layout_never_lose_the_data_area() {
        use crate::layout::{Element, FitStrategy};

        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        let sides = [Side::Top, Side::Bottom, Side::Left, Side::Right];
        let mut checked = 0usize;

        for _ in 0..1500 {
            let mut cfg = default_config();
            cfg.chart_area = ChartArea(Rect {
                x: (next() * 5000.0) as u32,
                y: (next() * 5000.0) as u32,
                width: 40 + (next() * 4000.0) as u32,
                height: 40 + (next() * 4000.0) as u32,
            });
            cfg.chart_title.top_margin = next() * 120.0;
            for side in &sides {
                super::axis_mut(&mut cfg, side).out_margin = next() * 90.0;
            }
            if next() < 0.5 {
                let mut bar = default_colorbar_options();
                bar.side = sides[(next() * 4.0) as usize % 4].clone();
                cfg.colorbar = Some(bar);
            }
            if next() < 0.5 {
                let mut overlay = crate::layer::OverlayLayer::default();
                overlay.y_axis.out_margin = next() * 150.0;
                cfg.overlays.push(overlay);
            }
            if cfg.validate().is_err() {
                continue;
            }

            for _ in 0..150 {
                let side = sides[(next() * 4.0) as usize % 4].clone();
                let (dx, dy) = ((next() - 0.5) * 120.0, (next() - 0.5) * 120.0);
                let before = cfg.clone();
                match (next() * 6.0) as u32 {
                    0 | 1 => {
                        let _ = cfg.nudge(Element::DataAreaEdge(side), &LaysOut, dx, dy);
                    }
                    2 => {
                        let _ = cfg.nudge(Element::DataArea, &LaysOut, dx, dy);
                    }
                    3 => {
                        let _ = cfg.set_side_margin(side, next() * 300.0);
                    }
                    4 => {
                        let ca = cfg.chart_area.0;
                        let _ = cfg.resize_chart_area_scaled(ChartArea(Rect {
                            x: ca.x,
                            y: ca.y,
                            width: 20 + (next() * 4000.0) as u32,
                            height: 20 + (next() * 4000.0) as u32,
                        }));
                    }
                    _ => {
                        let m = cfg.margins();
                        let _ = cfg.set_margins(
                            super::Margins {
                                top: m.top + dy,
                                ..m
                            },
                            FitStrategy::ProportionalScale,
                        );
                    }
                }
                assert!(
                    cfg.validate().is_ok(),
                    "an interaction left an invalid layout: {:?} -> {:?}",
                    before.margins(),
                    cfg.margins()
                );
                // The display copy (high-DPI scale) must stay valid too.
                let scale = 0.5 + next() * 3.5;
                let shown = cfg.scaled(scale);
                match shown.data_area() {
                    Ok(_)
                    | Err(LayoutError::OverflowHorizontal { .. })
                    | Err(LayoutError::OverflowVertical { .. })
                    | Err(LayoutError::EmptyDataArea) => {}
                    Err(e) => panic!("display scale {scale} hit a new rejection: {e:?}"),
                }
                checked += 1;
            }
        }
        assert!(checked > 100_000, "only {checked} steps exercised");
    }
}
