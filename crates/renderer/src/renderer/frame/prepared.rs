//! Prepared series, views, items and frames.

use super::*;

/// Complete owned draw packet for one series. Every pipeline, bind group,
/// buffer, and column handle needed by paint is resolved during prepare.
pub(in crate::renderer) struct PreparedSeries {
    pub(in crate::renderer) _arc_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) _column_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) _style_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) _stream_style_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) _stream_transform_charge: Option<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) area: Option<PreparedAreaLayer>,
    pub(in crate::renderer) field: Option<PreparedFieldLayer>,
    pub(in crate::renderer) mesh: Option<PreparedTriMeshLayer>,
    pub(in crate::renderer) mesh_contour: Option<PreparedTriMeshLayer>,
    pub(in crate::renderer) surface: Option<PreparedSurfaceLayer>,
    pub(in crate::renderer) floor: Option<PreparedSurfaceLayer>,
    pub(in crate::renderer) selected_surface: Option<PreparedSurfaceLayer>,
    pub(in crate::renderer) region: Option<PreparedTernaryRegionLayer>,
    pub(in crate::renderer) bar: Option<PreparedBarLayer>,
    pub(in crate::renderer) violin: Option<PreparedCategoryBarLayer>,
    pub(in crate::renderer) category_bars: Option<PreparedCategoryBarLayer>,
    pub(in crate::renderer) box_points: Option<PreparedCategoryBarLayer>,
    pub(in crate::renderer) contour: Option<PreparedContourLayer>,
    pub(in crate::renderer) vector: Option<PreparedVectorLayer>,
    pub(in crate::renderer) errorbar: Option<PreparedErrorBarLayer>,
    /// A stem series' stems (#206), the error-bar module's second draw.
    pub(in crate::renderer) stem: Option<PreparedErrorBarLayer>,
    pub(in crate::renderer) line: Option<PreparedLineLayer>,
    pub(in crate::renderer) parallel: Option<PreparedParallelLayer>,
    pub(in crate::renderer) line_extra: Option<PreparedStarLayer>,
    pub(in crate::renderer) scatter: Option<PreparedScatterLayer>,
    pub(in crate::renderer) line_3d: Option<PreparedLineLayer>,
    pub(in crate::renderer) scatter_3d: Option<PreparedScatterLayer>,
    pub(in crate::renderer) vector_3d: Option<PreparedVectorLayer>,
    pub(in crate::renderer) selected_bars: Vec<PreparedBarSelectionLayer>,
    pub(in crate::renderer) selected_fields: Vec<PreparedFieldSelectionLayer>,
    pub(in crate::renderer) picked: Vec<PreparedPickRingLayer>,
    pub(in crate::renderer) value_labels: Option<PreparedValueLabelLayer>,
    /// An energy-level series' CPU raster (#203): the blit pipeline and the
    /// raster's bind group, which keeps its texture alive.
    pub(in crate::renderer) energy_levels: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
}

/// Refcounted GPU view bindings captured by `prepare`. Bind groups keep their
/// backing textures/buffers alive, so paint needs no `ChartView` borrow.
pub(in crate::renderer) struct PreparedView {
    pub(in crate::renderer) grid_bind_group: wgpu::BindGroup,
    pub(in crate::renderer) decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer's textures, `None` where the layer drew nothing
    /// on that side; skipped entirely while the layer is hidden.
    pub(in crate::renderer) annotation_underlay_bind_group: Option<wgpu::BindGroup>,
    pub(in crate::renderer) annotation_overlay_bind_group: Option<wgpu::BindGroup>,
    pub(in crate::renderer) annotations_visible: bool,
    pub(in crate::renderer) _grid_charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) _decoration_charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) _annotation_charges: Vec<crate::gpu_memory::SharedCharge>,
    pub(in crate::renderer) _transform_charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) content_revision: Arc<std::sync::atomic::AtomicU64>,
    pub(in crate::renderer) expected_content_revision: u64,
    pub(in crate::renderer) panel_rect: Rect,
}

impl PreparedView {
    /// Snapshot a view's bindings. `annotations_visible` comes from the
    /// chart config at prepare time, so toggling the layer needs no refresh.
    pub(in crate::renderer) fn capture(
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

    pub(in crate::renderer) fn annotation_layers<'a>(
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
pub(in crate::renderer) struct PreparedItem {
    pub(in crate::renderer) view: PreparedView,
    pub(in crate::renderer) data_area: Rect,
    /// The rects the data slot is drawn in: the data area, or its pieces on
    /// a broken axis (#200).
    pub(in crate::renderer) data_pieces: super::composite::DataPieces,
    /// The transform each shifted piece of a `Panels` axis is drawn at
    /// (#200): the panel's own, offset by the piece's shift. `None` for a
    /// piece drawn at the panel's transform.
    pub(in crate::renderer) piece_transforms:
        [Option<PieceTransform>; super::composite::MAX_DATA_PIECES],
    pub(in crate::renderer) axis_pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) series: Vec<PreparedSeries>,
    pub(in crate::renderer) stream_display: Option<streaming_runtime::StreamDisplayPacket>,
    /// The panel's cached resident composite, when the panel qualifies; see
    /// [`ResidentDisplay`].
    pub(in crate::renderer) resident_display: Option<ResidentDisplayPacket>,
    /// [`column_fingerprint`] of the columns this panel's series read through
    /// the pool lookup (grid columns come from the scratch receipts).
    pub(in crate::renderer) column_fingerprint: u64,
}

/// One piece's transform uniform and its bind group (#200).
pub(in crate::renderer) struct PieceTransform {
    pub(in crate::renderer) _buffer: TrackedBuffer,
    pub(in crate::renderer) bind_group: wgpu::BindGroup,
}

/// No piece drawn at its own transform.
pub(in crate::renderer) fn no_piece_transforms()
-> [Option<PieceTransform>; super::composite::MAX_DATA_PIECES] {
    std::array::from_fn(|_| None)
}

/// The series layers of each piece drawn at its own transform (#200);
/// `None` for the pieces that read the panel's list.
pub(in crate::renderer) type OwnPieceLayers<'a> =
    [Option<Vec<data_render::SeriesLayers<'a>>>; super::composite::MAX_DATA_PIECES];

impl PreparedItem {
    /// The series layers of each piece drawn at its own transform.
    pub(in crate::renderer) fn own_piece_layers(&self) -> OwnPieceLayers<'_> {
        std::array::from_fn(|k| {
            let transform = self.piece_transforms.get(k)?.as_ref()?;
            Some(
                self.series
                    .iter()
                    .map(|series| series.layers_with(Some(&transform.bind_group)))
                    .collect(),
            )
        })
    }
}

/// An order-independent fingerprint of `(column id, allocation epoch)` pairs:
/// it changes when any of the columns is replaced, removed and added again,
/// or relocated into a new allocation, and stays when an unrelated column
/// changes. A sum of per-pair hashes, so it needs no sorted copy.
pub(in crate::renderer) fn column_fingerprint<'a>(
    pairs: impl Iterator<Item = (&'a str, u64)>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    pairs.fold(0u64, |sum, (id, epoch)| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        id.hash(&mut hasher);
        epoch.hash(&mut hasher);
        sum.wrapping_add(hasher.finish())
    })
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
    pub(in crate::renderer) renderer_identity: u64,
    pub(in crate::renderer) items: Vec<PreparedItem>,
    pub(in crate::renderer) column_sources: HashMap<ColumnId, u64>,
    pub(in crate::renderer) registered_charts: Vec<PreparedChartStamp>,
    /// Pool layout stamp — defrag, clear, or a buffer-relocating upsert can
    /// move column bytes or replace the backing buffer. Per-column allocation
    /// epochs above catch same-layout replacement of any captured draw input
    /// without invalidating a token for an unrelated column mutation.
    pub(in crate::renderer) pool_layout_generation: u64,
    /// Monotonic identity of the target pipeline set. This detects format
    /// ABA and sample-count changes, not just a different current format.
    pub(in crate::renderer) target_pipeline_generation: u64,
    /// One receipt per field or contour series: the grid columns it read and the
    /// epochs they had. `column_sources` cannot serve this — a matrix declares
    /// thousands of columns, and an owned map entry each would put a per-frame
    /// allocation per column on the prepare path.
    pub(in crate::renderer) grid_sources: Vec<Arc<GridSources>>,
}

#[derive(Debug)]
pub(in crate::renderer) struct PreparedChartStamp {
    pub(in crate::renderer) chart_id: ChartId,
    pub(in crate::renderer) desired: RenderRevision,
    pub(in crate::renderer) data: RenderRevision,
}

impl std::fmt::Debug for PreparedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedItem")
            .field("series", &self.series.len())
            .finish_non_exhaustive()
    }
}

impl PreparedSeries {
    pub(in crate::renderer) fn from_layers(
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
            mesh: layers.mesh.map(PreparedTriMeshLayer::own),
            mesh_contour: layers.mesh_contour.map(PreparedTriMeshLayer::own),
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
            vector: layers.vector.map(PreparedVectorLayer::own),
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
            errorbar: layers.errorbar.map(PreparedErrorBarLayer::own),
            stem: layers.stem.map(PreparedErrorBarLayer::own),
            line: layers.line.map(PreparedLineLayer::own),
            parallel: layers.parallel.map(PreparedParallelLayer::own),
            line_3d: layers.line_3d.map(PreparedLineLayer::own),
            scatter_3d: layers.scatter_3d.map(PreparedScatterLayer::own),
            vector_3d: layers.vector_3d.map(PreparedVectorLayer::own),
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
                    vertices: layer.vertices,
                    instances: layer.instances,
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
            energy_levels: layers
                .energy_levels
                .map(|layer| (layer.pipeline.clone(), layer.bind_group.clone())),
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

    pub(in crate::renderer) fn layers(&self) -> data_render::SeriesLayers<'_> {
        self.layers_with(None)
    }

    /// [`Self::layers`] with `transform` in place of every 2D layer's own
    /// transform bind group when given: the layers a `Panels` piece draws,
    /// at its moved map (#200). The 3D, ternary region and parallel layers
    /// keep theirs: those charts have no breaks.
    pub(in crate::renderer) fn layers_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> data_render::SeriesLayers<'a> {
        data_render::SeriesLayers {
            area: self
                .area
                .as_ref()
                .map(|layer| data_render::ColumnAreaLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
                    style_bg: &layer.style_bg,
                    pool_buffer: &layer.pool_buffer,
                    x: layer.x,
                    y: layer.y,
                    base: layer.base,
                    verts_per_instance: layer.verts_per_instance,
                    closed: layer.closed,
                }),
            mesh: self.mesh.as_ref().map(|layer| layer.borrow(transform)),
            mesh_contour: self
                .mesh_contour
                .as_ref()
                .map(|layer| layer.borrow(transform)),
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
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
                    style_bg: layer.style_bg.clone(),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                }),
            bar: self.bar.as_ref().map(|layer| data_render::ColumnBarLayer {
                envelope: layer.envelope.clone(),
                pipeline: &layer.pipeline,
                transform_bg: transform.unwrap_or(&layer.transform_bg),
                style_bg: &layer.style_bg,
                style_map_bg: layer.style_map_bg.as_ref(),
                pool_buffer: &layer.pool_buffer,
                edges: layer.edges,
                values: layer.values,
            }),
            violin: self
                .violin
                .as_ref()
                .map(|layer| layer.layer_with(transform)),
            category_bars: self
                .category_bars
                .as_ref()
                .map(|layer| layer.layer_with(transform)),
            box_points: self
                .box_points
                .as_ref()
                .map(|layer| layer.layer_with(transform)),
            vector: self
                .vector
                .as_ref()
                .map(|layer| layer.borrow_with(transform)),
            contour: self
                .contour
                .as_ref()
                .map(|layer| data_render::ColumnContourLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
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
            errorbar: self
                .errorbar
                .as_ref()
                .map(|layer| layer.borrow_with(transform)),
            stem: self.stem.as_ref().map(|layer| layer.borrow_with(transform)),
            line: self.line.as_ref().map(|layer| layer.borrow_with(transform)),
            parallel: self.parallel.as_ref().map(PreparedParallelLayer::borrow),
            line_3d: self.line_3d.as_ref().map(PreparedLineLayer::borrow),
            scatter_3d: self.scatter_3d.as_ref().map(PreparedScatterLayer::borrow),
            vector_3d: self.vector_3d.as_ref().map(|layer| layer.borrow_with(None)),
            line_extra: self
                .line_extra
                .as_ref()
                .map(|layer| data_render::ColumnStarLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
                    style_bg: &layer.style_bg,
                    texture_bg: &layer.texture_bg,
                    star_bg: &layer.star_bg,
                    indirect: &layer.indirect,
                }),
            scatter: self
                .scatter
                .as_ref()
                .map(|layer| layer.borrow_with(transform)),
            selected_bars: self
                .selected_bars
                .iter()
                .map(|layer| ColumnBarSelectionLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
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
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
                    selection_bg: layer.selection_bg.clone(),
                    selection_charge: Arc::clone(&layer.selection_charge),
                    field_bg: layer.field_bg.clone(),
                    charge: Arc::clone(&layer.charge),
                    drawable: layer.drawable,
                    vertices: layer.vertices,
                    instances: layer.instances.clone(),
                })
                .collect(),
            picked: self
                .picked
                .iter()
                .map(|layer| ColumnPickRingLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
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
            energy_levels: self.energy_levels.as_ref().map(|(pipeline, bind_group)| {
                data_render::AxisLayer {
                    pipeline,
                    bind_group,
                }
            }),
            value_labels: self.value_labels.as_ref().map(|layer| {
                data_render::ColumnValueLabelLayer {
                    pipeline: &layer.pipeline,
                    transform_bg: transform.unwrap_or(&layer.transform_bg),
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
