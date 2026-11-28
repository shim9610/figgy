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
            precise: layouts,
            queue,
            feature,
        } = *layouts;
        let includes = |field: &str| feature.is_none_or(|f| f.includes_field(field));
        let mut created = 0usize;
        macro_rules! prewarm_step {
            ($stage:expr, $create:expr $(,)?) => {{
                if budget == Some(created) {
                    return (false, created);
                }
                created += 1;
                crate::init::observe_value(observer, "renderer.prewarm", $stage, $create)
            }};
        }
        // The precise kinds in their order (#341), with the ternary entries
        // before the mapped error bar, the histogram envelope after the
        // mapped bars and the triangle mesh passes after the field. The
        // labelled contour kinds need the label engine: only the Fields
        // feature and the full prewarm include them, and both pass it.
        for kind in PrecisePipeline::ALL {
            if kind == PrecisePipeline::ErrorbarMapped && includes("ternary") {
                for kind in TernaryPipeline::ALL {
                    if self.ternary[kind as usize].is_none() {
                        prewarm_step!(kind.stage(), || self.ensure_ternary(kind, &layouts));
                    }
                }
            }
            if self.precise[kind as usize].is_none()
                && layouts.can_create(kind)
                && includes(kind.field())
            {
                prewarm_step!(kind.stage(), || self.ensure_precise(kind, &layouts));
            }
            if kind == PrecisePipeline::BarMapped
                && self.bar_envelope.is_none()
                && includes("bar_envelope")
            {
                prewarm_step!("histogram envelope", || self.ensure_bar_envelope(&layouts));
            }
            if kind == PrecisePipeline::Field && includes("field_tri_mesh") {
                // A triangle mesh's fill and contours (#210), prepared with
                // the fields: they read the field's group 2.
                for pass in data_render::tri_mesh::TriMeshPass::ALL {
                    if self.tri_mesh[pass as usize].is_none() {
                        prewarm_step!(pass.label(), || self.ensure_tri_mesh_pass(
                            layouts.device,
                            layouts.cache,
                            layouts.tri_mesh(pass),
                            layouts.surface_format,
                            pass,
                        ));
                    }
                }
            }
        }

        // The stacked category-bar fit (#466): a fit pipeline, so only the
        // full prewarm creates it, never a feature's preparation.
        if feature.is_none() && self.category_stack_fit.is_none() {
            prewarm_step!("category stack extent", || self
                .ensure_category_stack_fit(layouts.device, layouts.cache));
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
            let set = prewarm_step!(stage, || {
                create_style_set(
                    key,
                    layouts.device,
                    layouts.cache,
                    queue,
                    &self.shaders,
                    layouts.transform_bgl,
                    layouts.style_bgl,
                    layouts.star_data_bgl,
                    layouts.surface_format,
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
    pub(in crate::renderer) precise: PreciseLayouts<'a>,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) feature: Option<PreparationFeature>,
}
