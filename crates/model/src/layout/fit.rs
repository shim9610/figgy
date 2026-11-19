use super::{
    ChartArea, Config, DataArea, LayoutError, Side, chart_title_contribution, colorbar_parts,
    tick_contribution,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FitStrategy {
    /// Keep the chart title, tick marks and colourbar where they are; each
    /// side's axis `out_margin` takes the change.
    PreserveTitles,
}

// Per-side components used internally by fit.
//
// Flex: `out_margin`, `chart_title` (Top only),
// `colorbar_out_margin` (the colourbar's side only).
// Fixed (never scaled): `major_tick_length`, `colorbar_fixed`.
//
// `label_offset_{x,y}` and `minor_tick_length` are visual-only and don't
// contribute to margins, so they are absent here.
//
// The colourbar band is split rather than registered whole, and both halves
// matter. Registering all of it as fixed would leave fit unable to reclaim the
// bar's label space, so a tight target fails where it could have succeeded;
// omitting it entirely would let fit hand the data area a band that
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
        // The right and top bands are group geometry when overlays draw
        // axes there; fit and resize read and write the group through the
        // parent axis.
        out_margin: match side {
            Side::Right => super::right_out_margin_effective(cfg),
            Side::Top => super::top_out_margin_effective(cfg),
            Side::Bottom | Side::Left => axis.out_margin,
        },
        chart_title: match side {
            Side::Top => Some(chart_title_contribution(&cfg.chart_title)),
            Side::Bottom | Side::Left | Side::Right => None,
        },
        major_tick_length: tick_contribution(axis),
        colorbar_out_margin,
        colorbar_fixed,
    }
}

#[cfg(test)]
fn flex_total(c: &SideComponents) -> f32 {
    c.out_margin + c.chart_title.unwrap_or(0.0) + c.colorbar_out_margin.unwrap_or(0.0)
}

/// What a side totals: test support for the agreement with `margins()`.
#[cfg(test)]
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
    match side {
        Side::Right => super::sync_overlay_right_margins(cfg),
        Side::Top => super::sync_overlay_top_margins(cfg),
        Side::Bottom | Side::Left => {}
    }
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

/// Everything `PreserveTitles` holds still: the fixed terms plus the
/// text-bearing flex ones. The whole delta lands on the axis `out_margin`.
fn preserved_fixed(c: &SideComponents) -> f32 {
    c.major_tick_length
        + c.chart_title.unwrap_or(0.0)
        + c.colorbar_fixed
        + c.colorbar_out_margin.unwrap_or(0.0)
}

// fit_to_data_area.

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
            FitStrategy::PreserveTitles => {
                for (side, target_total) in sides.iter() {
                    let current = read_side(self, side);
                    let updated = preserve_titles(&current, *target_total)?;
                    new_components.push((side.clone(), updated));
                }
            }
        }

        for (side, comps) in new_components.iter() {
            write_side(self, side, comps);
        }
        // The data area is placed where the caller put it: a later draw
        // keeps it (#357).
        self.layout_state = crate::config::LayoutState::Fixed;
        Ok(())
    }

    /// Place subplot `index`'s frame at `area`, in this document's pixels
    /// (#205): anywhere, inside the parent chart, over another panel or
    /// beside every one. The subplot's plot must fit in the new frame; on an
    /// error, including an `index` past the subplot list
    /// ([`LayoutError::Infeasible`]), nothing changes.
    pub fn set_subplot_area(&mut self, index: usize, area: ChartArea) -> Result<(), LayoutError> {
        let mut probe = self.subplot_config(index).ok_or(LayoutError::Infeasible)?;
        probe.chart_area = area.clone();
        probe.data_area()?;
        self.subplots[index].config.chart_area = area;
        Ok(())
    }
}

// fit tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn config_with_colorbar(side: Side) -> Config {
        let mut cfg = crate::defaults::config();
        let mut bar = crate::defaults::color_bar_options();
        bar.side = side;
        cfg.colorbar = Some(bar);
        cfg
    }

    // After a successful fit_to_data_area, data_area() must equal target (within rounding).
    #[test]
    fn fit_to_data_area_matches_target() {
        let mut cfg = crate::defaults::config();
        let target = DataArea(Rect {
            x: 150,
            y: 150,
            width: 700,
            height: 500,
        });
        cfg.fit_to_data_area(target.clone(), FitStrategy::PreserveTitles)
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
        let mut cfg = crate::defaults::config();
        let before = cfg.clone();
        let bad = DataArea(Rect {
            x: 0,
            y: 0,
            width: 2000,
            height: 2000,
        });
        let res = cfg.fit_to_data_area(bad, FitStrategy::PreserveTitles);
        assert!(res.is_err());
        assert_eq!(cfg, before);
    }

    #[test]
    fn a_subplot_area_is_set_anywhere_and_refused_when_its_plot_cannot_fit() {
        let mut cfg = crate::defaults::config();
        let mut subplot = crate::defaults::subplot();
        subplot.config.chart_area = ChartArea(Rect {
            x: 10,
            y: 10,
            width: 300,
            height: 200,
        });
        cfg.subplots.push(subplot);
        let far = ChartArea(Rect {
            x: 5000,
            y: 3000,
            width: 320,
            height: 240,
        });
        cfg.set_subplot_area(0, far.clone()).unwrap();
        assert_eq!(cfg.subplots[0].config.chart_area, far);
        let before = cfg.clone();
        let tiny = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        });
        assert!(cfg.set_subplot_area(0, tiny).is_err());
        assert!(cfg.set_subplot_area(7, far).is_err());
        assert_eq!(cfg, before);
    }

    // fit must leave tick lengths (both major and minor) untouched.
    #[test]
    fn fit_keeps_tick_lengths_fixed() {
        let mut cfg = crate::defaults::config();
        let maj0 = cfg.left_y.major_tick_length;
        let min0 = cfg.left_y.minor_tick_length;

        let target = DataArea(Rect {
            x: 150,
            y: 150,
            width: 700,
            height: 500,
        });
        cfg.fit_to_data_area(target, FitStrategy::PreserveTitles)
            .unwrap();

        assert!(approx_eq(cfg.left_y.major_tick_length, maj0, 1e-3));
        assert!(approx_eq(cfg.left_y.minor_tick_length, min0, 1e-3));
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

    // PreserveTitles holds text space still and pushes the whole delta into
    // the axis margin — the colourbar's labels are text too.
    #[test]
    fn preserve_titles_leaves_the_colorbar_alone() {
        let mut cfg = config_with_colorbar(Side::Right);
        let bar0 = cfg.colorbar.clone().expect("colourbar");
        let out0 = cfg.right_y.out_margin;
        let da = cfg.data_area().expect("data area");
        // 4 px, not more: `right_y.out_margin` is small (that side's labels
        // and title are off), and this strategy may only spend the axis
        // margin.
        let target = DataArea(Rect {
            x: da.x,
            y: da.y,
            width: da.width + 4,
            height: da.height,
        });
        cfg.fit_to_data_area(target, FitStrategy::PreserveTitles)
            .expect("fit");

        assert_eq!(cfg.colorbar.as_ref().expect("colourbar"), &bar0);
        assert!(
            cfg.right_y.out_margin < out0,
            "the delta belongs to the axis margin"
        );
    }

    // A bar on one side must not be written to from another side's fit.
    #[test]
    fn fitting_a_side_without_a_bar_does_not_touch_it() {
        let mut cfg = config_with_colorbar(Side::Right);
        let bar0 = cfg.colorbar.clone().expect("colourbar");
        let da = cfg.data_area().expect("data area");
        let target = DataArea(Rect {
            x: da.x - 8,
            y: da.y,
            width: da.width + 8,
            height: da.height,
        });
        cfg.fit_to_data_area(target, FitStrategy::PreserveTitles)
            .expect("move the left edge");
        assert_eq!(cfg.colorbar.as_ref().expect("colourbar"), &bar0);
    }
}
