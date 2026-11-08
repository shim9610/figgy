//! The renderer's prewarm entry points and their stages.

use super::*;

/// A renderer's [`PrewarmLayouts`], borrowed field by field so that
/// `pipelines` stays free for the prewarm's `&mut`.
macro_rules! renderer_prewarm_layouts {
    ($renderer:expr) => {
        PrewarmLayouts {
            device: &$renderer.device,
            queue: &$renderer.queue,
            transform_bgl: &$renderer.transform_bgl,
            style_bgl: &$renderer.style_bgl,
            per_point_style_map_bgl: &$renderer.per_point_style_map_bgl,
            point_value_map_bgl: &$renderer.point_value_map_bgl,
            data_selection_bgl: &$renderer.data_selection_bgl,
            field_bgl: &$renderer.field_bgl,
            vector_bgl: &$renderer.vector_bgl,
            category_bar_data_bgl: &$renderer.category_bar_data_bgl,
            parallel_bgl: &$renderer.parallel_bgl,
            star_data_bgl: &$renderer.star_data_bgl,
            label_pipelines: $renderer.contour_label_pipelines.as_ref(),
            feature: None,
            surface_format: $renderer.surface_format,
        }
    };
}
/// A renderer prewarm's mesh stages, in order.
#[derive(Clone, Copy)]
pub(super) enum PrewarmMeshStage {
    Surface,
    TriSurface,
    View3d,
    TernaryRegion,
}

impl PrewarmMeshStage {
    const ALL: [Self; 4] = [
        Self::Surface,
        Self::TriSurface,
        Self::View3d,
        Self::TernaryRegion,
    ];
}

pub(super) fn prewarm_extent_failure(error: crate::gpu_extent::GpuExtentError) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed {
        resource: "fit extent compute pipelines",
        reason: error.to_string(),
    }
}

pub(super) fn prewarm_picking_failure(error: crate::gpu_pick::GpuPickError) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed {
        resource: "point picking compute pipelines",
        reason: error.to_string(),
    }
}

impl Renderer {
    /// Compile and retain the resident Cartesian pipeline groups before first use.
    ///
    /// Covers data/style rendering, contour-label compute, arc scans, fit reduction
    /// and GPU picking. The separate radial renderer and nonresident matrix/tile
    /// streaming pipelines are outside this scope. Chart-specific data, views,
    /// styles, atlases and result buffers can still require allocation afterwards.
    ///
    /// The observer receives a started/finished pair around each independently
    /// scheduled stage. On wasm every finished stage yields to the browser so
    /// host UI and file parsing remain live while the renderer is preparing.
    /// Repeated calls reuse installed caches while their target generation remains
    /// valid. A target-format or sample-count change can require new pipelines.
    /// This does not guarantee zero driver compilation cost on the first draw.
    pub async fn prewarm_all_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        let result = self.prewarm_all_observed_unreported(observer).await;
        self.reported(result)
    }

    pub(super) async fn prewarm_all_observed_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        self.health.check()?;
        if self.contour_label_pipelines.is_none() {
            #[cfg(target_arch = "wasm32")]
            crate::init::prewarm_compute_entries_js(
                &self.device,
                "contour.label.async",
                crate::gpu_contour::CONTOUR_ANCHOR_SHADER.source,
                &[
                    ("anchor_project", "anchor_seed"),
                    ("anchor_project", "anchor_step"),
                    ("anchor_project", "anchor_project"),
                    ("anchor_select", "anchor_select"),
                ],
                observer,
            )
            .await
            .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label compute pipelines",
                reason,
            })?;
            self.prewarm_contour_labels(observer);
            crate::init::yield_init_frame().await;
        }
        let layouts = renderer_prewarm_layouts!(self);
        self.pipelines
            .prewarm_all_observed(&layouts, observer)
            .await;
        for stage in PrewarmMeshStage::ALL {
            self.prewarm_mesh_stage(stage, observer);
            crate::init::yield_init_frame().await;
        }
        if self.arc_pipelines.is_none() {
            self.arc_pipelines = Some(
                data_render::line_arc::create_arc_scan_pipelines_observed_async(
                    &self.device,
                    observer,
                )
                .await
                .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                    resource: "line arc compute pipelines",
                    reason,
                })?,
            );
        }
        if self.errorbar_extent_engine.is_none() {
            started(observer, "renderer.prewarm", "fit extent compute");
            self.ensure_errorbar_extent_engine()
                .await
                .map_err(prewarm_extent_failure)?;
            finished(observer, "renderer.prewarm", "fit extent compute");
            crate::init::yield_init_frame().await;
        }
        started(observer, "renderer.prewarm", "point picking compute");
        self.enable_gpu_picking_observed_async(observer)
            .await
            .map_err(prewarm_picking_failure)?;
        finished(observer, "renderer.prewarm", "point picking compute");
        crate::init::yield_init_frame().await;
        Ok(())
    }

    /// [`Self::prewarm_all_observed`], creating every pipeline on this thread
    /// without waiting: on native nothing in a prewarm waits but the
    /// browser's yields, which this one has none of.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prewarm_all_observed_blocking(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        let result = self.prewarm_all_observed_blocking_unreported(observer);
        self.reported(result)
    }

    /// [`Self::prewarm_all`], without waiting.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prewarm_all_blocking(&mut self) -> Result<()> {
        let mut noop = |_| {};
        self.prewarm_all_observed_blocking(&mut noop)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn prewarm_all_observed_blocking_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        self.health.check()?;
        if self.contour_label_pipelines.is_none() {
            self.prewarm_contour_labels(observer);
        }
        let layouts = renderer_prewarm_layouts!(self);
        self.pipelines
            .prewarm_all_observed_blocking(&layouts, observer);
        for stage in PrewarmMeshStage::ALL {
            self.prewarm_mesh_stage(stage, observer);
        }
        if self.arc_pipelines.is_none() {
            self.arc_pipelines = Some(data_render::line_arc::create_arc_scan_pipelines_observed(
                &self.device,
                observer,
            ));
        }
        if self.errorbar_extent_engine.is_none() {
            started(observer, "renderer.prewarm", "fit extent compute");
            self.ensure_errorbar_extent_engine_blocking()
                .map_err(prewarm_extent_failure)?;
            finished(observer, "renderer.prewarm", "fit extent compute");
        }
        started(observer, "renderer.prewarm", "point picking compute");
        self.enable_gpu_picking_observed(observer)
            .map_err(prewarm_picking_failure)?;
        finished(observer, "renderer.prewarm", "point picking compute");
        Ok(())
    }

    pub(super) fn prewarm_contour_labels(&mut self, observer: &mut dyn FnMut(InitEvent)) {
        let pipelines = crate::init::observe_value(
            observer,
            "renderer.prewarm",
            "contour label engine",
            || crate::gpu_contour::ContourLabelPipelines::new(&self.device, &self.field_bgl),
        );
        self.contour_label_pipelines = Some(pipelines);
    }

    /// One of a prewarm's mesh stages: the 3D surface and its floor
    /// contours, a triangle mesh's surface and floor contours, the 3D
    /// scatter and line entries, or the ternary region.
    pub(super) fn prewarm_mesh_stage(
        &mut self,
        stage: PrewarmMeshStage,
        observer: &mut dyn FnMut(InitEvent),
    ) {
        match stage {
            PrewarmMeshStage::Surface => {
                // The 3D surface and its floor contours (`Config.view_3d`).
                started(observer, "renderer.prewarm", "surface");
                for pass in data_render::surface_mesh::SurfacePass::ALL {
                    self.pipelines.ensure_surface_pass(
                        &self.device,
                        [
                            &self.transform_bgl,
                            &self.style_bgl,
                            &self.field_bgl,
                            &self.surface_bgl,
                        ],
                        self.surface_format,
                        pass,
                    );
                }
                finished(observer, "renderer.prewarm", "surface");
            }
            PrewarmMeshStage::TriSurface => {
                // A triangle mesh on a 3D chart: its surface and its floor
                // contours (#210).
                started(observer, "renderer.prewarm", "triangle surface");
                for pass in data_render::surface_mesh::SurfacePass::ALL {
                    self.pipelines.ensure_tri_surface_pass(
                        &self.device,
                        [
                            &self.transform_bgl,
                            &self.style_bgl,
                            &self.field_bgl,
                            &self.surface_bgl,
                        ],
                        self.surface_format,
                        pass,
                    );
                }
                finished(observer, "renderer.prewarm", "triangle surface");
            }
            PrewarmMeshStage::View3d => {
                // The 3D scatter and line entries (#132).
                for kind in View3dPipeline::ALL {
                    if self.pipelines.view3d[kind as usize].is_none() {
                        started(observer, "renderer.prewarm", kind.stage());
                        self.pipelines.ensure_view3d_pass(
                            &self.device,
                            [
                                &self.transform_bgl,
                                &self.style_bgl,
                                &self.per_point_style_map_bgl,
                                &self.surface_bgl,
                                &self.view3d_value_map_bgl,
                                &self.vector_bgl,
                            ],
                            self.surface_format,
                            kind,
                        );
                        finished(observer, "renderer.prewarm", kind.stage());
                    }
                }
            }
            PrewarmMeshStage::TernaryRegion => {
                // A ternary series' region fill (`TernaryRegionStyle`).
                started(observer, "renderer.prewarm", "ternary region");
                for pass in data_render::ternary_region::TernaryRegionPass::ALL {
                    self.pipelines.ensure_ternary_region_pass(
                        &self.device,
                        &self.transform_bgl,
                        &self.region_bgl,
                        self.surface_format,
                        pass,
                    );
                }
                // The ring-start scan the region draw and pick read (#49).
                self.pipelines.ensure_ternary_region_scan(&self.device);
                finished(observer, "renderer.prewarm", "ternary region");
            }
        }
    }

    /// Prepare the resident Cartesian groups described by [`Self::prewarm_all_observed`]
    /// without progress callbacks.
    pub async fn prewarm_all(&mut self) -> Result<()> {
        let result = self.prewarm_all_unreported().await;
        self.reported(result)
    }

    pub(super) async fn prewarm_all_unreported(&mut self) -> Result<()> {
        let mut noop = |_| {};
        self.prewarm_all_observed(&mut noop).await
    }
}
