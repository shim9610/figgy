//! Creating the error bar, stem, mapped scatter and pick ring pipelines a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the error bar, stem, mapped scatter and pick ring pipelines the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_marker_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            errorbar: need_errorbar,
            stem: need_stem,
            scatter_mapped: need_scatter_mapped,
            pick_ring: need_pick_ring,
            pick_ring_mapped: need_pick_ring_mapped,
            ..
        } = *needs;
        if need_errorbar && self.errorbar.is_none() {
            self.errorbar = Some(
                data_render::create_errorbar_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.errorbar,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_stem && self.errorbar_stem.is_none() {
            self.errorbar_stem = Some(data_render::create_stem_pipeline(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_stem",
            ));
        }
        if need_scatter_mapped && self.scatter_mapped.is_none() {
            self.scatter_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    point_value_map_bgl,
                    surface_format,
                    self.sample_count,
                    "vs_mapped",
                    "fs_mapped",
                    "figgy scatter mapped pipeline",
                ),
            );
        }
        if need_pick_ring && self.pick_ring.is_none() {
            self.pick_ring = Some(data_render::create_scatter_columnar_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_pick_ring",
                "fs_pick_ring",
                "figgy picked point ring pipeline",
            ));
        }
        if need_pick_ring_mapped && self.pick_ring_mapped.is_none() {
            self.pick_ring_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    point_value_map_bgl,
                    surface_format,
                    self.sample_count,
                    "vs_pick_ring_mapped",
                    "fs_pick_ring",
                    "figgy picked point mapped ring pipeline",
                ),
            );
        }
    }
}
