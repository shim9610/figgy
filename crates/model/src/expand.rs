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
//! Layers with an area of their own (`OverlayLayer::chart_area` set to a
//! rectangle other than the parent's) are subplots. Layers with equal areas
//! form one area group drawn on top of each other there; the rules above
//! apply inside the group with its first layer (the lowest index) in the
//! parent's place:
//!
//! - The first layer's panel draws its `y_axis` as the left axis, the
//!   parent's grid and the parent's two X axes as they are written (the X
//!   range and style exist once, on the parent). It draws no title and no
//!   colourbar, and its top band carries no title margin.
//! - Each later layer of the group is drawn like an overlay of the parent:
//!   hidden X axes, a hidden left axis carrying its own range, its `y_axis`
//!   on the right.
//! - The group's margins are the X axes' bands, the first layer's Y band on
//!   the left and the widest of the parent's right band and the later
//!   layers' bands on the right, so every panel of the group shares one data
//!   area. A detached group never widens the parent's right band.

use crate::config::{AxisOptions, Config, TickVisibility};
use crate::default::{
    default_axis_options_x, default_axis_options_y, default_chart_title_options,
    default_colorbar_options, default_grid_options, default_legend,
};
use crate::layer::{AnnotationLayer, LayerRef};
use crate::layout::{ChartArea, DataArea, LayoutError, Margins, Rect, Side};

/// One internal panel of an expanded document.
#[derive(Clone, Debug, PartialEq)]
pub struct ExpandedPanel {
    /// The layer this panel draws: `Base` first, then `Overlay(i)` in order.
    pub layer: LayerRef,
    /// A single-layer config (no overlays, `active_layer == Base`) whose
    /// `data_area()` is the group data area.
    pub config: Config,
}

/// The panels of a document plus the data area they all share.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    /// Base panel first, then one panel per overlay in index order.
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

/// Where an overlay layer is drawn relative to the other layers of its area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayPlacement {
    /// On top of the parent's area; its axis is on the right.
    OnParent,
    /// The first layer of an area of its own: its axis is on the left and
    /// its panel draws the grid and the X axes of that area.
    AreaLead,
    /// A later layer of an area of its own, drawn on top of the area's first
    /// layer (`lead`); its axis is on the right.
    OnArea { lead: usize },
}

impl OverlayPlacement {
    /// The side its `y_axis` is drawn on.
    pub fn y_side(self) -> Side {
        match self {
            Self::AreaLead => Side::Left,
            Self::OnParent | Self::OnArea { .. } => Side::Right,
        }
    }
}

impl Config {
    /// The layer that receives pointer interaction, with the out-of-range
    /// fallback applied: an `Overlay(i)` with no `overlays[i]` reads as
    /// `Base`. The raw field is `self.active_layer`.
    pub fn active_layer(&self) -> LayerRef {
        match self.active_layer {
            LayerRef::Overlay(index) if index >= self.overlays.len() => LayerRef::Base,
            layer => layer,
        }
    }

    /// Set the interaction target layer. An `Overlay` index past the end of
    /// `overlays` is stored as `Base`. Returns the layer actually stored.
    pub fn set_active_layer(&mut self, layer: LayerRef) -> LayerRef {
        self.active_layer = match layer {
            LayerRef::Overlay(index) if index >= self.overlays.len() => LayerRef::Base,
            layer => layer,
        };
        self.active_layer
    }

    /// The layer whose Y axis scales `series_id`: the first overlay listing
    /// the id, else `Base`.
    pub fn layer_of_series(&self, series_id: &str) -> LayerRef {
        self.overlays
            .iter()
            .position(|overlay| overlay.owns_series(series_id))
            .map_or(LayerRef::Base, LayerRef::Overlay)
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

    /// The area overlay `index` is drawn in: its own `chart_area`, or the
    /// parent's when it has none. Panics when `index` is out of range.
    pub fn overlay_area(&self, index: usize) -> ChartArea {
        self.overlays[index]
            .chart_area
            .clone()
            .unwrap_or_else(|| self.chart_area.clone())
    }

    /// `true` when overlay `index` exists and is drawn in an area other than
    /// the parent's (a subplot).
    pub fn overlay_is_detached(&self, index: usize) -> bool {
        self.overlays
            .get(index)
            .and_then(|overlay| overlay.chart_area.as_ref())
            .is_some_and(|area| *area != self.chart_area)
    }

    /// Where overlay `index` is drawn, or `None` when it does not exist.
    pub fn overlay_placement(&self, index: usize) -> Option<OverlayPlacement> {
        let overlay = self.overlays.get(index)?;
        if !self.overlay_is_detached(index) {
            return Some(OverlayPlacement::OnParent);
        }
        let lead = self
            .overlays
            .iter()
            .position(|other| other.chart_area == overlay.chart_area)
            .unwrap_or(index);
        Some(if lead == index {
            OverlayPlacement::AreaLead
        } else {
            OverlayPlacement::OnArea { lead }
        })
    }

    /// The rectangle the whole document covers: the union of the parent's
    /// `chart_area` and every overlay's own area. Equal to `chart_area` for
    /// a document without subplots. Export renders this rectangle and a
    /// display fits it to the surface.
    pub fn document_area(&self) -> Rect {
        let mut union = self.chart_area.0;
        for area in self.overlays.iter().filter_map(|o| o.chart_area.as_ref()) {
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

    /// Move the parent's `chart_area` and every overlay's own area by
    /// `(dx, dy)` together, so their relative layout is kept. Coordinates
    /// saturate at zero; a document placed at the origin by
    /// [`Self::document_area`] never needs to.
    pub fn translate_areas(&mut self, dx: i64, dy: i64) {
        let shift = |rect: &mut Rect| {
            rect.x = (i64::from(rect.x) + dx).clamp(0, i64::from(u32::MAX)) as u32;
            rect.y = (i64::from(rect.y) + dy).clamp(0, i64::from(u32::MAX)) as u32;
        };
        shift(&mut self.chart_area.0);
        for area in self
            .overlays
            .iter_mut()
            .filter_map(|o| o.chart_area.as_mut())
        {
            shift(&mut area.0);
        }
    }

    /// The data area overlay `index` is drawn against: the parent's for a
    /// layer on the parent, else its area group's.
    pub fn overlay_data_area(&self, index: usize) -> Result<DataArea, LayoutError> {
        match self.overlay_placement(index) {
            None => Err(LayoutError::Infeasible),
            Some(OverlayPlacement::OnParent) => self.data_area(),
            Some(OverlayPlacement::AreaLead) => self.area_lead_panel(index)?.data_area(),
            Some(OverlayPlacement::OnArea { lead }) => self.area_lead_panel(lead)?.data_area(),
        }
    }

    /// Expand the document into its internal panels. See the module
    /// documentation for the rules. Fails when the layout or an area group
    /// has no data area or when overlays share a series id.
    pub fn expand(&self) -> Result<Expansion, LayoutError> {
        self.validate_overlays()
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
                config: self.overlay_panel_checked(index, &group, &data_area)?,
            });
        }
        Ok(Expansion { panels, data_area })
    }

    /// The internal panel of one overlay layer, or `None` when `index` is out
    /// of range or the layout has no data area. The hit map's overlay
    /// elements measure and drag against this panel.
    pub fn overlay_panel(&self, index: usize) -> Option<Config> {
        if index >= self.overlays.len() || self.validate_overlays().is_err() {
            return None;
        }
        let data_area = self.data_area().ok()?;
        self.overlay_panel_checked(index, &self.margins(), &data_area)
            .ok()
    }

    /// The panel of overlay `index` by its placement; `group` and `data_area`
    /// are the parent's and serve the layers on the parent.
    fn overlay_panel_checked(
        &self,
        index: usize,
        group: &Margins,
        data_area: &DataArea,
    ) -> Result<Config, LayoutError> {
        match self.overlay_placement(index) {
            None => Err(LayoutError::Infeasible),
            Some(OverlayPlacement::OnParent) => self.overlay_panel_with(index, group, data_area),
            Some(OverlayPlacement::AreaLead) => self.area_lead_panel(index),
            Some(OverlayPlacement::OnArea { lead }) => {
                let lead_panel = self.area_lead_panel(lead)?;
                let area_data = lead_panel.data_area()?;
                let mut panel = self.area_layer_panel(index, lead);
                align_margins(&mut panel, &lead_panel.margins(), &area_data)?;
                Ok(panel)
            }
        }
    }

    /// The right band of the area group led by `lead`: the widest of the
    /// parent's right band and every later member's band.
    fn area_right_band(&self, lead: usize) -> f32 {
        let band = |axis: &AxisOptions| axis.out_margin + axis.major_tick_length;
        let area = &self.overlays[lead].chart_area;
        self.overlays
            .iter()
            .enumerate()
            .skip(lead + 1)
            .filter(|(_, overlay)| overlay.chart_area == *area)
            .fold(band(&self.right_y), |widest, (_, overlay)| {
                widest.max(band(&overlay.y_axis))
            })
    }

    /// Panel of the first layer of an area group. Its margins are the
    /// group's by construction, so its `data_area()` is the group's.
    fn area_lead_panel(&self, index: usize) -> Result<Config, LayoutError> {
        let overlay = &self.overlays[index];
        let mut panel = self.area_panel_common(index);
        // The parent's X axes are drawn as written; the transform reads them.
        panel.top_x = self.top_x.clone();
        panel.bottom_x = self.bottom_x.clone();
        panel.left_y = overlay.y_axis.clone();
        // A hidden right axis carries only the group's right band.
        let mut right = hidden_axis(&overlay.y_axis, default_axis_options_y());
        right.major_tick_length = self.right_y.major_tick_length;
        right.out_margin = self.area_right_band(index) - right.major_tick_length;
        panel.right_y = right;
        panel.grid = self.grid.clone();
        retain_layer_picks(&mut panel, self, LayerRef::Overlay(index));
        panel.data_area()?;
        Ok(panel)
    }

    /// Panel of a later layer of the area group led by `lead`, before its
    /// margins are aligned to the group's.
    fn area_layer_panel(&self, index: usize, lead: usize) -> Config {
        let overlay = &self.overlays[index];
        let mut panel = self.area_panel_common(index);
        panel.top_x = hidden_axis(&self.top_x, default_axis_options_x());
        panel.bottom_x = hidden_axis(&self.bottom_x, default_axis_options_x());
        let mut left = hidden_axis(&overlay.y_axis, default_axis_options_y());
        left.out_margin = self.overlays[lead].y_axis.out_margin;
        left.major_tick_length = self.overlays[lead].y_axis.major_tick_length;
        panel.left_y = left;
        panel.right_y = overlay.y_axis.clone();
        panel.grid = hidden_grid();
        retain_layer_picks(&mut panel, self, LayerRef::Overlay(index));
        panel
    }

    /// What every panel of an area group shares: the layer's area, no
    /// overlays, no annotation layer, no title band, the layer's own legend
    /// and the z mapping without the strip.
    fn area_panel_common(&self, index: usize) -> Config {
        let overlay = &self.overlays[index];
        let mut panel = self.clone();
        panel.overlays.clear();
        panel.active_layer = LayerRef::Base;
        panel.annotations = AnnotationLayer::default();
        panel.chart_area = self.overlay_area(index);
        let mut title = default_chart_title_options();
        title.visible = false;
        title.top_margin = 0.0;
        panel.chart_title = title;
        panel.legend = layer_legend(overlay.legend.as_ref());
        panel.colorbar = self.colorbar.as_ref().map(hidden_colorbar);
        panel
    }

    fn base_panel(&self, group: &Margins, data_area: &DataArea) -> Result<Config, LayoutError> {
        let mut panel = self.clone();
        panel.overlays.clear();
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
        panel.grid = hidden_grid();
        let mut title = default_chart_title_options();
        title.visible = false;
        title.top_margin = self.chart_title.top_margin;
        panel.chart_title = title;
        panel.legend = layer_legend(overlay.legend.as_ref());
        // The colourbar is drawn once, by the base panel; the overlay panel
        // keeps only the z mapping its field series read.
        panel.colorbar = self.colorbar.as_ref().map(hidden_colorbar);
        retain_layer_picks(&mut panel, self, LayerRef::Overlay(index));
        align_margins(&mut panel, group, data_area)?;
        Ok(panel)
    }
}

/// A grid with every line off.
fn hidden_grid() -> crate::config::GridOptions {
    let mut grid = default_grid_options();
    grid.show_major_x = false;
    grid.show_major_y = false;
    grid.show_minor_x = false;
    grid.show_minor_y = false;
    grid
}

/// A layer's own legend, or a hidden canonical one.
fn layer_legend(legend: Option<&crate::legend::Legend>) -> crate::legend::Legend {
    match legend {
        Some(legend) => legend.clone(),
        None => {
            let mut legend = default_legend();
            legend.visible = false;
            legend
        }
    }
}

/// A hidden colourbar carrying only the z mapping field series read.
fn hidden_colorbar(bar: &crate::config::ColorBarOptions) -> crate::config::ColorBarOptions {
    let mut hidden = default_colorbar_options();
    hidden.visible = false;
    hidden.axis.scale = bar.axis.scale.clone();
    hidden.axis.min = bar.axis.min;
    hidden.axis.max = bar.axis.max;
    hidden.colormap = bar.colormap.clone();
    hidden.nan_color = bar.nan_color;
    hidden
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
    use crate::layout::{ChartArea, Rect};

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

    /// A parent in the top half of a 640x480 document, a subplot in the
    /// bottom half scaling `flow`, and a second layer on that subplot
    /// scaling `level`.
    fn subplot_config() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 640,
            height: 240,
        });
        let area = Some(ChartArea(Rect {
            x: 0,
            y: 240,
            width: 640,
            height: 240,
        }));
        let mut lead = OverlayLayer::default();
        lead.y_axis.min = 0.0;
        lead.y_axis.max = 10.0;
        lead.y_axis.out_margin = 70.0;
        lead.series = vec!["flow".into()];
        lead.chart_area = area.clone();
        cfg.overlays.push(lead);
        let mut twin = OverlayLayer::default();
        twin.y_axis.min = -1.0;
        twin.y_axis.max = 1.0;
        twin.y_axis.out_margin = 90.0;
        twin.series = vec!["level".into()];
        twin.chart_area = area;
        cfg.overlays.push(twin);
        cfg
    }

    #[test]
    fn placement_follows_the_layer_area() {
        let mut cfg = subplot_config();
        assert_eq!(cfg.overlay_placement(0), Some(OverlayPlacement::AreaLead));
        assert_eq!(
            cfg.overlay_placement(1),
            Some(OverlayPlacement::OnArea { lead: 0 })
        );
        assert_eq!(cfg.overlay_placement(2), None);
        assert_eq!(OverlayPlacement::AreaLead.y_side(), Side::Left);
        assert_eq!(OverlayPlacement::OnArea { lead: 0 }.y_side(), Side::Right);
        assert!(cfg.overlay_is_detached(0) && cfg.overlay_is_detached(1));
        assert!(!cfg.overlay_is_detached(2));
        // An area equal to the parent's is the parent's area.
        cfg.overlays[1].chart_area = Some(cfg.chart_area.clone());
        assert_eq!(cfg.overlay_placement(1), Some(OverlayPlacement::OnParent));
        assert_eq!(cfg.overlay_area(1), cfg.chart_area);
        cfg.overlays[1].chart_area = None;
        assert_eq!(cfg.overlay_placement(1), Some(OverlayPlacement::OnParent));
        assert_eq!(OverlayPlacement::OnParent.y_side(), Side::Right);
    }

    #[test]
    fn the_document_covers_every_area() {
        let mut cfg = subplot_config();
        assert_eq!(
            cfg.document_area(),
            Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            }
        );
        cfg.overlays[1].chart_area = Some(ChartArea(Rect {
            x: 700,
            y: 10,
            width: 100,
            height: 50,
        }));
        assert_eq!(
            cfg.document_area(),
            Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 480,
            }
        );
        cfg.translate_areas(5, 7);
        assert_eq!(cfg.chart_area.0.x, 5);
        assert_eq!(cfg.chart_area.0.y, 7);
        assert_eq!(cfg.overlay_area(0).0.y, 247);
        assert_eq!(cfg.overlay_area(1).0.x, 705);
        cfg.translate_areas(-100, -100);
        assert_eq!((cfg.chart_area.0.x, cfg.chart_area.0.y), (0, 0));
        // Without subplots the document is the chart area.
        let plain = default_config();
        assert_eq!(plain.document_area(), plain.chart_area.0);
    }

    // The area's first layer draws its own axis on the left, the parent's X
    // axes and grid; the later layer is an overlay of it. Both share the
    // area's data area, and the parent's band is not widened by them.
    #[test]
    fn an_area_group_shares_one_data_area_inside_its_area() {
        let cfg = subplot_config();
        for scale in [1.0_f32, 2.0, 0.5, 1.37] {
            let scaled = cfg.scaled(scale);
            let expansion = scaled.expand().unwrap();
            assert_eq!(expansion.panels.len(), 3);
            let lead = &expansion.panels[1].config;
            let twin = &expansion.panels[2].config;
            assert_eq!(lead.chart_area, scaled.overlay_area(0));
            assert_eq!(twin.chart_area, scaled.overlay_area(1));
            let area = scaled.overlay_data_area(0).unwrap();
            assert_eq!(lead.data_area().unwrap(), area, "scale {scale}");
            assert_eq!(twin.data_area().unwrap(), area, "scale {scale}");
            assert_eq!(scaled.overlay_data_area(1).unwrap(), area);
            let rect = scaled.overlay_area(0).0;
            assert!(area.y >= rect.y && area.y + area.height <= rect.y + rect.height);
            assert_eq!(
                expansion.panels[0].config.data_area().unwrap(),
                expansion.data_area
            );
        }
        let expansion = cfg.expand().unwrap();
        let base = &expansion.panels[0].config;
        let lead = &expansion.panels[1].config;
        let twin = &expansion.panels[2].config;
        // The parent's right band ignores the subplot's wider bands.
        assert_eq!(cfg.margins().right, {
            let mut alone = cfg.clone();
            alone.overlays.clear();
            alone.margins().right
        });
        assert!(base.overlays.is_empty());
        // Lead: own axis on the left, parent X axes and grid as written.
        assert_eq!(lead.left_y, cfg.overlays[0].y_axis);
        assert_eq!(lead.bottom_x, cfg.bottom_x);
        assert_eq!(lead.top_x, cfg.top_x);
        assert_eq!(lead.grid, cfg.grid);
        assert!(!lead.right_y.line_visible);
        assert!(!lead.chart_title.visible);
        assert_eq!(lead.chart_title.top_margin, 0.0);
        assert!(lead.annotations.items.is_empty());
        // Twin: hidden X axes, own range on a hidden left axis, own right axis.
        assert!(!twin.bottom_x.line_visible && !twin.top_x.line_visible);
        assert!(!twin.grid.show_major_x && !twin.grid.show_major_y);
        assert_eq!(twin.left_y.min, -1.0);
        assert!(!twin.left_y.line_visible);
        assert_eq!(twin.right_y, {
            let mut right = cfg.overlays[1].y_axis.clone();
            right.out_margin = twin.right_y.out_margin;
            right
        });
        // The group right band is the twin's wider band.
        assert_eq!(lead.margins().right, 95.0);
        assert_eq!(twin.margins().right, 95.0);
        assert_eq!(lead.margins().left, 75.0);
        assert_eq!(twin.margins().left, 75.0);
        assert_eq!(cfg.overlay_panel(0).as_ref(), Some(lead));
        assert_eq!(cfg.overlay_panel(1).as_ref(), Some(twin));
    }

    // A subplot edit leaves the parent's panel and the other area alone; an
    // X axis edit reaches every area, since the X axes exist once.
    #[test]
    fn an_area_edit_changes_only_that_areas_panels() {
        let mut cfg = subplot_config();
        let mut other = OverlayLayer::default();
        other.series = vec!["other".into()];
        cfg.overlays.push(other);
        let before = cfg.expand().unwrap();

        let mut lead_edit = cfg.clone();
        lead_edit.overlays[0].y_axis.line_color = crate::Color::new(1.0, 0.0, 0.0, 1.0);
        let after = lead_edit.expand().unwrap();
        assert_eq!(after.panels[0].config, before.panels[0].config);
        assert_ne!(after.panels[1].config, before.panels[1].config);
        assert_eq!(after.panels[2].config, before.panels[2].config);
        assert_eq!(after.panels[3].config, before.panels[3].config);

        let mut move_area = cfg.clone();
        move_area.overlays[0].chart_area.as_mut().unwrap().0.y = 260;
        move_area.overlays[1].chart_area.as_mut().unwrap().0.y = 260;
        let after = move_area.expand().unwrap();
        assert_eq!(after.panels[0].config, before.panels[0].config);
        assert_ne!(after.panels[1].config, before.panels[1].config);
        assert_ne!(after.panels[2].config, before.panels[2].config);
        assert_eq!(after.panels[3].config, before.panels[3].config);

        let mut x_edit = cfg.clone();
        x_edit.bottom_x.max = 3.0;
        let after = x_edit.expand().unwrap();
        for index in 0..4 {
            assert_ne!(after.panels[index].config, before.panels[index].config);
        }

        // The parent's title band does not reach a subplot.
        let mut title_edit = cfg.clone();
        title_edit.chart_title.top_margin = 40.0;
        let after = title_edit.expand().unwrap();
        assert_eq!(after.panels[1].config, before.panels[1].config);
        assert_eq!(after.panels[2].config, before.panels[2].config);
    }

    #[test]
    fn an_area_without_room_for_a_data_area_is_rejected() {
        let mut cfg = subplot_config();
        assert!(cfg.validate().is_ok());
        cfg.overlays[0].chart_area.as_mut().unwrap().0.height = 40;
        cfg.overlays[1].chart_area.as_mut().unwrap().0.height = 40;
        assert!(cfg.validate().is_err());
        assert!(cfg.expand().is_err());
        cfg.overlays[0].chart_area.as_mut().unwrap().0.height = 0;
        assert_eq!(cfg.validate(), Err(LayoutError::EmptyDataArea));
    }
}
