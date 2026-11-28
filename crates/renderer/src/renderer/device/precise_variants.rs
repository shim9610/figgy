//! Creating the precise (unstyled) pipeline variants a frame's items need.

use super::*;

/// The precise pipelines a frame's items need, read from their configs,
/// series and picks before any pipeline is created: a set of each kind
/// family, indexed by its kind (#341).
struct PreciseNeeds {
    precise: [bool; PRECISE_PIPELINES],
    ternary: [bool; TERNARY_PIPELINES],
    view3d: [bool; VIEW3D_PIPELINES],
    tri_mesh: [bool; data_render::tri_mesh::TriMeshPass::ALL.len()],
}

impl PreciseNeeds {
    fn has(&self, kind: PrecisePipeline) -> bool {
        self.precise[kind as usize]
    }

    /// Add `kind` when `needed`.
    fn add(&mut self, kind: PrecisePipeline, needed: bool) {
        self.precise[kind as usize] |= needed;
    }

    /// `products` are the frame's prepared panels, one per item, once the
    /// contour scratches have run. With them, a contour series that draws
    /// its labels needs only the labelled pass: the plain pass is its
    /// fallback when it has no label snapshot (`build_series_layers`, #465).
    /// Without them every contour needs the plain pass.
    fn scan(items: &[PanelDraw<'_>], products: Option<&[PreparedPanelProducts]>) -> Self {
        use PrecisePipeline as K;
        use data_render::tri_mesh::TriMeshPass;
        let mut needs = Self {
            precise: [false; PRECISE_PIPELINES],
            ternary: [false; TERNARY_PIPELINES],
            view3d: [false; VIEW3D_PIPELINES],
            tri_mesh: [false; TriMeshPass::ALL.len()],
        };
        // A selected grid cell or contour level draws through the field
        // selection, which compiles with its labelled twin.
        let mut field_selection = false;
        let mut contour_label = false;
        for (item_index, item) in items.iter().enumerate() {
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
                needs.add(K::PickRing, typed_point);
                needs.add(K::BarSelection, typed_bin);
                field_selection |= config.refs.iter().any(|picked| {
                    matches!(
                        picked,
                        PickedDataRef::MatrixCell { .. } | PickedDataRef::ContourLevel { .. }
                    )
                });
            }
            // The area fill is drawn under every style's line with the one
            // precise area pipeline.
            for series in item.series {
                needs.add(
                    K::Area,
                    series_has_line_fill(&item.chart_config.draw_style, &series.config.render_type),
                );
            }
            if style_variant(&item.chart_config.draw_style).is_some() {
                continue;
            }
            // Whether the panel has the camera its 3D entries draw with.
            let camera = item.chart_config.view_matrix().is_some();
            for (series_index, series) in item.series.iter().enumerate() {
                let primitives = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                );
                // On a 3D chart a heatmap draws as a surface and a contour on
                // the box floor, through the surface pipelines
                // (`ensure_surface_pipelines`), and a series with a height
                // column through the 3D entries below: none of them draws
                // through a planar pipeline, so none is compiled (#453).
                let planar = item.chart_config.view_3d().is_none();
                // As the layers decide (`build_series_layers_with_columns`):
                // a height column draws through the 3D entries when the
                // panel has a camera, and through the planar ones otherwise.
                let planar_series = !(camera && series.config.z_column.is_some());
                needs.add(
                    K::Field,
                    primitives.field && (planar || !draws_surface(&series.config.render_type)),
                );
                // A contour is the field quad through a different fragment entry
                // point, so it needs no line pipeline at all any more.
                // A series with a label snapshot draws through the labelled
                // pass; any other contour, the fallback included, through
                // the plain one (#465).
                let labelled = products.is_some_and(|products| {
                    products
                        .get(item_index)
                        .and_then(|panel| panel.contour.series.get(series_index))
                        .and_then(|series| series.scratch.as_ref())
                        .is_some_and(|scratch| scratch.label.is_some())
                });
                needs.add(K::Contour, primitives.contour && planar && !labelled);
                // A mesh on a 3D chart draws through the surface pipelines
                // (`ensure_surface_pipelines`), not the flat mesh ones.
                let mesh_flat = planar;
                needs.tri_mesh[TriMeshPass::Fill as usize] |= primitives.mesh && mesh_flat;
                needs.tri_mesh[TriMeshPass::Contour as usize] |=
                    primitives.mesh_contour && mesh_flat;
                // A picked triangle or level redraws through the mesh's
                // selection entry (#210).
                needs.tri_mesh[TriMeshPass::Selection as usize] |= (primitives.mesh
                    || primitives.mesh_contour)
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
                needs.add(K::Line, primitives.line && planar_series);
                // Streamline arrowheads draw through the path vector pipeline.
                needs.add(
                    K::VectorPath,
                    primitives.line
                        && series
                            .config
                            .render_type
                            .streamlines()
                            .is_some_and(|streamlines| streamlines.arrows.is_some()),
                );
                needs.add(
                    K::LineMapped,
                    primitives.line
                        && series
                            .config
                            .render_type
                            .streamlines()
                            .is_some_and(|streamlines| streamlines.color_map.is_some()),
                );
                contour_label |= primitives.contour
                    && planar
                    && series_wants_contour_labels(&series.config.render_type);
                // A parallel series' rows, and the picked row's redraw when
                // a point is picked (#141).
                needs.add(K::LineParallel, primitives.parallel);
                needs.add(K::LineParallelSelection, primitives.parallel && typed_point);
                needs.add(K::Scatter, primitives.scatter && planar_series);
                needs.add(K::Bar, primitives.bar);
                needs.add(K::CategoryBar, primitives.category_bars);
                needs.add(
                    K::CategoryBarSelection,
                    primitives.category_bars && typed_bin,
                );
                needs.add(K::BoxPlot, primitives.box_plot);
                needs.add(K::BoxPlotSelection, primitives.box_plot && typed_bin);
                needs.add(K::RangeBar, primitives.range_bars);
                needs.add(K::RangeBarSelection, primitives.range_bars && typed_bin);
                needs.add(K::RangeLink, primitives.range_links);
                needs.add(K::RangeLinkSelection, primitives.range_links && typed_bin);
                needs.add(K::Violin, primitives.violin);
                needs.add(K::ViolinSelection, primitives.violin && typed_bin);
                needs.add(K::BoxPoints, primitives.box_points);
                needs.add(K::BoxPointSelection, primitives.box_points && typed_point);
                // Value labels compile with any label the series can show;
                // `Emphasized` radar labels with or without a selection.
                let labels =
                    data_render::value_label_kind(&series.config.render_type, true).is_some();
                needs.add(K::ValueLabel, labels);
                needs.add(
                    K::ValueLabelSelection,
                    labels
                        && typed_picks.is_some_and(|config| {
                            config.refs.iter().any(|picked| {
                                matches!(picked, PickedDataRef::ValueLabel { series_id, .. }
                                    if *series_id == series.config.series_id)
                            })
                        }),
                );
                let vector_3d = item.chart_config.drawn_z_column(series.config).is_some()
                    && extract_vector(&series.config.render_type).is_some();
                if primitives.vector
                    && !vector_3d
                    && let Some((components, _)) = extract_vector(&series.config.render_type)
                {
                    match components.grid() {
                        Some(_) => needs.add(K::VectorGrid, true),
                        None => needs.add(K::VectorPoints, true),
                    }
                    // A selected grid cell draws through the field selection
                    // pipeline; a selected point anchor through the ring.
                }
                field_selection |= primitives.vector
                    && typed_picks.is_some_and(|config| {
                        config
                            .refs
                            .iter()
                            .any(|picked| matches!(picked, PickedDataRef::MatrixCell { .. }))
                    });
                needs.add(
                    K::BarMapped,
                    primitives.bar && series.style.bar_map.is_some(),
                );
                // The outline takes the mapped entry exactly when the draw does.
                needs.add(
                    K::BarSelectionMapped,
                    primitives.bar && series.style.bar_map.is_some() && typed_bin,
                );
                needs.add(K::Errorbar, has_errorbar(&series.config.render_type));
                needs.add(K::ErrorbarStem, has_stem(&series.config.render_type));
                // `has_index` only means a style-index column is bound.
                // Sparse `*_style_overrides` still need the mapped shader.
                if primitives.scatter && series.style.scatter_map.is_some() {
                    needs.add(K::ScatterMapped, true);
                    needs.add(K::PickRingMapped, typed_point);
                }
                // A ternary series draws through the ternary entries.
                if series.config.render_type.ternary_c_column().is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = typed_point;
                    let ternary = &mut needs.ternary;
                    ternary[TernaryPipeline::Line as usize] |= primitives.line;
                    ternary[TernaryPipeline::Scatter as usize] |= primitives.scatter && !mapped;
                    ternary[TernaryPipeline::ScatterMapped as usize] |=
                        primitives.scatter && mapped;
                    ternary[TernaryPipeline::Ring as usize] |=
                        primitives.scatter && !mapped && ring;
                    ternary[TernaryPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
                needs.add(
                    K::ErrorbarMapped,
                    has_errorbar(&series.config.render_type) && series.style.errorbar_map.is_some(),
                );
                // A scatter or line series on a 3D chart draws through the
                // 3D entries (#132); its picks compile when it is picked.
                if item.chart_config.view_3d().is_some() && series.config.z_column.is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = typed_point;
                    let view3d = &mut needs.view3d;
                    view3d[View3dPipeline::Line as usize] |= primitives.line;
                    view3d[View3dPipeline::Scatter as usize] |= primitives.scatter && !mapped;
                    view3d[View3dPipeline::ScatterMapped as usize] |= primitives.scatter && mapped;
                    view3d[View3dPipeline::Ring as usize] |= primitives.scatter && !mapped && ring;
                    view3d[View3dPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
                // A point vector on a 3D chart draws its arrows through the
                // camera (#211); its picked anchor through the 3D ring.
                if vector_3d {
                    needs.view3d[View3dPipeline::Vector as usize] |= primitives.vector;
                    needs.view3d[View3dPipeline::Ring as usize] |= primitives.vector && typed_point;
                }
            }
        }
        needs.add(K::FieldSelection, field_selection);
        needs.add(K::FieldSelectionLabelled, field_selection);
        needs.add(K::ContourLabelled, contour_label);
        needs.add(K::ContourLabel, contour_label);
        needs
    }
}

impl TargetPipelines {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn ensure_precise_variants_for_items(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
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
        products: Option<&[PreparedPanelProducts]>,
    ) {
        let needs = PreciseNeeds::scan(items, products);
        let layouts = PreciseLayouts {
            device,
            cache,
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
            surface_format,
        };
        for kind in PrecisePipeline::ALL {
            if needs.has(kind) {
                self.ensure_precise(kind, &layouts);
            }
        }
        if needs.has(PrecisePipeline::Bar) || needs.has(PrecisePipeline::BarMapped) {
            self.ensure_bar_envelope(&layouts);
        }
        for pass in data_render::tri_mesh::TriMeshPass::ALL {
            if needs.tri_mesh[pass as usize] {
                self.ensure_tri_mesh_pass(
                    device,
                    cache,
                    layouts.tri_mesh(pass),
                    surface_format,
                    pass,
                );
            }
        }
        for kind in TernaryPipeline::ALL {
            if needs.ternary[kind as usize] {
                self.ensure_ternary(kind, &layouts);
            }
        }
        for kind in View3dPipeline::ALL {
            if needs.view3d[kind as usize] {
                self.ensure_view3d_pass(
                    device,
                    cache,
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
    }
}

/// The render pipelines a style draws every series of a chart through, by
/// label (#299): a style's set is compiled whole (`create_style_set`).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn style_pipeline_labels(key: StyleKey) -> &'static [&'static str] {
    match key {
        StyleKey::Sketch => &[
            "figgy line styled pipeline",
            "figgy scatter styled pipeline",
            "figgy errorbar styled pipeline",
            "figgy stem pipeline",
        ],
        StyleKey::Milkyway => &[
            "figgy milkyway ribbon pipeline",
            "figgy milkyway stars pipeline",
            "figgy milkyway planets pipeline",
            "figgy milkyway jets pipeline",
        ],
        StyleKey::Constellation => &[
            "figgy point constellation line pipeline",
            "figgy point constellation stars pipeline",
        ],
    }
}

impl PreciseNeeds {
    /// The labels of the render pipelines these needs create, as the
    /// pipeline constructors name them, the histogram envelope, the
    /// triangle mesh, ternary and 3D entries included. A labelled contour
    /// waits for its labelled pass only, and a selection's labelled twin is
    /// not waited for.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn labels(&self, mut each: impl FnMut(&'static str)) {
        use PrecisePipeline as K;
        for kind in PrecisePipeline::ALL {
            let waited = match kind {
                K::Contour => !self.has(K::ContourLabelled),
                K::FieldSelectionLabelled => false,
                _ => true,
            };
            if self.has(kind) && waited {
                each(kind.label());
            }
        }
        if self.has(K::Bar) || self.has(K::BarMapped) {
            each("histogram envelope fill");
            // The envelope's compute pass, created with the fill.
            each(data_render::bar_envelope::ENVELOPE_SCAN_LABEL);
        }
        for pass in data_render::tri_mesh::TriMeshPass::ALL {
            if self.tri_mesh[pass as usize] {
                each(pass.label());
            }
        }
        for kind in TernaryPipeline::ALL {
            if self.ternary[kind as usize] {
                each(kind.label());
            }
        }
        for kind in View3dPipeline::ALL {
            if self.view3d[kind as usize] {
                each(kind.label());
            }
        }
    }
}

/// The labels of the render pipelines the frame needs to draw `series` on
/// a chart of `config` (#299): what [`PreciseNeeds::scan`] finds for that
/// series alone, or the whole set of a drawing style. A browser frame leaves
/// the series out until every one of them has compiled.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(in crate::renderer) fn series_pipeline_labels(
    view: &ChartView,
    config: &Config,
    series: &Series<'_>,
    each: impl FnMut(&'static str),
) {
    let item = PanelDraw {
        view,
        chart_config: config,
        series: std::slice::from_ref(series),
    };
    items_pipeline_labels(std::slice::from_ref(&item), each);
}

/// The labels of the render pipelines a draw of `items` needs (#299): what
/// [`PreciseNeeds::scan`] finds, the whole set of each drawing style, and
/// every pass of the 3D surfaces and ternary region fills, which their own
/// ensure paths create at once. A label can come more than once.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(in crate::renderer) fn items_pipeline_labels(
    items: &[PanelDraw<'_>],
    mut each: impl FnMut(&'static str),
) {
    PreciseNeeds::scan(items, None).labels(&mut each);
    for item in items {
        let config = item.chart_config;
        if let Some(variant) = style_variant(&config.draw_style) {
            for label in style_pipeline_labels(variant.key) {
                each(label);
            }
        }
        for series in item.series {
            let render_type = &series.config.render_type;
            if config.view_3d().is_some() {
                use data_render::surface_mesh::{SurfaceInput, SurfacePass};
                if draws_surface(render_type) || draws_floor(render_type) {
                    for pass in SurfacePass::ALL {
                        each(pass.label(SurfaceInput::Grid));
                    }
                }
                if draws_mesh_surface(render_type) || draws_mesh_floor(render_type) {
                    for pass in SurfacePass::ALL {
                        each(pass.label(SurfaceInput::Mesh));
                    }
                }
            }
            if config.ternary().is_some() && render_type.ternary_region().is_some() {
                for pass in data_render::ternary_region::TernaryRegionPass::ALL {
                    each(pass.label());
                }
                // The ring-start scan the region's draw dispatches first.
                each(data_render::ternary_region::RING_SCAN_LABEL);
            }
            // The compute passes the frame's preparation dispatches: the
            // contour label anchors and the arc prefix scan.
            if effective_series_primitives(&config.draw_style, render_type).contour
                && crate::renderer::prepare::series_wants_contour_labels(render_type)
            {
                for label in crate::gpu_contour::CONTOUR_ANCHOR_LABELS {
                    each(label);
                }
            }
            if crate::renderer::prepare::series_arc_need(&config.draw_style, series).0 {
                each(data_render::line_arc::ARC_SCAN_LABEL);
                // A 3D chart's line measures its chords through the camera
                // (`prepare_arc_items`), and only it waits for that entry
                // (#453).
                if render_type.ternary_c_column().is_none()
                    && series.config.z_column.is_some()
                    && config.view_matrix().is_some()
                {
                    each(data_render::line_arc::ARC_SCAN_3D_LABEL);
                }
            }
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod label_tests {
    /// Every label a frame waits for names render or compute pipelines the
    /// browser contract compiles in the background, so no series waits for
    /// a compile that is never issued.
    #[test]
    fn every_waited_label_is_a_contract_pipeline() {
        let contract: serde_json::Value =
            serde_json::from_str(include_str!("../../shader_pipeline_contract.json")).unwrap();
        let labels: Vec<String> = contract["pipelines"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["kind"] == "render" || p["kind"] == "compute")
            .filter_map(|p| p["descriptor"]["label"].as_str().map(str::to_owned))
            .collect();
        let mut needs = super::PreciseNeeds::scan(&[], None);
        // Every need at once, without labels and then with them.
        needs.precise = [true; super::PRECISE_PIPELINES];
        needs.precise[super::PrecisePipeline::ContourLabelled as usize] = false;
        let mut waited = Vec::new();
        needs.labels(|label| waited.push(label));
        needs.precise = [true; super::PRECISE_PIPELINES];
        needs.tri_mesh = [true; 3];
        needs.ternary = [true; super::TERNARY_PIPELINES];
        needs.view3d = [true; super::VIEW3D_PIPELINES];
        needs.labels(|label| waited.push(label));
        use crate::data_render::surface_mesh::{SurfaceInput, SurfacePass};
        for pass in SurfacePass::ALL {
            waited.push(pass.label(SurfaceInput::Grid));
            waited.push(pass.label(SurfaceInput::Mesh));
        }
        for pass in crate::data_render::ternary_region::TernaryRegionPass::ALL {
            waited.push(pass.label());
        }
        waited.push(crate::data_render::ternary_region::RING_SCAN_LABEL);
        waited.push(crate::data_render::line_arc::ARC_SCAN_LABEL);
        waited.push(crate::data_render::line_arc::ARC_SCAN_3D_LABEL);
        waited.extend(crate::gpu_contour::CONTOUR_ANCHOR_LABELS);
        for key in [
            super::StyleKey::Sketch,
            super::StyleKey::Milkyway,
            super::StyleKey::Constellation,
        ] {
            waited.extend(super::style_pipeline_labels(key));
        }
        assert!(waited.len() > 40);
        for label in waited {
            assert!(labels.iter().any(|l| l == label), "{label}");
        }
    }
}
