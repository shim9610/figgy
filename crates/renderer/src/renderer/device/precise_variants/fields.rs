//! Creating the field, mesh and contour pipelines, with the contour labels a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the field, mesh and contour pipelines, with the contour labels the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_field_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        data_selection_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        label_pipelines: Option<&crate::gpu_contour::ContourLabelPipelines>,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            field: need_field,
            contour: need_contour,
            mesh: need_mesh,
            mesh_contour: need_mesh_contour,
            mesh_selection: need_mesh_selection,
            contour_label: need_contour_label,
            ..
        } = *needs;
        if need_field && self.field.is_none() {
            self.field = Some(
                data_render::create_field_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    style_bgl,
                    field_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        for (needed, pass) in [
            (need_mesh, data_render::tri_mesh::TriMeshPass::Fill),
            (
                need_mesh_contour,
                data_render::tri_mesh::TriMeshPass::Contour,
            ),
        ] {
            if needed {
                self.ensure_tri_mesh_pass(
                    device,
                    [transform_bgl, style_bgl, field_bgl],
                    surface_format,
                    pass,
                );
            }
        }
        if need_mesh_selection {
            self.ensure_tri_mesh_pass(
                device,
                [transform_bgl, data_selection_bgl, field_bgl],
                surface_format,
                data_render::tri_mesh::TriMeshPass::Selection,
            );
        }
        if need_contour && self.contour.is_none() {
            self.contour = Some(data_render::create_field_columnar_pipeline_with_entry(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
                "fs_contour",
                "figgy field contour pipeline",
            ));
        }
        if need_contour_label
            && self.contour_labelled.is_none()
            && let Some(labels) = label_pipelines
        {
            self.contour_labelled = Some(data_render::create_labelled_contour_pipeline(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                labels.label_gap_bgl(),
                surface_format,
                self.sample_count,
            ));
        }
        if need_contour_label
            && self.contour_label.is_none()
            && let Some(labels) = label_pipelines
        {
            self.contour_label = Some(labels.render_pipeline(
                device,
                transform_bgl,
                surface_format,
                self.sample_count,
            ));
        }
    }
}
