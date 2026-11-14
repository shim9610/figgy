//! Creating the precise (unstyled) pipeline variants a frame's items need.

mod bars;
mod entries;
mod fields;
mod lines;
mod markers;
mod vectors;

use super::*;

/// The precise pipelines a frame's items need, read from their configs,
/// series and picks before any pipeline is created.
struct PreciseNeeds {
    parallel: bool,
    parallel_selection: bool,
    vector_points: bool,
    vector_grid: bool,
    vector_path: bool,
    field: bool,
    contour: bool,
    mesh: bool,
    mesh_contour: bool,
    mesh_selection: bool,
    contour_label: bool,
    line: bool,
    line_mapped: bool,
    area: bool,
    scatter: bool,
    errorbar: bool,
    stem: bool,
    bar: bool,
    bar_mapped: bool,
    bar_selection: bool,
    bar_selection_mapped: bool,
    category_bars: bool,
    category_bar_selection: bool,
    box_plot: bool,
    box_plot_selection: bool,
    range_bars: bool,
    range_bar_selection: bool,
    violin: bool,
    violin_selection: bool,
    box_points: bool,
    box_point_selection: bool,
    value_labels: bool,
    value_label_selection: bool,
    field_selection: bool,
    scatter_mapped: bool,
    errorbar_mapped: bool,
    pick_ring: bool,
    pick_ring_mapped: bool,
    ternary: [bool; TERNARY_PIPELINES],
    view3d: [bool; VIEW3D_PIPELINES],
}

impl PreciseNeeds {
    fn scan(items: &[PanelDraw<'_>]) -> Self {
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
        Self {
            parallel: need_parallel,
            parallel_selection: need_parallel_selection,
            vector_points: need_vector_points,
            vector_grid: need_vector_grid,
            vector_path: need_vector_path,
            field: need_field,
            contour: need_contour,
            mesh: need_mesh,
            mesh_contour: need_mesh_contour,
            mesh_selection: need_mesh_selection,
            contour_label: need_contour_label,
            line: need_line,
            line_mapped: need_line_mapped,
            area: need_area,
            scatter: need_scatter,
            errorbar: need_errorbar,
            stem: need_stem,
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
            field_selection: need_field_selection,
            scatter_mapped: need_scatter_mapped,
            errorbar_mapped: need_errorbar_mapped,
            pick_ring: need_pick_ring,
            pick_ring_mapped: need_pick_ring_mapped,
            ternary: need_ternary,
            view3d: need_view3d,
        }
    }
}

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
        items: &[PanelDraw<'_>],
    ) {
        let needs = PreciseNeeds::scan(items);
        self.create_line_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            point_value_map_bgl,
            parallel_bgl,
            surface_format,
        );
        self.create_field_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            data_selection_bgl,
            field_bgl,
            label_pipelines,
            surface_format,
        );
        self.create_bar_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            category_bar_data_bgl,
            surface_format,
        );
        self.create_vector_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            data_selection_bgl,
            field_bgl,
            vector_bgl,
            star_data_bgl,
            label_pipelines,
            surface_format,
        );
        self.create_marker_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            surface_format,
        );
        self.create_entry_pipelines(
            &needs,
            device,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            surface_bgl,
            view3d_value_map_bgl,
            vector_bgl,
            surface_format,
        );
    }
}
