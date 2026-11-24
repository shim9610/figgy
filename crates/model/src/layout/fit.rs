use super::{
    ChartArea, Config, DataArea, LayoutError, Side, chart_title_contribution, colorbar_parts,
    tick_contribution,
};
use crate::layer::{CellRef, InsetAnchor};

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
        let before = self.has_insets().then(|| PanelAreas::of(self));
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
        // Its insets keep their place in it (#392).
        if let Some(before) = before {
            self.follow_parent_areas_from(&before, None);
        }
        Ok(())
    }

    /// Place subplot `index`'s frame at `area`, in this document's pixels
    /// (#205): anywhere, inside the parent chart, over another panel or
    /// beside every one. The subplot's plot must fit in the new frame; on an
    /// error, including an `index` past the subplot list
    /// ([`LayoutError::Infeasible`]), nothing changes. The placed frame is
    /// the subplot's new placement: its [`Subplot::anchor`] is written from
    /// it, or cleared when the frame lies in no plot, and the insets nested
    /// in it follow it (#392).
    ///
    /// [`Subplot::anchor`]: crate::layer::Subplot::anchor
    pub fn set_subplot_area(&mut self, index: usize, area: ChartArea) -> Result<(), LayoutError> {
        let mut probe = self.subplot_config(index).ok_or(LayoutError::Infeasible)?;
        probe.chart_area = area.clone();
        probe.data_area()?;
        let before = self.has_insets().then(|| PanelAreas::of(self));
        self.subplots[index].config.chart_area = area;
        if let Some(before) = before {
            self.follow_parent_areas_from(&before, Some(index));
        }
        self.anchor_placed(index);
        Ok(())
    }

    /// Whether a subplot of this document is an inset: nested in another
    /// panel's data area ([`Self::insets`]) or anchored in one
    /// ([`Subplot::anchor`](crate::layer::Subplot::anchor)).
    pub fn has_insets(&self) -> bool {
        self.subplots.iter().any(|s| s.anchor.is_some()) || self.insets().next().is_some()
    }

    /// Carry every inset of `before` with its parent's data area (#392).
    ///
    /// An inset's placement is its [`Subplot::anchor`]: the edges of its
    /// frame as fractions of its parent's data area. Its frame
    /// (`chart_area`) is derived from it here: the anchor mapped into the
    /// parent's data area in `self`, rounded to whole pixels, grown about its
    /// centre to the size the inset's own plot needs and kept inside the
    /// parent's plot when it fits there. The growth is never written back,
    /// so a parent that grows again gets the remembered placement exactly.
    /// An inset without an anchor (a released document, a subplot never
    /// carried) is anchored from its frame in `before` the first time its
    /// parent's data area changes; while it stays where it was, nothing is
    /// written.
    ///
    /// Each call that moves a parent's data area (a plot-area drag or
    /// resize, a band drag, a layout fit, [`Self::fit_to_data_area`], a
    /// chart's new area) calls this with the document as it was before the
    /// call. Insets are visited parents first (the anchor's parent, else
    /// [`crate::layout::inset_parent`] in `before`), so an inset nested in an
    /// inset follows it, also while the growth keeps it larger than its
    /// parent's plot. `placed` is a subplot the edit placed itself (a frame
    /// drag or resize, [`Self::set_subplot_area`]): its frame stays as placed
    /// and only its own insets follow it. A parent without a data area, or a
    /// document whose subplot list changed, moves nothing.
    ///
    /// [`Subplot::anchor`]: crate::layer::Subplot::anchor
    pub fn follow_parent_areas(&mut self, before: &Config, placed: Option<usize>) {
        self.follow_parent_areas_from(&PanelAreas::of(before), placed);
    }

    /// [`Self::follow_parent_areas`] from the parts of the document before
    /// the edit that it reads, so an edit snapshots those instead of the
    /// whole document.
    pub(crate) fn follow_parent_areas_from(&mut self, before: &PanelAreas, placed: Option<usize>) {
        if self.subplots.len() != before.subplot_count() {
            return;
        }
        self.follow_children(before, placed, 0);
    }

    /// [`Self::follow_parent_areas`] for the insets whose parent is panel
    /// `panel` (0 the document, `k + 1` subplot `k`), then theirs.
    fn follow_children(&mut self, before: &PanelAreas, placed: Option<usize>, panel: usize) {
        for index in 0..before.subplot_count() {
            if before.parent_panel(index) != Some(panel) {
                continue;
            }
            if placed != Some(index) {
                self.follow_one(before, index, panel);
            }
            self.follow_children(before, placed, index + 1);
        }
    }

    /// Derive inset `index`'s frame from its anchor in parent panel `panel`.
    fn follow_one(&mut self, before: &PanelAreas, index: usize, panel: usize) {
        let usable = |d: &crate::layout::Rect| d.width > 0 && d.height > 0;
        let new = match panel {
            0 => Some(&*self),
            _ => self.subplots.get(panel - 1).map(|s| &s.config),
        }
        .and_then(|chart| chart.data_area().ok())
        .map(|d| d.0)
        .filter(usable);
        let old = before.panels.get(panel).map(|p| p.data).filter(usable);
        let (Some(old), Some(new)) = (old, new) else {
            return;
        };
        let stored = &before.panels[index + 1];
        let frame = stored.frame;
        let anchor = match stored.anchor {
            Some(anchor) => anchor,
            // A frame that owns its placement keeps it while its parent
            // stays where it was.
            None if old == new => return,
            None => InsetAnchor {
                parent: cell_of_panel(panel),
                frame: fractions_of(frame, old),
            },
        };
        let Some(placed) = rect_at(anchor.frame, new) else {
            return;
        };
        let derived = self.with_plot_room(index, placed, frame, new);
        let subplot = &mut self.subplots[index];
        subplot.config.chart_area.0 = derived;
        subplot.anchor = Some(anchor);
    }

    /// The panel subplot `index` is placed in: its anchor's parent, else
    /// the panel whose data area holds its frame
    /// ([`crate::layout::inset_parent`]). `None` for a panel of its own.
    pub(crate) fn parent_panel(&self, index: usize) -> Option<usize> {
        let subplot = self.subplots.get(index)?;
        if let Some(anchor) = subplot.anchor {
            return match anchor.parent {
                CellRef::Parent => Some(0),
                CellRef::Subplot(p) if p < self.subplots.len() && p != index => Some(p + 1),
                CellRef::Subplot(_) => None,
            };
        }
        crate::layout::inset_parent(self.subplots.len() + 1, |i| self.panel_rects(i), index + 1)
    }

    /// Panel `panel`'s frame and data area (0 the document, `k + 1` subplot
    /// `k`); an empty data area when it has none.
    fn panel_rects(&self, panel: usize) -> (crate::layout::Rect, crate::layout::Rect) {
        let config = match panel {
            0 => self,
            _ => &self.subplots[panel - 1].config,
        };
        let empty = crate::layout::Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        (
            config.chart_area.0,
            config.data_area().map_or(empty, |d| d.0),
        )
    }

    /// Anchor subplot `index` where its frame was placed (#392): in the
    /// plot that holds the frame, or, for a frame larger than the plot its
    /// anchor names, in that plot while it holds the frame's centre. A frame
    /// in no plot owns its placement (`anchor: None`).
    pub(crate) fn anchor_placed(&mut self, index: usize) {
        let Some(subplot) = self.subplots.get(index) else {
            return;
        };
        let frame = subplot.config.chart_area.0;
        let count = self.subplots.len() + 1;
        let nested = crate::layout::inset_parent(count, |i| self.panel_rects(i), index + 1);
        let kept = || {
            let panel = self.parent_panel(index)?;
            let plot = self.panel_rects(panel).1;
            let (cx, cy) = (
                u64::from(frame.x) * 2 + u64::from(frame.width),
                u64::from(frame.y) * 2 + u64::from(frame.height),
            );
            let inside = cx >= u64::from(plot.x) * 2
                && cx <= (u64::from(plot.x) + u64::from(plot.width)) * 2
                && cy >= u64::from(plot.y) * 2
                && cy <= (u64::from(plot.y) + u64::from(plot.height)) * 2;
            inside.then_some(panel)
        };
        let anchor = nested.or_else(kept).and_then(|panel| {
            let plot = self.panel_rects(panel).1;
            (plot.width > 0 && plot.height > 0).then(|| InsetAnchor {
                parent: cell_of_panel(panel),
                frame: fractions_of(frame, plot),
            })
        });
        self.subplots[index].anchor = anchor;
    }

    /// `frame` for subplot `index`, grown about its centre until the
    /// subplot's plot fits in it, as every frame edit requires
    /// ([`Self::set_subplot_area`]), at least
    /// [`crate::layout::MIN_VISIBLE_DATA_AREA_PX`] wide and high, and kept inside `within` (the parent's
    /// plot) when it fits there. The plot is measured at the subplot's
    /// stored text sizes, so the result depends on the frame alone. A frame
    /// its plot still cannot fit in after a few steps becomes `fallback`'s
    /// size about the same centre.
    fn with_plot_room(
        &self,
        index: usize,
        frame: crate::layout::Rect,
        fallback: crate::layout::Rect,
        within: crate::layout::Rect,
    ) -> crate::layout::Rect {
        let Some(subplot) = self.subplots.get(index) else {
            return frame;
        };
        let mut probe = subplot.config.clone();
        let (cx, cy) = (
            f64::from(frame.x) + f64::from(frame.width) / 2.0,
            f64::from(frame.y) + f64::from(frame.height) / 2.0,
        );
        let about = |width: u32, height: u32| crate::layout::Rect {
            x: (cx - f64::from(width) / 2.0).round().max(0.0) as u32,
            y: (cy - f64::from(height) / 2.0).round().max(0.0) as u32,
            width,
            height,
        };
        let mut rect = frame;
        // Each step settles one direction; one more covers a margin that
        // the first growth uncovers.
        const STEPS: usize = 4;
        const MIN_PLOT: u32 = crate::layout::MIN_VISIBLE_DATA_AREA_PX;
        let mut fits = false;
        for _ in 0..STEPS {
            probe.chart_area.0 = rect;
            let grow = |required: f32| (required.ceil() as u32).saturating_add(1);
            rect = match probe.data_area() {
                Ok(plot) if plot.width >= MIN_PLOT && plot.height >= MIN_PLOT => {
                    fits = true;
                    break;
                }
                // A plot too small to grab grows to the size the data area
                // keeps visible elsewhere.
                Ok(plot) => about(
                    rect.width + MIN_PLOT.saturating_sub(plot.width),
                    rect.height + MIN_PLOT.saturating_sub(plot.height),
                ),
                Err(LayoutError::OverflowHorizontal { required, .. }) => {
                    about(grow(required).max(rect.width + 1), rect.height)
                }
                Err(LayoutError::OverflowVertical { required, .. }) => {
                    about(rect.width, grow(required).max(rect.height + 1))
                }
                Err(_) => about(rect.width + 1, rect.height + 1),
            };
        }
        if !fits {
            rect = about(fallback.width, fallback.height);
        }
        let inside = |at: u32, length: u32, from: u32, span: u32| {
            if length <= span {
                at.clamp(from, from + span - length)
            } else {
                at
            }
        };
        rect.x = inside(rect.x, rect.width, within.x, within.width);
        rect.y = inside(rect.y, rect.height, within.y, within.height);
        rect
    }
}

/// The cell panel `panel` names (0 the document, `k + 1` subplot `k`).
fn cell_of_panel(panel: usize) -> CellRef {
    match panel {
        0 => CellRef::Parent,
        _ => CellRef::Subplot(panel - 1),
    }
}

/// `frame`'s edges as fractions of `area`, `[[left, right], [top, bottom]]`.
fn fractions_of(frame: crate::layout::Rect, area: crate::layout::Rect) -> [[f64; 2]; 2] {
    let fx = |v: u32| (f64::from(v) - f64::from(area.x)) / f64::from(area.width);
    let fy = |v: u32| (f64::from(v) - f64::from(area.y)) / f64::from(area.height);
    [
        [fx(frame.x), fx(frame.x + frame.width)],
        [fy(frame.y), fy(frame.y + frame.height)],
    ]
}

/// The frame whose edges are at `fractions` of `area`, rounded to whole
/// pixels, at least one pixel wide and high. `None` when it does not fit in
/// `u32` or a fraction is not finite.
fn rect_at(fractions: [[f64; 2]; 2], area: crate::layout::Rect) -> Option<crate::layout::Rect> {
    let edge = |fraction: f64, from: u32, length: u32| {
        (f64::from(from) + fraction * f64::from(length)).round()
    };
    let x0 = edge(fractions[0][0], area.x, area.width);
    let x1 = edge(fractions[0][1], area.x, area.width);
    let y0 = edge(fractions[1][0], area.y, area.height);
    let y1 = edge(fractions[1][1], area.y, area.height);
    let fits = |v: f64| (v >= 0.0 && v <= f64::from(u32::MAX)).then_some(v as u32);
    let (x, y) = (fits(x0)?, fits(y0)?);
    let width = fits((x1 - x0).max(1.0))?;
    let height = fits((y1 - y0).max(1.0))?;
    x.checked_add(width)?;
    y.checked_add(height)?;
    Some(crate::layout::Rect {
        x,
        y,
        width,
        height,
    })
}

// fit tests.

/// What [`Config::follow_parent_areas`] reads of a document as it was
/// before an edit (#392): each panel's frame and data area (0 the document,
/// `k + 1` subplot `k`; an empty data area when it has none) and each
/// subplot's anchor. One entry per panel, sized by the subplot list.
pub(crate) struct PanelAreas {
    panels: Vec<PanelArea>,
}

#[derive(Clone, Copy)]
struct PanelArea {
    frame: crate::layout::Rect,
    data: crate::layout::Rect,
    anchor: Option<InsetAnchor>,
}

impl PanelAreas {
    pub(crate) fn of(config: &Config) -> Self {
        let panels = (0..=config.subplots.len())
            .map(|panel| {
                let (frame, data) = config.panel_rects(panel);
                let anchor = panel
                    .checked_sub(1)
                    .and_then(|index| config.subplots[index].anchor);
                PanelArea {
                    frame,
                    data,
                    anchor,
                }
            })
            .collect();
        Self { panels }
    }

    fn subplot_count(&self) -> usize {
        self.panels.len() - 1
    }

    /// [`Config::parent_panel`] of the document this was taken from.
    fn parent_panel(&self, index: usize) -> Option<usize> {
        let subplot = self.panels.get(index + 1)?;
        if let Some(anchor) = subplot.anchor {
            return match anchor.parent {
                CellRef::Parent => Some(0),
                CellRef::Subplot(p) if p < self.subplot_count() && p != index => Some(p + 1),
                CellRef::Subplot(_) => None,
            };
        }
        crate::layout::inset_parent(
            self.panels.len(),
            |i| (self.panels[i].frame, self.panels[i].data),
            index + 1,
        )
    }
}

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

    /// A 2000x1600 parent with one inset in its plot, and an inset nested
    /// in that inset's plot (#392).
    fn parent_with_insets() -> Config {
        let mut cfg = crate::defaults::config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 2000,
            height: 1600,
        });
        let plot = cfg.data_area().unwrap().0;
        let mut inset = crate::defaults::subplot();
        inset.config.chart_area = ChartArea(Rect {
            x: plot.x + plot.width / 2,
            y: plot.y + 20,
            width: plot.width / 2 - 20,
            height: plot.height / 2,
        });
        cfg.subplots.push(inset);
        let inner = cfg.subplots[0].config.data_area().unwrap().0;
        let mut nested = crate::defaults::subplot();
        // Narrow bands, so the nested inset keeps a plot at every size the
        // tests place the parent at.
        for axis in [
            &mut nested.config.top_x,
            &mut nested.config.bottom_x,
            &mut nested.config.left_y,
            &mut nested.config.right_y,
        ] {
            axis.out_margin = 4.0;
        }
        nested.config.chart_area = ChartArea(Rect {
            x: inner.x + inner.width / 2,
            y: inner.y + 4,
            width: inner.width / 2 - 4,
            height: inner.height / 2,
        });
        cfg.subplots.push(nested);
        assert_eq!(cfg.insets().collect::<Vec<_>>(), vec![0, 1]);
        cfg
    }

    /// The frame's edges as fractions of `area`: left, top, right, bottom.
    fn fractions(frame: Rect, area: Rect) -> [f64; 4] {
        let fx = |v: u32| (f64::from(v) - f64::from(area.x)) / f64::from(area.width);
        let fy = |v: u32| (f64::from(v) - f64::from(area.y)) / f64::from(area.height);
        [
            fx(frame.x),
            fy(frame.y),
            fx(frame.x + frame.width),
            fy(frame.y + frame.height),
        ]
    }

    /// Each edge of the frame within `px` pixels of where `expected`'s
    /// fractions put it in `area`.
    fn assert_keeps_place(frame: Rect, area: Rect, expected: [f64; 4], px: f64, context: &str) {
        let got = fractions(frame, area);
        let scale = [area.width, area.height, area.width, area.height];
        for k in 0..4 {
            let off = (got[k] - expected[k]).abs() * f64::from(scale[k]);
            assert!(
                off <= px,
                "{context}: edge {k} is {off} px off ({got:?} vs {expected:?})"
            );
        }
    }

    fn inset_frame(cfg: &Config, index: usize) -> Rect {
        cfg.subplots[index].config.chart_area.0
    }

    fn plot(cfg: &Config) -> Rect {
        cfg.data_area().unwrap().0
    }

    // An inset keeps its place in its parent's plot when the plot is
    // placed smaller, larger or at another aspect, and the inset nested in
    // it follows it (#392).
    #[test]
    fn insets_keep_their_place_when_the_parent_plot_is_placed() {
        let mut cfg = parent_with_insets();
        let outer = fractions(inset_frame(&cfg, 0), plot(&cfg));
        let inner = fractions(
            inset_frame(&cfg, 1),
            cfg.subplots[0].config.data_area().unwrap().0,
        );
        for (x, y, width, height) in [
            (200, 240, 1000, 800),
            (120, 80, 1800, 1400),
            (400, 120, 1000, 1360),
            (160, 500, 1760, 900),
        ] {
            let target = DataArea(Rect {
                x,
                y,
                width,
                height,
            });
            cfg.fit_to_data_area(target, FitStrategy::PreserveTitles)
                .unwrap();
            let context = format!("{width}x{height}");
            // Every frame is derived from the remembered placement, so the
            // rounding of earlier steps never adds up.
            assert_keeps_place(inset_frame(&cfg, 0), plot(&cfg), outer, 0.5, &context);
            assert_eq!(cfg.insets().collect::<Vec<_>>(), vec![0, 1], "{context}");
            let host = cfg.subplots[0].config.data_area().unwrap().0;
            assert_keeps_place(inset_frame(&cfg, 1), host, inner, 0.5, &context);
            let anchor = cfg.subplots[0].anchor.expect("anchored");
            assert_eq!(anchor.parent, CellRef::Parent);
            assert_eq!(
                cfg.subplots[1].anchor.expect("anchored").parent,
                CellRef::Subplot(0)
            );
        }
    }

    // A plot-area resize and a plot-area drag move the inset with the plot;
    // an inset the user dragged follows later resizes from where it was
    // put (#392).
    #[test]
    fn a_dragged_inset_follows_later_plot_resizes_from_where_it_was_put() {
        use crate::layout::{Element, NudgeResult};
        let mut cfg = parent_with_insets();
        let before = fractions(inset_frame(&cfg, 0), plot(&cfg));
        assert_eq!(
            cfg.nudge(Element::DataAreaEdge(Side::Right), &Ems, -200.0, 0.0),
            NudgeResult::Moved
        );
        assert_keeps_place(inset_frame(&cfg, 0), plot(&cfg), before, 0.5, "resize");
        let at = inset_frame(&cfg, 0);
        let area = plot(&cfg);
        assert_eq!(
            cfg.nudge(Element::DataArea, &Ems, 30.0, 20.0),
            NudgeResult::Moved
        );
        let moved = inset_frame(&cfg, 0);
        assert_eq!(
            (moved.x, moved.y, moved.width, moved.height),
            (at.x + 30, at.y + 20, at.width, at.height)
        );
        assert_eq!(plot(&cfg).width, area.width);

        // The user drags the inset to the left: it stays where it was put
        // and its own inset goes with it.
        let nested = inset_frame(&cfg, 1);
        assert_eq!(
            cfg.nudge(Element::SubplotFrame(0), &Ems, -120.0, 10.0),
            NudgeResult::Moved
        );
        let dropped = inset_frame(&cfg, 0);
        assert_eq!((dropped.x, dropped.y), (moved.x - 120, moved.y + 10));
        let follow = inset_frame(&cfg, 1);
        assert_eq!((follow.x, follow.y), (nested.x - 120, nested.y + 10));
        assert_eq!((follow.width, follow.height), (nested.width, nested.height));

        // A later resize carries it from there.
        let placed = fractions(dropped, plot(&cfg));
        assert_eq!(
            cfg.nudge(Element::DataAreaEdge(Side::Bottom), &Ems, 0.0, -100.0),
            NudgeResult::Moved
        );
        assert_keeps_place(inset_frame(&cfg, 0), plot(&cfg), placed, 0.5, "after drag");

        // Resizing the inset's frame keeps it as placed; the nested inset
        // follows the inset's plot.
        let [[left, right], [top, bottom]] = cfg.subplots[1].anchor.expect("anchored").frame;
        let inner = [left, top, right, bottom];
        let resized = inset_frame(&cfg, 0);
        assert_eq!(
            cfg.nudge(Element::SubplotFrameEdge(0, Side::Right), &Ems, 60.0, 0.0),
            NudgeResult::Moved
        );
        assert_eq!(inset_frame(&cfg, 0).width, resized.width + 60);
        let host = cfg.subplots[0].config.data_area().unwrap().0;
        assert_keeps_place(inset_frame(&cfg, 1), host, inner, 0.5, "inset resize");
    }

    // A parent shrunk past what the inset's own plot needs keeps the inset
    // at the size its plot needs, about its proportional centre and inside
    // the parent's plot, so every subplot keeps a plot (#392).
    #[test]
    fn an_inset_keeps_room_for_its_plot_when_the_parent_shrinks_far() {
        let mut cfg = parent_with_insets();
        let start = plot(&cfg);
        let frame = inset_frame(&cfg, 0);
        let centre = fractions(frame, start);
        let small = Rect {
            x: start.x,
            y: start.y,
            width: 260,
            height: 200,
        };
        cfg.fit_to_data_area(DataArea(small), FitStrategy::PreserveTitles)
            .unwrap();
        let kept = inset_frame(&cfg, 0);
        assert!(
            cfg.subplot_config(0).unwrap().data_area().is_ok(),
            "{kept:?}"
        );
        assert!(kept.width > (f64::from(frame.width) * 260.0 / f64::from(start.width)) as u32);
        assert!(kept.within(&small), "{kept:?} in {small:?}");
        let mid = |a: f64, b: f64| (a + b) / 2.0;
        let got = fractions(kept, small);
        for (axis, (g, c), span) in [
            (
                0,
                (mid(got[0], got[2]), mid(centre[0], centre[2])),
                small.width,
            ),
            (
                1,
                (mid(got[1], got[3]), mid(centre[1], centre[3])),
                small.height,
            ),
        ] {
            // The centre stays unless the frame had to be pushed inside.
            let pushed = (kept.x == small.x || kept.x + kept.width == small.x + small.width)
                && axis == 0
                || (kept.y == small.y || kept.y + kept.height == small.y + small.height)
                    && axis == 1;
            assert!(
                pushed || (g - c).abs() * f64::from(span) <= 1.0,
                "{axis}: {g} vs {c}"
            );
        }
    }

    // A layout fit that moves the parent's plot carries the insets, and a
    // second fit changes nothing (#392).
    #[test]
    fn a_layout_fit_carries_the_insets_and_stays_a_fixed_point() {
        let mut cfg = parent_with_insets();
        cfg.left_y.out_margin += 160.0;
        cfg.bottom_x.out_margin += 120.0;
        let outer = fractions(inset_frame(&cfg, 0), plot(&cfg));
        let host = cfg.subplots[0].config.data_area().unwrap().0;
        let inner = fractions(inset_frame(&cfg, 1), host);
        let fitted = crate::layout::auto_layout(&cfg, &Ems);
        assert_ne!(plot(&fitted), plot(&cfg));
        assert_keeps_place(inset_frame(&fitted, 0), plot(&fitted), outer, 0.5, "fit");
        let host = fitted.subplots[0].config.data_area().unwrap().0;
        assert_keeps_place(inset_frame(&fitted, 1), host, inner, 1.5, "fit nested");
        assert_eq!(crate::layout::auto_layout(&fitted, &Ems), fitted);
    }

    // A parent shrunk below what the inset's plot needs and grown back
    // gives every inset the frame it had before the shrink, exactly, however
    // often it is repeated: the growth is never written back (#392).
    #[test]
    fn shrinking_below_the_minimum_and_growing_back_restores_the_frames() {
        let mut cfg = parent_with_insets();
        let start = plot(&cfg);
        // The first move anchors the insets from their frames.
        cfg.fit_to_data_area(
            DataArea(Rect {
                x: start.x + 1,
                ..start
            }),
            FitStrategy::PreserveTitles,
        )
        .unwrap();
        cfg.fit_to_data_area(DataArea(start), FitStrategy::PreserveTitles)
            .unwrap();
        let frames = [inset_frame(&cfg, 0), inset_frame(&cfg, 1)];
        let anchors = [cfg.subplots[0].anchor, cfg.subplots[1].anchor];
        for small in [
            Rect {
                width: 260,
                height: 200,
                ..start
            },
            Rect {
                width: start.width / 3,
                height: start.height / 3,
                ..start
            },
            Rect {
                width: 180,
                height: start.height,
                ..start
            },
        ] {
            for _ in 0..3 {
                cfg.fit_to_data_area(DataArea(small), FitStrategy::PreserveTitles)
                    .unwrap();
                assert!(cfg.subplot_config(0).unwrap().data_area().is_ok());
                assert_eq!([cfg.subplots[0].anchor, cfg.subplots[1].anchor], anchors);
                cfg.fit_to_data_area(DataArea(start), FitStrategy::PreserveTitles)
                    .unwrap();
                assert_eq!(
                    [inset_frame(&cfg, 0), inset_frame(&cfg, 1)],
                    frames,
                    "{small:?}"
                );
            }
        }
    }

    // A drag while the parent is shrunk places the inset anew: the dragged
    // frame is its new anchor, and growing the parent back maps that
    // placement, not the one before the shrink (#392).
    #[test]
    fn a_drag_while_shrunk_becomes_the_remembered_placement() {
        use crate::layout::{Element, NudgeResult};
        let mut cfg = parent_with_insets();
        let start = plot(&cfg);
        let small = Rect {
            width: 900,
            height: 700,
            ..start
        };
        cfg.fit_to_data_area(DataArea(small), FitStrategy::PreserveTitles)
            .unwrap();
        assert_eq!(
            cfg.nudge(Element::SubplotFrame(0), &Ems, -150.0, 40.0),
            NudgeResult::Moved
        );
        let dragged = inset_frame(&cfg, 0);
        let expected = fractions(dragged, plot(&cfg));
        let anchor = cfg.subplots[0].anchor.expect("anchored");
        assert_eq!(anchor.parent, CellRef::Parent);
        assert_eq!(
            [
                anchor.frame[0][0],
                anchor.frame[1][0],
                anchor.frame[0][1],
                anchor.frame[1][1]
            ],
            expected
        );
        cfg.fit_to_data_area(DataArea(start), FitStrategy::PreserveTitles)
            .unwrap();
        assert_keeps_place(inset_frame(&cfg, 0), plot(&cfg), expected, 0.5, "grown");

        // A frame dragged out of every plot owns its placement again.
        let mut out = cfg.clone();
        let frame = inset_frame(&out, 0);
        assert_eq!(
            out.set_subplot_area(0, ChartArea(Rect { x: 2100, ..frame })),
            Ok(())
        );
        assert_eq!(out.subplots[0].anchor, None);
    }

    // A released document (no anchor) is not changed by an edit that leaves
    // the plot where it was, and is anchored from its stored frame the first
    // time the plot moves (#392).
    #[test]
    fn an_unanchored_inset_is_anchored_only_when_its_parent_moves() {
        use crate::layout::{Element, NudgeResult};
        let mut cfg = parent_with_insets();
        let frame = inset_frame(&cfg, 0);
        let mut moved = cfg.clone();
        assert_eq!(
            moved.nudge(Element::Axis(Side::Left), &Ems, -3.0, 0.0),
            NudgeResult::Moved
        );
        assert_eq!(moved.subplots[0].anchor, None);
        assert_eq!(inset_frame(&moved, 0), frame);
        let expected = fractions(frame, plot(&cfg));
        assert_eq!(
            cfg.nudge(Element::DataArea, &Ems, 12.0, 0.0),
            NudgeResult::Moved
        );
        let anchor = cfg.subplots[0].anchor.expect("anchored");
        assert_eq!(
            [
                anchor.frame[0][0],
                anchor.frame[1][0],
                anchor.frame[0][1],
                anchor.frame[1][1]
            ],
            expected
        );
    }

    // `set_subplot_area` places the subplot and carries its insets; a
    // subplot list that changed moves nothing (#392).
    #[test]
    fn placing_an_inset_carries_its_own_insets() {
        let mut cfg = parent_with_insets();
        let host = cfg.subplots[0].config.data_area().unwrap().0;
        let inner = fractions(inset_frame(&cfg, 1), host);
        let frame = inset_frame(&cfg, 0);
        let area = ChartArea(Rect {
            x: frame.x - 100,
            y: frame.y + 30,
            width: frame.width + 80,
            height: frame.height + 40,
        });
        cfg.set_subplot_area(0, area.clone()).unwrap();
        assert_eq!(cfg.subplots[0].config.chart_area, area);
        let host = cfg.subplots[0].config.data_area().unwrap().0;
        assert_keeps_place(inset_frame(&cfg, 1), host, inner, 0.5, "placed");

        let before = cfg.clone();
        let mut fewer = cfg.clone();
        fewer.subplots.pop();
        let kept = fewer.clone();
        fewer.follow_parent_areas(&before, None);
        assert_eq!(fewer, kept);
    }

    /// A measure in ems: every glyph half its text's size wide.
    struct Ems;
    impl crate::text::MeasureText for Ems {
        fn measure_rich(&self, rt: &crate::text::RichText) -> crate::text::TextExtents {
            crate::text::TextExtents {
                width: rt.segments.len() as f32 * rt.font_size * 0.5,
                ascent: rt.font_size * 0.8,
                descent: rt.font_size * 0.2,
            }
        }
    }
}
