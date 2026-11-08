//! Creating the precise (unstyled) pipeline variants a frame's items need.

use super::*;

impl TargetPipelines {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn ensure_precise_variants_for_items(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        [surface_bgl, view3d_value_map_bgl]: [&wgpu::BindGroupLayout; 2],
        data_selection_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        category_bar_data_bgl: &wgpu::BindGroupLayout,
        parallel_bgl: &wgpu::BindGroupLayout,
        label_pipelines: Option<&crate::gpu_contour::ContourLabelPipelines>,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        let mut need_parallel = false;
        let mut need_parallel_selection = false;
        let mut need_vector_points = false;
        let mut need_vector_grid = false;
        let mut need_vector_path = false;
        let mut need_field = false;
        let mut need_contour = false;
        let mut need_mesh = false;
        let mut need_mesh_contour = false;
        let mut need_mesh_selection = false;
        let mut need_contour_label = false;
        let mut need_line = false;
        let mut need_line_mapped = false;
        let mut need_area = false;
        let mut need_scatter = false;
        let mut need_errorbar = false;
        let mut need_stem = false;
        let mut need_bar = false;
        let mut need_bar_mapped = false;
        let mut need_bar_selection = false;
        let mut need_bar_selection_mapped = false;
        let mut need_category_bars = false;
        let mut need_category_bar_selection = false;
        let mut need_box_plot = false;
        let mut need_box_plot_selection = false;
        let mut need_range_bars = false;
        let mut need_range_bar_selection = false;
        let mut need_violin = false;
        let mut need_violin_selection = false;
        let mut need_box_points = false;
        let mut need_box_point_selection = false;
        let mut need_value_labels = false;
        let mut need_value_label_selection = false;
        let mut need_field_selection = false;
        let mut need_scatter_mapped = false;
        let mut need_errorbar_mapped = false;
        let mut need_pick_ring = false;
        let mut need_pick_ring_mapped = false;
        let mut need_ternary = [false; TERNARY_PIPELINES];
        let mut need_view3d = [false; VIEW3D_PIPELINES];
        for item in items {
            let typed_picks = item
                .chart_config
                .picked_data
                .as_ref()
                .filter(|config| config.visible && !config.refs.is_empty());
            let typed_point = typed_picks.is_some_and(|config| {
                config
                    .refs
                    .iter()
                    .any(|picked| matches!(picked, PickedDataRef::Point { .. }))
            });
            let typed_bin = typed_picks.is_some_and(|config| {
                config
                    .refs
                    .iter()
                    .any(|picked| matches!(picked, PickedDataRef::HistogramBin { .. }))
            });
            if let Some(config) = typed_picks {
                need_pick_ring |= typed_point;
                need_bar_selection |= typed_bin;
                need_field_selection |= config.refs.iter().any(|picked| {
                    matches!(
                        picked,
                        PickedDataRef::MatrixCell { .. } | PickedDataRef::ContourLevel { .. }
                    )
                });
            }
            // The area fill is drawn under every style's line with the one
            // precise area pipeline.
            for series in item.series {
                need_area |=
                    series_has_line_fill(&item.chart_config.draw_style, &series.config.render_type);
            }
            if style_variant(&item.chart_config.draw_style).is_some() {
                continue;
            }
            for series in item.series {
                let primitives = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                );
                need_field |= primitives.field;
                // A contour is the field quad through a different fragment entry
                // point, so it needs no line pipeline at all any more.
                need_contour |= primitives.contour;
                // A mesh on a 3D chart draws through the surface pipelines
                // (`ensure_surface_pipelines`), not the flat mesh ones.
                let mesh_flat = item.chart_config.view_3d().is_none();
                need_mesh |= primitives.mesh && mesh_flat;
                need_mesh_contour |= primitives.mesh_contour && mesh_flat;
                // A picked triangle or level redraws through the mesh's
                // selection entry (#210).
                need_mesh_selection |= (primitives.mesh || primitives.mesh_contour)
                    && mesh_flat
                    && typed_picks.is_some_and(|config| {
                        config.refs.iter().any(|picked| {
                            matches!(
                                picked,
                                PickedDataRef::MeshTriangle { .. }
                                    | PickedDataRef::ContourLevel { .. }
                            )
                        })
                    });
                need_line |= primitives.line;
                // Streamline arrowheads draw through the path vector pipeline.
                need_vector_path |= primitives.line
                    && series
                        .config
                        .render_type
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.arrows.is_some());
                need_line_mapped |= primitives.line
                    && series
                        .config
                        .render_type
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.color_map.is_some());
                need_contour_label |=
                    primitives.contour && series_wants_contour_labels(&series.config.render_type);
                // A parallel series' rows, and the picked row's redraw when
                // a point is picked (#141).
                need_parallel |= primitives.parallel;
                need_parallel_selection |= primitives.parallel && typed_point;
                need_scatter |= primitives.scatter;
                need_bar |= primitives.bar;
                need_category_bars |= primitives.category_bars;
                need_category_bar_selection |= primitives.category_bars && typed_bin;
                need_box_plot |= primitives.box_plot;
                need_box_plot_selection |= primitives.box_plot && typed_bin;
                need_range_bars |= primitives.range_bars;
                need_range_bar_selection |= primitives.range_bars && typed_bin;
                need_violin |= primitives.violin;
                need_violin_selection |= primitives.violin && typed_bin;
                need_box_points |= primitives.box_points;
                need_box_point_selection |= primitives.box_points && typed_point;
                // Value labels compile with any label the series can show;
                // `Emphasized` radar labels with or without a selection.
                let labels =
                    data_render::value_label_kind(&series.config.render_type, true).is_some();
                need_value_labels |= labels;
                need_value_label_selection |= labels
                    && typed_picks.is_some_and(|config| {
                        config.refs.iter().any(|picked| {
                            matches!(picked, PickedDataRef::ValueLabel { series_id, .. }
                                if *series_id == series.config.series_id)
                        })
                    });
                let vector_3d = item.chart_config.drawn_z_column(series.config).is_some()
                    && extract_vector(&series.config.render_type).is_some();
                if primitives.vector
                    && !vector_3d
                    && let Some((components, _)) = extract_vector(&series.config.render_type)
                {
                    match components.grid() {
                        Some(_) => need_vector_grid = true,
                        None => need_vector_points = true,
                    }
                    // A selected grid cell draws through the field selection
                    // pipeline; a selected point anchor through the ring.
                }
                need_field_selection |= primitives.vector
                    && typed_picks.is_some_and(|config| {
                        config
                            .refs
                            .iter()
                            .any(|picked| matches!(picked, PickedDataRef::MatrixCell { .. }))
                    });
                need_bar_mapped |= primitives.bar && series.style.bar_map.is_some();
                // The outline takes the mapped entry exactly when the draw does.
                need_bar_selection_mapped |=
                    primitives.bar && series.style.bar_map.is_some() && typed_bin;
                need_errorbar |= has_errorbar(&series.config.render_type);
                need_stem |= has_stem(&series.config.render_type);
                // `has_index` only means a style-index column is bound.
                // Sparse `*_style_overrides` still need the mapped shader.
                if primitives.scatter && series.style.scatter_map.is_some() {
                    need_scatter_mapped = true;
                    if typed_point {
                        need_pick_ring_mapped = true;
                    }
                }
                // A ternary series draws through the ternary entries.
                if series.config.render_type.ternary_c_column().is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = typed_point;
                    need_ternary[TernaryPipeline::Line as usize] |= primitives.line;
                    need_ternary[TernaryPipeline::Scatter as usize] |=
                        primitives.scatter && !mapped;
                    need_ternary[TernaryPipeline::ScatterMapped as usize] |=
                        primitives.scatter && mapped;
                    need_ternary[TernaryPipeline::Ring as usize] |=
                        primitives.scatter && !mapped && ring;
                    need_ternary[TernaryPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
                if has_errorbar(&series.config.render_type) && series.style.errorbar_map.is_some() {
                    need_errorbar_mapped = true;
                }
                // A scatter or line series on a 3D chart draws through the
                // 3D entries (#132); its picks compile when it is picked.
                if item.chart_config.view_3d().is_some() && series.config.z_column.is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = typed_point;
                    need_view3d[View3dPipeline::Line as usize] |= primitives.line;
                    need_view3d[View3dPipeline::Scatter as usize] |= primitives.scatter && !mapped;
                    need_view3d[View3dPipeline::ScatterMapped as usize] |=
                        primitives.scatter && mapped;
                    need_view3d[View3dPipeline::Ring as usize] |=
                        primitives.scatter && !mapped && ring;
                    need_view3d[View3dPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
                // A point vector on a 3D chart draws its arrows through the
                // camera (#211); its picked anchor through the 3D ring.
                if vector_3d {
                    need_view3d[View3dPipeline::Vector as usize] |= primitives.vector;
                    need_view3d[View3dPipeline::Ring as usize] |= primitives.vector && typed_point;
                }
            }
        }
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
