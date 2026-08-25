use super::{
    ChartArea, Config, DataArea, LayoutError, Margins, Rect, Side, chart_title_contribution,
    colorbar_parts, tick_contribution,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FitStrategy {
    ProportionalScale,
    PreserveTitles,
    Absorb(Side),
}

// Per-side components used internally by fit/resize.
//
// Flex (scaled on fit/resize): `out_margin`, `chart_title` (Top only),
// `colorbar_out_margin` (the colourbar's side only).
// Fixed (never scaled): `major_tick_length`, `colorbar_fixed`.
//
// `label_offset_{x,y}` and `minor_tick_length` are visual-only and don't
// contribute to margins, so they are absent here.
//
// The colourbar band is split rather than registered whole, and both halves
// matter. Registering all of it as fixed would leave fit unable to reclaim the
// bar's label space, so a tight target fails where it could have succeeded;
// omitting it entirely would let fit and resize hand the data area a band that
// `margins()` still charges, and the two would disagree about where the data
// area is.

#[derive(Clone)]
struct SideComponents {
    out_margin: f32,          // flex
    chart_title: Option<f32>, // flex, Top only
    major_tick_length: f32,   // fixed
    /// The colourbar axis' `out_margin` — flex, and `Some` only on the side the
    /// bar occupies (`None` elsewhere, so `write_side` knows not to write).
    colorbar_out_margin: Option<f32>,
    /// `gap_px + thickness_px + colourbar tick length` — fixed, like the axis
    /// tick it sits next to. The strip does not grow with the window.
    colorbar_fixed: f32,
}

fn read_side(cfg: &Config, side: &Side) -> SideComponents {
    // Straight from the layout helper `margins()` uses, so "which side carries a
    // band" is decided once for both.
    let (colorbar_out_margin, colorbar_fixed) = match colorbar_parts(cfg, side) {
        Some((flex, fixed)) => (Some(flex), fixed),
        None => (None, 0.0),
    };
    let axis = super::axis_ref(cfg, side);
    SideComponents {
        out_margin: axis.out_margin,
        chart_title: match side {
            Side::Top => Some(chart_title_contribution(&cfg.chart_title)),
            Side::Bottom | Side::Left | Side::Right => None,
        },
        major_tick_length: tick_contribution(axis),
        colorbar_out_margin,
        colorbar_fixed,
    }
}

fn flex_total(c: &SideComponents) -> f32 {
    c.out_margin + c.chart_title.unwrap_or(0.0) + c.colorbar_out_margin.unwrap_or(0.0)
}

fn side_total(c: &SideComponents) -> f32 {
    flex_total(c) + c.major_tick_length + c.colorbar_fixed
}

// Write flex components back into Config. Tick lengths and the colourbar strip
// itself are fixed and not touched.
fn write_side(cfg: &mut Config, side: &Side, c: &SideComponents) {
    if let (Some(value), Some(bar)) = (c.colorbar_out_margin, cfg.colorbar.as_mut()) {
        bar.axis.out_margin = value;
    }
    super::axis_mut(cfg, side).out_margin = c.out_margin;
    if let (Side::Top, Some(v)) = (side, c.chart_title) {
        cfg.chart_title.top_margin = v;
    }
}

fn scale_proportional(c: &SideComponents, target: f32) -> Result<SideComponents, LayoutError> {
    // Tick and colourbar strip are fixed; scale only the flex part to reach
    // the target total.
    let flex_target = target - c.major_tick_length - c.colorbar_fixed;
    if flex_target < 0.0 {
        return Err(LayoutError::Infeasible);
    }
    let flex = flex_total(c);
    if flex <= 0.0 {
        if flex_target == 0.0 {
            return Ok(c.clone());
        }
        return Err(LayoutError::Infeasible);
    }
    let r = flex_target / flex;
    Ok(SideComponents {
        out_margin: c.out_margin * r,
        chart_title: c.chart_title.map(|v| v * r),
        major_tick_length: c.major_tick_length,
        colorbar_out_margin: c.colorbar_out_margin.map(|v| v * r),
        colorbar_fixed: c.colorbar_fixed,
    })
}

// PreserveTitles: keep chart_title fixed and adjust only out_margin (tick always
// fixed). The colourbar's label space is preserved for the same reason titles
// are — it holds text, and crushing it is what this strategy exists to avoid.
fn preserve_titles(c: &SideComponents, target: f32) -> Result<SideComponents, LayoutError> {
    let new_out = target - preserved_fixed(c);
    if new_out < 0.0 {
        return Err(LayoutError::Infeasible);
    }
    Ok(SideComponents {
        out_margin: new_out,
        ..c.clone()
    })
}

/// Everything `PreserveTitles` / `Absorb` hold still: the fixed terms plus the
/// text-bearing flex ones. The whole delta lands on the axis `out_margin`.
fn preserved_fixed(c: &SideComponents) -> f32 {
    c.major_tick_length
        + c.chart_title.unwrap_or(0.0)
        + c.colorbar_fixed
        + c.colorbar_out_margin.unwrap_or(0.0)
}

// Absorb: push the entire delta into out_margin. chart_title, tick, and the
// colourbar stay fixed.
fn absorb_single(c: &SideComponents, target: f32) -> Result<SideComponents, LayoutError> {
    let new_out = target - preserved_fixed(c);
    if new_out < 0.0 {
        return Err(LayoutError::Infeasible);
    }
    Ok(SideComponents {
        out_margin: new_out,
        ..c.clone()
    })
}

// Used by resize_chart_area_scaled: scale flex by `r`, leave tick fixed.
fn scale_flex(c: &SideComponents, r: f32) -> SideComponents {
    SideComponents {
        out_margin: c.out_margin * r,
        chart_title: c.chart_title.map(|v| v * r),
        major_tick_length: c.major_tick_length,
        colorbar_out_margin: c.colorbar_out_margin.map(|v| v * r),
        colorbar_fixed: c.colorbar_fixed,
    }
}

// fit_to_data_area + strategies, resize_chart_area variants, set_side_margin / set_margins.

impl Config {
    pub fn fit_to_data_area(
        &mut self,
        target: DataArea,
        strategy: FitStrategy,
    ) -> Result<(), LayoutError> {
        let ca = &self.chart_area;

        if target.x < ca.x
            || target.y < ca.y
            || target.x + target.width > ca.x + ca.width
            || target.y + target.height > ca.y + ca.height
            || target.width == 0
            || target.height == 0
        {
            return Err(LayoutError::TargetOutOfChartArea);
        }

        let need_top = (target.y - ca.y) as f32;
        let need_bottom = ((ca.y + ca.height) - (target.y + target.height)) as f32;
        let need_left = (target.x - ca.x) as f32;
        let need_right = ((ca.x + ca.width) - (target.x + target.width)) as f32;

        let sides = [
            (Side::Top, need_top),
            (Side::Bottom, need_bottom),
            (Side::Left, need_left),
            (Side::Right, need_right),
        ];

        let mut new_components: Vec<(Side, SideComponents)> = Vec::with_capacity(4);

        match &strategy {
            FitStrategy::ProportionalScale => {
                for (side, target_total) in sides.iter() {
                    let current = read_side(self, side);
                    let scaled = scale_proportional(&current, *target_total)?;
                    new_components.push((side.clone(), scaled));
                }
            }
            FitStrategy::PreserveTitles => {
                for (side, target_total) in sides.iter() {
                    let current = read_side(self, side);
                    let updated = preserve_titles(&current, *target_total)?;
                    new_components.push((side.clone(), updated));
                }
            }
            FitStrategy::Absorb(target_side) => {
                for (side, target_total) in sides.iter() {
                    let current = read_side(self, side);
                    let current_total = side_total(&current);
                    if side == target_side {
                        let updated = absorb_single(&current, *target_total)?;
                        new_components.push((side.clone(), updated));
                    } else {
                        if (current_total - *target_total).abs() > 1e-3 {
                            return Err(LayoutError::Infeasible);
                        }
                        new_components.push((side.clone(), current));
                    }
                }
            }
        }

        for (side, comps) in new_components.iter() {
            write_side(self, side, comps);
        }
        Ok(())
    }

    pub fn resize_chart_area(&mut self, new_area: ChartArea) -> Result<(), LayoutError> {
        let backup = self.chart_area.clone();
        self.chart_area = new_area;
        match self.data_area() {
            Ok(_) => Ok(()),
            Err(e) => {
                self.chart_area = backup;
                Err(e)
            }
        }
    }

    pub fn resize_chart_area_scaled(&mut self, new_area: ChartArea) -> Result<(), LayoutError> {
        let old_w = self.chart_area.width as f32;
        let old_h = self.chart_area.height as f32;
        if old_w <= 0.0 || old_h <= 0.0 {
            return Err(LayoutError::Infeasible);
        }
        let rx = new_area.width as f32 / old_w;
        let ry = new_area.height as f32 / old_h;

        // Top/Bottom scale by ry, Left/Right by rx.
        // Tick is fixed; only flex (out_margin / chart_title) is scaled.
        let scaled_sides: [(Side, SideComponents); 4] = [
            (Side::Top, scale_flex(&read_side(self, &Side::Top), ry)),
            (
                Side::Bottom,
                scale_flex(&read_side(self, &Side::Bottom), ry),
            ),
            (Side::Left, scale_flex(&read_side(self, &Side::Left), rx)),
            (Side::Right, scale_flex(&read_side(self, &Side::Right), rx)),
        ];

        let mut trial = self.clone();
        trial.chart_area = new_area.clone();
        for (side, c) in scaled_sides.iter() {
            write_side(&mut trial, side, c);
        }
        trial.data_area()?;

        self.chart_area = new_area;
        for (side, c) in scaled_sides.iter() {
            write_side(self, side, c);
        }
        Ok(())
    }

    pub fn set_side_margin(&mut self, side: Side, value: f32) -> Result<(), LayoutError> {
        if value < 0.0 {
            return Err(LayoutError::Infeasible);
        }
        let current = read_side(self, &side);
        let updated = scale_proportional(&current, value)?;

        let mut trial = self.clone();
        write_side(&mut trial, &side, &updated);
        trial.data_area()?;

        write_side(self, &side, &updated);
        Ok(())
    }

    pub fn set_margins(
        &mut self,
        margins: Margins,
        strategy: FitStrategy,
    ) -> Result<(), LayoutError> {
        let ca = &self.chart_area;
        let need_h = margins.left + margins.right;
        let need_v = margins.top + margins.bottom;
        if need_h > ca.width as f32 {
            return Err(LayoutError::OverflowHorizontal {
                required: need_h,
                available: ca.width,
            });
        }
        if need_v > ca.height as f32 {
            return Err(LayoutError::OverflowVertical {
                required: need_v,
                available: ca.height,
            });
        }
        let x = ca.x + margins.left.floor() as u32;
        let y = ca.y + margins.top.floor() as u32;
        let w = ca.width - (margins.left.floor() as u32 + margins.right.floor() as u32);
        let h = ca.height - (margins.top.floor() as u32 + margins.bottom.floor() as u32);
        if w == 0 || h == 0 {
            return Err(LayoutError::EmptyDataArea);
        }
        let target = DataArea(Rect {
            x,
            y,
            width: w,
            height: h,
        });
        self.fit_to_data_area(target, strategy)
    }
}

// fit / resize / margins tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::{default_colorbar_options, default_config};

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn config_with_colorbar(side: Side) -> Config {
        let mut cfg = default_config();
        let mut bar = default_colorbar_options();
        bar.side = side;
        cfg.colorbar = Some(bar);
        cfg
    }

    // After a successful fit_to_data_area, data_area() must equal target (within rounding).
    #[test]
    fn fit_to_data_area_matches_target() {
        let mut cfg = default_config();
        let target = DataArea(Rect {
            x: 150,
            y: 150,
            width: 700,
            height: 500,
        });
        cfg.fit_to_data_area(target.clone(), FitStrategy::ProportionalScale)
            .unwrap();
        let da = cfg.data_area().unwrap();
        assert_eq!(da.x, target.x);
        assert_eq!(da.y, target.y);
        assert_eq!(da.width, target.width);
        assert_eq!(da.height, target.height);
    }

    // On error, state must be unchanged (transactional).
    #[test]
    fn fit_to_data_area_err_preserves_state() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let bad = DataArea(Rect {
            x: 0,
            y: 0,
            width: 2000,
            height: 2000,
        });
        let res = cfg.fit_to_data_area(bad, FitStrategy::ProportionalScale);
        assert!(res.is_err());
        assert_eq!(cfg, before);
    }

    #[test]
    fn resize_chart_area_err_preserves_state() {
        let mut cfg = default_config();
        let before = cfg.clone();
        let tiny = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        });
        let res = cfg.resize_chart_area(tiny);
        assert!(res.is_err());
        assert_eq!(cfg, before);
    }

    // resize_chart_area_scaled scales flex but keeps tick fixed,
    // so total margin doesn't scale exactly by r — only the flex part does.
    #[test]
    fn resize_chart_area_scaled_scales_flex_not_tick() {
        let mut cfg = default_config();
        let out0_top = cfg.top_x.out_margin;
        let tick0_top = cfg.top_x.major_tick_length;
        let out0_left = cfg.left_y.out_margin;
        let tick0_left = cfg.left_y.major_tick_length;
        let chart_title0 = cfg.chart_title.top_margin;

        cfg.resize_chart_area_scaled(ChartArea(Rect {
            x: 0,
            y: 0,
            width: 2000,
            height: 1600,
        }))
        .unwrap();

        // Flex scales by 2x.
        assert!(approx_eq(cfg.top_x.out_margin, out0_top * 2.0, 1e-3));
        assert!(approx_eq(cfg.left_y.out_margin, out0_left * 2.0, 1e-3));
        assert!(approx_eq(
            cfg.chart_title.top_margin,
            chart_title0 * 2.0,
            1e-3
        ));
        // Tick stays fixed.
        assert!(approx_eq(cfg.top_x.major_tick_length, tick0_top, 1e-3));
        assert!(approx_eq(cfg.left_y.major_tick_length, tick0_left, 1e-3));
    }

    #[test]
    fn set_side_margin_basic() {
        let mut cfg = default_config();
        cfg.set_side_margin(Side::Left, 100.0).unwrap();
        let m = cfg.margins();
        assert!(approx_eq(m.left, 100.0, 1e-3));
    }

    // set_side_margin must not change tick length.
    #[test]
    fn set_side_margin_keeps_tick_fixed() {
        let mut cfg = default_config();
        let tick0 = cfg.left_y.major_tick_length;
        cfg.set_side_margin(Side::Left, 150.0).unwrap();
        assert!(approx_eq(cfg.left_y.major_tick_length, tick0, 1e-3));
    }

    #[test]
    fn set_margins_success() {
        let mut cfg = default_config();
        let m = Margins {
            top: 100.0,
            bottom: 80.0,
            left: 120.0,
            right: 80.0,
        };
        cfg.set_margins(m.clone(), FitStrategy::ProportionalScale)
            .unwrap();
        let got = cfg.margins();
        assert!(approx_eq(got.top, m.top, 1.0));
        assert!(approx_eq(got.bottom, m.bottom, 1.0));
        assert!(approx_eq(got.left, m.left, 1.0));
        assert!(approx_eq(got.right, m.right, 1.0));
    }

    // fit / resize must leave tick lengths (both major and minor) untouched.
    #[test]
    fn fit_keeps_tick_lengths_fixed() {
        let mut cfg = default_config();
        let maj0 = cfg.left_y.major_tick_length;
        let min0 = cfg.left_y.minor_tick_length;

        let target = DataArea(Rect {
            x: 150,
            y: 150,
            width: 700,
            height: 500,
        });
        cfg.fit_to_data_area(target, FitStrategy::ProportionalScale)
            .unwrap();

        assert!(approx_eq(cfg.left_y.major_tick_length, maj0, 1e-3));
        assert!(approx_eq(cfg.left_y.minor_tick_length, min0, 1e-3));
    }

    #[test]
    fn resize_keeps_tick_lengths_fixed() {
        let mut cfg = default_config();
        let maj0 = cfg.top_x.major_tick_length;
        let min0 = cfg.top_x.minor_tick_length;

        cfg.resize_chart_area_scaled(ChartArea(Rect {
            x: 0,
            y: 0,
            width: 2000,
            height: 1600,
        }))
        .unwrap();

        assert!(approx_eq(cfg.top_x.major_tick_length, maj0, 1e-3));
        assert!(approx_eq(cfg.top_x.minor_tick_length, min0, 1e-3));
    }

    // The invariant that ties this module to `margins()`: what fit thinks a
    // side totals must be what layout charges it. A colourbar band registered
    // in one and not the other would put the data area in two places.
    #[test]
    fn side_total_agrees_with_margins_on_every_side() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let cfg = config_with_colorbar(side.clone());
            let m = cfg.margins();
            for probe in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
                let charged = match probe {
                    Side::Top => m.top,
                    Side::Bottom => m.bottom,
                    Side::Left => m.left,
                    Side::Right => m.right,
                };
                let seen = side_total(&read_side(&cfg, &probe));
                assert!(
                    approx_eq(seen, charged, 1e-3),
                    "bar on {side:?}, probing {probe:?}: fit sees {seen}, margins charge {charged}"
                );
            }
        }
    }

    // A fit must be able to reclaim the bar's label space (it is text space,
    // like an axis' out_margin) but never the strip itself — a colourbar that
    // got thinner with the window would misreport its own colours' extent.
    #[test]
    fn proportional_fit_scales_the_colorbar_label_margin_and_not_the_strip() {
        let mut cfg = config_with_colorbar(Side::Right);
        let bar0 = cfg.colorbar.clone().expect("colourbar");
        let m = cfg.margins();
        // Halve the right margin's flex share by asking for a wider data area.
        let da = cfg.data_area().expect("data area");
        let target = DataArea(Rect {
            x: da.x,
            y: da.y,
            width: da.width + (m.right * 0.25) as u32,
            height: da.height,
        });
        cfg.fit_to_data_area(target.clone(), FitStrategy::ProportionalScale)
            .expect("fit");

        let bar = cfg.colorbar.as_ref().expect("colourbar");
        assert!(
            bar.axis.out_margin < bar0.axis.out_margin,
            "label margin should have shrunk: {} → {}",
            bar0.axis.out_margin,
            bar.axis.out_margin
        );
        assert!(approx_eq(bar.thickness_px, bar0.thickness_px, 1e-6));
        assert!(approx_eq(bar.gap_px, bar0.gap_px, 1e-6));
        assert!(approx_eq(
            bar.axis.major_tick_length,
            bar0.axis.major_tick_length,
            1e-6
        ));
        // And the fit actually landed where it was asked to.
        let after = cfg.data_area().expect("data area");
        assert_eq!(after.width, target.width);
    }

    // PreserveTitles and Absorb hold text space still and push the whole delta
    // into the axis margin — the colourbar's labels are text too.
    #[test]
    fn preserve_titles_and_absorb_leave_the_colorbar_alone() {
        for strategy in [
            FitStrategy::PreserveTitles,
            FitStrategy::Absorb(Side::Right),
        ] {
            let mut cfg = config_with_colorbar(Side::Right);
            let bar0 = cfg.colorbar.clone().expect("colourbar");
            let out0 = cfg.right_y.out_margin;
            let da = cfg.data_area().expect("data area");
            // 4 px, not more: `right_y.out_margin` starts at 8 (that side's
            // labels and title are off), and these two strategies may only
            // spend the axis margin.
            let target = DataArea(Rect {
                x: da.x,
                y: da.y,
                width: da.width + 4,
                height: da.height,
            });
            cfg.fit_to_data_area(target, strategy.clone())
                .unwrap_or_else(|e| panic!("{strategy:?} fit: {e:?}"));

            assert_eq!(cfg.colorbar.as_ref().expect("colourbar"), &bar0);
            assert!(
                cfg.right_y.out_margin < out0,
                "{strategy:?}: the delta belongs to the axis margin"
            );
        }
    }

    // resize_chart_area_scaled scales flex per side; the colourbar's label
    // margin is flex, its strip is not.
    #[test]
    fn scaled_resize_scales_the_colorbar_label_margin_only() {
        let mut cfg = config_with_colorbar(Side::Right);
        let bar0 = cfg.colorbar.clone().expect("colourbar");
        let area = cfg.chart_area.clone();
        cfg.resize_chart_area_scaled(ChartArea(Rect {
            x: area.x,
            y: area.y,
            width: area.width * 2,
            height: area.height,
        }))
        .expect("resize");

        let bar = cfg.colorbar.as_ref().expect("colourbar");
        assert!(approx_eq(
            bar.axis.out_margin,
            bar0.axis.out_margin * 2.0,
            1e-3
        ));
        assert!(approx_eq(bar.thickness_px, bar0.thickness_px, 1e-6));
        assert!(approx_eq(bar.gap_px, bar0.gap_px, 1e-6));
    }

    // A bar on one side must not be written to from another side's fit.
    #[test]
    fn fitting_a_side_without_a_bar_does_not_touch_it() {
        let mut cfg = config_with_colorbar(Side::Right);
        let bar0 = cfg.colorbar.clone().expect("colourbar");
        cfg.set_side_margin(Side::Left, cfg.margins().left * 0.8)
            .expect("set left margin");
        assert_eq!(cfg.colorbar.as_ref().expect("colourbar"), &bar0);
    }
}
