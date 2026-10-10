//! Owned preparation jobs. No chart, column, view, or mutable renderer borrow
//! survives the begin call. Publication only merges caches for the original target.
use super::*;

/// Independently prepared groups for the Cartesian renderer's resident pipelines.
/// Nonresident matrix/tile streaming has separate runtime pipelines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationFeature {
    Basic,
    Bars,
    Fields,
    Sketch,
    Milkyway,
    Constellation,
    Picking,
}
impl PreparationFeature {
    pub const ALL: [Self; 7] = [
        Self::Basic,
        Self::Bars,
        Self::Fields,
        Self::Sketch,
        Self::Milkyway,
        Self::Constellation,
        Self::Picking,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Bars => "bars",
            Self::Fields => "fields",
            Self::Sketch => "sketch",
            Self::Milkyway => "milkyway",
            Self::Constellation => "constellation",
            Self::Picking => "picking",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == name)
    }
    pub(super) fn style_key(self) -> Option<StyleKey> {
        match self {
            Self::Sketch => Some(StyleKey::Sketch),
            Self::Milkyway => Some(StyleKey::Milkyway),
            Self::Constellation => Some(StyleKey::Constellation),
            _ => None,
        }
    }
    pub(super) fn includes_field(self, field: &str) -> bool {
        match self {
            Self::Basic => matches!(
                field,
                "line"
                    | "scatter"
                    | "scatter_mapped"
                    | "pick_ring"
                    | "pick_ring_mapped"
                    | "errorbar"
                    | "errorbar_mapped"
            ),
            Self::Bars => field.starts_with("bar"),
            Self::Fields => field.starts_with("field") || field.starts_with("contour"),
            _ => false,
        }
    }
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn includes_label(self, label: &str) -> bool {
        let group = if label.contains("milkyway") {
            Self::Milkyway
        } else if label.contains("constellation") {
            Self::Constellation
        } else if label.contains("styled") {
            Self::Sketch
        } else if label.contains("histogram") || label.contains(" bar columnar") {
            Self::Bars
        } else if label.contains("field") || label.contains("contour") {
            Self::Fields
        } else {
            Self::Basic
        };
        self == group
    }
    /// Requirements come from renderer declarations, not host shader-name lists.
    pub fn for_chart(config: &Config, series: &[SeriesConfig]) -> Vec<Self> {
        let mut result = vec![Self::Basic];
        if let Some(style) = style_variant(&config.draw_style) {
            result.push(match style.key {
                StyleKey::Sketch => Self::Sketch,
                StyleKey::Milkyway => Self::Milkyway,
                StyleKey::Constellation => Self::Constellation,
            });
        }
        for item in series {
            let group = match item.render_type {
                DataRenderType::Histogram { .. } => Self::Bars,
                DataRenderType::Heatmap { .. }
                | DataRenderType::Contour { .. }
                | DataRenderType::HeatmapContour { .. } => Self::Fields,
                _ => Self::Basic,
            };
            if !result.contains(&group) {
                result.push(group);
            }
        }
        result
    }
}

/// Consuming this job does not borrow its originating renderer. Hosts deduplicate
/// jobs per renderer/feature and may drop uninstalled results after cancellation.
pub struct PipelinePreparation {
    feature: PreparationFeature,
    identity: u64,
    generation: u64,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    format: wgpu::TextureFormat,
    pipelines: TargetPipelines,
    transform: wgpu::BindGroupLayout,
    style: wgpu::BindGroupLayout,
    mapped: wgpu::BindGroupLayout,
    selection: wgpu::BindGroupLayout,
    field: wgpu::BindGroupLayout,
    star: wgpu::BindGroupLayout,
}
/// Opaque actual GPU objects. Installation neither replays data nor changes Config.
pub struct PreparedPipelines {
    job: PipelinePreparation,
    contour: Option<crate::gpu_contour::ContourLabelPipelines>,
    arc: Option<data_render::line_arc::ArcScanPipelines>,
    extent: Option<crate::gpu_errorbar::GpuErrorbarExtentEngine>,
    picker: RendererPicker,
}
fn failure(reason: impl std::fmt::Display) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed {
        resource: "pipeline preparation",
        reason: reason.to_string(),
    }
}
impl PipelinePreparation {
    pub fn feature(&self) -> PreparationFeature {
        self.feature
    }
    pub async fn compile(
        mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<PreparedPipelines> {
        let mut contour = None;
        let mut arc = None;
        let mut extent = None;
        let mut picker = RendererPicker::disabled();
        if self.feature == PreparationFeature::Picking {
            picker
                .enable_observed_async(
                    self.device.clone(),
                    self.queue.clone(),
                    self.ledger.clone(),
                    &self.pipelines.shaders.bar,
                    &self.pipelines.shaders.field,
                    &self.transform,
                    &self.field,
                    &self.mapped,
                    observer,
                )
                .await
                .map_err(failure)?;
        } else {
            #[cfg(target_arch = "wasm32")]
            data_render::prewarm_browser_render_feature(
                &self.device,
                self.format,
                self.pipelines.sample_count,
                Some(self.feature),
                observer,
            )
            .await
            .map_err(failure)?;
            if self.feature == PreparationFeature::Fields {
                #[cfg(target_arch = "wasm32")]
                crate::init::prewarm_compute_entries_js(
                    &self.device,
                    "contour.label.async",
                    include_str!("../contour_anchor.wgsl"),
                    &[
                        ("anchor_project", "anchor_seed"),
                        ("anchor_project", "anchor_step"),
                        ("anchor_project", "anchor_project"),
                        ("anchor_select", "anchor_select"),
                    ],
                    observer,
                )
                .await
                .map_err(failure)?;
                contour = Some(crate::gpu_contour::ContourLabelPipelines::new(
                    &self.device,
                    &self.field,
                ));
            }
            self.pipelines
                .prewarm_all_observed(
                    &self.device,
                    &self.queue,
                    &self.transform,
                    &self.style,
                    &self.mapped,
                    &self.selection,
                    &self.field,
                    &self.star,
                    contour.as_ref(),
                    Some(self.feature),
                    self.format,
                    observer,
                )
                .await;
            if self.feature == PreparationFeature::Basic {
                arc = Some(
                    data_render::line_arc::create_arc_scan_pipelines_observed_async(
                        &self.device,
                        observer,
                    )
                    .await
                    .map_err(failure)?,
                );
                crate::gpu_errorbar::GpuErrorbarExtentEngine::warm_device_async(&self.device)
                    .await
                    .map_err(failure)?;
                extent = Some(crate::gpu_errorbar::GpuErrorbarExtentEngine::new_tracked(
                    &self.device,
                    self.ledger.clone(),
                ));
            }
        }
        Ok(PreparedPipelines {
            job: self,
            contour,
            arc,
            extent,
            picker,
        })
    }
}
impl Renderer {
    pub fn preparation_ready(&self, feature: PreparationFeature) -> bool {
        let p = &self.pipelines;
        match feature {
            PreparationFeature::Basic => {
                p.line.is_some()
                    && p.scatter.is_some()
                    && p.scatter_mapped.is_some()
                    && p.pick_ring.is_some()
                    && p.pick_ring_mapped.is_some()
                    && p.errorbar.is_some()
                    && p.errorbar_mapped.is_some()
                    && self.arc_pipelines.is_some()
                    && self.errorbar_extent_engine.is_some()
            }
            PreparationFeature::Bars => {
                p.bar.is_some()
                    && p.bar_envelope.is_some()
                    && p.bar_mapped.is_some()
                    && p.bar_selection.is_some()
            }
            PreparationFeature::Fields => {
                p.field.is_some()
                    && p.field_selection.is_some()
                    && p.contour.is_some()
                    && p.contour_labelled.is_some()
                    && p.contour_label.is_some()
                    && self.contour_label_pipelines.is_some()
            }
            PreparationFeature::Picking => matches!(
                self.picker.pipeline_state,
                PickerPipelineState::Ready { .. }
            ),
            style => p
                .styled
                .contains_key(&style.style_key().expect("style variant")),
        }
    }
    pub fn begin_preparation(&self, feature: PreparationFeature) -> Option<PipelinePreparation> {
        if self.preparation_ready(feature) {
            return None;
        }
        Some(PipelinePreparation {
            feature,
            identity: self.renderer_identity,
            generation: self.target_pipeline_generation,
            device: self.device.clone(),
            queue: self.queue.clone(),
            ledger: self.gpu_ledger.clone(),
            format: self.surface_format,
            pipelines: TargetPipelines::empty_for_preparation(&self.pipelines),
            transform: self.transform_bgl.clone(),
            style: self.style_bgl.clone(),
            mapped: self.per_point_style_map_bgl.clone(),
            selection: self.data_selection_bgl.clone(),
            field: self.field_bgl.clone(),
            star: self.star_data_bgl.clone(),
        })
    }
    pub fn install_preparation(&mut self, prepared: PreparedPipelines) -> Result<()> {
        let PreparedPipelines {
            job,
            contour,
            arc,
            extent,
            picker,
        } = prepared;
        if self.renderer_identity != job.identity
            || self.target_pipeline_generation != job.generation
            || !Arc::ptr_eq(&self.device, &job.device)
            || self.surface_format != job.format
            || self.target_sample_count != job.pipelines.sample_count
        {
            return Err(FiggyError::StaleStateToken {
                reason: "preparation belongs to a different renderer or target generation".into(),
            });
        }
        self.pipelines.merge_preparation(job.pipelines);
        if self.contour_label_pipelines.is_none() {
            self.contour_label_pipelines = contour;
        }
        if self.arc_pipelines.is_none() {
            self.arc_pipelines = arc;
        }
        if self.errorbar_extent_engine.is_none() {
            self.errorbar_extent_engine = extent;
        }
        if !matches!(
            self.picker.pipeline_state,
            PickerPipelineState::Ready { .. }
        ) && matches!(picker.pipeline_state, PickerPipelineState::Ready { .. })
        {
            self.picker.pipeline_state = picker.pipeline_state;
        }
        Ok(())
    }
}

impl TargetPipelines {
    fn empty_for_preparation(source: &Self) -> Self {
        Self {
            shaders: source.shaders.clone(),
            axis: source.axis.clone(),
            sample_count: source.sample_count,
            styled: HashMap::new(),
            line: None,
            scatter: None,
            scatter_mapped: None,
            pick_ring: None,
            pick_ring_mapped: None,
            errorbar: None,
            errorbar_mapped: None,
            bar: None,
            bar_envelope: None,
            bar_mapped: None,
            bar_selection: None,
            field: None,
            field_selection: None,
            contour: None,
            contour_labelled: None,
            contour_label: None,
        }
    }
    fn merge_preparation(&mut self, other: Self) {
        if self.line.is_none() {
            self.line = other.line;
        }
        if self.scatter.is_none() {
            self.scatter = other.scatter;
        }
        if self.scatter_mapped.is_none() {
            self.scatter_mapped = other.scatter_mapped;
        }
        if self.pick_ring.is_none() {
            self.pick_ring = other.pick_ring;
        }
        if self.pick_ring_mapped.is_none() {
            self.pick_ring_mapped = other.pick_ring_mapped;
        }
        if self.errorbar.is_none() {
            self.errorbar = other.errorbar;
        }
        if self.errorbar_mapped.is_none() {
            self.errorbar_mapped = other.errorbar_mapped;
        }
        if self.bar.is_none() {
            self.bar = other.bar;
        }
        if self.bar_envelope.is_none() {
            self.bar_envelope = other.bar_envelope;
        }
        if self.bar_mapped.is_none() {
            self.bar_mapped = other.bar_mapped;
        }
        if self.bar_selection.is_none() {
            self.bar_selection = other.bar_selection;
        }
        if self.field.is_none() {
            self.field = other.field;
        }
        if self.field_selection.is_none() {
            self.field_selection = other.field_selection;
        }
        if self.contour.is_none() {
            self.contour = other.contour;
        }
        if self.contour_labelled.is_none() {
            self.contour_labelled = other.contour_labelled;
        }
        if self.contour_label.is_none() {
            self.contour_label = other.contour_label;
        }
        for (key, value) in other.styled {
            self.styled.entry(key).or_insert(value);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn renderer() -> Renderer {
        let (device, queue) = crate::test_gpu::device();
        Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Rgba8Unorm,
            4096,
        )
        .unwrap()
    }
    #[test]
    fn detached_preparation_preserves_edits_and_caches_each_group() {
        let mut r = renderer();
        let id = r
            .register_chart(crate::default::default_config(), vec![])
            .unwrap();
        for feature in PreparationFeature::ALL {
            let job = r.begin_preparation(feature).unwrap();
            let mut config = r.chart_config(id).unwrap().clone();
            config.bottom_x.max += 1.0;
            r.set_chart_config(id, config.clone()).unwrap();
            // A job owns no renderer borrow; mutations remain available even
            // after its compile future has been created.
            let mut observer = |_| {};
            let future = job.compile(&mut observer);
            r.set_chart_config(id, config.clone()).unwrap();
            let prepared = pollster::block_on(future).unwrap();
            r.install_preparation(prepared).unwrap();
            assert_eq!(r.chart_config(id).unwrap(), &config);
            assert!(r.preparation_ready(feature));
            assert!(r.begin_preparation(feature).is_none());
        }
    }
    #[test]
    fn detached_preparation_rejects_replaced_target_and_other_renderer() {
        let mut r = renderer();
        let prepared = pollster::block_on(
            r.begin_preparation(PreparationFeature::Sketch)
                .unwrap()
                .compile(&mut |_| {}),
        )
        .unwrap();
        r.target_pipeline_generation += 1;
        assert!(matches!(
            r.install_preparation(prepared),
            Err(FiggyError::StaleStateToken { .. })
        ));
        assert!(!r.preparation_ready(PreparationFeature::Sketch));
        let job = r.begin_preparation(PreparationFeature::Sketch).unwrap();
        drop(r);
        let prepared = pollster::block_on(job.compile(&mut |_| {})).unwrap();
        let mut replacement = renderer();
        assert!(matches!(
            replacement.install_preparation(prepared),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }
}
