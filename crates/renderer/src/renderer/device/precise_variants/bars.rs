//! Creating the histogram, category bar, box plot, range bar, violin and value label pipelines a frame needs.

use super::*;

impl TargetPipelines {
    /// Creates the histogram, category bar, box plot, range bar, violin and value label pipelines the frame's items need and this target lacks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_bar_pipelines(
        &mut self,
        needs: &PreciseNeeds,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        category_bar_data_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
    ) {
        let PreciseNeeds {
            bar: need_bar,
            bar_mapped: need_bar_mapped,
            bar_selection: need_bar_selection,
            bar_selection_mapped: need_bar_selection_mapped,
            category_bars: need_category_bars,
            category_bar_selection: need_category_bar_selection,
            box_plot: need_box_plot,
            box_plot_selection: need_box_plot_selection,
            range_bars: need_range_bars,
            range_bar_selection: need_range_bar_selection,
            violin: need_violin,
            violin_selection: need_violin_selection,
            box_points: need_box_points,
            box_point_selection: need_box_point_selection,
            value_labels: need_value_labels,
            value_label_selection: need_value_label_selection,
            ..
        } = *needs;
        if need_bar && self.bar.is_none() {
            self.bar = Some(data_render::create_bar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if (need_bar || need_bar_mapped) && self.bar_envelope.is_none() {
            self.bar_envelope = Some(data_render::bar_envelope::Pipelines::new(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_bar_mapped && self.bar_mapped.is_none() {
            self.bar_mapped = Some(
                data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_bar_selection && self.bar_selection.is_none() {
            self.bar_selection = Some(
                data_render::create_bar_selection_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_bar_selection_mapped && self.bar_selection_mapped.is_none() {
            self.bar_selection_mapped = Some(
                data_render::create_bar_selection_mapped_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        for (need, selection) in [
            (need_category_bars, false),
            (need_category_bar_selection, true),
        ] {
            let slot = if selection {
                &mut self.bar_category_selection
            } else {
                &mut self.bar_category
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_category_bar_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, selection) in [(need_box_plot, false), (need_box_plot_selection, true)] {
            let slot = if selection {
                &mut self.bar_box_plot_selection
            } else {
                &mut self.bar_box_plot
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_box_plot_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, selection) in [(need_range_bars, false), (need_range_bar_selection, true)] {
            let slot = if selection {
                &mut self.bar_range_selection
            } else {
                &mut self.bar_range
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_range_bar_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, points, selection) in [
            (need_violin, false, false),
            (need_violin_selection, false, true),
            (need_box_points, true, false),
            (need_box_point_selection, true, true),
        ] {
            let slot = match (points, selection) {
                (false, false) => &mut self.bar_violin,
                (false, true) => &mut self.bar_violin_selection,
                (true, false) => &mut self.bar_box_points,
                (true, true) => &mut self.bar_box_point_selection,
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_violin_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    points,
                    selection,
                ));
            }
        }
        for (need, selection) in [
            (need_value_labels, false),
            (need_value_label_selection, true),
        ] {
            let slot = if selection {
                &mut self.bar_value_label_selection
            } else {
                &mut self.bar_value_label
            };
            if need && slot.is_none() {
                // An equal layout to the renderer's own: wgpu compares
                // layouts by their entries, so its bind groups fit.
                let value_label_bgl = data_render::create_value_label_bind_group_layout(device);
                *slot = Some(data_render::create_value_label_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    &value_label_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
    }
}
