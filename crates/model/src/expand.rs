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
//! - Each overlay panel keeps the parent's X range for its data transform but
//!   draws no X axis, no grid, no title and no colourbar. Its left Y axis is a
//!   hidden copy of the parent's left axis carrying the overlay's range, so
//!   the data transform uses the overlay's scale; its right Y axis is the
//!   overlay's own axis. It draws the overlay's legend when it has one.
//! - Every panel's margins are lifted to the group margins, so `data_area()`
//!   returns the same rectangle for all of them. The group right margin is
//!   the widest band of the parent and overlay axes (`Config::margins`).
//! - A panel's config depends only on what that panel draws. Hidden parts
//!   (an overlay panel's X axes, title, grid, legend, colourbar) are
//!   canonical defaults carrying just their margin and transform fields, and
//!   each panel keeps only the picked references of its own series, so an
//!   edit of one layer leaves every other panel's config, and with it the
//!   renderer's per-panel raster cache, untouched.
//! - Expansion ignores `active_layer`; the panels always carry `Base`. The
//!   document's one annotation layer stays on the base panel, anchored in
//!   the parent's axes; overlay panels carry an empty one.
//!
//! Subplots (`Config::subplots`) are charts of their own. Each expands by
//! the same rules from its resolved config ([`Config::subplot_config`]: the
//! subplot's `config` with its linked axes following their targets), after
//! the parent's panels and in subplot order, so its panels share its own
//! chart area and data area, not the parent's. The document's picked
//! references of a subplot's series go to that subplot's panels, which split
//! them by the same rule.

use crate::config::{AxisOptions, Config, TickVisibility};
use crate::default::{
    default_axis_options_x, default_axis_options_y, default_chart_title_options,
    default_colorbar_options, default_grid_options, default_legend,
};
use crate::layer::{AnnotationLayer, CellRef, LayerRef};
use crate::layout::{DataArea, LayoutError, Margins, Rect, Side};

/// One internal panel of an expanded document.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandedPanel {
    /// The layer this panel draws: `Base` first, then `Overlay(i)` in order,
    /// then per subplot `Subplot(i)` and its `SubplotOverlay(i, j)`.
    pub layer: LayerRef,
    /// A single-layer config (no overlays, `active_layer == Base`) whose
    /// `data_area()` is the group data area.
    pub config: Config,
}

/// The panels of a document plus the data area they all share.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    /// Base panel first, then one panel per overlay in index order, then each
    /// subplot's panels in the same shape.
    pub panels: Vec<ExpandedPanel>,
    /// The group data area; equal to the parent's `data_area()`.
    pub data_area: DataArea,
}

impl Expansion {
    /// The panel of `layer`, or `None` for an overlay index past the end and
    /// for the annotation layer, which has no panel of its own.
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
    /// linked axis is one axis shared by its link group, so the X axis fits
    /// every series drawn against it: the series of every chart in its X link
    /// group, overlays included, since an overlay shares its chart's X axis.
    /// The Y axis fits the base-layer series of every chart in its Y link
    /// group; an overlay has a Y axis of its own. `Overlay(n)` and
    /// `SubplotOverlay(i, n)` fit only that overlay's Y axis, from that
    /// overlay's series.
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
        match layer {
            LayerRef::Base | LayerRef::Subplot(_) | LayerRef::Annotations => (
                root(cell(own), true) == root(cell(layer), true),
                matches!(own, LayerRef::Base | LayerRef::Subplot(_))
                    && root(cell(own), false) == root(cell(layer), false),
            ),
            LayerRef::Overlay(_) | LayerRef::SubplotOverlay(..) => (false, own == layer),
        }
    }

    /// The subplot rules: one level of nesting, one annotation layer per
    /// document (the parent's), links pointing at the parent or an earlier
    /// subplot, and every series id drawn by at most one subplot and no
    /// parent overlay. Each subplot's own overlays follow
    /// [`Self::validate_overlays`].
    pub fn validate_subplots(&self) -> Result<(), &'static str> {
        for (index, subplot) in self.subplots.iter().enumerate() {
            if !subplot.config.subplots.is_empty() {
                return Err("a subplot cannot have subplots of its own");
            }
            if !subplot.config.annotations.is_default() {
                return Err("the document's annotation layer belongs to the parent, not a subplot");
            }
            for link in [subplot.link_x, subplot.link_y].into_iter().flatten() {
                if let CellRef::Subplot(target) = link
                    && target >= index
                {
                    return Err("a subplot link must point at the parent or an earlier subplot");
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
                    return Err("a series id may belong to at most one subplot or overlay layer");
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
        config.annotations = AnnotationLayer::default();
        Some(config)
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
    pub fn validate_overlays(&self) -> Result<(), &'static str> {
        for (index, overlay) in self.overlays.iter().enumerate() {
            for (position, id) in overlay.series.iter().enumerate() {
                if overlay.series[..position].contains(id)
                    || self.overlays[..index].iter().any(|o| o.owns_series(id))
                {
                    return Err("a series id may belong to at most one overlay layer");
                }
            }
        }
        Ok(())
    }

    /// Expand the document into its internal panels. See the module
    /// documentation for the rules. Fails when the layout has no data area or
    /// when overlays share a series id.
    pub fn expand(&self) -> Result<Expansion, LayoutError> {
        self.validate_overlays()
            .map_err(|_| LayoutError::Infeasible)?;
        self.validate_subplots()
            .map_err(|_| LayoutError::Infeasible)?;
        let data_area = self.data_area()?;
        let group = self.margins();
        let mut panels = Vec::with_capacity(1 + self.overlays.len());
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
    /// computed without expanding.
    pub fn panel_layers(&self) -> Vec<LayerRef> {
        let mut layers = vec![LayerRef::Base];
        layers.extend((0..self.overlays.len()).map(LayerRef::Overlay));
        for (index, subplot) in self.subplots.iter().enumerate() {
            layers.push(LayerRef::Subplot(index));
            layers.extend(
                (0..subplot.config.overlays.len())
                    .map(|overlay| LayerRef::SubplotOverlay(index, overlay)),
            );
        }
        layers
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
        let overlay = &self.overlays[index];
        let mut panel = self.clone();
        panel.overlays.clear();
        panel.subplots.clear();
        panel.active_layer = LayerRef::Base;
        panel.annotations = AnnotationLayer::default();
        // The X range stays for the transform; the axes are drawn once, by
        // the base panel.
        panel.top_x = hidden_axis(&self.top_x, default_axis_options_x());
        panel.bottom_x = hidden_axis(&self.bottom_x, default_axis_options_x());
        // The hidden left axis carries the overlay's scale (the data
        // transform reads `left_y`) and the parent's left band, so the left
        // margin is unchanged.
        let mut left = hidden_axis(&overlay.y_axis, default_axis_options_y());
        left.out_margin = self.left_y.out_margin;
        left.major_tick_length = self.left_y.major_tick_length;
        panel.left_y = left;
        panel.right_y = overlay.y_axis.clone();
        let mut grid = default_grid_options();
        grid.show_major_x = false;
        grid.show_major_y = false;
        grid.show_minor_x = false;
        grid.show_minor_y = false;
        panel.grid = grid;
        let mut title = default_chart_title_options();
        title.visible = false;
        title.top_margin = self.chart_title.top_margin;
        panel.chart_title = title;
        panel.legend = match &overlay.legend {
            Some(legend) => legend.clone(),
            None => {
                let mut legend = default_legend();
                legend.visible = false;
                legend
            }
        };
        // The colourbar is drawn once, by the base panel; the overlay panel
        // keeps only the z mapping its field series read.
        panel.colorbar = self.colorbar.as_ref().map(|bar| {
            let mut hidden = default_colorbar_options();
            hidden.visible = false;
            hidden.axis.scale = bar.axis.scale.clone();
            hidden.axis.min = bar.axis.min;
            hidden.axis.max = bar.axis.max;
            hidden.colormap = bar.colormap.clone();
            hidden.nan_color = bar.nan_color;
            hidden
        });
        retain_layer_picks(&mut panel, self, LayerRef::Overlay(index));
        align_margins(&mut panel, group, data_area)?;
        Ok(panel)
    }
}

/// The linked part of an axis: range, scale, direction and tick spacing
/// (`major_spacing`, `minor_count`), so linked charts place the same ticks.
/// Line, label and title style stay each chart's own.
fn copy_axis_range(from: &AxisOptions, to: &mut AxisOptions) {
    to.scale = from.scale.clone();
    to.min = from.min;
    to.max = from.max;
    to.inverted = from.inverted;
    to.major_spacing = from.major_spacing;
    to.minor_count = from.minor_count;
}

/// Whether an axis keeps its range, scale and direction: what decides which
/// member of a link group was edited. Tick spacing is not compared, because
/// a range writer such as a view state re-derives it from an unchanged range;
/// it travels with the edited range instead.
fn same_axis_range(a: &AxisOptions, b: &AxisOptions) -> bool {
    a.scale == b.scale && a.min == b.min && a.max == b.max && a.inverted == b.inverted
}

/// A hidden axis: `template` (canonical defaults) carrying only `axis`'s
/// transform fields (scale, range, direction) and margin fields (outer margin,
/// major tick length), with its line, ticks, labels and title off.
fn hidden_axis(axis: &AxisOptions, template: AxisOptions) -> AxisOptions {
    AxisOptions {
        scale: axis.scale.clone(),
        min: axis.min,
        max: axis.max,
        inverted: axis.inverted,
        out_margin: axis.out_margin,
        major_tick_length: axis.major_tick_length,
        line_visible: false,
        tick: TickVisibility::None,
        ..template
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
        let owned = |id: &str| {
            matches!(
                self.layer_of_series(id),
                LayerRef::Subplot(i) | LayerRef::SubplotOverlay(i, _) if i == index
            )
        };
        if let Some(picked) = &self.picked_points {
            let mut picked = picked.clone();
            picked.refs.retain(|reference| owned(&reference.series_id));
            subplot.picked_points = Some(picked);
        }
        if let Some(picked) = &self.picked_data {
            let mut picked = picked.clone();
            picked.refs.retain(|reference| owned(reference.series_id()));
            subplot.picked_data = Some(picked);
        }
    }
}

/// Keep only the picked references of the series `layer` draws; the other
/// layers' selections are drawn by their own panels.
fn retain_layer_picks(panel: &mut Config, parent: &Config, layer: LayerRef) {
    if let Some(picked) = panel.picked_points.as_mut() {
        picked
            .refs
            .retain(|reference| parent.layer_of_series(&reference.series_id) == layer);
    }
    if let Some(picked) = panel.picked_data.as_mut() {
        picked
            .refs
            .retain(|reference| parent.layer_of_series(reference.series_id()) == layer);
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
    use crate::default::default_config;
    use crate::layer::OverlayLayer;
    use crate::layout::ChartArea;

    fn two_layer_config() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        });
        let mut overlay = OverlayLayer::default();
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
            ..Default::default()
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
        let mut cfg = default_config();
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

    // A panel's config depends only on what that panel draws: a base-layer
    // edit of hidden chrome leaves every overlay panel's config identical, an
    // overlay edit leaves the base panel identical, and each panel keeps the
    // picked references of its own series only.
    #[test]
    fn a_layer_edit_changes_only_that_layers_panel() {
        let mut cfg = two_layer_config();
        cfg.overlays.push(OverlayLayer {
            series: vec!["flow".into()],
            ..Default::default()
        });
        cfg.chart_title.visible = true;
        cfg.legend.visible = true;
        cfg.colorbar = Some(crate::default::default_colorbar_options());
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
        overlay_edit.overlays[0].legend = Some(crate::default::default_legend());
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
        picks.picked_points = Some(crate::config::PickedPointsConfig {
            refs: vec![
                crate::config::PickedPointRef {
                    source_id: None,
                    series_id: "temperature".into(),
                    point_index: 1,
                },
                crate::config::PickedPointRef {
                    source_id: None,
                    series_id: "pressure".into(),
                    point_index: 2,
                },
            ],
            ..Default::default()
        });
        let expansion = picks.expand().unwrap();
        let refs = |panel: &ExpandedPanel| {
            panel
                .config
                .picked_points
                .as_ref()
                .map(|picked| {
                    picked
                        .refs
                        .iter()
                        .map(|r| r.series_id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        assert_eq!(refs(&expansion.panels[0]), vec!["temperature".to_string()]);
        assert_eq!(refs(&expansion.panels[1]), vec!["pressure".to_string()]);
        assert!(refs(&expansion.panels[2]).is_empty());
        // A pick of an overlay series leaves the base panel's config alone.
        let mut base_only = cfg.clone();
        base_only.picked_points = Some(crate::config::PickedPointsConfig {
            refs: vec![crate::config::PickedPointRef {
                source_id: None,
                series_id: "pressure".into(),
                point_index: 2,
            }],
            ..Default::default()
        });
        let base_only = base_only.expand().unwrap();
        assert_eq!(
            base_only.panels[0].config.picked_points,
            Some(crate::config::PickedPointsConfig::default())
        );
        assert_eq!(
            base_only.panels[1].config.picked_points,
            picks.expand().unwrap().panels[1].config.picked_points
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
        let mut legend = crate::default::default_legend();
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
        cfg.colorbar = Some(crate::default::default_colorbar_options());
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
        use crate::layer::Subplot;
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 240,
        });
        cfg.bottom_x.min = 0.0;
        cfg.bottom_x.max = 24.0;
        let mut first = Subplot::default();
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
            ..Default::default()
        });
        let mut second = Subplot::default();
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
        nested.subplots[0].config.subplots.push(Default::default());
        assert!(nested.validate_subplots().is_err());
        let mut annotated = cfg.clone();
        annotated.subplots[0].config.annotations.visible = false;
        assert!(annotated.validate_subplots().is_err());
        assert!(annotated.expand().is_err());
        let mut parent_overlay = cfg.clone();
        parent_overlay.overlays.push(OverlayLayer {
            series: vec!["rate".into()],
            ..Default::default()
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
        let mut config = default_config();
        let mut overlay = OverlayLayer::default();
        overlay.series = vec!["po".into()];
        config.overlays.push(overlay);
        let mut first = default_config();
        let mut first_overlay = OverlayLayer::default();
        first_overlay.series = vec!["s0o".into()];
        first.overlays.push(first_overlay);
        config.subplots = vec![
            Subplot {
                config: first,
                series: vec!["s0".into(), "s0o".into()],
                ..Subplot::default()
            },
            Subplot {
                series: vec!["s1".into()],
                link_x: Some(CellRef::Parent),
                link_y: Some(CellRef::Subplot(0)),
                ..Subplot::default()
            },
            Subplot {
                series: vec!["s2".into()],
                link_x: Some(CellRef::Subplot(1)),
                ..Subplot::default()
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
        use crate::config::{
            DataSelectionsConfig, PickedDataRef, PickedPointRef, PickedPointsConfig,
        };
        let mut cfg = subplot_document();
        let point = |id: &str| PickedDataRef::Point {
            source_id: None,
            series_id: id.into(),
            point_index: 1,
        };
        cfg.picked_data = Some(DataSelectionsConfig {
            refs: ["temperature", "flow", "level", "rate"].map(point).to_vec(),
            ..Default::default()
        });
        cfg.picked_points = Some(PickedPointsConfig {
            refs: vec![PickedPointRef {
                source_id: None,
                series_id: "level".into(),
                point_index: 3,
            }],
            ..Default::default()
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
                vec!["level"],
                vec!["rate"]
            ]
        );
        let points: Vec<usize> = expansion
            .panels
            .iter()
            .map(|panel| panel.config.picked_points.as_ref().unwrap().refs.len())
            .collect();
        assert_eq!(points, [0, 0, 1, 0]);
        // Without a document selection a subplot keeps its own.
        let mut own = subplot_document();
        own.subplots[1].config.picked_data = Some(DataSelectionsConfig {
            refs: vec![point("rate")],
            ..Default::default()
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
        let plain = default_config();
        assert_eq!(plain.document_area(), plain.chart_area.0);
    }
}
