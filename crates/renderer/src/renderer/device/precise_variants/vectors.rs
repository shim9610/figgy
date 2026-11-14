//! Creating the vector pipelines and the field selection a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the vector pipelines and the field selection the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_vector_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        data_selection_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        label_pipelines: Option<&crate::gpu_contour::ContourLabelPipelines>,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            vector_points: need_vector_points,
            vector_grid: need_vector_grid,
            vector_path: need_vector_path,
            field_selection: need_field_selection,
            ..
        } = *needs;
        for (need, input) in [
            (need_vector_points, data_render::vector::VectorInput::Points),
            (need_vector_grid, data_render::vector::VectorInput::Grid),
            (need_vector_path, data_render::vector::VectorInput::Path),
        ] {
            let slot = match input {
                data_render::vector::VectorInput::Points => &mut self.vector_points,
                data_render::vector::VectorInput::Grid => &mut self.vector_grid,
                data_render::vector::VectorInput::Path => &mut self.vector_path,
                // A 3D chart's arrows compile with the 3D kinds (#211).
                data_render::vector::VectorInput::Points3d => continue,
            };
            if need && slot.is_none() {
                *slot = Some(
                    data_render::vector::create_vector_pipeline_with_sample_count(
                        device,
                        self.shaders.vector(input),
                        transform_bgl,
                        style_bgl,
                        vector_bgl,
                        star_data_bgl,
                        surface_format,
                        self.sample_count,
                        input,
                    ),
                );
            }
        }
        if need_field_selection && self.field_selection.is_none() {
            self.field_selection = Some(
                data_render::create_field_selection_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    data_selection_bgl,
                    field_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_field_selection
            && self.field_selection_labelled.is_none()
            && let Some(labels) = label_pipelines
        {
            self.field_selection_labelled = Some(
                data_render::create_field_selection_labelled_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    data_selection_bgl,
                    field_bgl,
                    labels.label_gap_bgl(),
                    surface_format,
                    self.sample_count,
                ),
            );
        }
    }
}
