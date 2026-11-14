//! Expansion of a layered document into internal panels.
//!
//! A `Config` with overlay layers is one public SSOT. The renderer draws it
//! as several panels composited on top of each other, one per layer, each a
//! plain single-layer `Config` sharing the parent's chart area and data area.
//! [`Config::expand`] is that pure derivation. The panels it returns are never
//! serialized or exposed as document state; they reference the same column
//! ids as the public series declarations and copy only configuration.
//!
//! Rules, in the order the panels are drawn:
//!
//! - The base panel is the parent without its overlays. It draws the shared
//!   decorations: X axes, the parent's Y axes, title, grid, merged legend and
//!   colourbar.
//! - Each overlay panel draws no grid, no title and no colourbar. The data
//!   transform reads `bottom_x` and `left_y`, so those carry the axes the
//!   overlay follows, hidden:
//!   - X: with `link_x` on, the panel keeps the parent's X axes, hidden (the
//!     base panel draws them). With it off, its `top_x` is the overlay's own
//!     `x_axis`, drawn, and its `bottom_x` a hidden copy of it carrying the
//!     parent's bottom band.
//!   - Y: its `left_y` is a hidden copy carrying the overlay's `y_axis`
//!     range, or with `link_y` on the parent's `left_y` range, and the
//!     parent's left band. Its `right_y` is the overlay's `y_axis`, drawn, or
//!     with `link_y` on a hidden copy of the parent's `right_y`.
//!
//!   It draws the overlay's legend when it has one.
//! - Every panel's margins are lifted to the group margins, so `data_area()`
//!   returns the same rectangle for all of them. The group right margin is
//!   the widest band of the parent's right axis and every overlay Y axis that
//!   is drawn, and the group top margin likewise for the top axes
//!   (`Config::margins`).
//! - A panel's config depends only on what that panel draws. Hidden parts
//!   (an overlay panel's linked axes, title, grid, legend, colourbar) are
//!   canonical defaults carrying just their margin and transform fields, and
//!   each panel keeps only the picked references of its own series, so an
//!   edit of one layer leaves every other panel's config, and with it the
//!   renderer's per-panel raster cache, untouched.
//! - Expansion ignores `active_layer`; the panels always carry `Base`. The
//!   document's one annotation layer stays on the base panel, anchored in
//!   the parent's axes; overlay panels carry an empty one.
//! - Each member of `Config::line_offsets` (offset spectra) is a panel of its
//!   own, after the overlay panels and in stacking order: the panel of the
//!   layer that scales it (the base panel or an overlay's) with `bottom_x`
//!   moved by the member's x shift and the Y axis the transform reads
//!   (`left_y`) by its y shift (decades on a logarithmic axis), every axis
//!   element, the title, the grid, the legend and the colourbar hidden, and
//!   only that member's picked references. The base and overlay panels draw
//!   no member. A member panel is not a layer of its own: it reports its
//!   owner's layer, which is what receives the pointer for it.
//!
//! Subplots (`Config::subplots`) are charts of their own. Each expands by
//! the same rules from its resolved config ([`Config::subplot_config`]: the
//! subplot's `config` with its linked axes following their targets), after
//! the parent's panels and in subplot order, so its panels share its own
//! chart area and data area, not the parent's. The document's picked
//! references of a subplot's series go to that subplot's panels, which split
//! them by the same rule.

use crate::config::{
    AxisBreaks, AxisOptions, AxisScale, Category, ChartTitleOptions, ColorBarOptions, Config,
    TickVisibility,
};
use crate::layer::{AnnotationLayer, CellRef, LayerRef, OverlayLayer, SubplotTextScale};
use crate::layout::{DataArea, LayoutError, Margins, Rect, Side};

/// One internal panel of an expanded document.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandedPanel {
    /// The layer this panel draws: `Base` first, then `Overlay(i)` in order,
    /// then the line offset members' panels, each reporting the layer that
    /// scales its member, then per subplot `Subplot(i)`, its
    /// `SubplotOverlay(i, j)` and its own members' panels.
    pub layer: LayerRef,
    /// A single-layer config (no overlays, `active_layer == Base`) whose
    /// `data_area()` is the group data area.
    pub config: Config,
}

/// The panels of a document plus the data area they all share.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    /// Base panel first, then one panel per overlay in index order, then one
    /// per line offset member in stacking order, then each subplot's panels
    /// in the same shape.
    pub panels: Vec<ExpandedPanel>,
    /// The group data area; equal to the parent's `data_area()`.
    pub data_area: DataArea,
}

impl Expansion {
    /// The panel of `layer`, or `None` for an overlay index past the end and
    /// for the annotation layer, which has no panel of its own. A layer with
    /// line offset members names its own panel, never a member's.
    pub fn panel(&self, layer: LayerRef) -> Option<&ExpandedPanel> {
        self.panels.iter().find(|panel| panel.layer == layer)
    }
}

impl Config {
    /// The layer that receives pointer interaction, with the out-of-range
    /// fallback applied: an `Overlay(i)` with no `overlays[i]` reads as
    /// `Base`. The raw field is `self.active_layer`.
    pub fn active_layer(&self) -> LayerRef {
        self.existing_layer(self.active_layer)
    }

    /// Set the interaction target layer. An `Overlay` or `Subplot` index past
    /// the end of its list is stored as `Base`, a `SubplotOverlay` whose
    /// overlay is missing as its subplot. Returns the layer actually stored.
    pub fn set_active_layer(&mut self, layer: LayerRef) -> LayerRef {
        self.active_layer = self.existing_layer(layer);
        self.active_layer
    }

    /// `layer` with the out-of-range fallbacks applied.
    fn existing_layer(&self, layer: LayerRef) -> LayerRef {
        match layer {
            LayerRef::Overlay(index) if index >= self.overlays.len() => LayerRef::Base,
            LayerRef::Subplot(index) if index >= self.subplots.len() => LayerRef::Base,
            LayerRef::SubplotOverlay(index, _) if index >= self.subplots.len() => LayerRef::Base,
            LayerRef::SubplotOverlay(index, overlay)
                if overlay >= self.subplots[index].config.overlays.len() =>
            {
                LayerRef::Subplot(index)
            }
            layer => layer,
        }
    }

    /// The layer that draws `series_id`: the first parent overlay listing the
    /// id, else the first subplot drawing it (on its overlay when one of the
    /// subplot's overlays lists it), else `Base`.
    pub fn layer_of_series(&self, series_id: &str) -> LayerRef {
        if let Some(index) = self
            .overlays
            .iter()
            .position(|overlay| overlay.owns_series(series_id))
        {
            return LayerRef::Overlay(index);
        }
        let Some(index) = self
            .subplots
            .iter()
            .position(|subplot| subplot.owns_series(series_id))
        else {
            return LayerRef::Base;
        };
        match self.subplots[index].config.layer_of_series(series_id) {
            LayerRef::Overlay(overlay) => LayerRef::SubplotOverlay(index, overlay),
            _ => LayerRef::Subplot(index),
        }
    }

    /// Whether series `series_id` takes part in fitting `layer`'s X and Y
    /// axes, as `(x, y)`. `Annotations` fits the base axes.
    ///
    /// `Base` and `Subplot(i)` fit that chart's `bottom_x` and `left_y`. A
    /// linked axis is one axis shared by its link group, so each fits every
    /// series drawn against it. The X axis fits the series of every chart in
    /// its X link group, the series of overlays that follow it (`link_x` on)
    /// included. The Y axis fits the base-layer series of every chart in its
    /// Y link group and the series of overlays that follow it (`link_y` on).
    ///
    /// `Overlay(n)` and `SubplotOverlay(i, n)` fit the overlay's own axes
    /// from that overlay's series: its own X axis when `link_x` is off (none
    /// while it is on) and its own Y axis. With `link_y` on, the Y axis it
    /// follows is its chart's `left_y`, fitted as that chart's layer fits it.
    pub fn series_fit_roles(&self, layer: LayerRef, series_id: &str) -> (bool, bool) {
        let cell = |layer: LayerRef| match layer {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                CellRef::Subplot(index)
            }
            _ => CellRef::Parent,
        };
        let root = |cell: CellRef, x: bool| match cell {
            CellRef::Parent => CellRef::Parent,
            CellRef::Subplot(index) => self.subplot_link_root(index, x),
        };
        let own = self.layer_of_series(series_id);
        let own_overlay = self.overlay_of_layer(own);
        // Whether the series is drawn against its chart's X and Y axes.
        let on_chart_x = own_overlay.is_none_or(|overlay| overlay.link_x);
        let on_chart_y = own_overlay.is_none_or(|overlay| overlay.link_y);
        let chart_y =
            |layer: LayerRef| on_chart_y && root(cell(own), false) == root(cell(layer), false);
        match layer {
            LayerRef::Base | LayerRef::Subplot(_) | LayerRef::Annotations => (
                on_chart_x && root(cell(own), true) == root(cell(layer), true),
                chart_y(layer),
            ),
            LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..) => {
                let overlay = self.overlay_of_layer(layer);
                let own_x = overlay.is_some_and(|overlay| !overlay.link_x);
                let linked_y = overlay.is_some_and(|overlay| overlay.link_y);
                (
                    own_x && own == layer,
                    if linked_y {
                        chart_y(layer)
                    } else {
                        own == layer
                    },
                )
            }
        }
    }

    /// The X and Y axes the series of this chart's overlay `overlay` are
    /// scaled by (each its own or, linked, the chart's `bottom_x` /
    /// `left_y`), or with `None` the chart's own `bottom_x` and `left_y`. An
    /// index past the end reads as `None`.
    pub fn scaling_axes(&self, overlay: Option<usize>) -> (&AxisOptions, &AxisOptions) {
        let overlay = overlay.and_then(|index| self.overlays.get(index));
        (
            overlay
                .and_then(OverlayLayer::own_x)
                .unwrap_or(&self.bottom_x),
            overlay
                .and_then(OverlayLayer::own_y)
                .unwrap_or(&self.left_y),
        )
    }

    /// The overlay `layer` names: `Overlay(n)` of the document or
    /// `SubplotOverlay(i, n)` of subplot `i`; `None` for any other layer and
    /// for an index past the end.
    pub fn overlay_of_layer(&self, layer: LayerRef) -> Option<&OverlayLayer> {
        match layer {
            LayerRef::Overlay(index) => self.overlays.get(index),
            LayerRef::SubplotOverlay(index, overlay) => self
                .subplots
                .get(index)
                .and_then(|subplot| subplot.config.overlays.get(overlay)),
            _ => None,
        }
    }

    /// The subplot rules: one level of nesting, one annotation layer per
    /// document (the parent's), links pointing at the parent or an earlier
    /// subplot, and every series id drawn by at most one subplot and no
    /// parent overlay. Each subplot's own overlays follow
    /// [`Self::validate_overlays`].
    pub fn validate_subplots(&self) -> Result<(), crate::RuleKind> {
        for (index, subplot) in self.subplots.iter().enumerate() {
            if !subplot.config.subplots.is_empty() {
                return Err(crate::RuleKind::SubplotNested);
            }
            if !subplot.config.annotations.is_default() {
                return Err(crate::RuleKind::SubplotAnnotations);
            }
            for link in [subplot.link_x, subplot.link_y].into_iter().flatten() {
                if let CellRef::Subplot(target) = link
                    && target >= index
                {
                    return Err(crate::RuleKind::SubplotLink);
                }
            }
            subplot.config.validate_overlays()?;
            let ids = subplot.series.iter().chain(
                subplot
                    .config
                    .overlays
                    .iter()
                    .flat_map(|overlay| overlay.series.iter()),
            );
            for id in ids {
                if self.overlays.iter().any(|overlay| overlay.owns_series(id))
                    || self.subplots[..index]
                        .iter()
                        .any(|other| other.owns_series(id))
                {
                    return Err(crate::RuleKind::SeriesInManyLayers);
                }
            }
        }
        Ok(())
    }

    /// The chart a link target names, as resolved so far.
    fn link_target_axis(&self, target: CellRef, x: bool) -> Option<&AxisOptions> {
        let config = match target {
            CellRef::Parent => self,
            CellRef::Subplot(index) => &self.subplots.get(index)?.config,
        };
        Some(if x { &config.bottom_x } else { &config.left_y })
    }

    /// The chart at the root of a link chain: the first chart, following
    /// `subplots[index]`'s link on one axis, that is not linked itself.
    pub fn subplot_link_root(&self, index: usize, x: bool) -> CellRef {
        let mut cell = CellRef::Subplot(index);
        // Links point at a lower index, so the chain ends; the bound guards
        // an unvalidated document.
        for _ in 0..=self.subplots.len() {
            let CellRef::Subplot(at) = cell else {
                return cell;
            };
            let Some(subplot) = self.subplots.get(at) else {
                return CellRef::Parent;
            };
            match if x { subplot.link_x } else { subplot.link_y } {
                Some(target) => cell = target,
                None => return cell,
            }
        }
        cell
    }

    /// Keep every linked axis equal to its target. With `previous` (the
    /// document before an edit), a host edit of a linked axis is carried to
    /// the root of its chain first, so the whole link group moves; an edit
    /// of the root itself wins over member edits made in the same call. An
    /// edit is a change of range, scale or direction; the edited axis' tick
    /// spacing travels with it. Range, scale, direction and tick spacing are
    /// linked; the axis' line, label and title style stay the subplot's own.
    pub fn sync_subplot_links(&mut self, previous: Option<&Config>) {
        for x in [true, false] {
            if let Some(previous) = previous {
                self.carry_link_edits(previous, x);
            }
            for index in 0..self.subplots.len() {
                let link = if x {
                    self.subplots[index].link_x
                } else {
                    self.subplots[index].link_y
                };
                let Some(target) = link else { continue };
                let Some(from) = self.link_target_axis(target, x).cloned() else {
                    continue;
                };
                let config = &mut self.subplots[index].config;
                copy_axis_range(
                    &from,
                    if x {
                        &mut config.bottom_x
                    } else {
                        &mut config.left_y
                    },
                );
            }
        }
    }

    fn carry_link_edits(&mut self, previous: &Config, x: bool) {
        let axis_of = |config: &Config, cell: CellRef| -> Option<AxisOptions> {
            let chart = match cell {
                CellRef::Parent => config,
                CellRef::Subplot(index) => &config.subplots.get(index)?.config,
            };
            Some(if x {
                chart.bottom_x.clone()
            } else {
                chart.left_y.clone()
            })
        };
        for index in 0..self.subplots.len() {
            let linked = if x {
                self.subplots[index].link_x
            } else {
                self.subplots[index].link_y
            };
            if linked.is_none() {
                continue;
            }
            let root = self.subplot_link_root(index, x);
            let (Some(root_now), Some(root_before)) =
                (axis_of(self, root), axis_of(previous, root))
            else {
                continue;
            };
            if !same_axis_range(&root_now, &root_before) {
                continue;
            }
            let member = CellRef::Subplot(index);
            let (Some(now), Some(before)) = (axis_of(self, member), axis_of(previous, member))
            else {
                continue;
            };
            if same_axis_range(&now, &before) {
                continue;
            }
            let chart = match root {
                CellRef::Parent => &mut *self,
                CellRef::Subplot(root) => &mut self.subplots[root].config,
            };
            copy_axis_range(
                &now,
                if x {
                    &mut chart.bottom_x
                } else {
                    &mut chart.left_y
                },
            );
        }
    }

    /// Subplot `index` as the chart it draws: its config with its linked
    /// axes following their targets, `Base` as its active layer and no
    /// annotation layer. `None` when the index is out of range.
    pub fn subplot_config(&self, index: usize) -> Option<Config> {
        let mut config = self.subplots.get(index)?.config.clone();
        for x in [true, false] {
            let link = if x {
                self.subplots[index].link_x
            } else {
                self.subplots[index].link_y
            };
            if let Some(target) = link {
                let root = match target {
                    CellRef::Parent => CellRef::Parent,
                    CellRef::Subplot(_) => self.subplot_link_root(index, x),
                };
                if let Some(from) = self.link_target_axis(root, x) {
                    copy_axis_range(
                        from,
                        if x {
                            &mut config.bottom_x
                        } else {
                            &mut config.left_y
                        },
                    );
                }
            }
        }
        config.active_layer = LayerRef::Base;
        config.annotations = AnnotationLayer::empty();
        let scale = self.subplot_text_scale(index);
        if scale != 1.0 {
            config.scale_text_sizes(None, scale);
        }
        Some(config)
    }

    /// The factor subplot `index` draws its text with (#292): `1.0` for
    /// [`SubplotTextScale::Fixed`], else [`subplot_text_factor`] of the
    /// smaller of the cell's width and height as a share of the parent
    /// chart's (`chart_area`). `1.0` for an index out of range and while
    /// the parent's chart area is empty.
    ///
    /// A display parameter chosen from the layout, like the tick plan: it
    /// is read whenever the subplot's chart is resolved
    /// ([`Self::subplot_config`]), so drawing, measuring, `auto_layout`,
    /// hit regions, drags and exports use the same sizes, and it is never
    /// stored. An export scale multiplies both chart areas, so the factor
    /// is the same at 1x and 2x.
    pub fn subplot_text_scale(&self, index: usize) -> f32 {
        let Some(subplot) = self.subplots.get(index) else {
            return 1.0;
        };
        if subplot.text_scale == SubplotTextScale::Fixed {
            return 1.0;
        }
        let (parent, cell) = (&self.chart_area.0, &subplot.config.chart_area.0);
        if parent.width == 0 || parent.height == 0 {
            return 1.0;
        }
        let across = cell.width as f32 / parent.width as f32;
        let down = cell.height as f32 / parent.height as f32;
        subplot_text_factor(across.min(down))
    }

    /// The rectangle the whole document covers: the union of the parent's
    /// `chart_area` and every subplot's. Equal to `chart_area` without
    /// subplots. Export renders this rectangle and a display fits it to the
    /// surface.
    pub fn document_area(&self) -> Rect {
        let mut union = self.chart_area.0;
        for area in self
            .subplots
            .iter()
            .map(|subplot| subplot.config.chart_area.0)
        {
            let x0 = union.x.min(area.x);
            let y0 = union.y.min(area.y);
            let x1 = (union.x + union.width).max(area.x + area.width);
            let y1 = (union.y + union.height).max(area.y + area.height);
            union = Rect {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            };
        }
        union
    }

    /// Move the parent's `chart_area` and every subplot's by `(dx, dy)`
    /// together, keeping their layout. Coordinates saturate at zero; a
    /// document placed by its [`Self::document_area`] never needs to.
    pub fn translate_areas(&mut self, dx: i64, dy: i64) {
        let shift = |rect: &mut Rect| {
            rect.x = (i64::from(rect.x) + dx).clamp(0, i64::from(u32::MAX)) as u32;
            rect.y = (i64::from(rect.y) + dy).clamp(0, i64::from(u32::MAX)) as u32;
        };
        shift(&mut self.chart_area.0);
        for subplot in &mut self.subplots {
            shift(&mut subplot.config.chart_area.0);
        }
    }

    /// Series ids may belong to at most one overlay. Ids with no matching
    /// series declaration are allowed (and ignored), so a document can be
    /// edited before its series are registered.
    pub fn validate_overlays(&self) -> Result<(), crate::RuleKind> {
        for (index, overlay) in self.overlays.iter().enumerate() {
            for (position, id) in overlay.series.iter().enumerate() {
                if overlay.series[..position].contains(id)
                    || self.overlays[..index].iter().any(|o| o.owns_series(id))
                {
                    return Err(crate::RuleKind::SeriesInManyOverlays);
                }
            }
        }
        Ok(())
    }

    /// Expand the document into its internal panels. See the module
    /// documentation for the rules. Fails when the layout has no data area,
    /// when overlays share a series id or when the line offsets break
    /// [`Self::validate_line_offset_layout`].
    pub fn expand(&self) -> Result<Expansion, LayoutError> {
        self.validate_overlays()
            .map_err(|_| LayoutError::Infeasible)?;
        self.validate_subplots()
            .map_err(|_| LayoutError::Infeasible)?;
        self.validate_line_offset_layout()
            .map_err(|_| LayoutError::Infeasible)?;
        let data_area = self.data_area()?;
        let group = self.margins();
        let members = self.offset_members();
        let mut panels = Vec::with_capacity(1 + self.overlays.len() + members.len());
        panels.push(ExpandedPanel {
            layer: LayerRef::Base,
            config: self.base_panel(&group, &data_area)?,
        });
        for index in 0..self.overlays.len() {
            panels.push(ExpandedPanel {
                layer: LayerRef::Overlay(index),
                config: self.overlay_panel_with(index, &group, &data_area)?,
            });
        }
        for (k, id) in members.iter().enumerate() {
            let layer = self.member_owner(id);
            let owner = match layer {
                LayerRef::Overlay(index) => 1 + index,
                _ => 0,
            };
            let config =
                self.member_panel_with(&panels[owner].config, id, k, &group, &data_area)?;
            panels.push(ExpandedPanel { layer, config });
        }
        for index in 0..self.subplots.len() {
            let mut subplot = self.subplot_config(index).ok_or(LayoutError::Infeasible)?;
            self.lend_subplot_picks(&mut subplot, index);
            for panel in subplot.expand()?.panels {
                panels.push(ExpandedPanel {
                    layer: match panel.layer {
                        LayerRef::Overlay(overlay) => LayerRef::SubplotOverlay(index, overlay),
                        _ => LayerRef::Subplot(index),
                    },
                    config: panel.config,
                });
            }
        }
        Ok(Expansion { panels, data_area })
    }

    /// The layer of every panel [`Self::expand`] returns, in the same order,
    /// computed without expanding. A line offset member's panel reports the
    /// layer that scales its member, so a layer appears once more per member
    /// it scales, after its own panel.
    pub fn panel_layers(&self) -> Vec<LayerRef> {
        let mut layers = vec![LayerRef::Base];
        layers.extend((0..self.overlays.len()).map(LayerRef::Overlay));
        layers.extend(self.offset_members().iter().map(|id| self.member_owner(id)));
        for (index, subplot) in self.subplots.iter().enumerate() {
            let chart = &subplot.config;
            layers.push(LayerRef::Subplot(index));
            layers.extend(
                (0..chart.overlays.len()).map(|overlay| LayerRef::SubplotOverlay(index, overlay)),
            );
            layers.extend(
                chart
                    .offset_members()
                    .iter()
                    .map(|id| match chart.member_owner(id) {
                        LayerRef::Overlay(overlay) => LayerRef::SubplotOverlay(index, overlay),
                        _ => LayerRef::Subplot(index),
                    }),
            );
        }
        layers
    }

    /// Whether this chart's own line offset group has a member, so it draws
    /// as more than one panel.
    pub fn has_offset_members(&self) -> bool {
        !self.offset_members().is_empty()
    }

    /// The member series ids of this chart's own line offset group, in
    /// stacking order; empty without one. A subplot's group is its own.
    fn offset_members(&self) -> &[String] {
        self.line_offsets
            .as_ref()
            .map_or(&[][..], |offsets| offsets.series.as_slice())
    }

    /// The layer of this chart that scales member `series_id`: an overlay
    /// that lists it, else the base layer.
    fn member_owner(&self, series_id: &str) -> LayerRef {
        match self.layer_of_series(series_id) {
            LayerRef::Overlay(index) => LayerRef::Overlay(index),
            _ => LayerRef::Base,
        }
    }

    /// How many panels this chart draws as, without its subplots: its base
    /// panel, one per overlay and one per line offset member.
    fn own_panel_count(&self) -> usize {
        1 + self.overlays.len() + self.offset_members().len()
    }

    /// The line offset member that panel `index` (in [`Self::panel_layers`]
    /// order) draws, `None` for a base, overlay or subplot panel and for an
    /// index past the end.
    pub fn panel_member(&self, index: usize) -> Option<&str> {
        let charts = std::iter::once(self).chain(self.subplots.iter().map(|s| &s.config));
        let mut start = 0;
        for chart in charts {
            let count = chart.own_panel_count();
            if index < start + count {
                let first = start + 1 + chart.overlays.len();
                return index
                    .checked_sub(first)
                    .and_then(|k| chart.offset_members().get(k))
                    .map(String::as_str);
            }
            start += count;
        }
        None
    }

    /// The panel that draws `series_id`, as an index in
    /// [`Self::panel_layers`] order: a line offset member's own panel, else
    /// the panel of [`Self::layer_of_series`].
    pub fn panel_of_series(&self, series_id: &str) -> usize {
        let (start, chart, layer) = match self.layer_of_series(series_id) {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                let start = self.own_panel_count()
                    + self.subplots[..index]
                        .iter()
                        .map(|subplot| subplot.config.own_panel_count())
                        .sum::<usize>();
                let chart = &self.subplots[index].config;
                (start, chart, chart.layer_of_series(series_id))
            }
            layer => (0, self, layer),
        };
        if let Some(k) = chart
            .line_offsets
            .as_ref()
            .and_then(|offsets| offsets.member(series_id))
        {
            return start + 1 + chart.overlays.len() + k;
        }
        start
            + match layer {
                LayerRef::Overlay(index) => 1 + index,
                _ => 0,
            }
    }

    /// The internal panel of one overlay layer, or `None` when `index` is out
    /// of range or the layout has no data area. The hit map's overlay
    /// elements measure and drag against this panel.
    pub fn overlay_panel(&self, index: usize) -> Option<Config> {
        if index >= self.overlays.len() || self.validate_overlays().is_err() {
            return None;
        }
        let data_area = self.data_area().ok()?;
        self.overlay_panel_with(index, &self.margins(), &data_area)
            .ok()
    }

    fn base_panel(&self, group: &Margins, data_area: &DataArea) -> Result<Config, LayoutError> {
        let mut panel = self.clone();
        panel.overlays.clear();
        panel.subplots.clear();
        // The members are drawn by panels of their own.
        panel.line_offsets = None;
        panel.active_layer = LayerRef::Base;
        // The document's one annotation layer is drawn with the base panel.
        retain_layer_picks(&mut panel, self, LayerRef::Base);
        align_margins(&mut panel, group, data_area)?;
        Ok(panel)
    }

    fn overlay_panel_with(
        &self,
        index: usize,
        group: &Margins,
        data_area: &DataArea,
    ) -> Result<Config, LayoutError> {
        // Every field of the parent is named: a new one is a decision here,
        // carried into the overlay panel or left at its hidden default.
        let Config {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            chart_title,
            // Grid and legend are drawn once, by the base panel.
            grid: _,
            legend: _,
            picked_data,
            colorbar,
            draw_style,
            coordinates,
            overlays,
            // A panel is a single chart; expansion ignores the active layer
            // and the annotation layer is drawn with the base panel.
            subplots: _,
            active_layer: _,
            annotations: _,
            // An overlay's category bar series form a group of their own
            // with the parent's options.
            bar_groups,
            // The members an overlay scales are drawn by member panels of
            // their own; the overlay panel draws none of them.
            line_offsets: _,
            plot_background: _,
            background: _,
        } = self;
        let OverlayLayer {
            y_axis,
            link_x,
            x_axis,
            link_y,
            // Which series the panel draws is decided by `layer_of_series`.
            series: _,
            legend: overlay_legend,
        } = &overlays[index];
        // The hidden left axis carries the scale of the Y axis the overlay
        // follows (the data transform reads `left_y`) and the parent's left
        // band, so the left margin is unchanged.
        let followed_y = if *link_y { left_y } else { y_axis };
        let left = {
            let AxisOptions {
                scale: _,
                min: _,
                max: _,
                major_spacing: _,
                minor_count: _,
                inverted: _,
                label_style: _,
                tick: _,
                title_option: _,
                out_margin,
                line_offset: _,
                line_visible: _,
                line_color: _,
                line_width: _,
                line_style: _,
                major_tick_length,
                minor_tick_length: _,
                // The hidden left axis maps the overlay's range.
                categories: _,
                category_groups: _,
                category_window: _,
                // The hidden left axis draws no line.
                cross_at: _,
                // The hidden left axis draws no ticks.
                log_ticks: _,
                // The hidden left axis maps the overlay's range, cut by the
                // overlay's own breaks (`hidden_axis` below).
                breaks: _,
                // The hidden left axis prints no labels.
                unit: _,
            } = left_y;
            let mut left = hidden_axis(followed_y, crate::defaults::axis_options_y());
            left.out_margin = *out_margin;
            left.major_tick_length = *major_tick_length;
            left
        };
        // The overlay's own Y axis takes the right side; a linked Y draws
        // none and keeps the parent's right band.
        let right = if *link_y {
            hidden_axis(right_y, crate::defaults::axis_options_y())
        } else {
            y_axis.clone()
        };
        // The X axes: the parent's, hidden (the base panel draws them), or
        // the overlay's own, drawn on the top side, with a hidden copy on the
        // bottom for the transform (which reads `bottom_x`) carrying the
        // parent's bottom band, so the bottom margin is unchanged.
        let (top, bottom) = if *link_x {
            (
                hidden_axis(top_x, crate::defaults::axis_options_x()),
                hidden_axis(bottom_x, crate::defaults::axis_options_x()),
            )
        } else {
            let mut bottom = hidden_axis(x_axis, crate::defaults::axis_options_x());
            bottom.out_margin = bottom_x.out_margin;
            bottom.major_tick_length = bottom_x.major_tick_length;
            (x_axis.clone(), bottom)
        };
        let grid = hidden_grid();
        // The title is hidden and keeps only the parent's band height.
        let title = hidden_title(chart_title);
        let legend = match overlay_legend {
            Some(legend) => legend.clone(),
            None => {
                let mut legend = crate::defaults::legend();
                legend.visible = false;
                legend
            }
        };
        // The colourbar is drawn once, by the base panel; the overlay panel
        // keeps only the z mapping its field series read.
        let colorbar = colorbar.as_ref().map(hidden_colorbar);
        let mut panel = Config {
            chart_area: chart_area.clone(),
            top_x: top,
            bottom_x: bottom,
            left_y: left,
            right_y: right,
            chart_title: title,
            grid,
            legend,
            picked_data: picked_data.clone(),
            colorbar,
            draw_style: *draw_style,
            coordinates: coordinates.clone(),
            overlays: Vec::new(),
            subplots: Vec::new(),
            active_layer: LayerRef::Base,
            annotations: AnnotationLayer::empty(),
            bar_groups: bar_groups.clone(),
            line_offsets: None,
            plot_background: None,
            background: None,
        };
        retain_layer_picks(&mut panel, self, LayerRef::Overlay(index));
        align_margins(&mut panel, group, data_area)?;
        Ok(panel)
    }

    /// The panel of line offset member `k`, `series_id`: `owner` (the panel
    /// of the layer that scales it) with the transform's axes moved by the
    /// member's shift and everything but its data hidden.
    fn member_panel_with(
        &self,
        owner: &Config,
        series_id: &str,
        k: usize,
        group: &Margins,
        data_area: &DataArea,
    ) -> Result<Config, LayoutError> {
        // Every field of the owner panel is named: a new one is a decision
        // here, carried into the member panel or left at its hidden default.
        let Config {
            chart_area,
            top_x,
            bottom_x,
            left_y,
            right_y,
            chart_title,
            // Grid and legend are drawn by the owner's chain of panels.
            grid: _,
            legend: _,
            // The member's references are taken from the document below.
            picked_data: _,
            colorbar,
            draw_style,
            coordinates,
            // A panel is a single chart without layers of its own.
            overlays: _,
            subplots: _,
            active_layer: _,
            annotations: _,
            bar_groups,
            line_offsets: _,
            plot_background: _,
            background: _,
        } = owner;
        let [dx, dy] = self
            .line_offsets
            .as_ref()
            .map_or([0.0, 0.0], |offsets| offsets.shift(k));
        let hidden = |axis: &AxisOptions, template: AxisOptions, by: f64| {
            let mut axis = hidden_axis(axis, template);
            shift_axis(&mut axis, by);
            axis.out_margin = 0.0;
            axis
        };
        let mut legend = crate::defaults::legend();
        legend.visible = false;
        let mut panel = Config {
            chart_area: chart_area.clone(),
            top_x: hidden(top_x, crate::defaults::axis_options_x(), 0.0),
            // The transform reads `bottom_x` and `left_y`.
            bottom_x: hidden(bottom_x, crate::defaults::axis_options_x(), dx),
            left_y: hidden(left_y, crate::defaults::axis_options_y(), dy),
            right_y: hidden(right_y, crate::defaults::axis_options_y(), 0.0),
            chart_title: hidden_title(chart_title),
            grid: hidden_grid(),
            legend,
            picked_data: self.picked_data.clone(),
            colorbar: colorbar.as_ref().map(hidden_colorbar),
            draw_style: *draw_style,
            coordinates: coordinates.clone(),
            overlays: Vec::new(),
            subplots: Vec::new(),
            active_layer: LayerRef::Base,
            annotations: AnnotationLayer::empty(),
            bar_groups: bar_groups.clone(),
            line_offsets: None,
            plot_background: None,
            background: None,
        };
        retain_picks(&mut panel, |id| id == series_id);
        align_margins(&mut panel, group, data_area)?;
        Ok(panel)
    }
}

/// Move `axis`' range by `by`: `by` data units on a linear axis, `by`
/// decades on a logarithmic one and `by` units of `F⁻¹(p)` on a probability
/// one (#209), so a value `v` lands where `v + by` (or `v · 10^by`, or
/// `F(F⁻¹(v) + by)`) did. A categorical axis becomes the numeric axis of its
/// range. Zero leaves the axis as it is.
fn shift_axis(axis: &mut AxisOptions, by: f64) {
    if by == 0.0 {
        return;
    }
    let (min, max) = axis.data_range();
    axis.categories = None;
    // The cut ranges move with the range, so the member's gaps fall where
    // its owner's do (#200).
    let cuts = axis
        .breaks
        .as_mut()
        .map_or(&mut [][..], |breaks| breaks.ranges.as_mut_slice());
    match axis.scale {
        AxisScale::Linear => {
            axis.min = min - by;
            axis.max = max - by;
            for cut in cuts {
                cut.from -= by;
                cut.to -= by;
            }
        }
        AxisScale::Logarithmic => {
            let factor = 10f64.powf(by);
            axis.min = min / factor;
            axis.max = max / factor;
            for cut in cuts {
                cut.from /= factor;
                cut.to /= factor;
            }
        }
        AxisScale::Probability(scale) => {
            // In transformed units; a bound past the f64 tails keeps its
            // value, which validation then reports.
            let shift = |p: f64| {
                scale
                    .forward(p)
                    .and_then(|y| scale.inverse(y - by))
                    .unwrap_or(p)
            };
            axis.min = shift(min);
            axis.max = shift(max);
            for cut in cuts {
                cut.from = shift(cut.from);
                cut.to = shift(cut.to);
            }
        }
    }
}

/// A grid that draws nothing.
fn hidden_grid() -> crate::config::GridOptions {
    let mut grid = crate::defaults::grid_options();
    grid.show_major_x = false;
    grid.show_major_y = false;
    grid.show_minor_x = false;
    grid.show_minor_y = false;
    grid
}

/// A hidden title that keeps only `title`'s band height.
fn hidden_title(title: &ChartTitleOptions) -> ChartTitleOptions {
    let ChartTitleOptions {
        text: _,
        visible: _,
        offset_x: _,
        offset_y: _,
        top_margin,
    } = title;
    let mut hidden = crate::defaults::chart_title_options();
    hidden.visible = false;
    hidden.top_margin = *top_margin;
    hidden
}

/// A hidden colourbar that keeps only `bar`'s z mapping, which a panel's
/// field and colour-mapped series read.
fn hidden_colorbar(bar: &ColorBarOptions) -> ColorBarOptions {
    let ColorBarOptions {
        visible: _,
        side: _,
        thickness_px: _,
        gap_px: _,
        length_frac: _,
        align: _,
        offset_x: _,
        offset_y: _,
        colormap,
        nan_color,
        border_color: _,
        border_width: _,
        axis,
    } = bar;
    let AxisOptions {
        scale,
        min,
        max,
        major_spacing: _,
        minor_count: _,
        // The ramp does not depend on the direction of the z axis.
        inverted: _,
        label_style: _,
        tick: _,
        title_option: _,
        out_margin: _,
        line_offset: _,
        line_visible: _,
        line_color: _,
        line_width: _,
        line_style: _,
        major_tick_length: _,
        minor_tick_length: _,
        // A colourbar axis is never categorical.
        categories: _,
        category_groups: _,
        category_window: _,
        // The hidden colourbar draws no axis line.
        cross_at: _,
        // The hidden colourbar draws no ticks.
        log_ticks: _,
        // A colourbar axis has no breaks (validation refuses them).
        breaks: _,
        // The hidden colourbar prints no labels.
        unit: _,
    } = axis;
    let mut hidden = crate::defaults::color_bar_options();
    hidden.visible = false;
    hidden.axis.scale = scale.clone();
    hidden.axis.min = *min;
    hidden.axis.max = *max;
    hidden.colormap = colormap.clone();
    hidden.nan_color = *nan_color;
    hidden
}

/// The smallest factor a subplot's text is drawn with
/// ([`subplot_text_factor`]).
pub const SUBPLOT_TEXT_SCALE_MIN: f32 = 0.5;

/// The text factor of a cell whose smaller side is the share `r` of the
/// parent chart's (#292): the cube root of `r`, held to
/// [`SUBPLOT_TEXT_SCALE_MIN`]`..=1`.
///
/// Text shrinks more slowly than its cell: a cell half the parent's size
/// draws its text at 0.79, a quarter at 0.63, and an eighth or less at 0.5.
/// A cell as large as the parent or larger draws at `1.0`, as does an `r`
/// that is not a number.
pub fn subplot_text_factor(r: f32) -> f32 {
    if r.is_nan() {
        return 1.0;
    }
    r.cbrt().clamp(SUBPLOT_TEXT_SCALE_MIN, 1.0)
}

/// The linked part of an axis: range, scale, direction and tick spacing
/// (`major_spacing`, `minor_count`, `log_ticks`), so linked charts place the
/// same ticks. Line, label and title style stay each chart's own.
fn copy_axis_range(from: &AxisOptions, to: &mut AxisOptions) {
    let AxisOptions {
        scale,
        min,
        max,
        major_spacing,
        minor_count,
        inverted,
        // Style and band: each chart's own.
        label_style: _,
        tick: _,
        title_option: _,
        out_margin: _,
        line_offset: _,
        line_visible: _,
        line_color: _,
        line_width: _,
        line_style: _,
        major_tick_length: _,
        minor_tick_length: _,
        // The categories are the range of a categorical axis, and their
        // group headers name runs of them (#203).
        categories,
        category_groups,
        category_window,
        // Where the line crosses: each chart's own, like the line style.
        cross_at: _,
        // The log tick ladder is tick spacing too.
        log_ticks,
        // The cut ranges are part of the mapping, so a linked axis maps
        // exactly like its target.
        breaks,
        // The spacing is read in shown units (#273), so the unit travels
        // with it and the linked axes print the same ticks.
        unit,
    } = from;
    to.scale = scale.clone();
    to.min = *min;
    to.max = *max;
    to.inverted = *inverted;
    to.major_spacing = *major_spacing;
    to.minor_count = *minor_count;
    to.log_ticks = *log_ticks;
    to.categories.clone_from(categories);
    to.category_groups.clone_from(category_groups);
    to.category_window = *category_window;
    to.breaks.clone_from(breaks);
    to.unit = *unit;
}

/// Whether an axis keeps its range, scale and direction: what decides which
/// member of a link group was edited. Tick spacing is not compared, because
/// a range writer such as a view state re-derives it from an unchanged range;
/// it travels with the edited range instead.
fn same_axis_range(a: &AxisOptions, b: &AxisOptions) -> bool {
    type Range<'a> = (
        AxisScale,
        f64,
        f64,
        bool,
        Option<&'a [Category]>,
        Option<[f64; 2]>,
        Option<&'a AxisBreaks>,
    );
    fn range(axis: &AxisOptions) -> Range<'_> {
        let AxisOptions {
            scale,
            min,
            max,
            // Tick spacing travels with the edited range (see above).
            major_spacing: _,
            minor_count: _,
            inverted,
            // Style and band: not linked.
            label_style: _,
            tick: _,
            title_option: _,
            out_margin: _,
            line_offset: _,
            line_visible: _,
            line_color: _,
            line_width: _,
            line_style: _,
            major_tick_length: _,
            minor_tick_length: _,
            categories,
            category_groups: _,
            category_window,
            // Not linked.
            cross_at: _,
            // Tick spacing travels with the edited range (see above).
            log_ticks: _,
            // The cut ranges are part of the mapping.
            breaks,
            // Printing only; it travels with the spacing (see above).
            unit: _,
        } = axis;
        (
            scale.clone(),
            *min,
            *max,
            *inverted,
            categories.as_deref(),
            *category_window,
            breaks.as_ref(),
        )
    }
    range(a) == range(b)
}

/// A hidden axis: `template` (canonical defaults) carrying only `axis`'s
/// transform fields (scale, range, direction) and margin fields (outer margin,
/// major tick length), with its line, ticks, labels and title off.
fn hidden_axis(axis: &AxisOptions, template: AxisOptions) -> AxisOptions {
    let AxisOptions {
        scale,
        // Through `data_range` below.
        min: _,
        max: _,
        // A hidden axis draws no ticks, so its spacing is the template's.
        major_spacing: _,
        minor_count: _,
        inverted,
        // Style: the template's, with line, ticks, labels and title off.
        label_style: _,
        tick: _,
        title_option: _,
        out_margin,
        line_offset: _,
        line_visible: _,
        line_color: _,
        line_width: _,
        line_style: _,
        major_tick_length,
        minor_tick_length: _,
        // A hidden axis draws no labels, but a categorical axis keeps its
        // categories: the panel's category bars resolve their per-bar
        // overrides by category id against them. It draws no group headers.
        categories,
        category_groups: _,
        category_window,
        // A hidden axis draws no line.
        cross_at: _,
        // A hidden axis draws no ticks, so its tick policy is the template's.
        log_ticks: _,
        // The cut ranges are part of the transform the hidden axis carries.
        breaks,
        // A hidden axis prints no labels.
        unit: _,
    } = axis;
    let AxisOptions {
        scale: _,
        min: _,
        max: _,
        major_spacing,
        minor_count,
        inverted: _,
        label_style,
        tick: _,
        title_option,
        out_margin: _,
        line_offset,
        line_visible: _,
        line_color,
        line_width,
        line_style,
        major_tick_length: _,
        minor_tick_length,
        categories: _,
        category_groups: _,
        category_window: _,
        cross_at: _,
        log_ticks,
        breaks: _,
        unit: _,
    } = template;
    let (min, max) = axis.data_range();
    AxisOptions {
        scale: scale.clone(),
        min,
        max,
        major_spacing,
        minor_count,
        inverted: *inverted,
        label_style,
        tick: TickVisibility::None,
        title_option,
        out_margin: *out_margin,
        line_offset,
        line_visible: false,
        line_color,
        line_width,
        line_style,
        major_tick_length: *major_tick_length,
        minor_tick_length,
        categories: categories.clone(),
        category_groups: None,
        category_window: *category_window,
        cross_at: None,
        log_ticks,
        breaks: breaks.clone(),
        unit: None,
    }
    .with_hidden_text()
}

trait HiddenText {
    fn with_hidden_text(self) -> Self;
}

impl HiddenText for AxisOptions {
    fn with_hidden_text(mut self) -> Self {
        self.label_style.visible = false;
        self.label_style.label_visible = false;
        self.title_option.visible = false;
        self
    }
}

impl Config {
    /// Hand subplot `index` the document's picked references of the series
    /// it draws, so its panels highlight them; its own expansion then splits
    /// them between its base and overlay panels. A document without a
    /// selection of a kind leaves the subplot's own.
    fn lend_subplot_picks(&self, subplot: &mut Config, index: usize) {
        // Only the two selections are lent; every other field is the
        // subplot's own.
        let Config {
            chart_area: _,
            top_x: _,
            bottom_x: _,
            left_y: _,
            right_y: _,
            chart_title: _,
            grid: _,
            legend: _,
            picked_data,
            colorbar: _,
            draw_style: _,
            coordinates: _,
            overlays: _,
            subplots: _,
            active_layer: _,
            annotations: _,
            bar_groups: _,
            line_offsets: _,
            plot_background: _,
            background: _,
        } = self;
        let owned = |id: &str| {
            matches!(
                self.layer_of_series(id),
                LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) if i == index
            )
        };
        if let Some(picked) = picked_data {
            let mut picked = picked.clone();
            picked.refs.retain(|reference| owned(reference.series_id()));
            subplot.picked_data = Some(picked);
        }
    }
}

/// Keep only the picked references of the series `layer`'s own panel draws
/// (not its line offset members, whose panels draw theirs); the other
/// layers' selections are drawn by their own panels.
fn retain_layer_picks(panel: &mut Config, parent: &Config, layer: LayerRef) {
    let member = |id: &str| {
        parent
            .line_offsets
            .as_ref()
            .is_some_and(|offsets| offsets.member(id).is_some())
    };
    retain_picks(panel, |id| {
        parent.layer_of_series(id) == layer && !member(id)
    });
}

/// Keep only the picked references whose series `keep` accepts.
fn retain_picks(panel: &mut Config, keep: impl Fn(&str) -> bool) {
    // Only the selection is filtered by layer; every other field is
    // the panel's as built.
    let Config {
        chart_area: _,
        top_x: _,
        bottom_x: _,
        left_y: _,
        right_y: _,
        chart_title: _,
        grid: _,
        legend: _,
        picked_data,
        colorbar: _,
        draw_style: _,
        coordinates: _,
        overlays: _,
        subplots: _,
        active_layer: _,
        annotations: _,
        bar_groups: _,
        line_offsets: _,
        plot_background: _,
        background: _,
    } = panel;
    if let Some(picked) = picked_data.as_mut() {
        picked.refs.retain(|reference| keep(reference.series_id()));
    }
}

/// Lift `panel`'s margins to the group's so its `data_area()` is exactly
/// `data_area`. A side whose total already equals the group's is left alone,
/// so a document without overlays expands to a byte-identical base panel.
fn align_margins(
    panel: &mut Config,
    group: &Margins,
    data_area: &DataArea,
) -> Result<(), LayoutError> {
    let sides = [Side::Top, Side::Bottom, Side::Left, Side::Right];
    let total = |m: &Margins, side: &Side| match side {
        Side::Top => m.top,
        Side::Bottom => m.bottom,
        Side::Left => m.left,
        Side::Right => m.right,
    };
    let mut lifted = Vec::new();
    for side in &sides {
        let own = panel.margins();
        let delta = total(group, side) - total(&own, side);
        if delta != 0.0 {
            let axis = crate::layout::axis_options_mut(panel, side);
            axis.out_margin += delta;
            lifted.push(side.clone());
        }
    }
    if panel.data_area()? == *data_area {
        return Ok(());
    }
    // Floating-point rounding of a lifted side can move a floored edge by one
    // pixel. Nudge the lifted margins by a few ulps until the rectangles agree.
    for side in &lifted {
        let original = crate::layout::axis_options_mut(panel, side).out_margin;
        for step in [1_i32, 2, 3, -1, -2, -3] {
            let mut value = original;
            for _ in 0..step.unsigned_abs() {
                value = if step > 0 {
                    value.next_up()
                } else {
                    value.next_down()
                };
            }
            crate::layout::axis_options_mut(panel, side).out_margin = value;
            if panel.data_area()? == *data_area {
                return Ok(());
            }
        }
        crate::layout::axis_options_mut(panel, side).out_margin = original;
    }
    Err(LayoutError::Infeasible)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probability axis shifts in units of `F⁻¹(p)` (#209): every value,
    /// cut ends included, lands where `F(F⁻¹(v) − by)` is.
    #[test]
    fn a_probability_axis_shifts_in_transformed_units() {
        let scale = crate::config::ProbabilityScale::Normal;
        let mut axis = crate::defaults::config().left_y;
        axis.scale = AxisScale::Probability(scale);
        axis.min = 0.01;
        axis.max = 0.99;
        axis.breaks = Some(crate::config::AxisBreaks {
            ranges: vec![crate::config::AxisBreak::new(0.4, 0.6)],
            ..crate::defaults::axis_breaks()
        });
        shift_axis(&mut axis, 0.5);
        let moved = |p: f64| scale.inverse(scale.forward(p).unwrap() - 0.5).unwrap();
        assert!((axis.min - moved(0.01)).abs() < 1e-15, "{}", axis.min);
        assert!((axis.max - moved(0.99)).abs() < 1e-15, "{}", axis.max);
        let cut = axis.breaks.as_ref().unwrap().ranges[0];
        assert!((cut.from - moved(0.4)).abs() < 1e-15 && (cut.to - moved(0.6)).abs() < 1e-15);
    }
    use crate::layer::OverlayLayer;
    use crate::layout::ChartArea;

    fn two_layer_config() -> Config {
        let mut cfg = crate::defaults::config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        let mut overlay = crate::defaults::overlay_layer();
        overlay.y_axis.min = -5.0;
        overlay.y_axis.max = 5.0;
        overlay.y_axis.out_margin = 90.0;
        overlay.series = vec!["pressure".into()];
        cfg.overlays.push(overlay);
        cfg
    }

    #[test]
    fn active_layer_falls_back_to_base_when_out_of_range() {
        let mut cfg = two_layer_config();
        assert_eq!(
            cfg.set_active_layer(LayerRef::Overlay(0)),
            LayerRef::Overlay(0)
        );
        assert_eq!(cfg.active_layer(), LayerRef::Overlay(0));
        assert_eq!(cfg.set_active_layer(LayerRef::Overlay(1)), LayerRef::Base);
        assert_eq!(cfg.active_layer, LayerRef::Base);
        cfg.active_layer = LayerRef::Overlay(7);
        assert_eq!(cfg.active_layer(), LayerRef::Base);
        assert_eq!(
            cfg.set_active_layer(LayerRef::Annotations),
            LayerRef::Annotations
        );
        // Deleting the active overlay recovers to Base on read.
        cfg.set_active_layer(LayerRef::Overlay(0));
        cfg.overlays.clear();
        assert_eq!(cfg.active_layer(), LayerRef::Base);
    }

    #[test]
    fn series_membership_is_looked_up_by_id() {
        let cfg = two_layer_config();
        assert_eq!(cfg.layer_of_series("pressure"), LayerRef::Overlay(0));
        assert_eq!(cfg.layer_of_series("temperature"), LayerRef::Base);
        let mut shared = cfg.clone();
        shared.overlays.push(OverlayLayer {
            series: vec!["pressure".into()],
            ..crate::defaults::overlay_layer()
        });
        assert!(shared.validate_overlays().is_err());
        assert!(shared.expand().is_err());
        let mut twice = cfg.clone();
        twice.overlays[0].series.push("pressure".into());
        assert!(twice.validate_overlays().is_err());
    }

    // A document without overlays expands to exactly itself: no margin lift,
    // no hidden axis, so every pixel of an existing chart is unchanged.
    #[test]
    fn a_single_layer_document_expands_to_itself() {
        let mut cfg = crate::defaults::config();
        cfg.active_layer = LayerRef::Annotations;
        cfg.annotations.visible = false;
        let expansion = cfg.expand().unwrap();
        assert_eq!(expansion.panels.len(), 1);
        let mut expected = cfg.clone();
        expected.active_layer = LayerRef::Base;
        assert_eq!(expansion.panels[0].config, expected);
        assert_eq!(expansion.panels[0].layer, LayerRef::Base);
        assert_eq!(expansion.data_area, cfg.data_area().unwrap());
    }

    // Every panel shares one data area, at every export scale.
    #[test]
    fn panels_agree_on_the_group_data_area() {
        let cfg = two_layer_config();
        for scale in [1.0_f32, 2.0, 0.5, 1.37] {
            let scaled = cfg.scaled(scale);
            let expansion = scaled.expand().unwrap();
            assert_eq!(expansion.panels.len(), 2);
            assert_eq!(expansion.data_area, scaled.data_area().unwrap());
            for panel in &expansion.panels {
                assert_eq!(
                    panel.config.data_area().unwrap(),
                    expansion.data_area,
                    "scale {scale}, layer {:?}",
                    panel.layer
                );
                assert!(panel.config.overlays.is_empty());
                assert_eq!(panel.config.active_layer, LayerRef::Base);
                assert_eq!(panel.config.chart_area, scaled.chart_area);
            }
        }
    }

    #[test]
    fn overlay_panels_keep_the_x_range_and_own_the_right_axis() {
        let mut cfg = two_layer_config();
        cfg.legend.visible = true;
        cfg.chart_title.visible = true;
        let expansion = cfg.expand().unwrap();
        let base = &expansion.panels[0].config;
        let overlay = &expansion.panels[1].config;
        assert_eq!(expansion.panels[1].layer, LayerRef::Overlay(0));
        // Shared decorations are drawn by the base panel only.
        assert!(base.chart_title.visible && !overlay.chart_title.visible);
        assert!(base.legend.visible && !overlay.legend.visible);
        assert!(base.grid.show_major_x && !overlay.grid.show_major_x);
        assert!(!overlay.grid.show_major_y && !overlay.grid.show_minor_x);
        for axis in [&overlay.top_x, &overlay.bottom_x, &overlay.left_y] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
            assert!(!axis.label_style.label_visible);
            assert!(!axis.title_option.visible);
        }
        // X range (the transform) is the parent's; Y range is the overlay's.
        assert_eq!(overlay.bottom_x.min, cfg.bottom_x.min);
        assert_eq!(overlay.bottom_x.max, cfg.bottom_x.max);
        assert_eq!(overlay.left_y.min, -5.0);
        assert_eq!(overlay.left_y.max, 5.0);
        assert_eq!(overlay.right_y.min, -5.0);
        assert!(overlay.right_y.line_visible);
        assert!(overlay.right_y.label_style.label_visible);
        // The group right band is the overlay's wider band; the base panel's
        // right axis was lifted to it.
        assert_eq!(cfg.margins().right, 95.0);
        assert_eq!(base.margins().right, 95.0);
        assert_eq!(overlay.margins().right, 95.0);
        assert_eq!(base.right_y.out_margin, 90.0);
        assert_eq!(
            expansion.panel(LayerRef::Overlay(0)),
            Some(&expansion.panels[1])
        );
        assert!(expansion.panel(LayerRef::Overlay(1)).is_none());
        assert!(expansion.panel(LayerRef::Annotations).is_none());
    }

    // With `link_x` off the overlay panel draws the overlay's own X axis on
    // the top side and scales by it (the hidden `bottom_x` the transform
    // reads), keeping the parent's bottom band; its band widens the shared
    // top band. With `link_y` on the panel scales by the parent's `left_y`
    // and draws no Y axis, and the overlay adds no right band.
    #[test]
    fn overlay_links_choose_the_axes_the_panel_scales_by_and_draws() {
        let mut cfg = two_layer_config();
        let parent = cfg.expand().unwrap().panels[0].config.clone();
        let mut own_x = crate::defaults::axis_options_x();
        own_x.min = 100.0;
        own_x.max = 400.0;
        own_x.out_margin = cfg.top_x.out_margin + 30.0;
        cfg.overlays[0].link_x = false;
        cfg.overlays[0].x_axis = own_x.clone();
        let expansion = cfg.expand().unwrap();
        let overlay = &expansion.panels[1].config;
        assert_eq!((overlay.bottom_x.min, overlay.bottom_x.max), (100.0, 400.0));
        assert!(!overlay.bottom_x.line_visible);
        assert_eq!(overlay.bottom_x.out_margin, cfg.bottom_x.out_margin);
        assert_eq!(overlay.top_x, own_x);
        // The base panel keeps the parent's X; the top band is the group's.
        let base = &expansion.panels[0].config;
        assert_eq!(
            (base.bottom_x.min, base.bottom_x.max),
            (cfg.bottom_x.min, cfg.bottom_x.max)
        );
        assert_eq!(cfg.margins().top, parent.margins().top + 30.0);
        assert_eq!(base.margins().top, cfg.margins().top);
        assert_eq!(overlay.margins().top, cfg.margins().top);
        assert_eq!(base.data_area().unwrap(), overlay.data_area().unwrap());
        // Linked back, the panel is the linked one again and the stored axis
        // stays.
        let mut linked = cfg.clone();
        linked.overlays[0].link_x = true;
        let panel = &linked.expand().unwrap().panels[1].config;
        assert_eq!(
            (panel.bottom_x.min, panel.bottom_x.max),
            (cfg.bottom_x.min, cfg.bottom_x.max)
        );
        assert!(!panel.top_x.line_visible);
        assert_eq!(linked.overlays[0].x_axis, own_x);
        assert_eq!(linked.margins().top, parent.margins().top);

        // Y linked: the parent's left range scales the panel, no Y axis is
        // drawn and the overlay's wider band no longer counts.
        let mut cfg = two_layer_config();
        cfg.left_y.min = -50.0;
        cfg.left_y.max = 70.0;
        let own_band = cfg.margins().right;
        cfg.overlays[0].link_y = true;
        let expansion = cfg.expand().unwrap();
        let overlay = &expansion.panels[1].config;
        assert_eq!((overlay.left_y.min, overlay.left_y.max), (-50.0, 70.0));
        assert!(!overlay.right_y.line_visible);
        assert!(!overlay.right_y.label_style.label_visible);
        assert!(cfg.margins().right < own_band);
        assert_eq!(
            expansion.panels[0].config.data_area().unwrap(),
            overlay.data_area().unwrap()
        );
    }

    // A panel's config depends only on what that panel draws: a base-layer
    // edit of hidden chrome leaves every overlay panel's config identical, an
    // overlay edit leaves the base panel identical, and each panel keeps the
    // picked references of its own series only.
    #[test]
    fn a_layer_edit_changes_only_that_layers_panel() {
        let mut cfg = two_layer_config();
        cfg.overlays.push(OverlayLayer {
            series: vec!["flow".into()],
            ..crate::defaults::overlay_layer()
        });
        cfg.chart_title.visible = true;
        cfg.legend.visible = true;
        cfg.colorbar = Some(crate::defaults::color_bar_options());
        let before = cfg.expand().unwrap();

        let mut base_edit = cfg.clone();
        base_edit.chart_title.text =
            crate::text::RichText::plain("Edited", crate::Color::BLACK, 20.0, "");
        base_edit.legend.content =
            crate::text::RichText::plain("Legend", crate::Color::BLACK, 12.0, "");
        base_edit.bottom_x.label_style.color = crate::Color::new(1.0, 0.0, 0.0, 1.0);
        base_edit.bottom_x.major_spacing = 0.5;
        base_edit.grid.major_x_color = crate::Color::new(0.0, 1.0, 0.0, 1.0);
        base_edit.colorbar.as_mut().unwrap().border_width = 4.0;
        base_edit.annotations.visible = false;
        base_edit.active_layer = LayerRef::Overlay(1);
        let after = base_edit.expand().unwrap();
        assert_ne!(after.panels[0].config, before.panels[0].config);
        assert_eq!(after.panels[1].config, before.panels[1].config);
        assert_eq!(after.panels[2].config, before.panels[2].config);

        let mut overlay_edit = cfg.clone();
        overlay_edit.overlays[0].y_axis.line_offset = 7.0;
        overlay_edit.overlays[0].legend = Some(crate::defaults::legend());
        let after = overlay_edit.expand().unwrap();
        assert_eq!(after.panels[0].config, before.panels[0].config);
        assert_ne!(after.panels[1].config, before.panels[1].config);
        assert_eq!(after.panels[2].config, before.panels[2].config);

        // The z mapping reaches the overlay panels (their field series read
        // it); the strip's own chrome does not.
        let mut z_edit = cfg.clone();
        z_edit.colorbar.as_mut().unwrap().axis.max = 9.0;
        let after = z_edit.expand().unwrap();
        assert_ne!(after.panels[1].config, before.panels[1].config);

        let mut picks = cfg.clone();
        picks.picked_data = Some(crate::config::DataSelectionsConfig {
            refs: vec![
                crate::config::PickedDataRef::Point {
                    source_id: None,
                    series_id: "temperature".into(),
                    point_index: 1,
                },
                crate::config::PickedDataRef::Point {
                    source_id: None,
                    series_id: "pressure".into(),
                    point_index: 2,
                },
            ],
            ..crate::defaults::data_selections_config()
        });
        let expansion = picks.expand().unwrap();
        let refs = |panel: &ExpandedPanel| {
            panel
                .config
                .picked_data
                .as_ref()
                .map(|picked| {
                    picked
                        .refs
                        .iter()
                        .map(|r| r.series_id().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        assert_eq!(refs(&expansion.panels[0]), vec!["temperature".to_string()]);
        assert_eq!(refs(&expansion.panels[1]), vec!["pressure".to_string()]);
        assert!(refs(&expansion.panels[2]).is_empty());
        // A pick of an overlay series leaves the base panel's config alone.
        let mut base_only = cfg.clone();
        base_only.picked_data = Some(crate::config::DataSelectionsConfig {
            refs: vec![crate::config::PickedDataRef::Point {
                source_id: None,
                series_id: "pressure".into(),
                point_index: 2,
            }],
            ..crate::defaults::data_selections_config()
        });
        let base_only = base_only.expand().unwrap();
        assert_eq!(
            base_only.panels[0].config.picked_data,
            Some(crate::defaults::data_selections_config())
        );
        assert_eq!(
            base_only.panels[1].config.picked_data,
            picks.expand().unwrap().panels[1].config.picked_data
        );
    }

    #[test]
    fn overlay_panel_lookup_matches_expansion() {
        let cfg = two_layer_config();
        let expansion = cfg.expand().unwrap();
        assert_eq!(
            cfg.overlay_panel(0).as_ref(),
            Some(&expansion.panels[1].config)
        );
        assert!(cfg.overlay_panel(1).is_none());
    }

    #[test]
    fn a_per_layer_legend_is_drawn_by_its_panel() {
        let mut cfg = two_layer_config();
        let mut legend = crate::defaults::legend();
        legend.visible = true;
        legend.content = crate::text::RichText::plain("Pressure", crate::Color::BLACK, 12.0, "");
        cfg.overlays[0].legend = Some(legend.clone());
        let expansion = cfg.expand().unwrap();
        assert_eq!(expansion.panels[1].config.legend, legend);
        assert_eq!(expansion.panels[0].config.legend, cfg.legend);
    }

    #[test]
    fn a_hidden_colorbar_band_is_absorbed_by_the_overlay_margin() {
        let mut cfg = two_layer_config();
        cfg.colorbar = Some(crate::defaults::color_bar_options());
        for side in [Side::Right, Side::Left, Side::Top, Side::Bottom] {
            cfg.colorbar.as_mut().unwrap().side = side.clone();
            let expansion = cfg.expand().unwrap();
            for panel in &expansion.panels {
                assert_eq!(
                    panel.config.data_area().unwrap(),
                    expansion.data_area,
                    "{side:?}"
                );
            }
            assert!(
                !expansion.panels[1]
                    .config
                    .colorbar
                    .as_ref()
                    .unwrap()
                    .visible
            );
            assert!(
                expansion.panels[0]
                    .config
                    .colorbar
                    .as_ref()
                    .unwrap()
                    .visible
            );
        }
    }

    // Lifting is exact for the common integer-valued layouts and still agrees
    // on the floored rectangle for awkward fractional bands.
    #[test]
    fn fractional_bands_still_share_one_rectangle() {
        let mut cfg = two_layer_config();
        cfg.right_y.major_tick_length = 5.3;
        cfg.overlays[0].y_axis.major_tick_length = 7.1;
        cfg.overlays[0].y_axis.out_margin = 61.7;
        let mut tested = 0;
        for out in [1.1_f32, 33.3, 59.99, 60.0, 60.01, 72.4, 99.9] {
            cfg.right_y.out_margin = out;
            let expansion = cfg.expand().unwrap();
            for panel in &expansion.panels {
                assert_eq!(
                    panel.config.data_area().unwrap(),
                    expansion.data_area,
                    "{out}"
                );
            }
            tested += 1;
        }
        assert_eq!(tested, 7);
    }

    /// A 640x240 parent and two subplots below it: subplot 0 with its own
    /// axes and an overlay, subplot 1 linked to the parent's X and to
    /// subplot 0's Y.
    fn subplot_document() -> Config {
        let mut cfg = crate::defaults::config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 240,
        });
        cfg.bottom_x.min = 0.0;
        cfg.bottom_x.max = 24.0;
        let mut first = crate::defaults::subplot();
        first.config.chart_area = ChartArea(Rect {
            x: 0,
            y: 240,
            width: 320,
            height: 240,
        });
        first.config.bottom_x.min = -5.0;
        first.config.bottom_x.max = 5.0;
        first.config.left_y.min = 100.0;
        first.config.left_y.max = 200.0;
        first.series = vec!["flow".into()];
        first.config.overlays.push(OverlayLayer {
            series: vec!["level".into()],
            ..crate::defaults::overlay_layer()
        });
        let mut second = crate::defaults::subplot();
        second.config.chart_area = ChartArea(Rect {
            x: 320,
            y: 240,
            width: 320,
            height: 240,
        });
        second.series = vec!["rate".into()];
        second.link_x = Some(CellRef::Parent);
        second.link_y = Some(CellRef::Subplot(0));
        cfg.subplots = vec![first, second];
        cfg.sync_subplot_links(None);
        cfg
    }

    /// Every text size [`Config::scale_text_sizes`] scales, in one fixed
    /// order: the chart title's, each axis' tick labels, title, rich
    /// category labels and group headers, and each legend's.
    fn text_sizes(chart: &Config) -> Vec<f32> {
        fn rich(text: &crate::text::RichText, out: &mut Vec<f32>) {
            out.push(text.font_size);
            out.extend(text.segments.iter().filter_map(|s| s.font_size));
        }
        fn axis(axis: &AxisOptions, out: &mut Vec<f32>) {
            out.push(axis.label_style.font_size);
            rich(&axis.title_option.text, out);
            for category in axis.categories.iter().flatten() {
                if let Some(label) = &category.rich_label {
                    rich(label, out);
                }
            }
            for group in axis.category_groups.iter().flat_map(|g| &g.groups) {
                rich(&group.label, out);
            }
        }
        let mut out = Vec::new();
        rich(&chart.chart_title.text, &mut out);
        for a in [&chart.top_x, &chart.bottom_x, &chart.left_y, &chart.right_y] {
            axis(a, &mut out);
        }
        rich(&chart.legend.content, &mut out);
        for overlay in &chart.overlays {
            axis(&overlay.y_axis, &mut out);
            axis(&overlay.x_axis, &mut out);
            if let Some(legend) = &overlay.legend {
                rich(&legend.content, &mut out);
            }
        }
        if let Some(bar) = &chart.colorbar {
            axis(&bar.axis, &mut out);
        }
        out
    }

    /// An 800x600 parent and one subplot `width` x `height`, carrying
    /// every kind of text the factor covers: a title with a segment of its
    /// own size, rich category labels and group headers on `bottom_x`, an
    /// overlay with a legend of its own and a colourbar.
    fn text_document(width: u32, height: u32) -> Config {
        use crate::text::{RichText, rich_segments_from_text};
        let mut cfg = crate::defaults::config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        let mut sub = crate::defaults::subplot();
        let chart = &mut sub.config;
        chart.chart_area = ChartArea(Rect {
            x: 800,
            y: 0,
            width,
            height,
        });
        chart.chart_title.text.segments = rich_segments_from_text("Title");
        chart.chart_title.text.segments[1].font_size = Some(40.0);
        let label = |text: &str| RichText::plain(text, crate::color::Color::BLACK, 14.0, "");
        chart.bottom_x.categories = Some(
            ["a", "b"]
                .into_iter()
                .map(|id| crate::config::Category {
                    rich_label: Some(label(id)),
                    ..crate::config::Category::new(id, id)
                })
                .collect(),
        );
        chart.bottom_x.category_groups = Some(crate::config::CategoryGroups {
            groups: vec![crate::config::CategoryGroup {
                label: label("both"),
                first: "a".into(),
                last: "b".into(),
                bracket: true,
            }],
            offset: [0.0; 2],
        });
        chart.legend.visible = true;
        chart.legend.content.segments = rich_segments_from_text("legend");
        let mut overlay = crate::defaults::overlay_layer();
        overlay.legend = Some(chart.legend.clone());
        chart.overlays.push(overlay);
        chart.colorbar = Some(crate::defaults::color_bar_options());
        cfg.subplots.push(sub);
        cfg
    }

    // The factor is the cube root of the cell's smaller share of the parent
    // chart, held to 0.5..=1: a full-size cell and a larger one draw as
    // stored, a half-size one at 0.79, a tiny one at the floor.
    #[test]
    fn the_subplot_text_factor_follows_the_smaller_share_of_the_parent() {
        assert_eq!(subplot_text_factor(1.0), 1.0);
        assert_eq!(subplot_text_factor(2.5), 1.0);
        assert!((subplot_text_factor(0.5) - 0.5f32.cbrt()).abs() < 1e-6);
        assert!((subplot_text_factor(0.5) - 0.7937).abs() < 1e-4);
        assert!((subplot_text_factor(0.25) - 0.62996).abs() < 1e-4);
        assert_eq!(subplot_text_factor(0.125), SUBPLOT_TEXT_SCALE_MIN);
        assert_eq!(subplot_text_factor(0.01), SUBPLOT_TEXT_SCALE_MIN);
        assert_eq!(subplot_text_factor(0.0), SUBPLOT_TEXT_SCALE_MIN);
        assert_eq!(subplot_text_factor(-1.0), SUBPLOT_TEXT_SCALE_MIN);
        assert_eq!(subplot_text_factor(f32::NAN), 1.0);
        assert_eq!(subplot_text_factor(f32::INFINITY), 1.0);
        // Never larger for a smaller cell.
        let mut last = 0.0;
        for step in 0..=200 {
            let factor = subplot_text_factor(step as f32 / 100.0);
            assert!(factor >= last && (SUBPLOT_TEXT_SCALE_MIN..=1.0).contains(&factor));
            last = factor;
        }

        for (width, height, expected) in [
            (800, 600, 1.0),
            (1600, 1200, 1.0),
            // The smaller share decides.
            (400, 600, 0.5f32.cbrt()),
            (800, 300, 0.5f32.cbrt()),
            (400, 300, 0.5f32.cbrt()),
            (200, 450, 0.25f32.cbrt()),
            (40, 30, 0.5),
            (1, 1, 0.5),
        ] {
            let cfg = text_document(width, height);
            let scale = cfg.subplot_text_scale(0);
            assert!((scale - expected).abs() < 1e-6, "{width}x{height}: {scale}");
        }
        let mut cfg = text_document(400, 300);
        assert_eq!(cfg.subplot_text_scale(1), 1.0, "past the list");
        cfg.chart_area.0.width = 0;
        assert_eq!(cfg.subplot_text_scale(0), 1.0, "an empty parent");
    }

    // The resolved chart carries every text size times the factor and
    // nothing else changed; the stored chart keeps its sizes. `Fixed` and a
    // full-size cell resolve to the stored sizes.
    #[test]
    fn a_subplot_resolves_with_its_text_scaled_and_stores_it_unscaled() {
        let cfg = text_document(400, 300);
        let scale = cfg.subplot_text_scale(0);
        let stored = &cfg.subplots[0].config;
        let resolved = cfg.subplot_config(0).unwrap();
        let (from, to) = (text_sizes(stored), text_sizes(&resolved));
        // Title and its own-size segment, 4 axes x 2, 2 category labels, a
        // group header, 2 legends, 2 overlay axes x 2, the colourbar's 2.
        assert_eq!(from.len(), 2 + 8 + 3 + 1 + 4 + 1 + 2);
        assert_eq!(from.len(), to.len());
        for (from, to) in from.iter().zip(&to) {
            assert!(*from > 0.0);
            assert_eq!(*to, from * scale);
        }
        // Only text sizes differ: with the stored ones back, the resolved
        // chart is the stored one.
        let mut back = resolved.clone();
        back.scale_text_sizes(Some(stored), 1.0);
        assert_eq!(&back, stored);
        // Bands, offsets and the plot are the stored chart's.
        assert_eq!(resolved.margins(), stored.margins());
        assert_eq!(resolved.data_area(), stored.data_area());

        let mut fixed = cfg.clone();
        fixed.subplots[0].text_scale = SubplotTextScale::Fixed;
        assert_eq!(fixed.subplot_text_scale(0), 1.0);
        assert_eq!(&fixed.subplot_config(0).unwrap(), stored);
        let full = text_document(800, 600);
        assert_eq!(full.subplot_config(0).unwrap(), full.subplots[0].config);

        // The expanded panel the renderer draws is the resolved chart's.
        let panels = cfg.expand().unwrap().panels;
        let panel = panels
            .iter()
            .find(|panel| panel.layer == LayerRef::Subplot(0))
            .unwrap();
        assert_eq!(
            panel.config.chart_title.text.font_size,
            stored.chart_title.text.font_size * scale
        );
        assert_eq!(
            panel.config.left_y.label_style.font_size,
            stored.left_y.label_style.font_size * scale
        );
        // The parent's own text is not a subplot's: it draws as stored.
        assert_eq!(panels[0].config.chart_title, cfg.chart_title);
        assert_eq!(panels[0].config.left_y, cfg.left_y);
    }

    // Resizing the cell or the parent chart moves the factor with it; an
    // export scale multiplies both areas and leaves it.
    #[test]
    fn the_subplot_text_scale_follows_its_cell_and_not_the_export_scale() {
        let mut cfg = text_document(400, 300);
        let small = cfg.subplot_text_scale(0);
        cfg.set_subplot_area(
            0,
            ChartArea(Rect {
                x: 800,
                y: 0,
                width: 600,
                height: 450,
            }),
        )
        .unwrap();
        let larger = cfg.subplot_text_scale(0);
        assert!((larger - 0.75f32.cbrt()).abs() < 1e-6 && larger > small);
        assert_eq!(
            cfg.subplot_config(0).unwrap().chart_title.text.font_size,
            cfg.subplots[0].config.chart_title.text.font_size * larger
        );
        cfg.chart_area.0.width = 1600;
        cfg.chart_area.0.height = 1200;
        assert!((cfg.subplot_text_scale(0) - 0.375f32.cbrt()).abs() < 1e-6);
        for scale in [2.0, 3.0] {
            let scaled = cfg.scaled(scale);
            assert_eq!(scaled.subplot_text_scale(0), cfg.subplot_text_scale(0));
            assert_eq!(
                scaled.subplot_config(0).unwrap().chart_title.text.font_size,
                cfg.subplot_config(0).unwrap().chart_title.text.font_size * scale
            );
        }
    }

    // `text_scale` is omitted while `Auto`, so a document written before
    // the field parses to `Auto` and re-serializes byte for byte; `Fixed`
    // round-trips. No computed size is ever written.
    #[cfg(feature = "serde")]
    #[test]
    fn the_subplot_text_scale_serializes_additively() {
        let cfg = text_document(400, 300);
        let written = serde_json::to_string(&cfg).unwrap();
        assert!(!written.contains("text_scale"), "{written}");
        let older: Config = serde_json::from_str(&written).unwrap();
        assert_eq!(older.subplots[0].text_scale, SubplotTextScale::Auto);
        assert_eq!(older, cfg);
        assert_eq!(serde_json::to_string(&older).unwrap(), written);
        // The stored sizes are the document's, not the drawn ones.
        let json: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(
            json["subplots"][0]["config"]["chart_title"]["text"]["font_size"],
            serde_json::json!(cfg.subplots[0].config.chart_title.text.font_size)
        );

        let mut fixed = cfg.clone();
        fixed.subplots[0].text_scale = SubplotTextScale::Fixed;
        let json = serde_json::to_value(&fixed).unwrap();
        assert_eq!(
            json["subplots"][0]["text_scale"],
            serde_json::json!("Fixed")
        );
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, fixed);
        assert_eq!(back.subplot_text_scale(0), 1.0);
        let explicit = written.replacen(
            r#""subplots":[{"#,
            r#""subplots":[{"text_scale":"Auto","#,
            1,
        );
        assert_ne!(explicit, written);
        assert_eq!(serde_json::from_str::<Config>(&explicit).unwrap(), cfg);
    }

    #[test]
    fn subplot_layers_fall_back_when_out_of_range() {
        let mut cfg = subplot_document();
        assert_eq!(
            cfg.set_active_layer(LayerRef::Subplot(1)),
            LayerRef::Subplot(1)
        );
        assert_eq!(cfg.set_active_layer(LayerRef::Subplot(2)), LayerRef::Base);
        assert_eq!(
            cfg.set_active_layer(LayerRef::SubplotOverlay(0, 0)),
            LayerRef::SubplotOverlay(0, 0)
        );
        assert_eq!(
            cfg.set_active_layer(LayerRef::SubplotOverlay(1, 0)),
            LayerRef::Subplot(1)
        );
        assert_eq!(
            cfg.set_active_layer(LayerRef::SubplotOverlay(5, 0)),
            LayerRef::Base
        );
        cfg.active_layer = LayerRef::Subplot(1);
        cfg.subplots.pop();
        assert_eq!(cfg.active_layer(), LayerRef::Base);
    }

    #[test]
    fn series_belong_to_their_subplot_and_ids_stay_unique() {
        let cfg = subplot_document();
        assert_eq!(cfg.layer_of_series("flow"), LayerRef::Subplot(0));
        assert_eq!(cfg.layer_of_series("level"), LayerRef::SubplotOverlay(0, 0));
        assert_eq!(cfg.layer_of_series("rate"), LayerRef::Subplot(1));
        assert_eq!(cfg.layer_of_series("other"), LayerRef::Base);
        assert!(cfg.validate_subplots().is_ok());
        assert!(cfg.validate().is_ok());

        let mut twice = cfg.clone();
        twice.subplots[1].series.push("flow".into());
        assert!(twice.validate_subplots().is_err());
        let mut forward = cfg.clone();
        forward.subplots[0].link_x = Some(CellRef::Subplot(1));
        assert!(forward.validate_subplots().is_err());
        let mut nested = cfg.clone();
        nested.subplots[0]
            .config
            .subplots
            .push(crate::defaults::subplot());
        assert!(nested.validate_subplots().is_err());
        let mut annotated = cfg.clone();
        annotated.subplots[0].config.annotations.visible = false;
        assert!(annotated.validate_subplots().is_err());
        assert!(annotated.expand().is_err());
        let mut parent_overlay = cfg.clone();
        parent_overlay.overlays.push(OverlayLayer {
            series: vec!["rate".into()],
            ..crate::defaults::overlay_layer()
        });
        assert!(parent_overlay.validate_subplots().is_err());
    }

    // A linked axis fits every series drawn against its link group; an
    // overlay's Y axis fits only its own series.
    #[test]
    fn fit_roles_follow_layers_and_link_groups() {
        use crate::layer::Subplot;

        // Parent: base `p`, overlay `po`. Subplot 0: own axes, base `s0`,
        // overlay `s0o`. Subplot 1: X linked to the parent, Y to subplot 0,
        // base `s1`. Subplot 2: X linked to subplot 1 (so to the parent).
        let mut config = crate::defaults::config();
        let mut overlay = crate::defaults::overlay_layer();
        overlay.series = vec!["po".into()];
        config.overlays.push(overlay);
        let mut first = crate::defaults::config();
        let mut first_overlay = crate::defaults::overlay_layer();
        first_overlay.series = vec!["s0o".into()];
        first.overlays.push(first_overlay);
        config.subplots = vec![
            Subplot {
                config: first,
                series: vec!["s0".into(), "s0o".into()],
                ..crate::defaults::subplot()
            },
            Subplot {
                series: vec!["s1".into()],
                link_x: Some(CellRef::Parent),
                link_y: Some(CellRef::Subplot(0)),
                ..crate::defaults::subplot()
            },
            Subplot {
                series: vec!["s2".into()],
                link_x: Some(CellRef::Subplot(1)),
                ..crate::defaults::subplot()
            },
        ];
        let roles = |fit, id| config.series_fit_roles(fit, id);

        // Base: X from every chart linked to the parent's X, overlay series
        // included; Y from the parent's base series only.
        assert_eq!(roles(LayerRef::Base, "p"), (true, true));
        assert_eq!(roles(LayerRef::Base, "po"), (true, false));
        assert_eq!(roles(LayerRef::Base, "s0"), (false, false));
        assert_eq!(roles(LayerRef::Base, "s0o"), (false, false));
        assert_eq!(roles(LayerRef::Base, "s1"), (true, false));
        assert_eq!(roles(LayerRef::Base, "s2"), (true, false));
        // The annotation layer fits the base axes.
        assert_eq!(roles(LayerRef::Annotations, "s1"), (true, false));
        // Subplot 1 fits the same X group as the parent, and the Y group it
        // shares with subplot 0.
        assert_eq!(roles(LayerRef::Subplot(1), "p"), (true, false));
        assert_eq!(roles(LayerRef::Subplot(1), "s0"), (false, true));
        assert_eq!(roles(LayerRef::Subplot(1), "s0o"), (false, false));
        assert_eq!(roles(LayerRef::Subplot(1), "s1"), (true, true));
        assert_eq!(roles(LayerRef::Subplot(1), "s2"), (true, false));
        // Subplot 0 owns its X axis; its overlay shares that X axis.
        assert_eq!(roles(LayerRef::Subplot(0), "s0o"), (true, false));
        assert_eq!(roles(LayerRef::Subplot(0), "s1"), (false, true));
        assert_eq!(roles(LayerRef::Subplot(0), "p"), (false, false));
        // Overlay layers fit only their own Y axis.
        assert_eq!(roles(LayerRef::Overlay(0), "po"), (false, true));
        assert_eq!(roles(LayerRef::Overlay(0), "p"), (false, false));
        assert_eq!(roles(LayerRef::SubplotOverlay(0, 0), "s0o"), (false, true));
        assert_eq!(roles(LayerRef::SubplotOverlay(0, 0), "s0"), (false, false));

        // An overlay with an X axis of its own fits it from its own series,
        // which leave the parent's X group; one that follows the parent's Y
        // joins the parent's Y group, and its layer fit fits that group.
        let mut config = config.clone();
        config.overlays[0].link_x = false;
        config.overlays[0].link_y = true;
        config.subplots[0].config.overlays[0].link_x = false;
        let roles = |fit, id| config.series_fit_roles(fit, id);
        assert_eq!(roles(LayerRef::Base, "po"), (false, true));
        assert_eq!(roles(LayerRef::Base, "p"), (true, true));
        assert_eq!(roles(LayerRef::Overlay(0), "po"), (true, true));
        assert_eq!(roles(LayerRef::Overlay(0), "p"), (false, true));
        assert_eq!(roles(LayerRef::Overlay(0), "s0"), (false, false));
        // Subplot 1 shares subplot 0's Y group, not the parent's.
        assert_eq!(roles(LayerRef::Subplot(1), "po"), (false, false));
        assert_eq!(roles(LayerRef::Subplot(0), "s0o"), (false, false));
        assert_eq!(roles(LayerRef::SubplotOverlay(0, 0), "s0o"), (true, true));
        assert_eq!(roles(LayerRef::SubplotOverlay(0, 0), "s0"), (false, false));
    }

    // Axes are a subplot's own unless linked; a linked axis follows its
    // target, through a chain, and an edit of any member of a link group
    // moves the whole group.
    #[test]
    fn linked_axes_follow_their_root_and_carry_member_edits() {
        let cfg = subplot_document();
        let first = cfg.subplot_config(0).unwrap();
        let second = cfg.subplot_config(1).unwrap();
        assert_eq!((first.bottom_x.min, first.bottom_x.max), (-5.0, 5.0));
        assert_eq!((second.bottom_x.min, second.bottom_x.max), (0.0, 24.0));
        assert_eq!((second.left_y.min, second.left_y.max), (100.0, 200.0));
        // The stored linked fields equal their targets after a sync.
        assert_eq!(cfg.subplots[1].config.bottom_x.max, 24.0);
        assert_eq!(cfg.subplot_link_root(1, true), CellRef::Parent);
        assert_eq!(cfg.subplot_link_root(1, false), CellRef::Subplot(0));
        assert_eq!(cfg.subplot_link_root(0, true), CellRef::Subplot(0));

        // A member edit moves its root and with it every member.
        let mut edited = cfg.clone();
        edited.subplots[1].config.left_y.max = 400.0;
        edited.subplots[1].config.bottom_x.min = 2.0;
        edited.sync_subplot_links(Some(&cfg));
        assert_eq!(edited.subplots[0].config.left_y.max, 400.0);
        assert_eq!(edited.subplots[1].config.left_y.max, 400.0);
        assert_eq!(edited.bottom_x.min, 2.0);
        assert_eq!(edited.subplots[1].config.bottom_x.min, 2.0);
        // An unlinked axis of the same subplot is its own.
        assert_eq!(edited.subplots[0].config.bottom_x.min, -5.0);

        // A root edit wins over a member edit in the same call.
        let mut both = cfg.clone();
        both.bottom_x.max = 30.0;
        both.subplots[1].config.bottom_x.max = 12.0;
        both.sync_subplot_links(Some(&cfg));
        assert_eq!(both.bottom_x.max, 30.0);
        assert_eq!(both.subplots[1].config.bottom_x.max, 30.0);

        // A writer that re-derives the root's tick spacing from its unchanged
        // range does not count as a root edit: the member's range still wins
        // and its spacing travels with it.
        let mut respaced = cfg.clone();
        respaced.bottom_x.major_spacing = 6.0;
        respaced.subplots[1].config.bottom_x.max = 12.0;
        respaced.subplots[1].config.bottom_x.major_spacing = 3.0;
        respaced.sync_subplot_links(Some(&cfg));
        assert_eq!(respaced.bottom_x.max, 12.0);
        assert_eq!(respaced.bottom_x.major_spacing, 3.0);
        assert_eq!(respaced.subplots[1].config.bottom_x.major_spacing, 3.0);

        // Without a previous document the stored member values are reset.
        let mut stale = cfg.clone();
        stale.subplots[1].config.bottom_x.max = 99.0;
        stale.sync_subplot_links(None);
        assert_eq!(stale.subplots[1].config.bottom_x.max, 24.0);
    }

    // An axis unit (#273) travels with the tick spacing it is read in: a
    // link group prints its root's unit, an overlay panel prints its
    // overlay axis' own, and the hidden axes of a panel print none.
    #[test]
    fn the_axis_unit_travels_with_the_tick_spacing() {
        use crate::config::AxisUnit;
        let volts = Some(AxisUnit::new(5.0 / 32768.0, 0.0));
        let mut cfg = subplot_document();
        cfg.bottom_x.unit = volts;
        cfg.sync_subplot_links(None);
        assert_eq!(cfg.subplots[1].config.bottom_x.unit, volts);
        assert_eq!(cfg.subplot_config(1).unwrap().bottom_x.unit, volts);
        assert_eq!(cfg.subplots[0].config.bottom_x.unit, None);
        // A unit edit of a member alone is not a range edit: the member
        // follows its root again.
        let mut alone = cfg.clone();
        alone.subplots[1].config.bottom_x.unit = None;
        alone.sync_subplot_links(Some(&cfg));
        assert_eq!(alone.subplots[1].config.bottom_x.unit, volts);

        let mut layered = two_layer_config();
        layered.overlays[0].y_axis.unit = volts;
        layered.bottom_x.unit = volts;
        let expansion = layered.expand().unwrap();
        let panel = &expansion.panels[1].config;
        assert_eq!(panel.right_y.unit, volts);
        for hidden in [&panel.left_y, &panel.bottom_x, &panel.top_x] {
            assert_eq!(hidden.unit, None);
        }
    }

    // The log tick policy is tick spacing: a link group shares it like
    // `major_spacing`, an overlay panel draws its overlay axis' own, and the
    // hidden axes of a panel take the template's (#42).
    #[test]
    fn the_log_tick_policy_travels_with_the_tick_spacing() {
        use crate::config::LogTickPolicy;
        let mut cfg = subplot_document();
        cfg.bottom_x.log_ticks = LogTickPolicy::Adaptive;
        cfg.sync_subplot_links(None);
        assert_eq!(
            cfg.subplots[1].config.bottom_x.log_ticks,
            LogTickPolicy::Adaptive
        );
        assert_eq!(
            cfg.subplot_config(1).unwrap().bottom_x.log_ticks,
            LogTickPolicy::Adaptive
        );
        // The unlinked axis of the first subplot keeps its own.
        assert_eq!(
            cfg.subplots[0].config.bottom_x.log_ticks,
            LogTickPolicy::Decades
        );
        // A member's range edit carries its policy to the root.
        let mut edited = cfg.clone();
        edited.subplots[1].config.left_y.max = 400.0;
        edited.subplots[1].config.left_y.log_ticks = LogTickPolicy::Adaptive;
        edited.sync_subplot_links(Some(&cfg));
        assert_eq!(
            edited.subplots[0].config.left_y.log_ticks,
            LogTickPolicy::Adaptive
        );
        // A policy edit of a member alone is not a range edit: the member
        // follows its root again.
        let mut alone = cfg.clone();
        alone.subplots[1].config.bottom_x.log_ticks = LogTickPolicy::Decades;
        alone.sync_subplot_links(Some(&cfg));
        assert_eq!(
            alone.subplots[1].config.bottom_x.log_ticks,
            LogTickPolicy::Adaptive
        );

        let mut layered = two_layer_config();
        layered.overlays[0].y_axis.log_ticks = LogTickPolicy::Adaptive;
        layered.bottom_x.log_ticks = LogTickPolicy::Adaptive;
        let expansion = layered.expand().unwrap();
        let panel = &expansion.panels[1].config;
        assert_eq!(panel.right_y.log_ticks, LogTickPolicy::Adaptive);
        for hidden in [&panel.left_y, &panel.bottom_x, &panel.top_x] {
            assert_eq!(hidden.log_ticks, LogTickPolicy::Decades);
        }
        assert_eq!(
            expansion.panels[0].config.bottom_x.log_ticks,
            LogTickPolicy::Adaptive
        );
    }

    // Every subplot expands as a chart of its own after the parent, in its
    // own area, and an edit of one subplot leaves every other panel alone.
    #[test]
    fn subplots_expand_in_their_own_areas() {
        let cfg = subplot_document();
        let expansion = cfg.expand().unwrap();
        let layers: Vec<LayerRef> = expansion.panels.iter().map(|p| p.layer).collect();
        assert_eq!(
            layers,
            vec![
                LayerRef::Base,
                LayerRef::Subplot(0),
                LayerRef::SubplotOverlay(0, 0),
                LayerRef::Subplot(1),
            ]
        );
        assert_eq!(cfg.panel_layers(), layers);
        let first = &expansion.panels[1].config;
        let overlay = &expansion.panels[2].config;
        let second = &expansion.panels[3].config;
        assert!(expansion.panels[0].config.subplots.is_empty());
        assert_eq!(first.chart_area, cfg.subplots[0].config.chart_area);
        assert_eq!(first.data_area().unwrap(), overlay.data_area().unwrap());
        assert_eq!(second.bottom_x.max, 24.0);
        let area = second.data_area().unwrap().0;
        assert!(area.x >= 320 && area.y >= 240);
        // The parent expands exactly as it does without subplots.
        let mut alone = cfg.clone();
        alone.subplots.clear();
        assert_eq!(expansion.panels[0], alone.expand().unwrap().panels[0]);

        let mut edit = cfg.clone();
        edit.subplots[0].config.legend.visible = true;
        let after = edit.expand().unwrap();
        assert_eq!(after.panels[0], expansion.panels[0]);
        assert_ne!(after.panels[1], expansion.panels[1]);
        assert_eq!(after.panels[2], expansion.panels[2]);
        assert_eq!(after.panels[3], expansion.panels[3]);
    }

    // The document's selection reaches the panel that draws each picked
    // series: a subplot's base and overlay panels as well as the parent's.
    #[test]
    fn subplot_panels_highlight_the_document_picks_of_their_series() {
        use crate::config::{DataSelectionsConfig, PickedDataRef};
        let mut cfg = subplot_document();
        let point = |id: &str| PickedDataRef::Point {
            source_id: None,
            series_id: id.into(),
            point_index: 1,
        };
        let mut refs = ["temperature", "flow", "level", "rate"].map(point).to_vec();
        refs.push(PickedDataRef::Point {
            source_id: None,
            series_id: "level".into(),
            point_index: 3,
        });
        cfg.picked_data = Some(DataSelectionsConfig {
            refs,
            ..crate::defaults::data_selections_config()
        });
        let expansion = cfg.expand().unwrap();
        let data: Vec<Vec<String>> = expansion
            .panels
            .iter()
            .map(|panel| {
                panel
                    .config
                    .picked_data
                    .as_ref()
                    .unwrap()
                    .refs
                    .iter()
                    .map(|r| r.series_id().to_string())
                    .collect()
            })
            .collect();
        assert_eq!(
            data,
            [
                vec!["temperature"],
                vec!["flow"],
                vec!["level", "level"],
                vec!["rate"]
            ]
        );
        // Without a document selection a subplot keeps its own.
        let mut own = subplot_document();
        own.subplots[1].config.picked_data = Some(DataSelectionsConfig {
            refs: vec![point("rate")],
            ..crate::defaults::data_selections_config()
        });
        let panels = own.expand().unwrap().panels;
        assert_eq!(panels[3].config.picked_data.as_ref().unwrap().refs.len(), 1);
        assert!(panels[1].config.picked_data.is_none());
    }

    #[test]
    fn the_document_covers_every_chart_and_scales_together() {
        let mut cfg = subplot_document();
        assert_eq!(
            cfg.document_area(),
            Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            }
        );
        let large = cfg.scaled(2.0);
        assert_eq!(large.document_area().width, 1280);
        assert_eq!(large.subplots[1].config.chart_area.0.x, 640);
        for scale in [1.0_f32, 2.0, 0.5, 1.37] {
            let expansion = cfg.scaled(scale).expand().unwrap();
            assert_eq!(
                expansion.panels[1].config.data_area().unwrap(),
                expansion.panels[2].config.data_area().unwrap()
            );
        }
        cfg.translate_areas(5, 7);
        assert_eq!(cfg.chart_area.0.x, 5);
        assert_eq!(cfg.subplots[1].config.chart_area.0.x, 325);
        assert_eq!(cfg.subplots[1].config.chart_area.0.y, 247);
        let plain = crate::defaults::config();
        assert_eq!(plain.document_area(), plain.chart_area.0);
    }

    fn categories(n: usize) -> Option<Vec<Category>> {
        Some(
            (0..n)
                .map(|i| Category::new(format!("k{i}"), format!("Kind {i}")))
                .collect(),
        )
    }

    /// An overlay panel's hidden axes keep a categorical axis' range and
    /// categories (its category bars resolve their overrides by id), and the
    /// overlay's own categorical axis is drawn on the panel's right side.
    #[test]
    fn overlay_panels_map_categorical_axes_by_their_range() {
        let mut cfg = two_layer_config();
        cfg.bottom_x.categories = categories(5);
        cfg.overlays[0].y_axis.categories = categories(3);
        let expansion = cfg.expand().unwrap();
        let overlay = &expansion.panels[1].config;
        assert_eq!(overlay.bottom_x.categories, categories(5));
        assert_eq!(overlay.bottom_x.data_range(), (-0.5, 4.5));
        assert!(!overlay.bottom_x.line_visible && !overlay.bottom_x.label_style.label_visible);
        assert_eq!(overlay.left_y.categories, categories(3));
        assert_eq!((overlay.left_y.min, overlay.left_y.max), (-0.5, 2.5));
        assert_eq!(overlay.right_y.categories, categories(3));
        assert_eq!(
            expansion.panels[0].config.bottom_x.categories,
            categories(5)
        );
    }

    /// A linked axis follows its target's categories with the rest of its
    /// range, and an edit of a member's categories moves the link group.
    #[test]
    fn linked_axes_follow_the_categories_of_their_target() {
        let mut cfg = subplot_document();
        cfg.bottom_x.categories = categories(4);
        let linked = cfg.subplot_config(1).unwrap();
        assert_eq!(linked.bottom_x.categories, categories(4));
        let own = cfg.subplot_config(0).unwrap();
        assert_eq!(own.bottom_x.categories, None);

        let mut edited = cfg.clone();
        edited.subplots[1].config.bottom_x.categories = categories(2);
        let before = cfg.clone();
        edited.sync_subplot_links(Some(&before));
        assert_eq!(edited.bottom_x.categories, categories(2));
        assert_eq!(edited.subplots[1].config.bottom_x.categories, categories(2));
    }

    fn offsets(ids: &[&str], step_y: f64, step_x: f64) -> crate::data_config::LineOffsets {
        crate::data_config::LineOffsets {
            series: ids.iter().map(|id| (*id).into()).collect(),
            step_y,
            step_x,
        }
    }

    /// Line offset members expand after the overlays, each the panel of the
    /// layer that scales it with the transform axes moved by its shift
    /// (decades on a log axis) and nothing but its own data and picks; the
    /// base and overlay panels draw none of them and keep everything else.
    #[test]
    fn line_offset_members_expand_into_shifted_panels() {
        use crate::config::{DataSelectionsConfig, PickedDataRef};
        let mut cfg = two_layer_config();
        cfg.bottom_x.min = 0.0;
        cfg.bottom_x.max = 10.0;
        cfg.left_y.scale = AxisScale::Logarithmic;
        cfg.left_y.min = 1.0;
        cfg.left_y.max = 1000.0;
        cfg.overlays[0].series.push("p2".into());
        cfg.line_offsets = Some(offsets(&["a", "b", "p2"], 0.5, 2.0));
        let pick = |id: &str| PickedDataRef::Point {
            source_id: None,
            series_id: id.into(),
            point_index: 1,
        };
        cfg.picked_data = Some(DataSelectionsConfig {
            refs: vec![
                pick("a"),
                pick("b"),
                pick("p2"),
                pick("base"),
                pick("pressure"),
            ],
            ..crate::defaults::data_selections_config()
        });
        let expansion = cfg.expand().unwrap();
        let layers: Vec<LayerRef> = expansion.panels.iter().map(|p| p.layer).collect();
        assert_eq!(layers, cfg.panel_layers());
        assert_eq!(
            layers,
            [
                LayerRef::Base,
                LayerRef::Overlay(0),
                LayerRef::Base,
                LayerRef::Base,
                LayerRef::Overlay(0)
            ]
        );
        let refs = |panel: &ExpandedPanel| -> Vec<String> {
            panel
                .config
                .picked_data
                .as_ref()
                .unwrap()
                .refs
                .iter()
                .map(|r| r.series_id().to_string())
                .collect()
        };
        let base = &expansion.panels[0].config;
        assert_eq!(base.line_offsets, None);
        assert_eq!(refs(&expansion.panels[0]), ["base"]);
        assert_eq!(refs(&expansion.panels[1]), ["pressure"]);
        for (index, (id, k)) in [(2, ("a", 0.0)), (3, ("b", 1.0)), (4, ("p2", 2.0))] {
            let panel = &expansion.panels[index].config;
            assert_eq!(cfg.panel_member(index), Some(id));
            assert_eq!(cfg.panel_of_series(id), index);
            assert_eq!(refs(&expansion.panels[index]), [id]);
            assert_eq!(panel.data_area().unwrap(), expansion.data_area);
            assert_eq!(panel.bottom_x.min, -2.0 * k);
            assert_eq!(panel.bottom_x.max, 10.0 - 2.0 * k);
            for axis in [&panel.top_x, &panel.bottom_x, &panel.left_y, &panel.right_y] {
                assert!(!axis.line_visible && !axis.label_style.visible);
                assert!(!axis.title_option.visible && axis.tick == TickVisibility::None);
            }
            assert!(!panel.legend.visible && !panel.chart_title.visible);
            assert!(!panel.grid.show_major_x && !panel.grid.show_major_y);
            assert!(panel.annotations.is_default());
            assert_eq!(panel.line_offsets, None);
        }
        // Decades on the base's log axis, data units on the overlay's.
        let b = &expansion.panels[3].config.left_y;
        assert!((b.min - 10f64.powf(-0.5)).abs() < 1e-12);
        assert!((b.max - 1000.0 * 10f64.powf(-0.5)).abs() < 1e-9);
        let p2 = &expansion.panels[4].config.left_y;
        assert_eq!((p2.min, p2.max), (-6.0, 4.0));
        assert_eq!(cfg.panel_of_series("pressure"), 1);
        assert_eq!(cfg.panel_of_series("base"), 0);
        assert_eq!(cfg.panel_member(0), None);
        assert_eq!(cfg.panel_member(1), None);
        assert_eq!(cfg.panel_member(5), None);
        // The pointer never names a member: the base layer answers for them.
        assert_eq!(expansion.panel(LayerRef::Base).unwrap().config, *base);
        assert_eq!(cfg.set_active_layer(LayerRef::Overlay(1)), LayerRef::Base);
        // A group without members is a single-layer chart.
        let mut empty = crate::defaults::config();
        empty.line_offsets = Some(offsets(&[], 1.0, 0.0));
        assert!(!empty.has_offset_members());
        assert_eq!(empty.panel_layers(), [LayerRef::Base]);
        // A member edit changes only the member panels.
        let mut stepped = cfg.clone();
        stepped.line_offsets.as_mut().unwrap().step_y = 0.25;
        let after = stepped.expand().unwrap();
        assert_eq!(after.panels[0], expansion.panels[0]);
        assert_eq!(after.panels[1], expansion.panels[1]);
        assert_eq!(after.panels[2], expansion.panels[2]);
        assert_ne!(after.panels[3], expansion.panels[3]);
    }

    /// A member's cut ranges move with its shifted range, so its pieces and
    /// gaps fall where its owner's do, in data units on a linear axis and
    /// in decades on a log one (#200).
    #[test]
    fn line_offset_members_keep_their_owners_gaps() {
        use crate::config::{AxisBreak, AxisBreakStyle, AxisBreaks};
        let mut cfg = two_layer_config();
        cfg.bottom_x.min = 0.0;
        cfg.bottom_x.max = 10.0;
        cfg.left_y.scale = AxisScale::Logarithmic;
        cfg.left_y.min = 1.0;
        cfg.left_y.max = 1e6;
        let cut = |from, to, style| {
            Some(AxisBreaks {
                ranges: vec![AxisBreak::new(from, to)],
                style,
                ..crate::defaults::axis_breaks()
            })
        };
        cfg.bottom_x.breaks = cut(3.0, 6.0, AxisBreakStyle::Continuous);
        cfg.left_y.breaks = cut(100.0, 1e4, AxisBreakStyle::Panels);
        cfg.line_offsets = Some(offsets(&["a", "b"], 0.5, 2.0));
        let expansion = cfg.expand().unwrap();
        let base = &expansion.panels[0].config;
        for index in [2, 3] {
            let member = &expansion.panels[index].config;
            for (own, owner) in [
                (&member.bottom_x, &base.bottom_x),
                (&member.left_y, &base.left_y),
            ] {
                let (own, owner) = (
                    crate::layout::BreakMap::of(own, 1000.0),
                    crate::layout::BreakMap::of(owner, 1000.0),
                );
                assert_eq!(own.knots().len(), 1);
                let (a, b) = (own.knots()[0], owner.knots()[0]);
                assert!(
                    (a.start - b.start).abs() < 1e-9 && (a.end - b.end).abs() < 1e-9,
                    "panel {index}: {a:?} vs {b:?}"
                );
            }
            assert_eq!(
                member.left_y.breaks.as_ref().map(|b| b.style),
                Some(AxisBreakStyle::Panels)
            );
        }
        // The second member is shifted by 2 on x and half a decade on y.
        let b = &expansion.panels[3].config;
        let x = &b.bottom_x.breaks.as_ref().unwrap().ranges[0];
        assert_eq!((x.from, x.to), (1.0, 4.0));
        let y = &b.left_y.breaks.as_ref().unwrap().ranges[0];
        assert!((y.from - 100.0 * 10f64.powf(-0.5)).abs() < 1e-9);
    }

    /// A subplot's line offsets are its own: its members expand among its
    /// own panels, shifted on its own axes, and the parent's group does not
    /// reach into it.
    #[test]
    fn subplot_line_offsets_are_its_own() {
        let mut cfg = subplot_document();
        cfg.subplots[0].series.push("flow2".into());
        cfg.subplots[0].config.line_offsets = Some(offsets(&["flow", "flow2"], 10.0, 0.0));
        cfg.line_offsets = Some(offsets(&["top", "top2"], 1.0, 0.0));
        let layers = cfg.panel_layers();
        assert_eq!(
            layers,
            [
                LayerRef::Base,
                LayerRef::Base,
                LayerRef::Base,
                LayerRef::Subplot(0),
                LayerRef::SubplotOverlay(0, 0),
                LayerRef::Subplot(0),
                LayerRef::Subplot(0),
                LayerRef::Subplot(1),
            ]
        );
        let expansion = cfg.expand().unwrap();
        assert_eq!(expansion.panels.len(), layers.len());
        assert_eq!(cfg.panel_of_series("flow2"), 6);
        assert_eq!(cfg.panel_member(6), Some("flow2"));
        assert_eq!(cfg.panel_of_series("rate"), 7);
        assert_eq!(cfg.panel_of_series("top2"), 2);
        assert_eq!(cfg.line_offset_of("flow2"), Some([0.0, 10.0]));
        assert_eq!(cfg.line_offset_of("top2"), Some([0.0, 1.0]));
        let member = &expansion.panels[6].config;
        let own = &expansion.panels[3].config;
        assert_eq!(member.chart_area, own.chart_area);
        assert_eq!(member.data_area().unwrap(), own.data_area().unwrap());
        assert_eq!((member.left_y.min, member.left_y.max), (90.0, 190.0));
        assert_eq!(member.bottom_x.min, own.bottom_x.min);
        // A parent member drawn by a subplot is a configuration error.
        let mut wrong = cfg.clone();
        wrong.line_offsets = Some(offsets(&["flow"], 1.0, 0.0));
        assert!(wrong.validate_line_offset_layout().is_err());
        assert!(wrong.expand().is_err());
        let mut wrong = cfg;
        wrong.subplots[1].config.line_offsets = Some(offsets(&["flow"], 1.0, 0.0));
        assert!(wrong.validate_line_offset_layout().is_err());
    }

    /// A document with `parent` overlays on the parent and one subplot per
    /// entry of `subplots`, each with that many overlays of its own. Every
    /// layer draws one series: `base`, `po{k}`, `s{i}`, `s{i}o{n}`.
    fn layered(parent: usize, subplots: &[usize]) -> Config {
        let categories = || {
            Some(
                ["a", "b", "c"]
                    .map(|id| crate::config::Category::new(id, id))
                    .to_vec(),
            )
        };
        let overlay = |id: String| {
            let mut overlay = crate::defaults::overlay_layer();
            overlay.y_axis.min = 0.0;
            overlay.y_axis.max = 1.0;
            overlay.y_axis.out_margin = 2.0;
            overlay.series = vec![id];
            overlay
        };
        let mut cfg = crate::defaults::config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        cfg.bottom_x.categories = categories();
        cfg.overlays = (0..parent).map(|k| overlay(format!("po{k}"))).collect();
        cfg.subplots = subplots
            .iter()
            .enumerate()
            .map(|(i, &count)| {
                let mut subplot = crate::defaults::subplot();
                subplot.config.chart_area = ChartArea(Rect {
                    x: 200 * i as u32,
                    y: 480,
                    width: 200,
                    height: 200,
                });
                subplot.config.bottom_x.categories = categories();
                subplot.config.overlays =
                    (0..count).map(|n| overlay(format!("s{i}o{n}"))).collect();
                subplot.series = vec![format!("s{i}")];
                subplot
            })
            .collect();
        cfg
    }

    /// Every series id `layered` places, and one placed nowhere.
    fn layered_ids(cfg: &Config) -> Vec<String> {
        let mut ids = vec!["base".to_string(), "nowhere".to_string()];
        ids.extend((0..cfg.overlays.len()).map(|k| format!("po{k}")));
        for (i, subplot) in cfg.subplots.iter().enumerate() {
            ids.push(format!("s{i}"));
            ids.extend((0..subplot.config.overlays.len()).map(|n| format!("s{i}o{n}")));
        }
        ids
    }

    /// A category bar, box plot and range bar series named `id`.
    fn indexed_series(id: &str) -> [crate::data_config::SeriesConfig; 3] {
        use crate::data_config::{
            CategoryBarLabelFormat, DataRenderType, RangeColor, SeriesConfig,
        };
        let series = |render_type| SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "positions".into(),
            y_column: format!("{id}.values"),
            z_column: None,
            render_type,
        };
        let color = crate::Color::from_rgb8(40, 120, 200);
        [
            series(DataRenderType::CategoryBars {
                color,
                style: crate::defaults::category_bar_style(),
                labels: crate::defaults::category_bar_series_labels(),
                label_format: CategoryBarLabelFormat::Value,
                overrides: None,
            }),
            series(DataRenderType::BoxPlot {
                color,
                style: crate::defaults::box_plot_style(),
                labels: crate::defaults::box_plot_series_labels(),
                outliers: None,
                overrides: None,
                violin: None,
                points: None,
            }),
            series(DataRenderType::RangeBars {
                color,
                style: crate::defaults::category_bar_style(),
                start: format!("{id}.start"),
                whiskers: None,
                color_by: RangeColor::Single,
                progress: None,
                connector: None,
                slot_width: None,
                labels: crate::defaults::category_bar_series_labels(),
            }),
        ]
    }

    /// LAYER_OF_SERIES_IN_RANGE over one document: every layer
    /// `layer_of_series` names indexes the lists it names, the subplot
    /// configs keep their overlays, and every reader of those indexes
    /// (`panel_of_series`, `line_offset_of`, `expand`, the bar, box and range
    /// validations) answers without panicking.
    fn assert_layers_in_range(cfg: &Config) {
        let layers = cfg.panel_layers();
        for index in 0..cfg.subplots.len() {
            let subplot = cfg.subplot_config(index).expect("in range");
            assert_eq!(
                subplot.overlays.len(),
                cfg.subplots[index].config.overlays.len()
            );
        }
        assert!(cfg.subplot_config(cfg.subplots.len()).is_none());
        assert!(cfg.subplot_config(usize::MAX).is_none());
        for id in layered_ids(cfg) {
            match cfg.layer_of_series(&id) {
                LayerRef::Overlay(i) => assert!(i < cfg.overlays.len(), "{id}"),
                LayerRef::Subplot(i) => assert!(i < cfg.subplots.len(), "{id}"),
                LayerRef::SubplotOverlay(i, n) => {
                    assert!(i < cfg.subplots.len(), "{id}");
                    assert!(n < cfg.subplots[i].config.overlays.len(), "{id}");
                }
                LayerRef::Base | LayerRef::Annotations => {}
            }
            assert!(cfg.panel_of_series(&id) < layers.len(), "{id}");
            assert_eq!(cfg.line_offset_of(&id), None, "{id}");
            for item in indexed_series(&id) {
                let one = std::slice::from_ref(&item);
                // Each answers `Ok` or names a rule; none panics.
                let _bars = cfg.validate_category_bars(one);
                let _boxes = cfg.validate_box_plots(one);
                let _ranges = cfg.validate_range_bars(one);
            }
        }
        let expansion = cfg.expand().expect("the document expands");
        assert_eq!(
            expansion
                .panels
                .iter()
                .map(|panel| panel.layer)
                .collect::<Vec<_>>(),
            layers
        );
    }

    // LAYER_OF_SERIES_IN_RANGE (#152): 0, 1 and 40 parent overlays, 0, 1
    // and 5 subplots with 0 to 3 overlays each, before and after every
    // mutation path (an out-of-range active layer, a removed overlay, a
    // removed subplot, a removed subplot overlay).
    #[test]
    fn every_layer_of_a_series_indexes_its_lists() {
        for parent in [0, 1, 40] {
            for subplots in [&[][..], &[1][..], &[0, 1, 2, 3, 2][..]] {
                let mut cfg = layered(parent, subplots);
                assert_eq!(cfg.validate_overlays(), Ok(()));
                assert_eq!(cfg.validate_subplots(), Ok(()));
                assert_layers_in_range(&cfg);
                for layer in [
                    LayerRef::Overlay(parent),
                    LayerRef::Overlay(usize::MAX),
                    LayerRef::Subplot(subplots.len()),
                    LayerRef::SubplotOverlay(subplots.len(), 0),
                    LayerRef::SubplotOverlay(0, 99),
                ] {
                    let stored = cfg.set_active_layer(layer);
                    assert_eq!(cfg.active_layer(), stored);
                    assert_ne!(stored, layer, "{layer:?} is out of range");
                    assert_layers_in_range(&cfg);
                }
                if parent > 0 {
                    let mut fewer = cfg.clone();
                    fewer.set_active_layer(LayerRef::Overlay(parent - 1));
                    fewer.overlays.pop();
                    assert_eq!(fewer.active_layer(), LayerRef::Base);
                    assert_layers_in_range(&fewer);
                }
                if let Some(last) = subplots.len().checked_sub(1) {
                    let mut fewer = cfg.clone();
                    fewer.set_active_layer(LayerRef::Subplot(last));
                    fewer.subplots.pop();
                    assert_eq!(fewer.active_layer(), LayerRef::Base);
                    assert_layers_in_range(&fewer);
                    let mut fewer = cfg.clone();
                    if fewer.subplots[last].config.overlays.pop().is_some() {
                        assert_layers_in_range(&fewer);
                    }
                }
            }
        }
        // A duplicated id is refused, and `layer_of_series` still names the
        // first overlay listing it.
        let mut twice = layered(3, &[1]);
        twice.overlays[2].series.push("po0".into());
        assert!(twice.validate_overlays().is_err());
        assert!(twice.expand().is_err());
        assert_eq!(twice.layer_of_series("po0"), LayerRef::Overlay(0));
        let mut twice = layered(0, &[2, 1]);
        twice.subplots[1].config.overlays[0]
            .series
            .push("s0o1".into());
        assert!(twice.validate_subplots().is_err());
        assert_eq!(
            twice.layer_of_series("s0o1"),
            LayerRef::SubplotOverlay(0, 1)
        );
    }

    /// PANELS_MATCH_LAYERS (#152): `expand` gives one panel per entry of
    /// `panel_layers`, in that order, or a layout error, for a single-layer
    /// chart, overlays, line offset groups of 0, 1 and 200 members (past any
    /// old cap), subplots with overlays and members of their own and a chart
    /// area squeezed below its margins. `panel_layers` always starts with the
    /// base, and a nested subplot is refused.
    #[test]
    fn expanded_panels_follow_the_panel_layers() {
        use crate::layout::LayoutError;
        let members =
            |count: usize| -> Vec<String> { (0..count).map(|k| format!("m{k}")).collect() };
        let with_members = |mut cfg: Config, count: usize| {
            let ids = members(count);
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            cfg.line_offsets = Some(offsets(&refs, 0.5, 0.0));
            cfg
        };
        let mut documents = vec![crate::defaults::config(), layered(0, &[]), layered(1, &[])];
        for count in [0, 1, 200] {
            documents.push(with_members(layered(0, &[]), count));
            documents.push(with_members(layered(3, &[]), count));
        }
        let mut subplots = with_members(layered(2, &[1, 0, 2]), 3);
        subplots.subplots[0].series.push("sm0".into());
        subplots.subplots[0].series.push("sm1".into());
        subplots.subplots[0].config.line_offsets = Some(offsets(&["s0", "sm0", "sm1"], 1.0, 0.0));
        documents.push(subplots.clone());
        for (width, height) in [(40, 30), (1, 1)] {
            let mut squeezed = with_members(layered(2, &[]), 5);
            squeezed.chart_area = ChartArea(Rect {
                x: 0,
                y: 0,
                width,
                height,
            });
            documents.push(squeezed);
        }
        let mut expanded = 0;
        for cfg in &documents {
            let layers = cfg.panel_layers();
            assert_eq!(layers.first(), Some(&LayerRef::Base));
            match cfg.expand() {
                Ok(expansion) => {
                    assert_eq!(expansion.panels.len(), layers.len());
                    for (panel, layer) in expansion.panels.iter().zip(&layers) {
                        assert_eq!(panel.layer, *layer);
                    }
                    expanded += 1;
                }
                Err(
                    LayoutError::OverflowHorizontal { .. }
                    | LayoutError::OverflowVertical { .. }
                    | LayoutError::EmptyDataArea,
                ) => assert!(cfg.chart_area.0.width <= 40, "{:?}", cfg.chart_area),
                Err(error) => panic!("{error:?} for {} layers", layers.len()),
            }
        }
        assert_eq!(expanded, documents.len() - 2);
        let with_200 = with_members(layered(3, &[]), 200);
        assert_eq!(with_200.panel_layers().len(), 1 + 3 + 200);
        // A subplot of a subplot is refused: `panel_layers` does not recurse.
        let mut nested = layered(0, &[1]);
        nested.subplots[0].config.subplots = layered(0, &[0]).subplots;
        assert!(nested.validate_subplots().is_err());
        assert!(nested.expand().is_err());
    }

    // OVERLAY_PANEL_INDEX (#152): `overlay_panel(i)` is `Some` exactly for
    // `i < overlays.len()`, and a drag of an overlay axis through a hit map
    // built before an overlay was removed is refused, not indexed.
    #[test]
    fn overlay_panels_exist_exactly_for_the_overlays() {
        use crate::layout::{NudgeReject, NudgeResult};
        use crate::select::HitMap;
        use crate::text::{MeasureText, TextExtents};
        struct Fixed;
        impl MeasureText for Fixed {
            fn measure_rich(&self, rt: &crate::text::RichText) -> TextExtents {
                TextExtents {
                    width: rt.segments.len() as f32 * 7.0,
                    ascent: 9.0,
                    descent: 3.0,
                }
            }
        }
        for count in [0, 1, 40] {
            let mut cfg = layered(count, &[]);
            for index in 0..count {
                assert!(cfg.overlay_panel(index).is_some(), "{index} of {count}");
            }
            assert!(cfg.overlay_panel(count).is_none());
            assert!(cfg.overlay_panel(usize::MAX).is_none());
            let Some(last) = count.checked_sub(1) else {
                continue;
            };
            cfg.set_active_layer(LayerRef::Overlay(last));
            let map = HitMap::for_active_layer(&cfg);
            let axis = map
                .find(&crate::select::Selectable::element_id(
                    &crate::select::OverlayAxisElement { layer: last },
                ))
                .expect("the overlay axis element");
            let drag = |cfg: &mut Config| {
                map.get(axis)
                    .and_then(|element| element.as_draggable())
                    .expect("draggable")
                    .drag_by(cfg, &Fixed, 3.0, 0.0)
            };
            let result = drag(&mut cfg);
            assert!(
                matches!(result, NudgeResult::Moved | NudgeResult::Rejected(_)),
                "{result:?}"
            );
            // The map outlives the overlay it names: a stale layer is
            // `overlays.len()` now.
            let mut fewer = cfg.clone();
            fewer.overlays.pop();
            assert_eq!(
                drag(&mut fewer),
                NudgeResult::Rejected(NudgeReject::OutOfBounds)
            );
            let mut beyond = cfg.clone();
            let element = crate::select::OverlayAxisElement { layer: usize::MAX };
            assert_eq!(
                crate::drag::Draggable::drag_by(&element, &mut beyond, &Fixed, 3.0, 0.0),
                NudgeResult::Rejected(NudgeReject::OutOfBounds)
            );
            assert_eq!(beyond, cfg);
        }
    }

    /// The link patterns `linked_documents` builds: subplot `i`'s target
    /// for `n` subplots, `None` for no link.
    fn link_patterns(n: usize) -> Vec<Vec<Option<CellRef>>> {
        let all = |f: &dyn Fn(usize) -> Option<CellRef>| (0..n).map(f).collect();
        vec![
            all(&|_| None),
            // A chain down to the parent.
            all(&|i| Some(i.checked_sub(1).map_or(CellRef::Parent, CellRef::Subplot))),
            all(&|i| Some(CellRef::Subplot(i))),
            // Forward links, the last one past the list.
            all(&|i| Some(CellRef::Subplot(i + 1))),
            // 2-cycles (the odd last one links to itself).
            all(&|i| Some(CellRef::Subplot(if i ^ 1 < n { i ^ 1 } else { i }))),
            // One cycle through every subplot.
            all(&|i| Some(CellRef::Subplot((i + 1) % n.max(1)))),
            all(&|_| Some(CellRef::Subplot(n))),
            all(&|_| Some(CellRef::Subplot(usize::MAX))),
            // Every other subplot unlinked, the rest pointing past the list.
            all(&|i| (i % 2 == 1).then_some(CellRef::Subplot(usize::MAX - i))),
        ]
    }

    /// Documents with 0, 1 and 64 subplots and every pair of link
    /// patterns on X and Y, validated or not.
    fn linked_documents() -> Vec<Config> {
        use crate::layer::Subplot;
        let mut documents = Vec::new();
        for n in [0, 1, 64] {
            let patterns = link_patterns(n);
            for (k, x) in patterns.iter().enumerate() {
                // Y uses the next pattern, so the axes' chains differ.
                let y = &patterns[(k + 1) % patterns.len()];
                let mut cfg = crate::defaults::config();
                cfg.subplots = (0..n)
                    .map(|i| Subplot {
                        link_x: x[i],
                        link_y: y[i],
                        ..crate::defaults::subplot()
                    })
                    .collect();
                documents.push(cfg);
            }
        }
        documents
    }

    /// LINK_ROOT_IN_RANGE (#152): the root of any subplot's link chain is
    /// the parent or a subplot in the list, for any index and any links (a
    /// lower index, itself, forward, cycles, targets past the list), before
    /// and after every path that edits the links' axes or reads them back
    /// from JSON.
    #[test]
    fn a_link_root_is_the_parent_or_a_listed_subplot() {
        let check = |cfg: &Config| {
            let len = cfg.subplots.len();
            for index in 0..len + 2 {
                for x in [true, false] {
                    match cfg.subplot_link_root(index, x) {
                        CellRef::Parent => {}
                        CellRef::Subplot(k) => {
                            assert!(k < len, "root {k} of {index} (x {x}) past {len}")
                        }
                    }
                }
                if index < len {
                    assert!(cfg.subplot_config(index).is_some());
                } else {
                    assert!(cfg.subplot_config(index).is_none());
                }
            }
        };
        for cfg in linked_documents() {
            check(&cfg);
            let mut synced = cfg.clone();
            synced.sync_subplot_links(None);
            check(&synced);
            // A host edit of every linked axis is carried to its root.
            let mut edited = synced.clone();
            for subplot in &mut edited.subplots {
                subplot.config.bottom_x.max += 7.0;
                subplot.config.left_y.min -= 3.0;
            }
            edited.sync_subplot_links(Some(&synced));
            check(&edited);
            // An edit of the parent with no subplot edit.
            let mut parent = synced.clone();
            parent.bottom_x.min -= 1.0;
            parent.sync_subplot_links(Some(&synced));
            check(&parent);
            // Removing subplots leaves links past the shorter list.
            let mut fewer = edited.clone();
            fewer.subplots.truncate(fewer.subplots.len() / 2);
            fewer.sync_subplot_links(Some(&edited));
            check(&fewer);
            #[cfg(feature = "serde")]
            {
                let json = serde_json::to_string(&cfg).unwrap();
                let back: Config = serde_json::from_str(&json).unwrap();
                assert_eq!(back.subplots.len(), cfg.subplots.len());
                check(&back);
            }
        }
    }
}
