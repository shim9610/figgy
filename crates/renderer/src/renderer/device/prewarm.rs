//! Compiling every pipeline of a target ahead of first use.

use super::*;

impl TargetPipelines {
    /// Compile every target-format render pipeline, including variants that no
    /// current chart needs. Cold-start hosts use this before accepting their
    /// first chart so a later render-type/style switch cannot move shader
    /// compilation back onto the interaction that requested it.
    #[allow(clippy::too_many_arguments)]
    /// Create the pipelines [`Self::prewarm_pipelines`] creates, yielding to
    /// the browser after each one (no yield on native).
    pub(in crate::renderer) async fn prewarm_all_observed(
        &mut self,
        layouts: &PrewarmLayouts<'_>,
        observer: &mut dyn FnMut(InitEvent),
    ) {
        loop {
            let (done, created) = self.prewarm_pipelines(layouts, observer, Some(1));
            if created > 0 {
                crate::init::yield_init_frame().await;
            }
            if done {
                return;
            }
        }
    }

    /// [`Self::prewarm_all_observed`] in one call, without yielding.
    #[cfg(not(target_arch = "wasm32"))]
    pub(in crate::renderer) fn prewarm_all_observed_blocking(
        &mut self,
        layouts: &PrewarmLayouts<'_>,
        observer: &mut dyn FnMut(InitEvent),
    ) {
        // Without a budget the core never pauses: it creates every pipeline.
        self.prewarm_pipelines(layouts, observer, None);
    }

    /// Create every pipeline `feature` (all of them for `None`) needs and
    /// does not have yet, at most `budget` of them (`None`: no limit). Returns
    /// whether every one exists now, and how many this call created. Each
    /// pipeline is created once: a call after a paused one goes on where it
    /// stopped. The waiting drivers are [`Self::prewarm_all_observed`], which
    /// yields to the browser after each pipeline, and
    /// `Self::prewarm_all_observed_blocking`.
    pub(super) fn prewarm_pipelines(
        &mut self,
        layouts: &PrewarmLayouts<'_>,
        observer: &mut dyn FnMut(InitEvent),
        budget: Option<usize>,
    ) -> (bool, usize) {
        let PrewarmLayouts {
            device,
            queue,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            data_selection_bgl,
            field_bgl,
            vector_bgl,
            category_bar_data_bgl,
            parallel_bgl,
            star_data_bgl,
            label_pipelines,
            feature,
            surface_format,
        } = *layouts;
        let mut created = 0usize;
        macro_rules! prewarm_step {
            ($observer:expr, $scope:expr, $stage:expr, $create:expr $(,)?) => {{
                if budget == Some(created) {
                    return (false, created);
                }
                created += 1;
                crate::init::observe_value($observer, $scope, $stage, $create)
            }};
        }
        macro_rules! ensure {
            ($field:ident, $stage:literal, $create:expr) => {
                if self.$field.is_none()
                    && feature.is_none_or(|f| f.includes_field(stringify!($field)))
                {
                    self.$field = Some(prewarm_step!(observer, "renderer.prewarm", $stage, || {
                        $create
                    }));
                }
            };
        }

        ensure!(
            line,
            "precise line",
            data_render::create_line_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            line_mapped,
            "colour-mapped line",
            data_render::create_line_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            area,
            "line area fill",
            data_render::create_line_area_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            line_parallel,
            "parallel rows",
            data_render::create_line_parallel_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                parallel_bgl,
                surface_format,
                self.sample_count,
                false,
            )
        );
        ensure!(
            line_parallel_selection,
            "selected parallel row",
            data_render::create_line_parallel_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                parallel_bgl,
                surface_format,
                self.sample_count,
                true,
            )
        );
        ensure!(
            scatter,
            "precise scatter",
            data_render::create_scatter_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            errorbar,
            "precise errorbar",
            data_render::create_errorbar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            errorbar_stem,
            "precise stem",
            data_render::create_stem_pipeline(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_stem",
            )
        );
        ensure!(
            scatter_mapped,
            "mapped scatter",
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
            )
        );
        ensure!(
            pick_ring,
            "picked point ring",
            data_render::create_scatter_columnar_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_pick_ring",
                "fs_pick_ring",
                "figgy picked point ring pipeline",
            )
        );
        ensure!(
            pick_ring_mapped,
            "mapped picked point ring",
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
            )
        );
        for kind in TernaryPipeline::ALL {
            if self.ternary[kind as usize].is_none()
                && feature.is_none_or(|f| f.includes_field("ternary"))
            {
                self.ternary[kind as usize] = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    kind.stage(),
                    || {
                        create_ternary_pipeline(
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
                        )
                    }
                ));
            }
        }
        ensure!(
            errorbar_mapped,
            "mapped errorbar",
            data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar,
            "histogram bars",
            data_render::create_bar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_mapped,
            "mapped histogram bars",
            data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_envelope,
            "histogram envelope",
            data_render::bar_envelope::Pipelines::new(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_selection,
            "selected histogram bin",
            data_render::create_bar_selection_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_selection_mapped,
            "mapped selected histogram bin",
            data_render::create_bar_selection_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_category_selection
            } else {
                &mut self.bar_category
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_category")) {
                *slot = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    if selection {
                        "selected category bar"
                    } else {
                        "category bars"
                    },
                    || {
                        data_render::create_category_bar_pipeline_with_sample_count(
                            device,
                            &self.shaders.bar,
                            transform_bgl,
                            style_bgl,
                            per_point_style_map_bgl,
                            category_bar_data_bgl,
                            surface_format,
                            self.sample_count,
                            selection,
                        )
                    },
                ));
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_box_plot_selection
            } else {
                &mut self.bar_box_plot
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_box_plot")) {
                *slot = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    if selection {
                        "selected box"
                    } else {
                        "box plots"
                    },
                    || {
                        data_render::create_box_plot_pipeline_with_sample_count(
                            device,
                            &self.shaders.bar,
                            transform_bgl,
                            style_bgl,
                            per_point_style_map_bgl,
                            category_bar_data_bgl,
                            surface_format,
                            self.sample_count,
                            selection,
                        )
                    },
                ));
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_range_selection
            } else {
                &mut self.bar_range
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_range")) {
                *slot = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    if selection {
                        "selected range bar"
                    } else {
                        "range bars"
                    },
                    || {
                        data_render::create_range_bar_pipeline_with_sample_count(
                            device,
                            &self.shaders.bar,
                            transform_bgl,
                            style_bgl,
                            per_point_style_map_bgl,
                            category_bar_data_bgl,
                            surface_format,
                            self.sample_count,
                            selection,
                        )
                    },
                ));
            }
        }
        for (points, selection) in [(false, false), (false, true), (true, false), (true, true)] {
            let (slot, field, label) = match (points, selection) {
                (false, false) => (&mut self.bar_violin, "bar_violin", "violins"),
                (false, true) => (
                    &mut self.bar_violin_selection,
                    "bar_violin",
                    "selected violin",
                ),
                (true, false) => (&mut self.bar_box_points, "bar_box_points", "box points"),
                (true, true) => (
                    &mut self.bar_box_point_selection,
                    "bar_box_points",
                    "selected box point",
                ),
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field(field)) {
                *slot = Some(prewarm_step!(observer, "renderer.prewarm", label, || {
                    data_render::create_violin_pipeline_with_sample_count(
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
                    )
                }));
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_value_label_selection
            } else {
                &mut self.bar_value_label
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_value_label")) {
                let value_label_bgl = data_render::create_value_label_bind_group_layout(device);
                *slot = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    if selection {
                        "selected value label"
                    } else {
                        "value labels"
                    },
                    || {
                        data_render::create_value_label_pipeline_with_sample_count(
                            device,
                            &self.shaders.bar,
                            transform_bgl,
                            &value_label_bgl,
                            per_point_style_map_bgl,
                            category_bar_data_bgl,
                            surface_format,
                            self.sample_count,
                            selection,
                        )
                    },
                ));
            }
        }
        ensure!(
            field,
            "heatmap field",
            data_render::create_field_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
            )
        );
        // A triangle mesh's fill and contours (#210), prepared with the
        // fields: they read the field's group 2.
        for pass in data_render::tri_mesh::TriMeshPass::ALL {
            if self.tri_mesh[pass as usize].is_none()
                && feature.is_none_or(|f| f.includes_field("field_tri_mesh"))
            {
                let shader = self
                    .tri_mesh_shader
                    .get_or_insert_with(|| data_render::tri_mesh::shader_module(device));
                self.tri_mesh[pass as usize] = Some(prewarm_step!(
                    observer,
                    "renderer.prewarm",
                    pass.label(),
                    || {
                        data_render::tri_mesh::create_tri_mesh_pipeline(
                            device,
                            shader,
                            transform_bgl,
                            // The selection entry binds the data selection
                            // uniform as group 1.
                            if pass == data_render::tri_mesh::TriMeshPass::Selection {
                                data_selection_bgl
                            } else {
                                style_bgl
                            },
                            field_bgl,
                            surface_format,
                            self.sample_count,
                            pass,
                        )
                    }
                ));
            }
        }
        ensure!(
            vector_points,
            "vector points",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_points,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Points,
            )
        );
        ensure!(
            vector_path,
            "vector path",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_path,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Path,
            )
        );
        ensure!(
            vector_grid,
            "vector grid",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_grid,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Grid,
            )
        );
        ensure!(
            contour,
            "contour field",
            data_render::create_field_columnar_pipeline_with_entry(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
                "fs_contour",
                "figgy field contour pipeline",
            )
        );
        // The labelled contour pipelines need the label engine. Only the
        // Fields feature and the full prewarm include them, and both pass it;
        // a feature without it does not include these fields.
        if let Some(label_pipelines) = label_pipelines {
            ensure!(
                contour_labelled,
                "label-gapped contour field",
                data_render::create_labelled_contour_pipeline(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    style_bgl,
                    field_bgl,
                    label_pipelines.label_gap_bgl(),
                    surface_format,
                    self.sample_count,
                )
            );
        }
        ensure!(
            field_selection,
            "selected field data",
            data_render::create_field_selection_pipeline_with_sample_count(
                device,
                &self.shaders.field,
                transform_bgl,
                data_selection_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
            )
        );
        if let Some(label_pipelines) = label_pipelines {
            ensure!(
                field_selection_labelled,
                "selected labelled field data",
                data_render::create_field_selection_labelled_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    data_selection_bgl,
                    field_bgl,
                    label_pipelines.label_gap_bgl(),
                    surface_format,
                    self.sample_count,
                )
            );
            ensure!(
                contour_label,
                "contour labels",
                label_pipelines.render_pipeline(
                    device,
                    transform_bgl,
                    surface_format,
                    self.sample_count
                )
            );
        }

        for (key, stage) in [
            (StyleKey::Sketch, "hand-drawn style"),
            (StyleKey::Milkyway, "milkyway style"),
            (StyleKey::Constellation, "constellation style"),
        ] {
            if feature.is_some_and(|f| f.style_key() != Some(key)) || self.styled.contains_key(&key)
            {
                continue;
            }
            let set = prewarm_step!(observer, "renderer.prewarm", stage, || {
                create_style_set(
                    key,
                    device,
                    queue,
                    &self.shaders,
                    transform_bgl,
                    style_bgl,
                    star_data_bgl,
                    surface_format,
                    self.sample_count,
                )
            });
            self.styled.insert(key, set);
        }
        (true, created)
    }
}

/// The device and layouts a pipeline prewarm builds with, for
/// [`TargetPipelines::prewarm_all_observed`] and its `_blocking` twin.
#[derive(Clone, Copy)]
pub(in crate::renderer) struct PrewarmLayouts<'a> {
    pub(in crate::renderer) device: &'a wgpu::Device,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) transform_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) style_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) per_point_style_map_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) point_value_map_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) data_selection_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) field_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) vector_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) category_bar_data_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) parallel_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) star_data_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) label_pipelines: Option<&'a crate::gpu_contour::ContourLabelPipelines>,
    pub(in crate::renderer) feature: Option<PreparationFeature>,
    pub(in crate::renderer) surface_format: wgpu::TextureFormat,
}
