//! Triangle mesh fields (#210): the assembled module, its pipelines and the
//! layer one mesh series draws as.
//!
//! A mesh reads the field's group 2 unchanged
//! ([`create_field_data_bind_group_layout`](super::create_field_data_bind_group_layout))
//! under [`FIELD_FLAG_MESH`](super::FIELD_FLAG_MESH): `FieldParams` and the
//! grid table are reinterpreted as SHADER_COMMON.md §5.25 says, so the fill
//! colours through the heatmap's `field_value_color` and the contours through
//! its level search. One instance per triangle and three vertices; no vertex
//! buffers. On a 3D chart the mesh draws as a surface
//! ([`super::surface_mesh::TRI_SURFACE_SHADER`]) and its contours on the box
//! floor through this module's floor entries (`mesh_floor.wgsl`).

/// The assembled triangle mesh module: the field's shared fragments, then the
/// mesh input, its flat entries and its floor entries (which project through
/// the camera, `project_3d.wgsl`, as the field module's floor does).
pub(crate) const TRI_MESH_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/probability.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/field_grid.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/field_paint.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/common/project_3d.wgsl",
    "data_render/tri_mesh/mesh_input.wgsl",
    "data_render/tri_mesh/mesh_entry.wgsl",
    "data_render/tri_mesh/mesh_floor.wgsl",
);

/// Vertices per triangle instance.
pub const TRI_MESH_VERTICES: u32 = 3;

/// The grid table entry of a mesh's value column (`MESH_VALUES`).
pub const MESH_TABLE_VALUES: usize = 0;
/// The grid table entry of a mesh's triangle index column (`MESH_INDICES`).
pub const MESH_TABLE_INDICES: usize = 1;
/// The grid table entry of a mesh's height column on a 3D chart
/// (`MESH_HEIGHTS`, the series' `z_column`); a flat mesh's table has two
/// entries.
pub const MESH_TABLE_HEIGHTS: usize = 2;

/// Which triangle mesh pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TriMeshPass {
    /// The fill (`vs_tri` / `fs_tri`).
    Fill,
    /// The contour strokes over the same triangles (`vs_tri` /
    /// `fs_tri_contour`).
    Contour,
    /// The picked triangle's outline or the picked level's widened stroke
    /// (`vs_tri` / `fs_tri_selection`), with the data selection uniform as
    /// group 1 in place of the style.
    Selection,
}

// `TargetPipelines::tri_mesh` has `ALL.len()` slots indexed by
// `pass as usize`: the last discriminant is the last slot.
const _: () = assert!(TriMeshPass::Selection as usize + 1 == TriMeshPass::ALL.len());

impl TriMeshPass {
    pub const ALL: [TriMeshPass; 3] = [
        TriMeshPass::Fill,
        TriMeshPass::Contour,
        TriMeshPass::Selection,
    ];

    fn entries(self) -> (&'static str, &'static str) {
        match self {
            TriMeshPass::Fill => ("vs_tri", "fs_tri"),
            TriMeshPass::Contour => ("vs_tri", "fs_tri_contour"),
            TriMeshPass::Selection => ("vs_tri", "fs_tri_selection"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TriMeshPass::Fill => "figgy triangle mesh field pipeline",
            TriMeshPass::Contour => "figgy triangle mesh contour pipeline",
            TriMeshPass::Selection => "figgy selected triangle mesh pipeline",
        }
    }
}

/// The triangle mesh module.
pub(crate) fn shader_module(device: &wgpu::Device) -> wgpu::ShaderModule {
    crate::gpu_compile::shader_module(
        device,
        wgpu::ShaderModuleDescriptor {
            label: Some("figgy triangle mesh shader"),
            source: wgpu::ShaderSource::Wgsl(TRI_MESH_SHADER.source.into()),
        },
    )
}

/// One triangle mesh pipeline: groups transform, style (the data selection
/// uniform for [`TriMeshPass::Selection`]) and the field's group 2,
/// premultiplied blending into `target_format` at `sample_count`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_tri_mesh_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: TriMeshPass,
) -> wgpu::RenderPipeline {
    let (vertex_entry, fragment_entry) = pass.entries();
    super::create_field_render_pipeline(
        device,
        shader,
        [transform_bgl, style_bgl, field_bgl],
        None,
        target_format,
        sample_count,
        vertex_entry,
        fragment_entry,
        pass.label(),
    )
}

/// One triangle mesh series' fill or contour draw: the field's group 2
/// snapshot and the triangle count to instance.
pub struct ColumnTriMeshLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// Owned: the fill's `Style` holds the chart's `colorbar.nan_color`, the
    /// contour's its stroke, built where the mesh is resolved.
    pub style_bg: wgpu::BindGroup,
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group 2 buffers' lump charge alive.
    pub charge: crate::gpu_memory::SharedCharge,
    /// The triangles to instance; 0 skips the draw.
    pub triangles: u32,
}

/// Draw `layer` into the current pass.
pub fn issue_tri_mesh_layer(pass: &mut wgpu::RenderPass<'_>, layer: &ColumnTriMeshLayer<'_>) {
    if layer.triangles == 0 {
        return;
    }
    pass.set_pipeline(layer.pipeline);
    pass.set_bind_group(0, layer.transform_bg, &[]);
    pass.set_bind_group(1, &layer.style_bg, &[]);
    pass.set_bind_group(2, &layer.field_bg, &[]);
    pass.draw(0..TRI_MESH_VERTICES, 0..layer.triangles);
}
