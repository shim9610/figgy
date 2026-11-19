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
                    | "line_mapped"
                    | "line_parallel"
                    | "line_parallel_selection"
                    | "area"
                    | "scatter"
                    | "scatter_mapped"
                    | "pick_ring"
                    | "pick_ring_mapped"
                    | "ternary"
                    | "parallel"
                    | "errorbar"
                    | "errorbar_mapped"
                    | "errorbar_stem"
            ),
            Self::Bars => field.starts_with("bar"),
            Self::Fields => {
                field.starts_with("field")
                    || field.starts_with("contour")
                    || field.starts_with("vector")
            }
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
        } else if label.contains("histogram")
            || label.contains(" bar columnar")
            || label.contains("box p")
            || label.contains("violin")
            || label.contains("value label")
        {
            // Every pipeline of the bar module (#299): a label the group
            // does not claim is compiled on the main thread by the
            // group's own prewarm instead of by the browser's compiler.
            Self::Bars
        } else if label.contains("field")
            || label.contains("contour")
            || label.contains("triangle mesh")
        {
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
                DataRenderType::Histogram(_)
                | DataRenderType::CategoryBars(_)
                | DataRenderType::BoxPlot(_)
                | DataRenderType::RangeBars(_)
                | DataRenderType::RangeLinks(_) => Self::Bars,
                // Vectors prepare with the fields: both read the pool as
                // storage and share the colourbar.
                DataRenderType::Field(_)
                | DataRenderType::RgbImage(_)
                | DataRenderType::Vector(_)
                // Streamline lines draw through the line pipelines (always
                // prepared) and their arrowheads through a vector pipeline.
                | DataRenderType::Streamlines(_)
                // A triangle mesh reads the field's group 2 (#210).
                | DataRenderType::TriMesh(_) => Self::Fields,
                DataRenderType::Xy(_)
                // A radar series draws through the line and scatter pipelines.
                | DataRenderType::Radar(_)
                // So does a ternary series, through its ternary entries, and
                // a parallel series through the line module's parallel
                // entries.
                | DataRenderType::Ternary(_)
                | DataRenderType::Parallel(_)
                // A monotonic envelope draws through the line pipeline
                // (#271).
                | DataRenderType::MonotonicLine(_) => Self::Basic,
                // Drawn on the CPU and composited (#203, #234): no GPU
                // pipeline.
                DataRenderType::EnergyLevels(_) | DataRenderType::Feynman(_) => continue,
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
    device: wgpu::Device,
    queue: wgpu::Queue,
    ledger: Arc<GpuLedger>,
    health: DeviceHealth,
    format: wgpu::TextureFormat,
    pipelines: TargetPipelines,
    transform: wgpu::BindGroupLayout,
    style: wgpu::BindGroupLayout,
    mapped: wgpu::BindGroupLayout,
    value_map: wgpu::BindGroupLayout,
    selection: wgpu::BindGroupLayout,
    field: wgpu::BindGroupLayout,
    star: wgpu::BindGroupLayout,
    vector: wgpu::BindGroupLayout,
    category_bar: wgpu::BindGroupLayout,
    parallel: wgpu::BindGroupLayout,
    /// The renderer's background compiles, which a browser preparation
    /// waits on instead of issuing its own (#299).
    #[cfg(target_arch = "wasm32")]
    compile_queue: Option<crate::browser_compile::CompileQueue>,
}
/// Opaque actual GPU objects. Installation neither replays data nor changes Config.
pub struct PreparedPipelines {
    job: PipelinePreparation,
    contour: Option<crate::gpu_contour::ContourLabelPipelines>,
    arc: Option<data_render::line_arc::ArcScanPipelines>,
    extent: Option<crate::gpu_extent::GpuErrorbarExtentEngine>,
    picker: RendererPicker,
}
fn failure(reason: impl std::fmt::Display) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed {
        resource: "pipeline preparation",
        reason: reason.to_string(),
    }
}
/// The picker's shaders, borrowed from a preparation's pipelines.
macro_rules! picker_shaders {
    ($job:expr) => {
        crate::gpu_data_pick::DataPickShaders {
            bar: &$job.pipelines.shaders.bar,
            field: &$job.pipelines.shaders.field,
            vector_grid: &$job.pipelines.shaders.vector_grid,
            line: &$job.pipelines.shaders.line,
            set: $job.pipelines.shaders.set,
        }
    };
}

/// A preparation's [`PrewarmLayouts`], borrowed field by field so that
/// `pipelines` stays free for the prewarm's `&mut`.
macro_rules! preparation_layouts {
    ($job:expr, $contour:expr) => {
        PrewarmLayouts {
            precise: PreciseLayouts {
                device: &$job.device,
                transform_bgl: &$job.transform,
                style_bgl: &$job.style,
                per_point_style_map_bgl: &$job.mapped,
                point_value_map_bgl: &$job.value_map,
                data_selection_bgl: &$job.selection,
                field_bgl: &$job.field,
                vector_bgl: &$job.vector,
                category_bar_data_bgl: &$job.category_bar,
                parallel_bgl: &$job.parallel,
                star_data_bgl: &$job.star,
                label_pipelines: $contour,
                surface_format: $job.format,
            },
            queue: &$job.queue,
            feature: Some($job.feature),
        }
    };
}

impl PipelinePreparation {
    pub fn feature(&self) -> PreparationFeature {
        self.feature
    }
    /// Compile the feature's pipelines. In the browser this awaits
    /// WebGPU's asynchronous compiler and yields between pipelines; on
    /// native nothing waits, and `Self::compile_blocking` does the same
    /// work without an executor.
    pub async fn compile(
        mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<PreparedPipelines> {
        let mut parts = CompiledParts::new();
        if self.feature == PreparationFeature::Picking {
            parts
                .picker
                .enable_observed_async(
                    self.device.clone(),
                    self.queue.clone(),
                    self.ledger.clone(),
                    self.health.clone(),
                    picker_shaders!(self),
                    &self.transform,
                    &self.field,
                    &self.mapped,
                    &self.vector,
                    &self.category_bar,
                    &self.parallel,
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
                self.pipelines.shaders.set,
                self.compile_queue.as_ref(),
                observer,
            )
            .await
            .map_err(failure)?;
            if self.feature == PreparationFeature::Fields {
                #[cfg(target_arch = "wasm32")]
                crate::init::prewarm_compute_entries_js(
                    &self.device,
                    "contour.label.async",
                    crate::gpu_contour::CONTOUR_ANCHOR_SHADER_BY_SET
                        .of(self.pipelines.shaders.set)
                        .source,
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
                parts.contour = Some(self.contour_pipelines());
            }
            let layouts = preparation_layouts!(self, parts.contour.as_ref());
            self.pipelines
                .prewarm_all_observed(&layouts, observer)
                .await;
            if self.feature == PreparationFeature::Basic {
                parts.arc = Some(
                    data_render::line_arc::create_arc_scan_pipelines_by_set_async(
                        &self.device,
                        self.pipelines.shaders.set,
                        observer,
                    )
                    .await
                    .map_err(failure)?,
                );
                crate::gpu_extent::GpuErrorbarExtentEngine::warm_device_async(&self.device)
                    .await
                    .map_err(failure)?;
                parts.extent = Some(self.extent_engine());
            }
        }
        Ok(self.prepared(parts))
    }

    /// [`Self::compile`] on native, without an executor: a native device
    /// compiles each pipeline when it is created, so nothing waits.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn compile_blocking(
        mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<PreparedPipelines> {
        let mut parts = CompiledParts::new();
        if self.feature == PreparationFeature::Picking {
            parts
                .picker
                .enable_observed(
                    self.device.clone(),
                    self.queue.clone(),
                    self.ledger.clone(),
                    self.health.clone(),
                    picker_shaders!(self),
                    &self.transform,
                    &self.field,
                    &self.mapped,
                    &self.vector,
                    &self.category_bar,
                    &self.parallel,
                    observer,
                )
                .map_err(failure)?;
        } else {
            if self.feature == PreparationFeature::Fields {
                parts.contour = Some(self.contour_pipelines());
            }
            let layouts = preparation_layouts!(self, parts.contour.as_ref());
            self.pipelines
                .prewarm_all_observed_blocking(&layouts, observer);
            if self.feature == PreparationFeature::Basic {
                parts.arc = Some(data_render::line_arc::create_arc_scan_pipelines_by_set(
                    &self.device,
                    self.pipelines.shaders.set,
                    observer,
                ));
                parts.extent = Some(self.extent_engine());
            }
        }
        Ok(self.prepared(parts))
    }

    fn contour_pipelines(&self) -> crate::gpu_contour::ContourLabelPipelines {
        crate::gpu_contour::ContourLabelPipelines::new_by_set(
            &self.device,
            &self.field,
            self.pipelines.shaders.set,
        )
    }

    fn extent_engine(&self) -> crate::gpu_extent::GpuErrorbarExtentEngine {
        crate::gpu_extent::GpuErrorbarExtentEngine::new_tracked(&self.device, self.ledger.clone())
            .with_health(self.health.clone())
    }

    fn prepared(self, parts: CompiledParts) -> PreparedPipelines {
        PreparedPipelines {
            job: self,
            contour: parts.contour,
            arc: parts.arc,
            extent: parts.extent,
            picker: parts.picker,
        }
    }
}

/// What a compile builds besides the job's own pipelines.
struct CompiledParts {
    contour: Option<crate::gpu_contour::ContourLabelPipelines>,
    arc: Option<data_render::line_arc::ArcScanPipelines>,
    extent: Option<crate::gpu_extent::GpuErrorbarExtentEngine>,
    picker: RendererPicker,
}

impl CompiledParts {
    fn new() -> Self {
        Self {
            contour: None,
            arc: None,
            extent: None,
            picker: RendererPicker::disabled(),
        }
    }
}

impl Renderer {
    pub fn preparation_ready(&self, feature: PreparationFeature) -> bool {
        let p = &self.pipelines;
        match feature {
            PreparationFeature::Basic | PreparationFeature::Bars | PreparationFeature::Fields => {
                // Every precise kind the feature prewarms but the line fill
                // and the stem, which this check has never waited for.
                let precise = PrecisePipeline::ALL.into_iter().all(|kind| {
                    !feature.includes_field(kind.field())
                        || matches!(kind, PrecisePipeline::Area | PrecisePipeline::ErrorbarStem)
                        || p.precise(kind).is_some()
                });
                precise
                    && match feature {
                        PreparationFeature::Basic => {
                            p.ternary.iter().all(Option::is_some)
                                && self.arc_pipelines.is_some()
                                && self.errorbar_extent_engine.is_some()
                        }
                        PreparationFeature::Bars => p.bar_envelope.is_some(),
                        _ => self.contour_label_pipelines.is_some(),
                    }
            }
            PreparationFeature::Picking => matches!(
                self.picker.pipeline_state,
                PickerPipelineState::Ready { .. }
            ),
            PreparationFeature::Sketch => p.styled.contains_key(&StyleKey::Sketch),
            PreparationFeature::Milkyway => p.styled.contains_key(&StyleKey::Milkyway),
            PreparationFeature::Constellation => p.styled.contains_key(&StyleKey::Constellation),
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
            health: self.health.clone(),
            format: self.surface_format,
            pipelines: TargetPipelines::empty_for_preparation(&self.pipelines),
            transform: self.transform_bgl.clone(),
            style: self.style_bgl.clone(),
            mapped: self.per_point_style_map_bgl.clone(),
            value_map: self.point_value_map_bgl.clone(),
            selection: self.data_selection_bgl.clone(),
            field: self.field_bgl.clone(),
            star: self.star_data_bgl.clone(),
            vector: self.vector_bgl.clone(),
            category_bar: self.category_bar_data_bgl.clone(),
            parallel: self.parallel_bgl.clone(),
            #[cfg(target_arch = "wasm32")]
            compile_queue: self.compile_queue.clone(),
        })
    }
    pub fn install_preparation(&mut self, prepared: PreparedPipelines) -> Result<()> {
        let result = self.install_preparation_unreported(prepared);
        self.reported(result)
    }

    fn install_preparation_unreported(&mut self, prepared: PreparedPipelines) -> Result<()> {
        let PreparedPipelines {
            job,
            contour,
            arc,
            extent,
            picker,
        } = prepared;
        if self.renderer_identity != job.identity
            || self.target_pipeline_generation != job.generation
            || self.device != job.device
            || self.surface_format != job.format
            || self.target_sample_count != job.pipelines.sample_count
            // Begun before the renderer moved to the full shaders
            // (#299): its pipelines are the basic ones.
            || self.pipelines.shaders.set != job.pipelines.shaders.set
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
            precise: std::array::from_fn(|_| None),
            ternary: Default::default(),
            view3d: Default::default(),
            surface_shader: source.surface_shader.clone(),
            surface: Default::default(),
            tri_mesh_shader: source.tri_mesh_shader.clone(),
            tri_mesh: Default::default(),
            tri_surface_shader: source.tri_surface_shader.clone(),
            tri_surface: Default::default(),
            ternary_region_shader: source.ternary_region_shader.clone(),
            ternary_region: Default::default(),
            // Compute pipelines: the target's format does not enter them.
            ternary_region_scan: source.ternary_region_scan.clone(),
            bar_envelope: None,
        }
    }
    fn merge_preparation(&mut self, other: Self) {
        if self.ternary_region_scan.is_none() {
            self.ternary_region_scan = other.ternary_region_scan.clone();
        }
        for (slot, other) in self.precise.iter_mut().zip(other.precise) {
            if slot.is_none() {
                *slot = other;
            }
        }
        for (slot, other) in self.ternary.iter_mut().zip(other.ternary) {
            if slot.is_none() {
                *slot = other;
            }
        }
        for (slot, other) in self.view3d.iter_mut().zip(other.view3d) {
            if slot.is_none() {
                *slot = other;
            }
        }
        if self.surface_shader.is_none() {
            self.surface_shader = other.surface_shader;
        }
        for (slot, other) in self.surface.iter_mut().zip(other.surface) {
            if slot.is_none() {
                *slot = other;
            }
        }
        if self.tri_mesh_shader.is_none() {
            self.tri_mesh_shader = other.tri_mesh_shader;
        }
        for (slot, other) in self.tri_mesh.iter_mut().zip(other.tri_mesh) {
            if slot.is_none() {
                *slot = other;
            }
        }
        if self.tri_surface_shader.is_none() {
            self.tri_surface_shader = other.tri_surface_shader;
        }
        for (slot, other) in self.tri_surface.iter_mut().zip(other.tri_surface) {
            if slot.is_none() {
                *slot = other;
            }
        }
        if self.bar_envelope.is_none() {
            self.bar_envelope = other.bar_envelope;
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
            RendererDevice::new(device, queue).expect("figgy poll thread"),
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
            let prepared = crate::test_gpu::block_on(future).unwrap();
            r.install_preparation(prepared).unwrap();
            assert_eq!(r.chart_config(id).unwrap(), &config);
            assert!(r.preparation_ready(feature));
            assert!(r.begin_preparation(feature).is_none());
        }
    }
    #[test]
    fn detached_preparation_rejects_replaced_target_and_other_renderer() {
        let mut r = renderer();
        let prepared = crate::test_gpu::block_on(
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
        let prepared = crate::test_gpu::block_on(job.compile(&mut |_| {})).unwrap();
        let mut replacement = renderer();
        assert!(matches!(
            replacement.install_preparation(prepared),
            Err(FiggyError::StaleStateToken { .. })
        ));
    }
}
