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

use crate::config::{AxisOptions, Config, TickVisibility};
use crate::default::{
    default_axis_options_x, default_axis_options_y, default_chart_title_options,
    default_colorbar_options, default_grid_options, default_legend,
};
use crate::layer::{AnnotationLayer, LayerRef};
use crate::layout::{DataArea, LayoutError, Margins, Side};

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

    /// Expand the document into its internal panels. See the module
    /// documentation for the rules. Fails when the layout has no data area or
    /// when overlays share a series id.
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
                config: self.overlay_panel_with(index, &group, &data_area)?,
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
        self.overlay_panel_with(index, &self.margins(), &data_area)
            .ok()
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
}
