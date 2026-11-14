//! Display fitting, panel expansion and placement, and the panel helpers the prepare and paint steps share.

use super::*;

/// Uniformly fit a logical chart document into a physical surface.
pub fn fit_display_panel(logical_size: (u32, u32), surface_size: (u32, u32)) -> (f32, Rect) {
    let doc_w = logical_size.0.max(1) as f32;
    let doc_h = logical_size.1.max(1) as f32;
    let surface_w = surface_size.0.max(1);
    let surface_h = surface_size.1.max(1);
    let scale = ((surface_w as f32) / doc_w).min((surface_h as f32) / doc_h);
    let panel_w = ((doc_w * scale).round().max(1.0) as u32).min(surface_w);
    let panel_h = ((doc_h * scale).round().max(1.0) as u32).min(surface_h);
    let x = (surface_w - panel_w) / 2;
    let y = (surface_h - panel_h) / 2;
    (
        scale,
        Rect {
            x,
            y,
            width: panel_w,
            height: panel_h,
        },
    )
}

/// The annotation raster of a chart at its config's scale: the config
/// already carries export or display scaling in its pixel dims, exactly as
/// the axis raster is drawn, so the annotation raster is drawn at 1:1.
pub(in crate::renderer) fn annotation_raster_for_chart(
    chart: &Chart,
    selected: Option<usize>,
) -> Result<crate::annotation::AnnotationRaster> {
    let config = chart.config();
    let Config {
        chart_area,
        // The data area, the value axes and the polar switch reach the
        // raster through `pixel_space_for_config`, which reads the whole
        // config; the rest is not part of the annotation raster.
        top_x: _,
        bottom_x: _,
        left_y: _,
        right_y: _,
        chart_title: _,
        grid: _,
        legend: _,
        picked_data: _,
        colorbar: _,
        draw_style,
        coordinates: _,
        overlays: _,
        // Their markers are drawn on the layer (#205), through `config`.
        subplots: _,
        active_layer: _,
        annotations,
        bar_groups: _,
        // The members draw no annotations; the layer is anchored in the
        // base panel's axes.
        line_offsets: _,
        // The plot fill is under the grid, not the annotations (#203).
        plot_background: _,
        background: _,
    } = config;
    let area = chart_area.0;
    crate::annotation::rasterize(&crate::annotation::RasterRequest {
        layer: annotations,
        space: crate::annotation::pixel_space_for_config(config),
        origin: [area.x as f32, area.y as f32],
        size: (area.width.max(1), area.height.max(1)),
        scale: 1.0,
        selected,
        policy: crate::text_render::FontPolicy::for_style(draw_style),
        markers: Some(config),
    })
}

/// Derive the scaled display config and centered panel for a surface.
///
/// The whole document ([`Config::document_area`], the union of the parent's
/// area and every subplot's area) is fitted to the surface. The returned
/// rect is where the parent's `chart_area` lands, the `panel_rect` that
/// `create_chart_view` and `refresh_axis` take; for a document without
/// subplots it is the centered panel itself.
pub fn display_config_for_surface(
    config: &Config,
    surface_size: (u32, u32),
) -> (Config, Rect, f32) {
    let logical = config.document_area();
    let (scale, document_rect) = fit_display_panel((logical.width, logical.height), surface_size);
    let display_config = place_document(config, scale, document_rect);
    let panel_rect = display_config.chart_area.0;
    (display_config, panel_rect, scale)
}

/// `config` scaled by `scale` and moved so its parent's `chart_area` starts
/// at `origin` on the surface: a held display placement
/// ([`Renderer::hold_chart_display`], #205). Areas moved off the surface's
/// top or left edge stop at it.
pub(in crate::renderer) fn held_display_config(
    config: &Config,
    scale: f32,
    origin: [i64; 2],
) -> (Config, Rect, f32) {
    let mut placed = config.scaled(scale);
    let at = placed.chart_area.0;
    placed.translate_areas(origin[0] - i64::from(at.x), origin[1] - i64::from(at.y));
    let panel_rect = placed.chart_area.0;
    (placed, panel_rect, scale)
}

/// `config` scaled by `scale` and moved so its document area starts at
/// `document_rect`'s origin. A document without subplots is placed exactly
/// as before subplots existed: its `chart_area` becomes `document_rect`.
pub fn place_document(config: &Config, scale: f32, document_rect: Rect) -> Config {
    let mut placed = config.scaled(scale);
    if config.subplots.is_empty() {
        placed.chart_area = crate::layout::ChartArea(document_rect);
        return placed;
    }
    let document = placed.document_area();
    placed.translate_areas(
        i64::from(document_rect.x) - i64::from(document.x),
        i64::from(document_rect.y) - i64::from(document.y),
    );
    placed
}

/// Whether `config` draws as more than one panel: it has overlays,
/// subplots or line offset members.
pub(in crate::renderer) fn is_layered(config: &Config) -> bool {
    !config.overlays.is_empty() || !config.subplots.is_empty() || config.has_offset_members()
}

/// The panel that draws series `series_id` among the `count` panels
/// `config` drew as (`Config::panel_of_series`). A panel past them — a
/// document that drew as its base panel alone — is the base panel.
pub(crate) fn series_panel_index(config: &Config, count: usize, series_id: &str) -> usize {
    let index = config.panel_of_series(series_id);
    if index < count { index } else { 0 }
}

/// The panel that draws `layer` among the `count` panels `config` drew as
/// (`Config::panel_layers` order). A layer past them — after the fallback
/// to the base panel — is drawn by the base panel.
pub(in crate::renderer) fn panel_index(
    config: &Config,
    count: usize,
    layer: crate::LayerRef,
) -> usize {
    config
        .panel_layers()
        .iter()
        .take(count)
        .position(|panel| *panel == layer)
        .unwrap_or(0)
}

/// The internal panels a document draws as, base first. A document without
/// overlays and subplots is drawn exactly as it is written (no expansion, no
/// data-area requirement), so every existing single-layer chart is
/// unchanged; a layered document goes through `Config::expand`.
pub(crate) fn expand_for_render(config: &Config) -> Result<Vec<Config>> {
    validate_polar_config(config)?;
    for index in 0..config.subplots.len() {
        if let Some(subplot) = config.subplot_config(index) {
            validate_polar_config(&subplot)?;
        }
    }
    if !is_layered(config) {
        let mut panel = config.clone();
        panel.active_layer = crate::LayerRef::Base;
        return Ok(vec![panel]);
    }
    if config.validate_overlays().is_err() {
        return Err(FiggyError::InvalidConfig {
            field: "overlays",
            kind: crate::RuleKind::SeriesInManyOverlays,
        });
    }
    if let Err(reason) = config.validate_subplots() {
        return Err(FiggyError::InvalidConfig {
            field: "subplots",
            kind: reason,
        });
    }
    if let Err(reason) = config.validate_line_offset_layout() {
        return Err(FiggyError::InvalidConfig {
            field: "line_offsets",
            kind: reason,
        });
    }
    match config.expand() {
        Ok(expansion) => Ok(expansion
            .panels
            .into_iter()
            .map(|panel| panel.config)
            .collect()),
        // No data area at this size (a preview squeezed below its margins):
        // a single-layer chart draws its chrome and no data there, so a
        // layered one draws its base panel the same way instead of failing
        // the frame. The next larger size expands normally again.
        Err(
            crate::layout::LayoutError::OverflowHorizontal { .. }
            | crate::layout::LayoutError::OverflowVertical { .. }
            | crate::layout::LayoutError::EmptyDataArea,
        ) => {
            let mut panel = config.clone();
            panel.overlays.clear();
            panel.subplots.clear();
            panel.line_offsets = None;
            panel.active_layer = crate::LayerRef::Base;
            Ok(vec![panel])
        }
        Err(crate::layout::LayoutError::TargetOutOfChartArea) => Err(FiggyError::InvalidConfig {
            field: "subplots",
            kind: crate::RuleKind::LayerOutsideChart,
        }),
        Err(crate::layout::LayoutError::Infeasible) => Err(FiggyError::InvalidConfig {
            field: "overlays",
            kind: crate::RuleKind::NoFeasibleLayout,
        }),
    }
}

/// The view of each panel, base first. A view created or refreshed with
/// fewer overlays than the document now has cannot draw it; refresh first.
/// Line offset members count like overlays: each has a panel view of its
/// own in `overlay_views`.
pub(in crate::renderer) fn layer_views(view: &ChartView, panels: usize) -> Result<Vec<&ChartView>> {
    if view.overlay_views.len() + 1 < panels {
        return Err(FiggyError::StaleStateToken {
            reason: format!(
                "the chart view holds {} overlay or line offset panels but the document has \
                 {}; refresh the view before drawing",
                view.overlay_views.len(),
                panels - 1
            ),
        });
    }
    Ok(std::iter::once(view)
        .chain(view.overlay_views.iter())
        .take(panels)
        .collect())
}

/// The grid and decoration rasters of one panel, laid out once. An overlay
/// panel in the Milkyway style gets a transparent grid texture: the backdrop
/// is the base panel's. With `grid` false the grid buffer is empty: the
/// caller fills the grid texture from the cached backdrop instead.
pub(in crate::renderer) fn raster_panel_layers(
    config: &Config,
    overlay: bool,
    selection: &[crate::select::SelectionBox],
    grid: bool,
    scale: f32,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let milkyway = matches!(config.draw_style, DrawStyle::Milkyway(_));
    let (grid_rgba, dec_rgba) =
        axis_render::raster_panel_layers(config, selection, grid && !(overlay && milkyway), scale)?;
    if grid && overlay && milkyway {
        let (w, h) = (config.chart_area.0.width, config.chart_area.0.height);
        let len = (w as usize)
            .checked_mul(h as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(FiggyError::InvalidChartArea {
                width: w,
                height: h,
            })?;
        let mut blank = Vec::new();
        blank
            .try_reserve_exact(len)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "transparent overlay grid raster",
                reason: error.to_string(),
            })?;
        blank.resize(len, 0);
        return Ok((blank, dec_rgba));
    }
    Ok((grid_rgba, dec_rgba))
}

/// The texture size of a panel raster laid out from `config` and drawn at
/// `panel_rect`, checked before anything is rasterised: the raster is sized
/// from the config's chart area and the texture from the rect, so the two
/// must agree, and the texture must fit the device (#152).
pub(in crate::renderer) fn checked_panel_raster_size(
    caps: RendererDeviceCaps,
    config: &Config,
    panel_rect: Rect,
) -> Result<(u32, u32)> {
    let area = config.chart_area.0;
    if (area.width, area.height) != (panel_rect.width, panel_rect.height) {
        return Err(FiggyError::InvalidChartArea {
            width: panel_rect.width,
            height: panel_rect.height,
        });
    }
    validate_texture_extent(
        caps,
        "figgy panel texture dimension",
        panel_rect.width,
        panel_rect.height,
    )?;
    Ok((panel_rect.width, panel_rect.height))
}

/// Every panel's tick warnings as one list. An overlay panel reports only
/// the axes it draws, tagged with its layer: its own Y axis on the right
/// and, with `link_x` off, its own X axis on the top. Its other axes are
/// hidden copies and would repeat the base warnings.
pub(in crate::renderer) fn merged_render_warnings(
    config: &Config,
    panels: &[Config],
) -> Vec<crate::RenderWarning> {
    let layers = config.panel_layers();
    let mut merged = Vec::new();
    for (index, (panel, layer)) in panels.iter().zip(layers).enumerate() {
        // A line offset member's axes are hidden copies of its owner's.
        if config.panel_member(index).is_some() {
            continue;
        }
        for warning in axis_render::tick_warnings(panel) {
            let axis = warning.axis();
            let tagged = match (layer, axis) {
                (crate::LayerRef::Base, _) => Some(warning),
                (crate::LayerRef::Overlay(index), Some(crate::WarningAxis::RightY)) => {
                    Some(warning.with_axis(crate::WarningAxis::OverlayY(index)))
                }
                (crate::LayerRef::Overlay(index), Some(crate::WarningAxis::TopX))
                    if config.overlays.get(index).is_some_and(|o| !o.link_x) =>
                {
                    Some(warning.with_axis(crate::WarningAxis::OverlayX(index)))
                }
                // A subplot is a chart of its own: all of its own warnings,
                // tagged with the subplot.
                (crate::LayerRef::Subplot(index), Some(axis)) => {
                    crate::SubplotAxis::from_panel(axis)
                        .map(|axis| warning.with_axis(crate::WarningAxis::Subplot { index, axis }))
                }
                (
                    crate::LayerRef::SubplotOverlay(index, overlay),
                    Some(crate::WarningAxis::RightY),
                ) => Some(warning.with_axis(crate::WarningAxis::Subplot {
                    index,
                    axis: crate::SubplotAxis::OverlayY(overlay),
                })),
                (
                    crate::LayerRef::SubplotOverlay(index, overlay),
                    Some(crate::WarningAxis::TopX),
                ) if config
                    .overlay_of_layer(crate::LayerRef::SubplotOverlay(index, overlay))
                    .is_some_and(|o| !o.link_x) =>
                {
                    Some(warning.with_axis(crate::WarningAxis::Subplot {
                        index,
                        axis: crate::SubplotAxis::OverlayX(overlay),
                    }))
                }
                _ => None,
            };
            merged.extend(tagged);
        }
    }
    merged
}

/// Where one internal panel is drawn on the surface.
pub(in crate::renderer) struct PanelPlace {
    /// The panel's rectangle on the surface: the host's panel rect for the
    /// parent's panels, else the subplot's own area moved with the parent.
    pub(in crate::renderer) rect: Rect,
    /// The first panel of a chart: it owns the grid slot and the Milkyway
    /// backdrop there.
    pub(in crate::renderer) draws_backdrop: bool,
}

/// The place of every panel of `config` (as `expand_for_render` returned
/// them) when the parent's area is drawn at `panel_rect`.
pub(in crate::renderer) fn panel_places(
    config: &Config,
    panels: &[Config],
    panel_rect: Rect,
) -> Vec<PanelPlace> {
    let parent = config.chart_area.0;
    let dx = i64::from(panel_rect.x) - i64::from(parent.x);
    let dy = i64::from(panel_rect.y) - i64::from(parent.y);
    let shift = |value: u32, by: i64| (i64::from(value) + by).clamp(0, i64::from(u32::MAX)) as u32;
    panels
        .iter()
        .zip(config.panel_layers())
        .enumerate()
        .map(|(index, (panel, layer))| {
            // A line offset member draws over its owner's backdrop.
            let member = config.panel_member(index).is_some();
            match layer {
                crate::LayerRef::Subplot(_) | crate::LayerRef::SubplotOverlay(..) => {
                    let area = panel.chart_area.0;
                    PanelPlace {
                        rect: Rect {
                            x: shift(area.x, dx),
                            y: shift(area.y, dy),
                            width: area.width,
                            height: area.height,
                        },
                        draws_backdrop: matches!(layer, crate::LayerRef::Subplot(_)) && !member,
                    }
                }
                layer => PanelPlace {
                    rect: panel_rect,
                    draws_backdrop: layer == crate::LayerRef::Base && !member,
                },
            }
        })
        .collect()
}

/// Each selection box goes to the topmost panel whose rectangle holds the
/// box's centre; a box outside every panel goes to the topmost panel.
pub(in crate::renderer) fn route_selection_boxes(
    places: &[PanelPlace],
    selection: &[crate::select::SelectionBox],
) -> Vec<Vec<crate::select::SelectionBox>> {
    let mut routed = vec![Vec::new(); places.len()];
    for selection_box in selection {
        let (cx, cy) = selection_box.rect().center();
        let index = places
            .iter()
            .rposition(|place| crate::layout::RectF::from_rect(&place.rect).contains(cx, cy))
            .unwrap_or(places.len() - 1);
        routed[index].push(selection_box.clone());
    }
    routed
}

/// Panel `index`'s panel rect and data area, for
/// [`crate::layout::inset_paint_order`].
pub(super) fn panel_rects(prepared: &[PreparedItem], index: usize) -> (Rect, Rect) {
    prepared.get(index).map_or(
        (
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
        ),
        |item| (item.view.panel_rect, item.data_area),
    )
}

/// Whether panel `index` of a prepared frame is nested in another panel's
/// data area: its rect lies inside that data area and differs from that
/// panel's own rect (an inset subplot, #203). Overlay and member panels
/// share their owner's rect and are never nested.
pub(in crate::renderer) fn nested_panel(prepared: &[PreparedItem], index: usize) -> bool {
    crate::layout::inset_parent(prepared.len(), |i| panel_rects(prepared, i), index).is_some()
}

/// The document's annotation layer is rasterized once, on the base view; its
/// underlay stays under the base panel's data and its overlay moves to the
/// topmost panel drawn in the parent's area, so it sits above every layer's
/// decoration there. Panels in areas of their own are not its place: the
/// annotation textures cover the parent's area only.
pub(in crate::renderer) fn lift_annotation_overlays(
    items: &mut [PreparedItem],
    group_sizes: &[usize],
) {
    let mut start = 0;
    for &size in group_sizes {
        if size > 1 && start + size <= items.len() {
            let base_rect = items[start].view.panel_rect;
            let top = (start..start + size)
                .rev()
                .find(|&index| items[index].view.panel_rect == base_rect)
                .unwrap_or(start);
            if top != start {
                let overlay = items[start].view.annotation_overlay_bind_group.take();
                let visible = items[start].view.annotations_visible;
                let top = &mut items[top].view;
                top.annotation_overlay_bind_group = overlay;
                top.annotations_visible = visible;
            }
        }
        start += size;
    }
}

/// How many panels `config` draws as; see [`expand_for_render`].
pub(in crate::renderer) fn render_panel_count(config: &Config) -> Result<usize> {
    if !is_layered(config) {
        Ok(1)
    } else {
        Ok(expand_for_render(config)?.len())
    }
}
