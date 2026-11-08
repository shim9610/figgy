//! The prepared draw layers of each series kind.

use super::*;

/// Owned snapshot of one series' constellation/milkyway star pass, cloned
/// out of the arc scratch by the prepare phase. wgpu resources are
/// internally refcounted, so these clones keep the GPU objects alive (and
/// the draw phase cache-independent) even if `arc_cache` evicts or rebuilds
/// the entry before `paint_prepared` runs.
pub(in crate::renderer) struct PreparedStar {
    pub(in crate::renderer) indirect: wgpu::Buffer,
    pub(in crate::renderer) vs_bg: wgpu::BindGroup,
}

/// One series' arc-scan snapshot inside a token: the prefix buffer slice the
/// line binds and the star pass when the style has one. The frame's captured
/// column allocation epochs guard the source identity at paint time.
pub(in crate::renderer) struct PreparedArc {
    pub(in crate::renderer) prefix: ArcPrefix,
    pub(in crate::renderer) star: Option<PreparedStar>,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
}

pub(in crate::renderer) struct PreparedArcSeries {
    pub(in crate::renderer) arc: Option<PreparedArc>,
}

pub(in crate::renderer) struct PreparedArcItem {
    pub(in crate::renderer) view_revision: u64,
    pub(in crate::renderer) series: Vec<PreparedArcSeries>,
}

pub(in crate::renderer) struct PreparedContourScratch {
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) drawable: bool,
    pub(in crate::renderer) label: Option<crate::gpu_contour::ContourLabelSnapshot>,
}

pub(in crate::renderer) struct PreparedContourSeries {
    pub(in crate::renderer) scratch: Option<PreparedContourScratch>,
    /// The series' value label atlas, baked by the contour label module's
    /// atlas builder; `None` for a series that draws no value labels.
    pub(in crate::renderer) value_labels: Option<Arc<data_render::ValueLabelAtlas>>,
    /// An energy-level series' CPU raster (#203); `None` for every other
    /// series.
    pub(in crate::renderer) energy_levels: Option<Arc<super::prepare::EnergyLevelRaster>>,
}

pub(in crate::renderer) struct PreparedContourItem {
    pub(in crate::renderer) series: Vec<PreparedContourSeries>,
}

pub(in crate::renderer) struct PreparedSeriesProducts<'a> {
    pub(in crate::renderer) arcs: &'a [PreparedArcSeries],
    pub(in crate::renderer) contours: &'a [PreparedContourSeries],
}

pub(in crate::renderer) struct PreparedAreaLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) base: ColumnHandle,
    pub(in crate::renderer) verts_per_instance: u32,
    pub(in crate::renderer) closed: bool,
}

/// A parallel coordinates series' rows and its picked row's redraw (#141).
pub(in crate::renderer) struct PreparedParallelLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) data: data_render::ParallelData,
    pub(in crate::renderer) selected: Option<(wgpu::RenderPipeline, wgpu::BindGroup, u32)>,
}

impl PreparedParallelLayer {
    pub(super) fn own(layer: data_render::ColumnParallelLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg.clone(),
            data: layer.data,
            selected: layer
                .selected
                .map(|selected| (selected.pipeline.clone(), selected.style_bg, selected.row)),
        }
    }

    pub(super) fn borrow(&self) -> data_render::ColumnParallelLayer<'_> {
        data_render::ColumnParallelLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: &self.style_bg,
            data: self.data.clone(),
            selected: self.selected.as_ref().map(|(pipeline, style_bg, row)| {
                data_render::ParallelSelection {
                    pipeline,
                    style_bg: style_bg.clone(),
                    row: *row,
                }
            }),
        }
    }
}

pub(in crate::renderer) struct PreparedLineLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) arc: Option<ArcPrefix>,
    pub(in crate::renderer) verts_per_instance: u32,
    pub(in crate::renderer) texture_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) closed: bool,
    /// A colour-mapped stroke: the value column and the groups 2 and 3 of
    /// the mapped line pipeline.
    pub(in crate::renderer) value_map: Option<(ColumnHandle, wgpu::BindGroup, wgpu::BindGroup)>,
    pub(in crate::renderer) third: Option<ColumnHandle>,
    pub(in crate::renderer) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

impl PreparedLineLayer {
    pub(super) fn own(layer: ColumnLineLayer<'_>) -> Self {
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

    pub(super) fn borrow(&self) -> ColumnLineLayer<'_> {
        self.borrow_with(None)
    }

    /// The layer as drawn with `transform` in place of its own transform
    /// bind group when given: a `Panels` piece's moved map (#200).
    pub(super) fn borrow_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> ColumnLineLayer<'a> {
        ColumnLineLayer {
            pipeline: &self.pipeline,
            transform_bg: transform.unwrap_or(&self.transform_bg),
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

pub(in crate::renderer) struct PreparedScatterLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) value_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) quad_vb: wgpu::Buffer,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) style_index: Option<ColumnHandle>,
    pub(in crate::renderer) color_value: Option<ColumnHandle>,
    pub(in crate::renderer) size_value: Option<ColumnHandle>,
    pub(in crate::renderer) texture_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) third: Option<ColumnHandle>,
    pub(in crate::renderer) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

impl PreparedScatterLayer {
    pub(super) fn own(layer: ColumnScatterLayer<'_>) -> Self {
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

    pub(super) fn borrow(&self) -> ColumnScatterLayer<'_> {
        self.borrow_with(None)
    }

    /// The layer as drawn with `transform` in place of its own transform
    /// bind group when given: a `Panels` piece's moved map (#200).
    pub(super) fn borrow_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> ColumnScatterLayer<'a> {
        ColumnScatterLayer {
            pipeline: &self.pipeline,
            transform_bg: transform.unwrap_or(&self.transform_bg),
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

pub(in crate::renderer) struct PreparedPickRingLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) value_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) quad_vb: wgpu::Buffer,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) style_index: Option<ColumnHandle>,
    pub(in crate::renderer) color_value: Option<ColumnHandle>,
    pub(in crate::renderer) size_value: Option<ColumnHandle>,
    pub(in crate::renderer) instance: u32,
    pub(in crate::renderer) third: Option<ColumnHandle>,
    pub(in crate::renderer) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

pub(in crate::renderer) struct PreparedBarSelectionLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) edges: ColumnHandle,
    pub(in crate::renderer) values: ColumnHandle,
    pub(in crate::renderer) instance: u32,
    pub(in crate::renderer) instances: u32,
    pub(in crate::renderer) category: Option<data_render::CategoryBarData>,
}

pub(in crate::renderer) struct PreparedFieldSelectionLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) selection_bg: wgpu::BindGroup,
    pub(in crate::renderer) selection_charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) drawable: bool,
    pub(in crate::renderer) vertices: u32,
    pub(in crate::renderer) instances: std::ops::Range<u32>,
}

pub(in crate::renderer) struct PreparedStarLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) texture_bg: wgpu::BindGroup,
    pub(in crate::renderer) star_bg: wgpu::BindGroup,
    pub(in crate::renderer) indirect: wgpu::Buffer,
}

/// Owned field layer. The grid table, level list, colourmap stops, lookup
/// metadata and `FieldParams` are all inside `field_bg`, which owns their
/// buffers — so the token needs nothing else to redraw the field.
pub(in crate::renderer) struct PreparedFieldLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) drawable: bool,
}

/// Owned triangle mesh layer (#210): a refcounted clone of the mesh's group
/// 2, which owns every table the draw reads.
pub(in crate::renderer) struct PreparedTriMeshLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) triangles: u32,
}

impl PreparedTriMeshLayer {
    pub(super) fn own(layer: data_render::tri_mesh::ColumnTriMeshLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg,
            field_bg: layer.field_bg,
            charge: layer.charge,
            triangles: layer.triangles,
        }
    }

    /// Borrowed for a draw, through `transform` when a panel piece of a
    /// broken axis draws it.
    pub(super) fn borrow<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> data_render::tri_mesh::ColumnTriMeshLayer<'a> {
        data_render::tri_mesh::ColumnTriMeshLayer {
            pipeline: &self.pipeline,
            transform_bg: transform.unwrap_or(&self.transform_bg),
            style_bg: self.style_bg.clone(),
            field_bg: self.field_bg.clone(),
            charge: Arc::clone(&self.charge),
            triangles: self.triangles,
        }
    }
}

/// Owned surface layer (a heatmap on a 3D chart): refcounted clones of the
/// field's group 2 and the surface's group 3.
pub(in crate::renderer) struct PreparedSurfaceLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) surface_bg: wgpu::BindGroup,
    pub(in crate::renderer) vertices: u32,
    pub(in crate::renderer) instances: u32,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) surface_charge: crate::gpu_memory::SharedCharge,
}

pub(in crate::renderer) struct PreparedTernaryRegionLayer {
    pub(in crate::renderer) stencil: wgpu::RenderPipeline,
    pub(in crate::renderer) cover: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) region_bg: wgpu::BindGroup,
    pub(in crate::renderer) count: u32,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) ring_charge: crate::gpu_memory::SharedCharge,
}

impl PreparedTernaryRegionLayer {
    pub(super) fn own(layer: data_render::ternary_region::ColumnTernaryRegionLayer<'_>) -> Self {
        Self {
            stencil: layer.stencil.clone(),
            cover: layer.cover.clone(),
            transform_bg: layer.transform_bg.clone(),
            region_bg: layer.region_bg,
            count: layer.count,
            charge: layer.charge,
            ring_charge: layer.ring_charge,
        }
    }

    pub(super) fn borrow(&self) -> data_render::ternary_region::ColumnTernaryRegionLayer<'_> {
        data_render::ternary_region::ColumnTernaryRegionLayer {
            stencil: &self.stencil,
            cover: &self.cover,
            transform_bg: &self.transform_bg,
            region_bg: self.region_bg.clone(),
            count: self.count,
            charge: self.charge.clone(),
            ring_charge: self.ring_charge.clone(),
        }
    }
}

impl PreparedSurfaceLayer {
    pub(super) fn own(layer: data_render::surface_mesh::ColumnSurfaceLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg,
            field_bg: layer.field_bg,
            surface_bg: layer.surface_bg,
            vertices: layer.vertices,
            instances: layer.instances,
            charge: layer.charge,
            surface_charge: layer.surface_charge,
        }
    }

    pub(super) fn borrow(&self) -> data_render::surface_mesh::ColumnSurfaceLayer<'_> {
        data_render::surface_mesh::ColumnSurfaceLayer {
            pipeline: &self.pipeline,
            transform_bg: &self.transform_bg,
            style_bg: self.style_bg.clone(),
            field_bg: self.field_bg.clone(),
            surface_bg: self.surface_bg.clone(),
            vertices: self.vertices,
            instances: self.instances,
            charge: Arc::clone(&self.charge),
            surface_charge: Arc::clone(&self.surface_charge),
        }
    }
}

/// Owned vector layer: refcounted clones of the group-2 state, like the field.
pub(in crate::renderer) struct PreparedVectorLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) vector_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) count: u32,
    /// Streamline arrowheads: the indirect arguments and the arc slot group.
    pub(in crate::renderer) indirect: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    /// A 3D chart's arrows (#211): the camera group.
    pub(in crate::renderer) view3d: Option<data_render::surface_mesh::View3dBinding>,
}

impl PreparedVectorLayer {
    pub(super) fn own(layer: data_render::vector::ColumnVectorLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg,
            vector_bg: layer.vector_bg,
            charge: layer.charge,
            count: layer.count,
            indirect: layer
                .indirect
                .map(|(buffer, slots)| (buffer.clone(), slots.clone())),
            view3d: layer.view3d,
        }
    }

    /// The borrowed layer, with `transform` in place of its own when given
    /// (a `Panels` piece, #200); a 3D layer keeps its own.
    pub(super) fn borrow_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> data_render::vector::ColumnVectorLayer<'a> {
        data_render::vector::ColumnVectorLayer {
            pipeline: &self.pipeline,
            transform_bg: transform
                .filter(|_| self.view3d.is_none())
                .unwrap_or(&self.transform_bg),
            style_bg: self.style_bg.clone(),
            vector_bg: self.vector_bg.clone(),
            charge: Arc::clone(&self.charge),
            count: self.count,
            indirect: self
                .indirect
                .as_ref()
                .map(|(buffer, slots)| (buffer, slots)),
            view3d: self.view3d.clone(),
        }
    }
}

/// Owned contour layer. The group-2 tables, the lump charge and the label state
/// are refcounted, so the token draws from its own clones even after a later
/// prepare replaces the scratch they came from.
pub(in crate::renderer) struct PreparedContourLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) field_bg: wgpu::BindGroup,
    pub(in crate::renderer) charge: crate::gpu_memory::SharedCharge,
    pub(in crate::renderer) drawable: bool,
    pub(in crate::renderer) label: Option<PreparedContourLabel>,
}

/// Owned label layer. Same reasoning as the lines above: the pipeline, bind group
/// and buffers are refcounted clones, so a later prepare replacing the scratch
/// cannot pull them out from under a token that can still be painted.
pub(in crate::renderer) struct PreparedContourLabel {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

pub(in crate::renderer) struct PreparedBarLayer {
    pub(in crate::renderer) envelope: Option<data_render::bar_envelope::Snapshot>,
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) edges: ColumnHandle,
    pub(in crate::renderer) values: ColumnHandle,
}

/// Owned category bar layer: its data group and the charge of its params.
pub(in crate::renderer) struct PreparedCategoryBarLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) data: data_render::CategoryBarData,
    pub(in crate::renderer) count: u32,
    pub(in crate::renderer) vertices: u32,
}

impl PreparedCategoryBarLayer {
    pub(super) fn new(layer: data_render::ColumnCategoryBarLayer<'_>) -> Self {
        Self {
            pipeline: layer.pipeline.clone(),
            transform_bg: layer.transform_bg.clone(),
            style_bg: layer.style_bg.clone(),
            data: layer.data,
            count: layer.count,
            vertices: layer.vertices,
        }
    }

    /// The layer bound to `transform`, or its own transform when `None`
    /// (a shifted `Panels` piece, #200).
    pub(super) fn layer_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> data_render::ColumnCategoryBarLayer<'a> {
        data_render::ColumnCategoryBarLayer {
            pipeline: &self.pipeline,
            transform_bg: transform.unwrap_or(&self.transform_bg),
            style_bg: &self.style_bg,
            data: self.data.clone(),
            count: self.count,
            vertices: self.vertices,
        }
    }
}

/// Owned value label layer: the label group (uniform and atlas), the
/// category bar data it reads, and the selected label's box pipeline.
pub(in crate::renderer) struct PreparedValueLabelLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) labels: data_render::ValueLabelData,
    pub(in crate::renderer) data: data_render::CategoryBarData,
    pub(in crate::renderer) selected: Option<(u32, wgpu::RenderPipeline)>,
}

pub(in crate::renderer) struct PreparedErrorBarLayer {
    pub(in crate::renderer) pipeline: wgpu::RenderPipeline,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_bg: wgpu::BindGroup,
    pub(in crate::renderer) style_map_bg: Option<wgpu::BindGroup>,
    pub(in crate::renderer) pool_buffer: wgpu::Buffer,
    pub(in crate::renderer) x: ColumnHandle,
    pub(in crate::renderer) y: ColumnHandle,
    pub(in crate::renderer) err_y_lo: ColumnHandle,
    pub(in crate::renderer) err_y_hi: ColumnHandle,
    pub(in crate::renderer) err_x_lo: ColumnHandle,
    pub(in crate::renderer) err_x_hi: ColumnHandle,
    pub(in crate::renderer) style_index: Option<ColumnHandle>,
}

impl PreparedErrorBarLayer {
    pub(super) fn own(layer: ColumnErrorBarDraw<'_>) -> Self {
        Self {
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
        }
    }

    pub(super) fn borrow_with<'a>(
        &'a self,
        transform: Option<&'a wgpu::BindGroup>,
    ) -> ColumnErrorBarDraw<'a> {
        ColumnErrorBarDraw {
            pipeline: &self.pipeline,
            transform_bg: transform.unwrap_or(&self.transform_bg),
            style_bg: &self.style_bg,
            style_map_bg: self.style_map_bg.as_ref(),
            pool_buffer: &self.pool_buffer,
            x: self.x,
            y: self.y,
            err_y_lo: self.err_y_lo,
            err_y_hi: self.err_y_hi,
            err_x_lo: self.err_x_lo,
            err_x_hi: self.err_x_hi,
            style_index: self.style_index,
        }
    }
}
