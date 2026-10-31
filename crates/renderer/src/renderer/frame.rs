//! Frame assembly: chart views and their layer textures, the prepared frame
//! and its per-series products, resident display caches, panel placement for
//! layered documents, and the prepare, refresh and paint entry points.

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
pub(super) fn annotation_raster_for_chart(
    chart: &Chart,
    selected: Option<usize>,
) -> crate::annotation::AnnotationRaster {
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
        picked_points: _,
        picked_data: _,
        colorbar: _,
        draw_style,
        polar: _,
        ternary: _,
        view_3d: _,
        overlays: _,
        subplots: _,
        active_layer: _,
        annotations,
        bar_groups: _,
        // The members draw no annotations; the layer is anchored in the
        // base panel's axes.
        line_offsets: _,
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

/// Key + bytes of one baked constellation backdrop. `bake_gen` is a
/// monotonically increasing stamp; a `ChartView` whose grid texture already
/// holds this generation skips the (w·h·4)-byte re-upload too.
pub(super) struct SpaceBgCache {
    pub(super) key: SpaceBgKey,
    pub(super) bake_gen: u64,
    pub(super) rgba: Vec<u8>,
}

/// What the Milkyway backdrop is baked from. [`Self::bake`] reads the key
/// and nothing else, so a backdrop parameter that is not compared here does
/// not reach the raster.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct SpaceBgKey {
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) nebula: u32,
    pub(super) dust: u32,
    pub(super) seed: u32,
}

impl SpaceBgKey {
    fn bake(&self) -> Result<Vec<u8>> {
        let SpaceBgKey {
            w,
            h,
            nebula,
            dust,
            seed,
        } = *self;
        let Some(mut canvas) = crate::raster::Canvas::new(w, h) else {
            return Err(FiggyError::RasterWrapFailed {
                reason: format!("raster target allocation failed for {w}x{h}"),
            });
        };
        crate::axis_render::draw_space_background(
            &mut canvas,
            seed,
            f32::from_bits(nebula),
            f32::from_bits(dust),
        );
        Ok(canvas.into_rgba())
    }
}

/// Owned snapshot of one series' constellation/milkyway star pass, cloned
/// out of the arc scratch by the prepare phase. wgpu resources are
/// internally refcounted, so these clones keep the GPU objects alive (and
/// the draw phase cache-independent) even if `arc_cache` evicts or rebuilds
/// the entry before `paint_prepared` runs.
pub(super) struct PreparedStar {
    pub(super) indirect: wgpu::Buffer,
    pub(super) vs_bg: wgpu::BindGroup,
}

/// One series' arc-scan snapshot inside a token: the prefix buffer slice the
/// line binds and the star pass when the style has one. The frame's captured
/// column allocation epochs guard the source identity at paint time.
pub(super) struct PreparedArc {
    pub(super) prefix: ArcPrefix,
    pub(super) star: Option<PreparedStar>,
    pub(super) charge: crate::gpu_memory::SharedCharge,
}

pub(super) struct PreparedArcSeries {
    pub(super) arc: Option<PreparedArc>,
}

pub(super) struct PreparedArcItem {
    pub(super) view_revision: u64,
    pub(super) series: Vec<PreparedArcSeries>,
}

pub(super) struct PreparedContourScratch {
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) drawable: bool,
    pub(super) label: Option<crate::gpu_contour::ContourLabelSnapshot>,
}

pub(super) struct PreparedContourSeries {
    pub(super) scratch: Option<PreparedContourScratch>,
    /// The series' value label atlas, baked by the contour label module's
    /// atlas builder; `None` for a series that draws no value labels.
    pub(super) value_labels: Option<Arc<data_render::ValueLabelAtlas>>,
}

pub(super) struct PreparedContourItem {
    pub(super) series: Vec<PreparedContourSeries>,
}

pub(super) struct PreparedSeriesProducts<'a> {
    pub(super) arcs: &'a [PreparedArcSeries],
    pub(super) contours: &'a [PreparedContourSeries],
}

pub(super) struct PreparedAreaLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) x: ColumnHandle,
    pub(super) y: ColumnHandle,
    pub(super) base: ColumnHandle,
    pub(super) verts_per_instance: u32,
    pub(super) closed: bool,
}

pub(super) struct PreparedLineLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) x: ColumnHandle,
    pub(super) y: ColumnHandle,
    pub(super) arc: Option<ArcPrefix>,
    pub(super) verts_per_instance: u32,
    pub(super) texture_bg: Option<wgpu::BindGroup>,
    pub(super) closed: bool,
    /// A colour-mapped stroke: the value column and the groups 2 and 3 of
    /// the mapped line pipeline.
    pub(super) value_map: Option<(ColumnHandle, wgpu::BindGroup, wgpu::BindGroup)>,
    pub(super) third: Option<ColumnHandle>,
    pub(super) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

impl PreparedLineLayer {
    fn own(layer: ColumnLineLayer<'_>) -> Self {
        PreparedLineLayer {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg.clone(),
            pool_buffer: layer.pool_buffer.clone(),
            x: layer.x,
            y: layer.y,
            arc: layer.arc,
            verts_per_instance: layer.verts_per_instance,
            texture_bg: layer.texture_bg.cloned(),
            closed: layer.closed,
            value_map: layer
                .value_map
                .map(|map| (map.column, map.neutral_bg.clone(), map.value_map_bg.clone())),
            third: layer.third,
            view3d: layer.view3d,
        }
    }

    fn borrow(&self) -> ColumnLineLayer<'_> {
        ColumnLineLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: &self.style_bg,
            pool_buffer: &self.pool_buffer,
            x: self.x,
            y: self.y,
            arc: self.arc.clone(),
            verts_per_instance: self.verts_per_instance,
            texture_bg: self.texture_bg.as_ref(),
            closed: self.closed,
            value_map: self.value_map.as_ref().map(|(column, neutral, map)| {
                data_render::LineValueMap {
                    column: *column,
                    neutral_bg: neutral,
                    value_map_bg: map,
                }
            }),
            third: self.third,
            view3d: self.view3d.clone(),
        }
    }
}

pub(super) struct PreparedScatterLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) style_map_bg: Option<wgpu::BindGroup>,
    pub(super) value_map_bg: Option<wgpu::BindGroup>,
    pub(super) quad_vb: wgpu::Buffer,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) x: ColumnHandle,
    pub(super) y: ColumnHandle,
    pub(super) style_index: Option<ColumnHandle>,
    pub(super) color_value: Option<ColumnHandle>,
    pub(super) size_value: Option<ColumnHandle>,
    pub(super) texture_bg: Option<wgpu::BindGroup>,
    pub(super) third: Option<ColumnHandle>,
    pub(super) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

impl PreparedScatterLayer {
    fn own(layer: ColumnScatterLayer<'_>) -> Self {
        PreparedScatterLayer {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg.clone(),
            style_map_bg: layer.style_map_bg.cloned(),
            value_map_bg: layer.value_map_bg.cloned(),
            quad_vb: layer.quad_vb.clone(),
            pool_buffer: layer.pool_buffer.clone(),
            x: layer.x,
            y: layer.y,
            style_index: layer.style_index,
            color_value: layer.color_value,
            size_value: layer.size_value,
            texture_bg: layer.texture_bg.cloned(),
            third: layer.third,
            view3d: layer.view3d,
        }
    }

    fn borrow(&self) -> ColumnScatterLayer<'_> {
        ColumnScatterLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: &self.style_bg,
            style_map_bg: self.style_map_bg.as_ref(),
            value_map_bg: self.value_map_bg.as_ref(),
            quad_vb: &self.quad_vb,
            pool_buffer: &self.pool_buffer,
            x: self.x,
            y: self.y,
            style_index: self.style_index,
            color_value: self.color_value,
            size_value: self.size_value,
            texture_bg: self.texture_bg.as_ref(),
            third: self.third,
            view3d: self.view3d.clone(),
        }
    }
}

pub(super) struct PreparedPickRingLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) style_map_bg: Option<wgpu::BindGroup>,
    pub(super) value_map_bg: Option<wgpu::BindGroup>,
    pub(super) quad_vb: wgpu::Buffer,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) x: ColumnHandle,
    pub(super) y: ColumnHandle,
    pub(super) style_index: Option<ColumnHandle>,
    pub(super) color_value: Option<ColumnHandle>,
    pub(super) size_value: Option<ColumnHandle>,
    pub(super) instance: u32,
    pub(super) third: Option<ColumnHandle>,
    pub(super) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

pub(super) struct PreparedBarSelectionLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) style_map_bg: Option<wgpu::BindGroup>,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) edges: ColumnHandle,
    pub(super) values: ColumnHandle,
    pub(super) instance: u32,
    pub(super) instances: u32,
    pub(super) category: Option<data_render::CategoryBarData>,
}

pub(super) struct PreparedFieldSelectionLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) selection_bg: wgpu::BindGroup,
    pub(super) selection_charge: crate::gpu_memory::SharedCharge,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) drawable: bool,
}

pub(super) struct PreparedStarLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) texture_bg: wgpu::BindGroup,
    pub(super) star_bg: wgpu::BindGroup,
    pub(super) indirect: wgpu::Buffer,
}

/// Owned field layer. The grid table, level list, colourmap stops, lookup
/// metadata and `FieldParams` are all inside `field_bg`, which owns their
/// buffers — so the token needs nothing else to redraw the field.
pub(super) struct PreparedFieldLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) drawable: bool,
}

/// Owned surface layer (a heatmap on a 3D chart): refcounted clones of the
/// field's group 2 and the surface's group 3.
pub(super) struct PreparedSurfaceLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) surface_bg: wgpu::BindGroup,
    pub(super) quads: u32,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) surface_charge: crate::gpu_memory::SharedCharge,
}

pub(super) struct PreparedTernaryRegionLayer {
    pub(super) stencil: wgpu::RenderPipeline,
    pub(super) cover: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) region_bg: wgpu::BindGroup,
    pub(super) count: u32,
    pub(super) charge: crate::gpu_memory::SharedCharge,
}

impl PreparedTernaryRegionLayer {
    fn own(layer: data_render::ternary_region::ColumnTernaryRegionLayer<'_>) -> Self {
        Self {
            stencil: layer.stencil.clone(),
            cover: layer.cover.clone(),
            transform_bg: layer.transform_bg.clone(),
            region_bg: layer.region_bg,
            count: layer.count,
            charge: layer.charge,
        }
    }

    fn borrow(&self) -> data_render::ternary_region::ColumnTernaryRegionLayer<'_> {
        data_render::ternary_region::ColumnTernaryRegionLayer {
            stencil: &self.stencil,
            cover: &self.cover,
            transform_bg: &self.transform_bg,
            region_bg: self.region_bg.clone(),
            count: self.count,
            charge: self.charge.clone(),
        }
    }
}

impl PreparedSurfaceLayer {
    fn own(layer: data_render::surface_mesh::ColumnSurfaceLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg,
            field_bg: layer.field_bg,
            surface_bg: layer.surface_bg,
            quads: layer.quads,
            charge: layer.charge,
            surface_charge: layer.surface_charge,
        }
    }

    fn borrow(&self) -> data_render::surface_mesh::ColumnSurfaceLayer<'_> {
        data_render::surface_mesh::ColumnSurfaceLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: self.style_bg.clone(),
            field_bg: self.field_bg.clone(),
            surface_bg: self.surface_bg.clone(),
            quads: self.quads,
            charge: Arc::clone(&self.charge),
            surface_charge: Arc::clone(&self.surface_charge),
        }
    }
}

/// Owned vector layer: refcounted clones of the group-2 state, like the field.
pub(super) struct PreparedVectorLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) vector_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) count: u32,
    /// Streamline arrowheads: the indirect arguments and the arc slot group.
    pub(super) indirect: Option<(wgpu::Buffer, wgpu::BindGroup)>,
}

/// Owned contour layer. The group-2 tables, the lump charge and the label state
/// are refcounted, so the token draws from its own clones even after a later
/// prepare replaces the scratch they came from.
pub(super) struct PreparedContourLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) field_bg: wgpu::BindGroup,
    pub(super) charge: crate::gpu_memory::SharedCharge,
    pub(super) drawable: bool,
    pub(super) label: Option<PreparedContourLabel>,
}

/// Owned label layer. Same reasoning as the lines above: the pipeline, bind group
/// and buffers are refcounted clones, so a later prepare replacing the scratch
/// cannot pull them out from under a token that can still be painted.
pub(super) struct PreparedContourLabel {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

pub(super) struct PreparedBarLayer {
    pub(super) envelope: Option<data_render::bar_envelope::Snapshot>,
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) style_map_bg: Option<wgpu::BindGroup>,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) edges: ColumnHandle,
    pub(super) values: ColumnHandle,
}

/// Owned category bar layer: its data group and the charge of its params.
pub(super) struct PreparedCategoryBarLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) data: data_render::CategoryBarData,
    pub(super) count: u32,
    pub(super) vertices: u32,
}

impl PreparedCategoryBarLayer {
    fn new(layer: data_render::ColumnCategoryBarLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg.clone(),
            data: layer.data,
            count: layer.count,
            vertices: layer.vertices,
        }
    }

    fn layer(&self) -> data_render::ColumnCategoryBarLayer<'_> {
        data_render::ColumnCategoryBarLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: &self.style_bg,
            data: self.data.clone(),
            count: self.count,
            vertices: self.vertices,
        }
    }
}

/// Owned value label layer: the label group (uniform and atlas), the
/// category bar data it reads, and the selected label's box pipeline.
pub(super) struct PreparedValueLabelLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) labels: data_render::ValueLabelData,
    pub(super) data: data_render::CategoryBarData,
    pub(super) selected: Option<(u32, wgpu::RenderPipeline)>,
}

pub(super) struct PreparedErrorBarLayer {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) style_bg: wgpu::BindGroup,
    pub(super) style_map_bg: Option<wgpu::BindGroup>,
    pub(super) pool_buffer: wgpu::Buffer,
    pub(super) x: ColumnHandle,
    pub(super) y: ColumnHandle,
    pub(super) err_y_lo: ColumnHandle,
    pub(super) err_y_hi: ColumnHandle,
    pub(super) err_x_lo: ColumnHandle,
    pub(super) err_x_hi: ColumnHandle,
    pub(super) style_index: Option<ColumnHandle>,
}

/// Complete owned draw packet for one series. Every pipeline, bind group,
/// buffer, and column handle needed by paint is resolved during prepare.
pub(super) struct PreparedSeries {
    pub(super) _arc_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(super) _column_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(super) _style_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(super) _stream_style_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(super) _stream_transform_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(super) area: Option<PreparedAreaLayer>,
    pub(super) field: Option<PreparedFieldLayer>,
    pub(super) surface: Option<PreparedSurfaceLayer>,
    pub(super) floor: Option<PreparedSurfaceLayer>,
    pub(super) selected_surface: Option<PreparedSurfaceLayer>,
    pub(super) region: Option<PreparedTernaryRegionLayer>,
    pub(super) bar: Option<PreparedBarLayer>,
    pub(super) violin: Option<PreparedCategoryBarLayer>,
    pub(super) category_bars: Option<PreparedCategoryBarLayer>,
    pub(super) box_points: Option<PreparedCategoryBarLayer>,
    pub(super) contour: Option<PreparedContourLayer>,
    pub(super) vector: Option<PreparedVectorLayer>,
    pub(super) errorbar: Option<PreparedErrorBarLayer>,
    pub(super) line: Option<PreparedLineLayer>,
    pub(super) line_extra: Option<PreparedStarLayer>,
    pub(super) scatter: Option<PreparedScatterLayer>,
    pub(super) line_3d: Option<PreparedLineLayer>,
    pub(super) scatter_3d: Option<PreparedScatterLayer>,
    pub(super) selected_bars: Vec<PreparedBarSelectionLayer>,
    pub(super) selected_fields: Vec<PreparedFieldSelectionLayer>,
    pub(super) picked: Vec<PreparedPickRingLayer>,
    pub(super) value_labels: Option<PreparedValueLabelLayer>,
}

/// Refcounted GPU view bindings captured by `prepare`. Bind groups keep their
/// backing textures/buffers alive, so paint needs no `ChartView` borrow.
pub(super) struct PreparedView {
    pub(super) grid_bind_group: wgpu::BindGroup,
    pub(super) decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer's textures, `None` where the layer drew nothing
    /// on that side; skipped entirely while the layer is hidden.
    pub(super) annotation_underlay_bind_group: Option<wgpu::BindGroup>,
    pub(super) annotation_overlay_bind_group: Option<wgpu::BindGroup>,
    pub(super) annotations_visible: bool,
    pub(super) _grid_charge: crate::gpu_memory::SharedCharge,
    pub(super) _decoration_charge: crate::gpu_memory::SharedCharge,
    pub(super) _annotation_charges: Vec<crate::gpu_memory::SharedCharge>,
    pub(super) _transform_charge: crate::gpu_memory::SharedCharge,
    pub(super) content_revision: Arc<std::sync::atomic::AtomicU64>,
    pub(super) expected_content_revision: u64,
    pub(super) panel_rect: Rect,
}

impl PreparedView {
    /// Snapshot a view's bindings. `annotations_visible` comes from the
    /// chart config at prepare time, so toggling the layer needs no refresh.
    pub(super) fn capture(
        view: &ChartView,
        expected_content_revision: u64,
        annotations_visible: bool,
    ) -> Self {
        Self {
            grid_bind_group: view.grid_bind_group.clone(),
            decoration_bind_group: view.decoration_bind_group.clone(),
            annotation_underlay_bind_group: view
                .annotation_underlay
                .as_ref()
                .map(|l| l.bind_group.clone()),
            annotation_overlay_bind_group: view
                .annotation_overlay
                .as_ref()
                .map(|l| l.bind_group.clone()),
            annotations_visible,
            _grid_charge: view.grid_texture.shared_charge(),
            _decoration_charge: view.decoration_texture.shared_charge(),
            _annotation_charges: view
                .annotation_underlay
                .iter()
                .chain(view.annotation_overlay.iter())
                .map(|l| l.texture.shared_charge())
                .collect(),
            _transform_charge: view.transform_buffer.shared_charge(),
            content_revision: Arc::clone(&view.content_revision),
            expected_content_revision,
            panel_rect: view.panel_rect,
        }
    }

    pub(super) fn annotation_layers<'a>(
        &'a self,
        pipeline: &'a wgpu::RenderPipeline,
    ) -> (Option<AxisLayer<'a>>, Option<AxisLayer<'a>>) {
        if !self.annotations_visible {
            return (None, None);
        }
        let layer = |bind_group: &'a Option<wgpu::BindGroup>| {
            bind_group.as_ref().map(|bind_group| AxisLayer {
                pipeline,
                bind_group,
            })
        };
        (
            layer(&self.annotation_underlay_bind_group),
            layer(&self.annotation_overlay_bind_group),
        )
    }
}

/// Complete owned render input for one panel.
pub(super) struct PreparedItem {
    pub(super) view: PreparedView,
    pub(super) data_area: Rect,
    pub(super) axis_pipeline: wgpu::RenderPipeline,
    pub(super) series: Vec<PreparedSeries>,
    pub(super) stream_display: Option<streaming_runtime::StreamDisplayPacket>,
    /// The panel's cached resident composite, when the panel qualifies; see
    /// [`ResidentDisplay`].
    pub(super) resident_display: Option<ResidentDisplayPacket>,
    /// [`column_fingerprint`] of the columns this panel's series read through
    /// the pool lookup (grid columns come from the scratch receipts).
    pub(super) column_fingerprint: u64,
}

/// An order-independent fingerprint of `(column id, allocation epoch)` pairs:
/// it changes when any of the columns is replaced, removed and added again,
/// or relocated into a new allocation, and stays when an unrelated column
/// changes. A sum of per-pair hashes, so it needs no sorted copy.
pub(super) fn column_fingerprint<'a>(pairs: impl Iterator<Item = (&'a str, u64)>) -> u64 {
    use std::hash::{Hash, Hasher};
    pairs.fold(0u64, |sum, (id, epoch)| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        epoch.hash(&mut hasher);
        sum.wrapping_add(hasher.finish())
    })
}

/// One panel's cached series bodies: every fill and every line, marker,
/// bar, field, contour and vector body of the panel's series, drawn once into
/// a transparent panel-sized texture. `prepare` draws it again only when its
/// [`ResidentDisplayKey`] changes; `paint_prepared` blits it between the grid
/// and the live picked highlights, so a repaint costs one textured quad
/// instead of every point again.
///
/// The cache holds the data layer only. Grid, decoration, annotation layers
/// and the picked and selected highlights keep their own textures or draw
/// live, and the key reads nothing they depend on, so editing or selecting
/// any of them never draws the bodies again. Each panel of a layered document
/// has its own entry, so editing one layer leaves every other layer's bodies.
///
/// Only Precise panels qualify: their pipelines blend premultiplied over,
/// which composites the same through a transparent texture (up to UNORM
/// rounding at partly covered pixels, as for the stream display). Styled blend
/// equations (additive, max) depend on what is under them, so those panels
/// keep drawing directly.
pub(super) struct ResidentDisplay {
    /// The panel view's content-revision cell. Its address names the panel;
    /// once the view and every token holding it are gone, the entry retires.
    pub(super) view: std::sync::Weak<std::sync::atomic::AtomicU64>,
    pub(super) key: ResidentDisplayKey,
    pub(super) serial: u64,
    pub(super) format: wgpu::TextureFormat,
    pub(super) sample_count: u32,
    /// Resolved, panel-sized composite the blit samples.
    pub(super) texture: TrackedTexture,
    /// Multisampled target resolved into `texture`; `None` at 1x.
    pub(super) msaa: Option<TrackedTexture>,
    /// The depth-stencil attachment of a 3D panel's composite, at the
    /// target's sample count (`surface_mesh::DEPTH_STENCIL_FORMAT`); `None`
    /// for every flat panel, whose pipelines have no depth state.
    pub(super) depth: Option<TrackedTexture>,
    pub(super) bind_group: wgpu::BindGroup,
}

/// Everything the series bodies are drawn from, and nothing else: the
/// data-to-pixel transform, the parts of the chart configuration the body
/// pipelines read, each series' configuration and style scale, the font
/// generation (contour labels), the allocation epochs of exactly the columns
/// the panel reads, and the pool layout and pipeline generations. A column
/// another panel reads never draws this one again.
/// Titles, legend, axes, grid, annotations and selections are not in it.
pub(super) struct ResidentDisplayKey {
    pub(super) size: (u32, u32),
    /// The data area relative to the panel.
    pub(super) data_area: Rect,
    pub(super) transform: data_render::ScatterTransform,
    pub(super) draw_style: crate::config::DrawStyle,
    pub(super) polar: Option<crate::polar::PolarOptions>,
    pub(super) colorbar: Option<crate::config::ColorBarOptions>,
    /// How category bars share a slot (`Config::bar_groups`): group width,
    /// gap and stacking are drawn into the bodies.
    pub(super) bar_groups: Option<crate::data_config::BarGroups>,
    /// The panel's category axes (`bottom_x`, `left_y`) when it draws a
    /// category bar or box plot series, `None` otherwise: a bar or box
    /// override names its category by id, resolved to a row against these.
    pub(super) categories: Option<[Option<Vec<crate::config::Category>>; 2]>,
    /// The tick-label face: contour labels baked into the bodies take it
    /// (`ContourLabelAtlasInput::from_config`).
    pub(super) label_font: String,
    /// Each series' configuration and the display scale its style was built
    /// at: a style is a pure function of the two, so a host or the registry
    /// rebuilding every style after one series edit draws only the panels
    /// whose series changed.
    pub(super) series: Vec<(SeriesConfig, u32)>,
    pub(super) font_generation: u64,
    pub(super) columns: u64,
    pub(super) pool_layout_generation: u64,
    pub(super) target_pipeline_generation: u64,
    /// A 3D panel's camera uniform as the composite draws it: the camera,
    /// the fitted box (which the axes' label band sizes), the z axis range
    /// and the light. `None` for a flat panel.
    pub(super) camera: Option<data_render::surface_mesh::View3dGpu>,
    /// Whether the composite carries the stencil attachment: a ternary
    /// panel with a region fill. Its parameters are in `series`.
    pub(super) stencil: bool,
}

/// The current frame's inputs to compare against a [`ResidentDisplayKey`]
/// without copying them.
pub(super) struct ResidentDisplayProbe<'a> {
    pub(super) size: (u32, u32),
    pub(super) data_area: Rect,
    pub(super) transform: data_render::ScatterTransform,
    pub(super) chart_config: &'a Config,
    pub(super) series: &'a [Series<'a>],
    pub(super) font_generation: u64,
    pub(super) columns: u64,
    pub(super) pool_layout_generation: u64,
    pub(super) target_pipeline_generation: u64,
    pub(super) camera: Option<data_render::surface_mesh::View3dGpu>,
    pub(super) stencil: bool,
}

impl ResidentDisplayProbe<'_> {
    /// Whether the panel's composite needs the depth-stencil attachment.
    pub(super) fn needs_depth_stencil(&self) -> bool {
        self.camera.is_some() || self.stencil
    }
}

impl ResidentDisplayKey {
    pub(super) fn matches(&self, probe: &ResidentDisplayProbe<'_>) -> bool {
        self.size == probe.size
            && self.data_area == probe.data_area
            && bytemuck::bytes_of(&self.transform) == bytemuck::bytes_of(&probe.transform)
            && self.font_generation == probe.font_generation
            && self.columns == probe.columns
            && self.pool_layout_generation == probe.pool_layout_generation
            && self.target_pipeline_generation == probe.target_pipeline_generation
            && match (&self.camera, &probe.camera) {
                (Some(a), Some(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
                (None, None) => true,
                _ => false,
            }
            && self.stencil == probe.stencil
            && self.draw_style == probe.chart_config.draw_style
            && self.polar == probe.chart_config.polar
            && self.colorbar == probe.chart_config.colorbar
            && self.bar_groups == probe.chart_config.bar_groups
            && match &self.categories {
                Some([bottom, left]) => {
                    has_category_bars(probe.series)
                        && *bottom == probe.chart_config.bottom_x.categories
                        && *left == probe.chart_config.left_y.categories
                }
                None => !has_category_bars(probe.series),
            }
            && self.label_font == probe.chart_config.bottom_x.label_style.label_font
            && self.series.len() == probe.series.len()
            && self
                .series
                .iter()
                .zip(probe.series)
                .all(|((config, scale), now)| {
                    config == now.config && *scale == now.style.display_scale.to_bits()
                })
    }

    pub(super) fn from_probe(probe: &ResidentDisplayProbe<'_>) -> Self {
        Self {
            size: probe.size,
            data_area: probe.data_area,
            transform: probe.transform,
            draw_style: probe.chart_config.draw_style,
            polar: probe.chart_config.polar.clone(),
            colorbar: probe.chart_config.colorbar.clone(),
            bar_groups: probe.chart_config.bar_groups.clone(),
            categories: has_category_bars(probe.series).then(|| {
                [
                    probe.chart_config.bottom_x.categories.clone(),
                    probe.chart_config.left_y.categories.clone(),
                ]
            }),
            label_font: probe.chart_config.bottom_x.label_style.label_font.clone(),
            series: probe
                .series
                .iter()
                .map(|series| (series.config.clone(), series.style.display_scale.to_bits()))
                .collect(),
            font_generation: probe.font_generation,
            columns: probe.columns,
            pool_layout_generation: probe.pool_layout_generation,
            target_pipeline_generation: probe.target_pipeline_generation,
            camera: probe.camera,
            stencil: probe.stencil,
        }
    }
}

/// Whether any of the panel's series is a category bar or a box plot
/// series: their overrides name their category by id.
fn has_category_bars(series: &[Series<'_>]) -> bool {
    series.iter().any(|series| {
        let rt = &series.config.render_type;
        rt.category_bars().is_some() || rt.box_plot().is_some()
    })
}

/// A token's hold on one [`ResidentDisplay`]: the bind group to blit and the
/// serial it was drawn at, so a later re-draw makes an older token stale.
pub(super) struct ResidentDisplayPacket {
    /// Address of the view's content-revision cell, the entry's identity.
    pub(super) view: usize,
    pub(super) serial: u64,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) _charge: crate::gpu_memory::SharedCharge,
}

/// Token returned by [`Renderer::prepare`] and consumed (by reference,
/// repeatably) by [`Renderer::paint_prepared`].
///
/// Owned and `'static`: it captures the resolved pipelines, bind groups,
/// buffers, column handles, panel geometry, and prepare-only GPU products.
/// Hosts pass only this token to the immutable paint phase.
#[derive(Debug)]
pub struct PreparedFrame {
    /// Prevents a token from being recorded through a different renderer,
    /// even when both renderers happen to have matching pool generations.
    pub(super) renderer_identity: u64,
    pub(super) items: Vec<PreparedItem>,
    pub(super) column_sources: HashMap<ColumnId, u64>,
    pub(super) registered_charts: Vec<PreparedChartStamp>,
    /// Pool layout stamp — defrag, clear, or a buffer-relocating upsert can
    /// move column bytes or replace the backing buffer. Per-column allocation
    /// epochs above catch same-layout replacement of any captured draw input
    /// without invalidating a token for an unrelated column mutation.
    pub(super) pool_layout_generation: u64,
    /// Monotonic identity of the target pipeline set. This detects format
    /// ABA and sample-count changes, not just a different current format.
    pub(super) target_pipeline_generation: u64,
    /// One receipt per field or contour series: the grid columns it read and the
    /// epochs they had. `column_sources` cannot serve this — a matrix declares
    /// thousands of columns, and an owned map entry each would put a per-frame
    /// allocation per column on the prepare path.
    pub(super) grid_sources: Vec<Arc<GridSources>>,
}

#[derive(Debug)]
pub(super) struct PreparedChartStamp {
    pub(super) chart_id: ChartId,
    pub(super) desired: RenderRevision,
    pub(super) data: RenderRevision,
}

impl std::fmt::Debug for PreparedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedItem")
            .field("series", &self.series.len())
            .finish_non_exhaustive()
    }
}

impl PreparedSeries {
    pub(super) fn from_layers(
        layers: data_render::SeriesLayers<'_>,
        arc_charge: Option<crate::gpu_memory::SharedCharge>,
        style_charge: Option<crate::gpu_memory::SharedCharge>,
    ) -> Self {
        Self {
            _arc_charge: arc_charge,
            _column_charge: None,
            _style_charge: style_charge,
            _stream_style_charge: None,
            _stream_transform_charge: None,
            area: layers.area.map(|layer| PreparedAreaLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                base: layer.base,
                verts_per_instance: layer.verts_per_instance,
                closed: layer.closed,
            }),
            surface: layers.surface.map(PreparedSurfaceLayer::own),
            floor: layers.floor.map(PreparedSurfaceLayer::own),
            selected_surface: layers.selected_surface.map(PreparedSurfaceLayer::own),
            region: layers.region.map(PreparedTernaryRegionLayer::own),
            field: layers.field.map(|layer| PreparedFieldLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                field_bg: layer.field_bg,
                charge: layer.charge,
                drawable: layer.drawable,
            }),
            bar: layers.bar.map(|layer| PreparedBarLayer {
                envelope: layer.envelope,
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                style_map_bg: layer.style_map_bg.cloned(),
                pool_buffer: layer.pool_buffer.clone(),
                edges: layer.edges,
                values: layer.values,
            }),
            violin: layers.violin.map(PreparedCategoryBarLayer::new),
            category_bars: layers.category_bars.map(PreparedCategoryBarLayer::new),
            box_points: layers.box_points.map(PreparedCategoryBarLayer::new),
            vector: layers.vector.map(|layer| PreparedVectorLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                vector_bg: layer.vector_bg,
                charge: layer.charge,
                count: layer.count,
                indirect: layer
                    .indirect
                    .map(|(buffer, slots)| (buffer.clone(), slots.clone())),
            }),
            contour: layers.contour.map(|layer| PreparedContourLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg,
                field_bg: layer.field_bg,
                charge: layer.charge,
                drawable: layer.drawable,
                label: layer.label.map(|label| PreparedContourLabel {
                    pipeline: label.pipeline.clone(),
                    snapshot: label.snapshot,
                }),
            }),
            errorbar: layers.errorbar.map(|layer| PreparedErrorBarLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                style_map_bg: layer.style_map_bg.cloned(),
                pool_buffer: layer.pool_buffer.clone(),
                x: layer.x,
                y: layer.y,
                err_y_lo: layer.err_y_lo,
                err_y_hi: layer.err_y_hi,
                err_x_lo: layer.err_x_lo,
                err_x_hi: layer.err_x_hi,
                style_index: layer.style_index,
            }),
            line: layers.line.map(PreparedLineLayer::own),
            line_3d: layers.line_3d.map(PreparedLineLayer::own),
            scatter_3d: layers.scatter_3d.map(PreparedScatterLayer::own),
            line_extra: layers.line_extra.map(|layer| PreparedStarLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                style_bg: layer.style_bg.clone(),
                texture_bg: layer.texture_bg.clone(),
                star_bg: layer.star_bg.clone(),
                indirect: layer.indirect.clone(),
            }),
            scatter: layers.scatter.map(PreparedScatterLayer::own),
            selected_bars: layers
                .selected_bars
                .into_iter()
                .map(|layer| PreparedBarSelectionLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    style_bg: layer.style_bg,
                    style_map_bg: layer.style_map_bg.cloned(),
                    pool_buffer: layer.pool_buffer.clone(),
                    edges: layer.edges,
                    values: layer.values,
                    instance: layer.instance,
                    instances: layer.instances,
                    category: layer.category,
                })
                .collect(),
            selected_fields: layers
                .selected_fields
                .into_iter()
                .map(|layer| PreparedFieldSelectionLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    selection_bg: layer.selection_bg,
                    selection_charge: layer.selection_charge,
                    field_bg: layer.field_bg,
                    charge: layer.charge,
                    drawable: layer.drawable,
                })
                .collect(),
            picked: layers
                .picked
                .into_iter()
                .map(|layer| PreparedPickRingLayer {
                    pipeline: layer.pipeline.clone(),
                    transform_bg: layer.transform_bg.clone(),
                    style_bg: layer.style_bg,
                    style_map_bg: layer.style_map_bg.cloned(),
                    value_map_bg: layer.value_map_bg.cloned(),
                    quad_vb: layer.quad_vb.clone(),
                    pool_buffer: layer.pool_buffer.clone(),
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    color_value: layer.color_value,
                    size_value: layer.size_value,
                    instance: layer.instance,
                    third: layer.third,
                    view3d: layer.view3d,
                })
                .collect(),
            value_labels: layers.value_labels.map(|layer| PreparedValueLabelLayer {
                pipeline: layer.pipeline.clone(),
                transform_bg: layer.transform_bg.clone(),
                labels: layer.labels,
                data: layer.data,
                selected: layer
                    .selected
                    .map(|(row, pipeline)| (row, pipeline.clone())),
            }),
        }
    }

    pub(super) fn layers(&self) -> data_render::SeriesLayers<'_> {
        data_render::SeriesLayers {
            area: self
                .area
                .as_ref()
                .map(|layer| data_render::ColumnAreaLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: &layer.style_bg,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    base: layer.base,
                    verts_per_instance: layer.verts_per_instance,
                    closed: layer.closed,
                }),
            surface: self.surface.as_ref().map(PreparedSurfaceLayer::borrow),
            floor: self.floor.as_ref().map(PreparedSurfaceLayer::borrow),
            selected_surface: self
                .selected_surface
                .as_ref()
                .map(PreparedSurfaceLayer::borrow),
            region: self.region.as_ref().map(PreparedTernaryRegionLayer::borrow),
            field: self
                .field
                .as_ref()
                .map(|layer| data_render::ColumnFieldLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                }),
            bar: self.bar.as_ref().map(|layer| data_render::ColumnBarLayer {
                envelope: layer.envelope.clone(),
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                pool_buffer: &layer.pool_buffer,
                edges: layer.edges,
                values: layer.values,
            }),
            violin: self.violin.as_ref().map(PreparedCategoryBarLayer::layer),
            category_bars: self
                .category_bars
                .as_ref()
                .map(PreparedCategoryBarLayer::layer),
            box_points: self
                .box_points
                .as_ref()
                .map(PreparedCategoryBarLayer::layer),
            vector: self
                .vector
                .as_ref()
                .map(|layer| data_render::vector::ColumnVectorLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    vector_bg: layer.vector_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    count: layer.count,
                    indirect: layer
                        .indirect
                        .as_ref()
                        .map(|(buffer, slots)| (buffer, slots)),
                }),
            contour: self
                .contour
                .as_ref()
                .map(|layer| data_render::ColumnContourLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                    label: layer
                        .label
                        .as_ref()
                        .map(|label| data_render::ColumnContourLabel {
                            pipeline: &label.pipeline,
                            snapshot: label.snapshot.clone(),
                        }),
                }),
            errorbar: self.errorbar.as_ref().map(|layer| ColumnErrorBarDraw {
                pipeline: &layer.pipeline,
                transform_bg: &layer.transform_bg,
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                pool_buffer: &layer.pool_buffer,
                x: layer.x,
                y: layer.y,
                err_y_lo: layer.err_y_lo,
                err_y_hi: layer.err_y_hi,
                err_x_lo: layer.err_x_lo,
                err_x_hi: layer.err_x_hi,
                style_index: layer.style_index,
            }),
            line: self.line.as_ref().map(PreparedLineLayer::borrow),
            line_3d: self.line_3d.as_ref().map(PreparedLineLayer::borrow),
            scatter_3d: self.scatter_3d.as_ref().map(PreparedScatterLayer::borrow),
            line_extra: self
                .line_extra
                .as_ref()
                .map(|layer| data_render::ColumnStarLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: &layer.style_bg,
                    texture_bg: &layer.texture_bg,
                    star_bg: &layer.star_bg,
                    indirect: &layer.indirect,
                }),
            scatter: self.scatter.as_ref().map(PreparedScatterLayer::borrow),
            selected_bars: self
                .selected_bars
                .iter()
                .map(|layer| ColumnBarSelectionLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    style_map_bg: layer.style_map_bg.as_ref(),
                    pool_buffer: &layer.pool_buffer,
                    edges: layer.edges,
                    values: layer.values,
                    instance: layer.instance,
                    instances: layer.instances,
                    category: layer.category.clone(),
                })
                .collect(),
            selected_fields: self
                .selected_fields
                .iter()
                .map(|layer| ColumnFieldSelectionLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    selection_bg: layer.selection_bg.clone(),
                    selection_charge: Arc::clone(&layer.selection_charge),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                })
                .collect(),
            picked: self
                .picked
                .iter()
                .map(|layer| ColumnPickRingLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    style_bg: layer.style_bg.clone(),
                    style_map_bg: layer.style_map_bg.as_ref(),
                    value_map_bg: layer.value_map_bg.as_ref(),
                    quad_vb: &layer.quad_vb,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    style_index: layer.style_index,
                    color_value: layer.color_value,
                    size_value: layer.size_value,
                    instance: layer.instance,
                    third: layer.third,
                    view3d: layer.view3d.clone(),
                })
                .collect(),
            value_labels: self.value_labels.as_ref().map(|layer| {
                data_render::ColumnValueLabelLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: &layer.transform_bg,
                    labels: layer.labels.clone(),
                    data: layer.data.clone(),
                    selected: layer
                        .selected
                        .as_ref()
                        .map(|(row, pipeline)| (*row, pipeline)),
                }
            }),
        }
    }
}

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    pub(super) grid_texture: TrackedTexture,
    pub(super) grid_bind_group: wgpu::BindGroup,
    pub(super) decoration_texture: TrackedTexture,
    pub(super) decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer: spans below the data, everything else above
    /// every decoration. `None` where the layer drew nothing on that side.
    /// Refreshed by `refresh_axis*` (the anchor space may have moved) and by
    /// `refresh_annotations` (the items changed); never by anything else.
    pub(super) annotation_underlay: Option<crate::annotation::LayerTexture>,
    pub(super) annotation_overlay: Option<crate::annotation::LayerTexture>,
    pub(super) annotation_map: crate::annotation::AnnotationMap,
    /// The annotation whose selection box the overlay carries.
    pub(super) annotation_selected: Option<usize>,
    pub(super) layer_stamps: ChartViewLayerStamps,
    pub(super) transform_buffer: TrackedBuffer,
    pub(super) transform_bg: wgpu::BindGroup,
    pub(super) content_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Changes only when the streamed data prefix's coordinate space changes.
    /// Decoration texture rewrites deliberately leave this revision untouched.
    pub(super) stream_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Panel pixel rect in surface coordinates.
    pub(super) panel_rect: Rect,
    /// Generation of the renderer's cached constellation backdrop currently
    /// uploaded into `grid_texture` (`None` = non-cached content). Lets
    /// `refresh_axis` skip the full-panel texture write on cache hits.
    pub(super) grid_space_gen: Option<u64>,
    /// Tick-guard substitutions in the current axis raster, derived from the
    /// chart that `create_chart_view` or the last successful `refresh_axis*`
    /// rasterized. On a layered view this is the merged list of every panel,
    /// overlay warnings tagged `WarningAxis::OverlayY(index)`.
    pub(super) tick_warnings: Vec<crate::RenderWarning>,
    /// `tick_warnings` followed by the annotation layer's skipped items.
    pub(super) render_warnings: Vec<crate::RenderWarning>,
    /// One nested single-panel view per overlay layer, in `Config::overlays`
    /// order. Each holds its own grid and decoration textures and transform
    /// uniform; none nests further (one level of layering).
    pub(super) overlay_views: Vec<ChartView>,
    /// What this panel's textures hold: the internal panel config, the
    /// selection boxes composited into its decoration texture, the font
    /// generation and the panel rect of the last raster. A refresh that
    /// arrives with the same key uploads nothing, which is what keeps the
    /// other layers' textures untouched when one layer is edited.
    pub(super) raster_key: Option<PanelRasterKey>,
    /// Counts the rasters uploaded into this panel's textures.
    pub(super) raster_generation: u64,
}

/// Cache key of one panel's grid and decoration rasters.
#[derive(Clone, PartialEq)]
pub(super) struct PanelRasterKey {
    pub(super) config: Config,
    pub(super) selection: Vec<crate::select::SelectionBox>,
    pub(super) font_generation: u64,
    pub(super) panel_rect: Rect,
}

impl PanelRasterKey {
    /// The annotation layer has its own rasters and revision, so it is not
    /// part of the axis key: an annotation edit keeps every panel's cache.
    pub(super) fn new(
        config: &Config,
        selection: &[crate::select::SelectionBox],
        font_generation: u64,
        panel_rect: Rect,
    ) -> Self {
        let mut config = config.clone();
        config.annotations = crate::AnnotationLayer::default();
        Self {
            config,
            selection: selection.to_vec(),
            font_generation,
            panel_rect,
        }
    }
}

/// How many times each raster layer of a [`ChartView`] has been written,
/// starting at 1 for the view's creation. Lets a host or a test prove that a
/// refresh touched only the layer it was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartViewLayerStamps {
    pub grid: u64,
    pub decoration: u64,
    pub annotations: u64,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect {
        self.panel_rect
    }

    /// The number of overlay layers this view holds textures for, which is
    /// the overlay count of the config it was created or last refreshed with.
    pub fn overlay_count(&self) -> usize {
        self.overlay_views.len()
    }

    /// How many rasters have been uploaded into each panel's textures, base
    /// panel first and then one entry per overlay. An edit of one layer
    /// advances only that layer's entry (and the top panel's, when the
    /// selection boxes it carries change); a frame that reuses a panel's
    /// cached raster leaves its entry alone.
    pub fn layer_raster_generations(&self) -> Vec<u64> {
        std::iter::once(self.raster_generation)
            .chain(self.overlay_views.iter().map(|view| view.raster_generation))
            .collect()
    }

    /// Renderer-measured annotation hit map of the current annotation
    /// raster, in the chart's own coordinates (the chart area's frame).
    /// Rebuilt by `refresh_axis*` and `refresh_annotations`; a `HitId` from
    /// it is valid until the next of those calls.
    pub fn annotation_map(&self) -> &crate::annotation::AnnotationMap {
        &self.annotation_map
    }

    /// The pointer route of the active layer at `position`: while
    /// `config.active_layer()` is `Annotations` (and the layer is shown) the
    /// annotation items of this view sit on top of `hit_map`, otherwise only
    /// `hit_map` answers and clicks pass through the items. `config` is the
    /// config the view was last refreshed with (the display config of a
    /// scaled view) and `hit_map` its [`crate::HitMap::for_active_layer`] map, so
    /// both maps measure in the same pixels.
    pub fn hit_test_active_layer(
        &self,
        config: &Config,
        hit_map: &crate::HitMap,
        position: [f32; 2],
    ) -> Option<crate::annotation::LayerHit> {
        self.annotation_map
            .route(config.active_layer(), &config.annotations, position, || {
                hit_map.hit_test(
                    config,
                    &crate::CpuTextMeasure::for_style(&config.draw_style),
                    position[0],
                    position[1],
                )
            })
    }

    /// The annotation whose selection box the overlay currently carries, as
    /// last passed to [`Renderer::refresh_annotations`].
    pub fn annotation_selected(&self) -> Option<usize> {
        self.annotation_selected
    }

    pub fn layer_stamps(&self) -> ChartViewLayerStamps {
        self.layer_stamps
    }

    pub(super) fn rebuild_render_warnings(&mut self) {
        self.render_warnings = self
            .tick_warnings
            .iter()
            .chain(self.annotation_map.warnings())
            .cloned()
            .collect();
    }

    /// Bind groups of the annotation textures for the stream display
    /// composition, which draws underlay and overlay itself.
    pub(crate) fn annotation_bind_groups(
        &self,
        visible: bool,
    ) -> (Option<&wgpu::BindGroup>, Option<&wgpu::BindGroup>) {
        if !visible {
            return (None, None);
        }
        (
            self.annotation_underlay.as_ref().map(|l| &l.bind_group),
            self.annotation_overlay.as_ref().map(|l| &l.bind_group),
        )
    }

    /// The tick settings this view's current axis raster drew differently
    /// from how its chart's config wrote them: an impossible `major_spacing`
    /// drawn at automatic spacing, a clamped `minor_count`, an adjusted log
    /// decade step. Empty when every setting is drawn as written.
    ///
    /// [`Renderer::create_chart_view`] sets it, and every successful
    /// [`Renderer::refresh_axis`] / [`Renderer::refresh_axis_with_selection`]
    /// re-derives it from the chart it rasterized; nothing else changes it.
    /// A host that refreshes only when its dirty flag or
    /// [`ChartRenderStamp::needs_raster_since`] says the raster changed reads
    /// it right after that refresh. Compare with the previous list to report
    /// changes only; a NaN `requested` never compares equal.
    ///
    /// The tick warnings are followed by the annotation layer's
    /// `AnnotationSkipped` warnings, re-derived by `refresh_annotations` and
    /// by every `refresh_axis*` (the latter redoes the annotation raster).
    pub fn render_warnings(&self) -> &[crate::RenderWarning] {
        &self.render_warnings
    }

    pub(super) fn advance_content_revision(&self) -> Result<u64> {
        self.content_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view content revision",
            })
    }

    pub(super) fn advance_stream_revision(&self) -> Result<u64> {
        self.stream_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view stream revision",
            })
    }
}

/// Whether `config` draws as more than one panel: it has overlays,
/// subplots or line offset members.
pub(super) fn is_layered(config: &Config) -> bool {
    !config.overlays.is_empty() || !config.subplots.is_empty() || config.has_offset_members()
}

/// The panel that draws series `series_id` among the `count` panels
/// `config` drew as (`Config::panel_of_series`). A panel past them — a
/// document that drew as its base panel alone — is the base panel.
pub(super) fn series_panel_index(config: &Config, count: usize, series_id: &str) -> usize {
    let index = config.panel_of_series(series_id);
    if index < count { index } else { 0 }
}

/// The panel that draws `layer` among the `count` panels `config` drew as
/// (`Config::panel_layers` order). A layer past them — after the fallback
/// to the base panel — is drawn by the base panel.
pub(super) fn panel_index(config: &Config, count: usize, layer: crate::LayerRef) -> usize {
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
            reason: "a series id may belong to at most one overlay layer",
        });
    }
    if let Err(reason) = config.validate_subplots() {
        return Err(FiggyError::InvalidConfig {
            field: "subplots",
            reason,
        });
    }
    if let Err(reason) = config.validate_line_offset_layout() {
        return Err(FiggyError::InvalidConfig {
            field: "line_offsets",
            reason,
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
        Err(_) => {
            let mut panel = config.clone();
            panel.overlays.clear();
            panel.subplots.clear();
            panel.line_offsets = None;
            panel.active_layer = crate::LayerRef::Base;
            Ok(vec![panel])
        }
    }
}

/// The view of each panel, base first. A view created or refreshed with
/// fewer overlays than the document now has cannot draw it; refresh first.
/// Line offset members count like overlays: each has a panel view of its
/// own in `overlay_views`.
pub(super) fn layer_views(view: &ChartView, panels: usize) -> Result<Vec<&ChartView>> {
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
pub(super) fn raster_panel_layers(
    config: &Config,
    overlay: bool,
    selection: &[crate::select::SelectionBox],
    grid: bool,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let milkyway = matches!(config.draw_style, DrawStyle::Milkyway(_));
    let (grid_rgba, dec_rgba) =
        axis_render::raster_panel_layers(config, selection, grid && !(overlay && milkyway))?;
    if grid && overlay && milkyway {
        let (w, h) = (config.chart_area.0.width, config.chart_area.0.height);
        return Ok((vec![0; (w as usize) * (h as usize) * 4], dec_rgba));
    }
    Ok((grid_rgba, dec_rgba))
}

/// Every panel's tick warnings as one list. An overlay panel reports only
/// its own Y axis (right, or left for the first layer of an area of its
/// own), tagged with its layer; its other axes are copies of the base
/// panel's and would repeat the base warnings.
pub(super) fn merged_render_warnings(
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
                _ => None,
            };
            merged.extend(tagged);
        }
    }
    merged
}

/// Where one internal panel is drawn on the surface.
pub(super) struct PanelPlace {
    /// The panel's rectangle on the surface: the host's panel rect for the
    /// parent's panels, else the subplot's own area moved with the parent.
    pub(super) rect: Rect,
    /// The first panel of a chart: it owns the grid slot and the Milkyway
    /// backdrop there.
    pub(super) draws_backdrop: bool,
}

/// The place of every panel of `config` (as `expand_for_render` returned
/// them) when the parent's area is drawn at `panel_rect`.
pub(super) fn panel_places(
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
pub(super) fn route_selection_boxes(
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

/// The document's annotation layer is rasterized once, on the base view; its
/// underlay stays under the base panel's data and its overlay moves to the
/// topmost panel drawn in the parent's area, so it sits above every layer's
/// decoration there. Panels in areas of their own are not its place: the
/// annotation textures cover the parent's area only.
pub(super) fn lift_annotation_overlays(items: &mut [PreparedItem], group_sizes: &[usize]) {
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
pub(super) fn render_panel_count(config: &Config) -> usize {
    if !is_layered(config) {
        1
    } else {
        expand_for_render(config).map_or(1, |panels| panels.len())
    }
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

/// Prepare-only resources for an explicitly identified renderer-owned chart.
/// Config and ordered series are borrowed from the Renderer registry.
pub struct RegisteredChartDrawItem<'a> {
    pub chart_id: ChartId,
    pub view: &'a ChartView,
}

impl Renderer {
    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly.
    pub fn create_chart_view(&self, chart: &Chart, panel_rect: Rect) -> Result<ChartView> {
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        let mut view = self.create_panel_view(&panels[0], panel_rect, false, font_generation)?;
        for (panel, place) in panels[1..].iter().zip(&places[1..]) {
            view.overlay_views.push(self.create_panel_view(
                panel,
                place.rect,
                !place.draws_backdrop,
                font_generation,
            )?);
        }
        // Annotation layer: two optional textures from one shared raster
        // pass. One layer per document, on the base view, anchored in the
        // parent's own axis space (the group data area).
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let annotations = annotation_raster_for_chart(chart, None);
        view.annotation_underlay = annotations
            .underlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_overlay = annotations
            .overlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_map = annotations.map;
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        view.rebuild_render_warnings();
        Ok(view)
    }

    /// One panel's grid, decoration and transform resources from one
    /// internal config. `overlay` panels never draw the Milkyway backdrop:
    /// the grid slot belongs to the first panel of their area. No panel carries annotation
    /// textures here; `create_chart_view` gives them to the base view.
    pub(super) fn create_panel_view(
        &self,
        config: &Config,
        panel_rect: Rect,
        overlay: bool,
        font_generation: u64,
    ) -> Result<ChartView> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        // Both raster layers from one placement pass: the grid (drawn below
        // data) and the decoration (drawn above). An overlay panel in the
        // Milkyway style gets a transparent grid: the backdrop is the base
        // panel's.
        let (grid_rgba, dec_rgba) = raster_panel_layers(config, overlay, &[], true)?;
        let grid = self.create_layer_texture(&grid_rgba, w, h)?;
        let decoration = self.create_layer_texture(&dec_rgba, w, h)?;

        let t = data_render::scatter_transform_from_config(config);
        let transform_buffer = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_scatter_transform_uniform_buffer(&self.device, &t),
        );
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid.texture,
            grid_bind_group: grid.bind_group,
            decoration_texture: decoration.texture,
            decoration_bind_group: decoration.bind_group,
            annotation_underlay: None,
            annotation_overlay: None,
            annotation_map: crate::annotation::AnnotationMap::empty(
                crate::annotation::pixel_space_for_config(config),
            ),
            annotation_selected: None,
            layer_stamps: ChartViewLayerStamps {
                grid: 1,
                decoration: 1,
                annotations: 1,
            },
            transform_buffer,
            transform_bg,
            content_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            stream_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            panel_rect,
            grid_space_gen: None,
            tick_warnings: axis_render::tick_warnings(config),
            render_warnings: Vec::new(),
            overlay_views: Vec::new(),
            raster_key: Some(PanelRasterKey::new(
                config,
                &[],
                font_generation,
                panel_rect,
            )),
            raster_generation: 1,
        })
    }

    /// Upload one panel-sized raster as a texture with its bind group.
    pub(super) fn create_layer_texture(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<crate::annotation::LayerTexture> {
        let texture = TrackedTexture::new(
            &self.gpu_ledger,
            GpuResourceKind::PanelTexture,
            data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = data_render::create_texture_bind_group(
            &self.device,
            &self.texture_bgl,
            &view,
            &self.sampler,
        );
        Ok(crate::annotation::LayerTexture {
            texture,
            bind_group,
        })
    }

    /// Re-rasterize only the annotation layer of `view` from `chart`:
    /// the underlay and overlay textures, the annotation hit map and the
    /// annotation warnings. The grid and decoration textures are not
    /// touched, so an annotation edit never rewrites axis labels. `selected`
    /// is the annotation whose selection box the overlay draws; the view
    /// remembers it for the next `refresh_axis*`.
    ///
    /// Call it when [`ChartRenderStamp::needs_annotation_raster_since`] says
    /// the items changed or when the annotation selection changed. A panel
    /// rect that differs from the view's moves the anchor space as well, so
    /// that case refreshes every layer like [`Self::refresh_axis`].
    pub fn refresh_annotations(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        view.annotation_selected = selected;
        if view.panel_rect != panel_rect {
            return self.refresh_axis(view, chart, panel_rect);
        }
        view.advance_content_revision()?;
        self.refresh_annotation_layers(view, chart, panel_rect)
    }

    pub(super) fn refresh_annotation_layers(
        &self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let raster = annotation_raster_for_chart(chart, view.annotation_selected);
        self.refresh_optional_layer(
            &mut view.annotation_underlay,
            raster.underlay.as_deref(),
            w,
            h,
        )?;
        self.refresh_optional_layer(
            &mut view.annotation_overlay,
            raster.overlay.as_deref(),
            w,
            h,
        )?;
        view.annotation_map = raster.map;
        view.layer_stamps.annotations += 1;
        view.rebuild_render_warnings();
        Ok(())
    }

    pub(super) fn refresh_optional_layer(
        &self,
        slot: &mut Option<crate::annotation::LayerTexture>,
        rgba: Option<&[u8]>,
        w: u32,
        h: u32,
    ) -> Result<()> {
        match (slot.as_mut(), rgba) {
            (Some(layer), Some(rgba)) => {
                self.refresh_one_layer(&mut layer.texture, &mut layer.bind_group, rgba, w, h)
            }
            (None, Some(rgba)) => {
                *slot = Some(self.create_layer_texture(rgba, w, h)?);
                Ok(())
            }
            (_, None) => {
                *slot = None;
                Ok(())
            }
        }
    }

    /// Write only the data transform uniform; leave axis/grid/decorations unchanged.
    ///
    /// An axis-range edit normally needs [`Self::refresh_axis`] as well to update
    /// tick labels and the grid. In the prepare/paint flow, refresh those raster
    /// layers and let [`Self::prepare`] write the transform from the same config;
    /// no separate call to this method is needed.
    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        let panels = expand_for_render(chart.config())?;
        let views = layer_views(view, panels.len())?;
        view.advance_stream_revision()?;
        view.advance_content_revision()?;
        for (panel, layer_view) in panels.iter().zip(views) {
            let t = data_render::scatter_transform_from_config(panel);
            data_render::update_scatter_transform(&self.queue, &layer_view.transform_buffer, &t);
        }
        Ok(())
    }

    /// Re-rasterize both grid and decoration textures. Updates in place via
    /// `write_texture` when the size matches; allocates new textures
    /// otherwise. The data transform uniform needs no separate call in the
    /// prepare/paint flow — [`Self::prepare`] (and the `paint`/`draw`
    /// facades) writes it every frame; [`Self::update_transform`] remains
    /// for imperative one-off updates outside that flow.
    ///
    /// `&mut self`: the constellation backdrop cache (`space_bg`) lives on
    /// the renderer and may be (re)baked here — same prepare-phase mutability
    /// rule as `paint`.
    pub fn refresh_axis(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.refresh_axis_with_selection(view, chart, panel_rect, None)
    }

    /// [`Self::refresh_axis`] plus the selection highlight of the element
    /// `selected` names composited into the decoration layer (selection draws
    /// above the data, never below). `selected` is the element id the hit
    /// map reports (`Selectable::element_id`, `"axis_bottom"`,
    /// `"overlay_0:legend"`, ...); the box is built from the element's drawn
    /// parts on the document's active layer, the same box
    /// [`Self::selection_box`] returns for host UI. An id no element of the
    /// active layer has draws nothing.
    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<&str>,
    ) -> Result<()> {
        let selection: Vec<crate::select::SelectionBox> = selected
            .and_then(|id| self.selection_box(chart, id))
            .into_iter()
            .collect();
        let selection = &selection[..];
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        // The view follows the document's layer count: a layer added since
        // the last refresh gets its textures here, a removed one drops them.
        view.overlay_views.truncate(panels.len() - 1);
        while view.overlay_views.len() < panels.len() - 1 {
            let index = view.overlay_views.len() + 1;
            let place = &places[index];
            let layer = self.create_panel_view(
                &panels[index],
                place.rect,
                !place.draws_backdrop,
                font_generation,
            )?;
            view.overlay_views.push(layer);
        }
        // Each selection box is composited into the topmost panel of the
        // area it sits in, so it is above every layer's decoration there, as
        // it was above the single one.
        let routed = route_selection_boxes(&places, selection);
        for (index, panel) in panels.iter().enumerate() {
            let layer_view = if index == 0 {
                &mut *view
            } else {
                &mut view.overlay_views[index - 1]
            };
            self.refresh_panel_view(
                layer_view,
                panel,
                places[index].rect,
                &routed[index],
                !places[index].draws_backdrop,
                font_generation,
            )?;
        }
        view.panel_rect = panel_rect;
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        // The anchor space follows the axes and the data area, which this
        // refresh may have moved, so the annotation layer is redone here as
        // well; its own revision covers item edits between axis refreshes.
        self.refresh_annotation_layers(view, chart, panel_rect)?;
        Ok(())
    }

    /// The selection box of the element `element_id` names on the active
    /// layer of `chart`, in absolute surface coordinates: what
    /// [`Self::refresh_axis_with_selection`] draws, for host UI that outlines
    /// or hit-tests the selection itself. `None` for an id the active layer
    /// has no element for, or an element that draws nothing.
    pub fn selection_box(
        &self,
        chart: &Chart,
        element_id: &str,
    ) -> Option<crate::select::SelectionBox> {
        let config = chart.config();
        let map = crate::select::HitMap::for_active_layer(config);
        let id = map.find(element_id)?;
        map.selection_box(
            id,
            config,
            &crate::text_render::CpuTextMeasure::for_style(&config.draw_style),
        )
    }

    /// Refresh one panel's textures from its internal config, unless the
    /// textures already hold exactly this raster.
    pub(super) fn refresh_panel_view(
        &mut self,
        view: &mut ChartView,
        config: &Config,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
        overlay: bool,
        font_generation: u64,
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);
        let key = PanelRasterKey::new(config, selection, font_generation, panel_rect);
        if view.raster_key.as_ref() == Some(&key) {
            // Same raster: the revisions still advance so a token prepared
            // before this call is retired, exactly as for a real rewrite.
            // The Milkyway backdrop keeps its own cache and stamp: a view
            // created with a direct grid raster adopts the shared bake here.
            view.advance_content_revision()?;
            if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
                && !overlay
            {
                self.refresh_space_backdrop(view, c, w, h)?;
            }
            return Ok(());
        }

        // One placement pass lays out both layers; a base panel in the
        // Milkyway style takes its grid texture from the cached backdrop
        // instead of a grid raster.
        let backdrop = matches!(config.draw_style, DrawStyle::Milkyway(_)) && !overlay;
        let (grid_rgba, dec_rgba) = raster_panel_layers(config, overlay, selection, !backdrop)?;
        if view.panel_rect != panel_rect {
            view.advance_stream_revision()?;
        }
        view.advance_content_revision()?;

        // Grid layer. The constellation backdrop is axis-range independent,
        // so it is cached by (size, nebula, dust, seed): a pan/zoom/config
        // commit re-rasters only the decoration layer, and a view whose grid
        // texture already holds the cached generation skips the upload too.
        // Overlay panels never draw the backdrop; the base panel owns it.
        if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
            && !overlay
        {
            self.refresh_space_backdrop(view, c, w, h)?;
        } else {
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &grid_rgba,
                w,
                h,
            )?;
            view.grid_space_gen = None;
            view.layer_stamps.grid += 1;
        }

        self.refresh_one_layer(
            &mut view.decoration_texture,
            &mut view.decoration_bind_group,
            &dec_rgba,
            w,
            h,
        )?;
        view.layer_stamps.decoration += 1;
        view.panel_rect = panel_rect;
        view.tick_warnings = axis_render::tick_warnings(config);
        view.raster_key = Some(key);
        view.raster_generation = view.raster_generation.wrapping_add(1);

        Ok(())
    }

    /// Bake the Milkyway backdrop for `(w, h, nebula, dust, seed)` unless the
    /// renderer already holds it, and upload it into the view's grid texture
    /// unless the view already holds that generation.
    pub(super) fn refresh_space_backdrop(
        &mut self,
        view: &mut ChartView,
        c: &crate::config::MilkywayOptions,
        w: u32,
        h: u32,
    ) -> Result<()> {
        let key = SpaceBgKey {
            w,
            h,
            nebula: c.nebula.to_bits(),
            dust: c.dust.to_bits(),
            seed: c.seed,
        };
        if !self.space_bg.as_ref().is_some_and(|s| s.key == key) {
            let rgba = key.bake()?;
            let bake_gen = self.space_bg.as_ref().map_or(1, |s| s.bake_gen + 1);
            self.space_bg = Some(SpaceBgCache {
                key,
                bake_gen,
                rgba,
            });
        }
        let cache = self.space_bg.as_ref().expect("space_bg baked above");
        if view.grid_space_gen != Some(cache.bake_gen) {
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &cache.rgba,
                w,
                h,
            )?;
            view.grid_space_gen = Some(cache.bake_gen);
            view.layer_stamps.grid += 1;
        }
        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    pub(super) fn refresh_one_layer(
        &self,
        tex: &mut TrackedTexture,
        bg: &mut wgpu::BindGroup,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<()> {
        let same_size = tex.width() == w && tex.height() == h;
        if same_size {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        } else {
            // The replacement is charged before the assignment below drops
            // the old one, so a resize reads as `old + new` for one frame —
            // which is what the device is holding.
            let new_tex = TrackedTexture::new(
                &self.gpu_ledger,
                GpuResourceKind::PanelTexture,
                data_render::upload_rgba_texture(
                    &self.device,
                    &self.queue,
                    self.caps.max_texture_dimension_2d,
                    w,
                    h,
                    rgba,
                )?,
            );
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device,
                &self.texture_bgl,
                &new_view_t,
                &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
        Ok(())
    }

    /// Draw multiple chart panels into one RenderPass.
    ///
    /// Facade over [`Self::prepare`] + [`Self::paint_prepared`] for hosts
    /// that own the renderer exclusively during their frame (winit loop,
    /// wasm wrapper, export): both phases run back to back under one
    /// `&mut self`, so no interleaving is possible and no token handling is
    /// needed. Hosts whose paint callback only provides `&self` (egui's
    /// `CallbackTrait::paint`, iced's `shader::Primitive::draw`) should call
    /// the two phases separately instead of wrapping the renderer in a
    /// `Mutex` — see the host-integration notes above.
    ///
    /// Takes a pass already opened by the host via `begin_render_pass`.
    /// `Renderer` owns the pool buffer, pipelines, and bind groups, so no
    /// external resources are needed.
    ///
    /// Returns `FiggyError::UnknownColumn` if a series references an unknown
    /// column id (or a stale handle).
    ///
    /// `target_size` is the color attachment pixel size of the current pass;
    /// panel_rect / data_area are clamped to it because wgpu validation would
    /// otherwise abort. Pass the surface pixel size reported by the host
    /// (winit, egui CallbackInfo, iced shader::Primitive, …).
    pub fn paint(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.prepare(items)?;
        self.paint_prepared(pass, target_size, &prepared)
    }

    pub(super) fn preparation_parts(
        &mut self,
    ) -> (&HashMap<ChartId, ChartRenderState>, PrepareContext<'_>) {
        (
            &self.chart_states,
            PrepareContext {
                device: &self.device,
                queue: &self.queue,
                caps: &self.caps,
                pool: &self.pool,
                gpu_ledger: &self.gpu_ledger,
                style_bgl: &self.style_bgl,
                field_bgl: &self.field_bgl,
                surface_bgl: &self.surface_bgl,
                region_bgl: &self.region_bgl,
                vector_bgl: &self.vector_bgl,
                category_bar_data_bgl: &self.category_bar_data_bgl,
                value_label_bgl: &self.value_label_bgl,
                field_cell_tables: &self.field_cell_tables,
                transform_bgl: &self.transform_bgl,
                per_point_style_map_bgl: &self.per_point_style_map_bgl,
                point_value_map_bgl: &self.point_value_map_bgl,
                neutral_point_value_map_bg: &self.neutral_point_value_map_bg,
                neutral_point_value_map_buffers: &self.neutral_point_value_map_buffers,
                view3d_value_map_bgl: &self.view3d_value_map_bgl,
                data_selection_bgl: &self.data_selection_bgl,
                star_data_bgl: &self.star_data_bgl,
                sampler: &self.sampler,
                quad_vb: &self.quad_vb,
                pipelines: &mut self.pipelines,
                field_cache: &mut self.field_cache,
                value_map_cache: &mut self.value_map_cache,
                vector_cache: &mut self.vector_cache,
                contour_cache: &mut self.contour_cache,
                arc_cache: &mut self.arc_cache,
                texture_bgl: &self.texture_bgl,
                target_sample_count: self.target_sample_count,
                resident_displays: &mut self.resident_displays,
                resident_display_serial: &mut self.resident_display_serial,
                contour_label_pipelines: &mut self.contour_label_pipelines,
                arc_pipelines: &mut self.arc_pipelines,
                surface_format: self.surface_format,
                renderer_identity: self.renderer_identity,
                target_pipeline_generation: self.target_pipeline_generation,
                #[cfg(test)]
                arc_chunk_override: self.arc_chunk_override,
            },
        )
    }

    /// Mutable half of the frame: finish every state change the draw needs,
    /// before any render pass opens.
    ///
    /// Does not rerasterize axes/decorations or rebuild caller-owned styles.
    /// Refresh a changed view with [`Self::refresh_axis`] (or its selection
    /// variant), and recreate affected styles with [`Self::create_style_for_series`]
    /// before calling this method. Unchanged inputs may be reused.
    ///
    /// Runs the styled-pipeline ensure (compiles pipeline sets the items
    /// need but the cache lacks), writes each panel's data→NDC transform
    /// uniform from `item.chart_config`, and (re)dispatches the per-series
    /// GPU arc-length scans — submitting their compute encoder on the queue,
    /// which orders it before the host's later pass submission. Returns an
    /// owned [`PreparedFrame`] token snapshotting everything the immutable
    /// draw phase will reference.
    ///
    /// Because the transform uniform and the arc dispatch are written here
    /// from the same config snapshot, they cannot desync — calling
    /// [`Self::update_transform`] per frame becomes unnecessary (though
    /// harmless) for callers of this API.
    ///
    /// Contract: call after this frame's data uploads (`add_column`),
    /// `ensure_target_format`, and `refresh_axis`; before the host records and
    /// submits the command buffer containing the render pass. Once a command
    /// buffer has been recorded from the token, submit it before any later
    /// mutation of the same [`ChartView`] or another `prepare` for that view.
    /// The returned token owns the resolved draw input. `paint_prepared`
    /// therefore receives no second
    /// copy of chart/view/series/style inputs. It rejects renderer resource
    /// changes that invalidate the captured GPU work: target-pipeline rebuild,
    /// pool layout change, replacement of any captured column allocation, or
    /// another transform/axis write to a captured `ChartView`.
    pub fn prepare(&mut self, items: &[ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        for item in items {
            for series in item.series {
                self.require_resident_series(series.config)?;
            }
        }
        self.preparation_parts().1.prepare(items)
    }

    /// Prepare renderer-owned charts without copying their Config or ordered
    /// SeriesConfig values. GPU styles are cached derivatives of those series;
    /// chart identity is always the explicit id, never inferred from resources.
    ///
    /// Refresh each view's raster before this call as for [`Self::prepare`].
    /// Mutating or removing a captured chart makes its token stale. Changes to
    /// unrelated charts do not invalidate the captured chart stamps.
    pub fn prepare_registered(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        self.prepare_registered_mixed(items)
            .map_err(streaming_runtime::StreamRequestError::into_figgy)
    }

    pub(super) fn prepare_registered_resident(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart prepare inputs",
                reason: error.to_string(),
            };
        // Resolve every id and fallible contour invariant before creating any
        // GPU styles or advancing the content revision of a supplied view.
        for item in items {
            let state = self
                .chart_states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            for series in &state.series {
                self.require_resident_series(series)?;
                validate_contour_level_count(&series.series_id, &series.render_type)?;
                validate_polar_series(&state.config, series)?;
                preflight_contour_label_spacing(
                    &series.series_id,
                    &series.render_type,
                    effective_series_primitives(&state.config.draw_style, &series.render_type)
                        .contour,
                    1.0,
                )?;
            }
        }
        for item in items {
            self.ensure_registered_styles(item.chart_id)?;
        }
        let (states, mut preparation) = self.preparation_parts();
        let mut series_inputs = Vec::new();
        let mut stamps = Vec::new();
        series_inputs
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        stamps.try_reserve_exact(items.len()).map_err(allocation)?;
        for item in items {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            let styles = &state
                .prepared_styles
                .as_ref()
                .expect("registered styles were prepared")
                .styles;
            let mut series = Vec::new();
            series
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            series.extend(
                state
                    .series
                    .iter()
                    .zip(styles)
                    .map(|(config, style)| Series { config, style }),
            );
            series_inputs.push(series);
            stamps.push(PreparedChartStamp {
                chart_id: item.chart_id,
                desired: state.revisions.desired,
                data: state.revisions.data,
            });
        }
        let mut draws = Vec::new();
        draws.try_reserve_exact(items.len()).map_err(allocation)?;
        for (item, series) in items.iter().zip(&series_inputs) {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            draws.push(ChartDrawItem {
                view: item.view,
                chart_config: &state.config,
                series,
            });
        }
        let mut frame = preparation.prepare(&draws)?;
        frame.registered_charts = stamps;
        Ok(frame)
    }

    /// Select the current resident or owned streaming display for each chart
    /// from its explicit id. Paint receives only the prepared token.
    pub(crate) fn prepare_registered_mixed(
        &mut self,
        items: &[RegisteredChartDrawItem<'_>],
    ) -> streaming_runtime::StreamResult<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "mixed chart prepare inputs",
                reason: error.to_string(),
            };
        let mut resident = Vec::new();
        let mut displays = Vec::new();
        let mut streamed = Vec::new();
        // Prepared items a resident chart contributes: one per panel of its
        // layered document (`expand_for_render`); zero for a streamed chart.
        let mut panel_counts = Vec::new();
        resident
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        displays
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        streamed
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        panel_counts
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        for item in items {
            let config = self.chart_config(item.chart_id)?;
            let active = self.chart_has_stream_surface(item.chart_id);
            panel_counts.push(if active {
                0
            } else {
                render_panel_count(config)
            });
            if active {
                self.preflight_chart_stream_display(item.chart_id, item.view)?;
            }
            if !active {
                resident.push(RegisteredChartDrawItem {
                    chart_id: item.chart_id,
                    view: item.view,
                });
            }
            streamed.push(active);
            displays.push(None);
        }
        let mut frame = self.prepare_registered_resident(&resident)?;
        for ((item, active), display) in items.iter().zip(streamed).zip(&mut displays) {
            if active {
                *display = Some(
                    self.prepare_chart_stream_display(item.chart_id, item.view)?
                        .ok_or(crate::streaming::StreamError::WrongState)?,
                );
            }
        }
        let mut resident_items = std::mem::take(&mut frame.items).into_iter();
        frame
            .items
            .try_reserve_exact(items.len() - resident.len() + panel_counts.iter().sum::<usize>())
            .map_err(allocation)?;
        frame
            .registered_charts
            .try_reserve_exact(items.len() - resident.len())
            .map_err(allocation)?;
        for (item, (display, count)) in items.iter().zip(displays.into_iter().zip(panel_counts)) {
            if let Some(display) = display {
                let state = self
                    .chart_states
                    .get(&item.chart_id)
                    .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
                frame.registered_charts.push(PreparedChartStamp {
                    chart_id: item.chart_id,
                    desired: state.revisions.desired,
                    data: state.revisions.data,
                });
                frame.items.push(PreparedItem {
                    view: PreparedView::capture(
                        item.view,
                        item.view
                            .content_revision
                            .load(std::sync::atomic::Ordering::Acquire),
                        state.config.annotations.visible,
                    ),
                    data_area: state
                        .config
                        .data_area()
                        .map(|area| area.0)
                        .unwrap_or(item.view.panel_rect),
                    axis_pipeline: self.pipelines.axis.clone(),
                    series: Vec::new(),
                    stream_display: Some(display),
                    resident_display: None,
                    column_fingerprint: 0,
                });
            } else {
                for _ in 0..count {
                    frame.items.push(resident_items.next().ok_or(
                        streaming_runtime::StreamRequestError::Scheduler(
                            crate::streaming::StreamError::WrongState,
                        ),
                    )?);
                }
            }
        }
        Ok(frame)
    }

    /// Immutable half of the frame: pure command recording against a
    /// [`PreparedFrame`] from [`Self::prepare`].
    ///
    /// `&self` — safe to call from host paint callbacks that only hand out
    /// shared access (egui `CallbackTrait::paint` via `&CallbackResources`,
    /// iced `shader::Primitive::draw` via `&Storage`), with no lock around
    /// the renderer. Repeatable: the same token may be recorded into more
    /// than one pass (egui re-record, iced mid-frame pass restart).
    ///
    /// All chart/view/series/style input comes from `prepared`; the host does
    /// not reconstruct or pass it again. Renderer-level stamps detect
    /// invalidating `&mut self` calls since `prepare`. On mismatch nothing is
    /// recorded and [`FiggyError::StalePreparedFrame`] is returned; recovery
    /// is a fresh `prepare`. A recorded command buffer is outside Rust's
    /// borrow graph, so the host must submit it before mutating or preparing
    /// the same [`ChartView`] again; validation can only run while recording.
    pub fn paint_prepared(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &PreparedFrame,
    ) -> Result<()> {
        self.validate_prepared(prepared)?;
        self.paint_prepared_items(pass, target_size, &prepared.items, true)
    }

    /// Serial of the cached series bodies of `view`'s panel, `None` while it
    /// has none: the count of draws a test can read per panel.
    #[cfg(test)]
    pub(crate) fn resident_display_serial(&self, view: &ChartView) -> Option<u64> {
        let id = Arc::as_ptr(&view.content_revision) as usize;
        self.resident_displays
            .iter()
            .find(|display| display.view.as_ptr() as usize == id)
            .map(|display| display.serial)
    }

    #[cfg(test)]
    pub(crate) fn resident_display_count(&self) -> usize {
        self.resident_displays.len()
    }

    /// Stamp checks guarding `paint_prepared` — see [`PreparedFrame`] field
    /// docs for what each stamp catches. Kept separate from recording so a
    /// stale token fails before any pass state is touched.
    pub(super) fn validate_prepared(&self, prepared: &PreparedFrame) -> Result<()> {
        let stale = |reason: String| FiggyError::StalePreparedFrame { reason };
        if prepared.renderer_identity != self.renderer_identity {
            return Err(stale(
                "prepared frame belongs to a different renderer".into(),
            ));
        }
        if prepared.target_pipeline_generation != self.target_pipeline_generation {
            return Err(stale(format!(
                "prepared at target pipeline generation {}, renderer is now at {} \
                 (the target format or sample count changed between prepare and paint_prepared)",
                prepared.target_pipeline_generation, self.target_pipeline_generation
            )));
        }
        for stamp in &prepared.registered_charts {
            let unchanged = self.chart_states.get(&stamp.chart_id).is_some_and(|state| {
                state.revisions.desired == stamp.desired && state.revisions.data == stamp.data
            });
            if !unchanged {
                return Err(stale(format!(
                    "registered chart {:?} changed or was removed between prepare and paint_prepared",
                    stamp.chart_id,
                )));
            }
        }
        if prepared.pool_layout_generation != self.pool.layout_generation() {
            return Err(stale(format!(
                "prepared at pool layout generation {}, pool is now at {} \
                 (the pool layout changed between prepare and paint_prepared)",
                prepared.pool_layout_generation,
                self.pool.layout_generation()
            )));
        }
        for (idx, item) in prepared.items.iter().enumerate() {
            let current = item
                .view
                .content_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if current != item.view.expected_content_revision {
                return Err(stale(format!(
                    "panel {idx} view changed between prepare and paint_prepared"
                )));
            }
            if let Some(display) = &item.resident_display
                && !self.resident_displays.iter().any(|held| {
                    held.view.as_ptr() as usize == display.view && held.serial == display.serial
                })
            {
                return Err(stale(format!(
                    "panel {idx} cached composite was drawn again after prepare"
                )));
            }
            if let Some(display) = &item.stream_display
                && !self.stream_display_packet_is_current(display)
            {
                return Err(stale(format!(
                    "panel {idx} stream display changed or was cancelled after prepare"
                )));
            }
        }
        for (id, allocation_epoch) in &prepared.column_sources {
            if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                return Err(stale(format!(
                    "column {:?} changed allocation between prepare and paint_prepared",
                    id
                )));
            }
        }
        // A field's grid columns, walked from the shared receipt: reads only, so
        // a 5000-column matrix costs no allocation to validate.
        for sources in &prepared.grid_sources {
            for (id, allocation_epoch) in sources.ids.iter().zip(&sources.epochs) {
                if self.pool.allocation_epoch(id) != Some(*allocation_epoch) {
                    return Err(stale(format!(
                        "grid column {:?} changed allocation between prepare and paint_prepared",
                        id
                    )));
                }
            }
        }
        Ok(())
    }

    /// Read the instance-count word from a prepared contour label's indirect
    /// draw arguments. This exists only for native unit tests and wasm browser
    /// conformance tests, where pixels cannot distinguish all 1024 instances.
    #[cfg(any(test, target_arch = "wasm32"))]
    #[doc(hidden)]
    pub async fn contour_label_instance_count_for_test(
        &self,
        prepared: &PreparedFrame,
        item_index: usize,
        series_index: usize,
    ) -> Result<Option<u32>> {
        self.validate_prepared(prepared)?;
        let Some(indirect) = prepared
            .items
            .get(item_index)
            .and_then(|item| item.series.get(series_index))
            .and_then(|series| series.contour.as_ref())
            .and_then(|contour| contour.label.as_ref())
            .map(|label| label.snapshot.indirect())
        else {
            return Ok(None);
        };

        // gpu-alloc: Readback
        let readback = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Readback,
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy contour label indirect count readback"),
                size: 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy contour label indirect count copy"),
            });
        encoder.copy_buffer_to_buffer(indirect, 4, &readback, 0, 4);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        rx.await
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async sender dropped: {error}"),
            })?
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label indirect count readback",
                reason: format!("map_async: {error:?}"),
            })?;
        let mapped =
            slice
                .get_mapped_range()
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "contour label indirect count readback",
                    reason: format!("get_mapped_range: {error:?}"),
                })?;
        let count = u32::from_le_bytes(mapped[..4].try_into().expect("four-byte readback"));
        drop(mapped);
        readback.unmap();
        Ok(Some(count))
    }

    pub(super) fn prepare_arc_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
    ) -> Result<Vec<PreparedArcItem>> {
        self.preparation_parts().1.prepare_arc_items(items)
    }

    pub(super) fn resolve_prepared_items(
        &mut self,
        items: &[ChartDrawItem<'_>],
        prepared_arcs: &[PreparedArcItem],
        prepared_contours: &[PreparedContourItem],
        pipelines: &TargetPipelines,
    ) -> Result<(Vec<PreparedItem>, HashMap<ColumnId, u64>)> {
        self.preparation_parts().1.resolve_prepared_items(
            items,
            prepared_arcs,
            prepared_contours,
            pipelines,
        )
    }

    pub(super) fn paint_prepared_items<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        prepared: &'a [PreparedItem],
        chrome: bool,
    ) -> Result<()> {
        for item in prepared {
            if let Some(display) = &item.stream_display
                && display.size != target_size
            {
                return Err(FiggyError::StalePreparedFrame {
                    reason: "stream display size differs from paint target".into(),
                });
            }
        }
        // Every panel's grid and data, then every panel's decoration
        // (`composite::FRAME_SLOTS`): on every layer the decoration is above
        // the data.
        for slot in super::composite::FRAME_SLOTS {
            for item in prepared {
                if let Some(display) = &item.stream_display {
                    // The stream display is the whole panel in one texture;
                    // it is drawn once, in the first slot.
                    if slot == crate::sink::Slot::Below
                        && let Some(panel) =
                            data_render::clamp_rect_to_target(item.view.panel_rect, target_size)
                    {
                        // D covers the whole target, unlike a panel-sized axis texture.
                        pass.set_viewport(
                            0.0,
                            0.0,
                            target_size.0 as f32,
                            target_size.1 as f32,
                            0.0,
                            1.0,
                        );
                        pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                        pass.set_pipeline(&item.axis_pipeline);
                        pass.set_bind_group(0, &display.bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    continue;
                }
                let panel_rect = item.view.panel_rect;
                // The series layers are read by the data slot only.
                let series_list: Vec<_>;
                let series_layers: &[data_render::SeriesLayers<'_>] = match slot {
                    crate::sink::Slot::Below => {
                        series_list = item.series.iter().map(PreparedSeries::layers).collect();
                        &series_list
                    }
                    crate::sink::Slot::Above => &[],
                };
                // Without the chrome only the data slot is drawn.
                let (annotation_underlay, annotation_overlay) = if chrome {
                    item.view.annotation_layers(&item.axis_pipeline)
                } else {
                    (None, None)
                };

                // The single style decision per panel: the chart's `DrawStyle`
                // resolves to a cached styled pipeline set (compiled by the
                // prepare phase) or `None` for precise — the precise path stays
                // untouched.
                // Bundle every series's primitives for the panel into one call.

                // Cached bodies cover the whole panel; a panel the target clips
                // draws them directly, as before.
                let cached = item
                    .resident_display
                    .as_ref()
                    .filter(|_| {
                        panel_rect.x.checked_add(panel_rect.width) <= Some(target_size.0)
                            && panel_rect.y.checked_add(panel_rect.height) <= Some(target_size.1)
                    })
                    .map(|display| AxisLayer {
                        pipeline: &item.axis_pipeline,
                        bind_group: &display.bind_group,
                    });
                super::composite::draw_chart_panel(
                    pass,
                    target_size,
                    panel_rect,
                    item.data_area,
                    &super::composite::PanelChrome {
                        below: chrome.then_some(AxisLayer {
                            pipeline: &item.axis_pipeline,
                            bind_group: &item.view.grid_bind_group,
                        }),
                        annotation_underlay,
                        above: chrome.then_some(AxisLayer {
                            pipeline: &item.axis_pipeline,
                            bind_group: &item.view.decoration_bind_group,
                        }),
                        annotation_overlay,
                    },
                    series_layers,
                    cached,
                    slot,
                );
            }
        }
        Ok(())
    }
}

impl Renderer {
    /// The GPU styles of chart `id`'s series as registered
    /// (`ChartState::prepared_styles`), built when missing or stale: the
    /// frame prepares them before its draws, and a 3D chart's point pick
    /// binds them (#132).
    pub(super) fn ensure_registered_styles(&mut self, id: ChartId) -> Result<()> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart styles",
                reason: error.to_string(),
            };
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if state
            .prepared_styles
            .as_ref()
            .is_some_and(|styles| styles.series_revision == state.revisions.series)
        {
            return Ok(());
        }
        let mut styles = Vec::new();
        styles
            .try_reserve_exact(state.series.len())
            .map_err(allocation)?;
        for series in &state.series {
            styles.push(self.create_style_for_series_scaled_inner(series, 1.0)?);
        }
        let series_revision = state.revisions.series;
        self.chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?
            .prepared_styles = Some(RegisteredChartStyles {
            series_revision,
            styles,
        });
        Ok(())
    }
}
