//! Creating the line, area, parallel and scatter pipelines a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the line, area, parallel and scatter pipelines the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_line_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        parallel_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            parallel: need_parallel,
            parallel_selection: need_parallel_selection,
            line: need_line,
            line_mapped: need_line_mapped,
            area: need_area,
            scatter: need_scatter,
            ..
        } = *needs;
        if need_line && self.line.is_none() {
            self.line = Some(
                data_render::create_line_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.line,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_line_mapped && self.line_mapped.is_none() {
            self.line_mapped = Some(data_render::create_line_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_area && self.area.is_none() {
            self.area = Some(data_render::create_line_area_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        for (need, slot, selection) in [
            (need_parallel, &mut self.line_parallel, false),
            (
                need_parallel_selection,
                &mut self.line_parallel_selection,
                true,
            ),
        ] {
            if need && slot.is_none() {
                *slot = Some(
                    data_render::create_line_parallel_pipeline_with_sample_count(
                        device,
                        &self.shaders.line,
                        transform_bgl,
                        style_bgl,
                        parallel_bgl,
                        surface_format,
                        self.sample_count,
                        selection,
                    ),
                );
            }
        }
        if need_scatter && self.scatter.is_none() {
            self.scatter = Some(
                data_render::create_scatter_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
    }
}
