//! Creating the ternary and 3D entries and the mapped error bar a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the ternary and 3D entries and the mapped error bar the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_entry_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        surface_bgl: &wgpu::BindGroupLayout,
        view3d_value_map_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            errorbar_mapped: need_errorbar_mapped,
            ternary: need_ternary,
            view3d: need_view3d,
            ..
        } = *needs;
        for kind in TernaryPipeline::ALL {
            if need_ternary[kind as usize] && self.ternary[kind as usize].is_none() {
                self.ternary[kind as usize] = Some(create_ternary_pipeline(
                    kind,
                    device,
                    &self.shaders,
                    [
                        transform_bgl,
                        style_bgl,
                        per_point_style_map_bgl,
                        point_value_map_bgl,
                    ],
                    surface_format,
                    self.sample_count,
                ));
            }
        }
        for kind in View3dPipeline::ALL {
            if need_view3d[kind as usize] {
                self.ensure_view3d_pass(
                    device,
                    [
                        transform_bgl,
                        style_bgl,
                        per_point_style_map_bgl,
                        surface_bgl,
                        view3d_value_map_bgl,
                        vector_bgl,
                    ],
                    surface_format,
                    kind,
                );
            }
        }
        if need_errorbar_mapped && self.errorbar_mapped.is_none() {
            self.errorbar_mapped = Some(
                data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                    device,
                    &self.shaders.errorbar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
    }
}
