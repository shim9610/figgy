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
                DataRenderType::Histogram { .. }
                | DataRenderType::CategoryBars { .. }
                | DataRenderType::BoxPlot { .. }
                | DataRenderType::RangeBars { .. } => Self::Bars,
                // Vectors prepare with the fields: both read the pool as
                // storage and share the colourbar.
                DataRenderType::Field(_)
                | DataRenderType::RgbImage { .. }
                | DataRenderType::Vector { .. }
                // Streamline lines draw through the line pipelines (always
                // prepared) and their arrowheads through a vector pipeline.
                | DataRenderType::Streamlines { .. } => Self::Fields,
                DataRenderType::Xy(_)
                // A radar series draws through the line and scatter pipelines.
                | DataRenderType::Radar { .. }
                // So does a ternary series, through its ternary entries, and
                // a parallel series through the line module's parallel
                // entries.
                | DataRenderType::Ternary { .. }
                | DataRenderType::Parallel { .. } => Self::Basic,
                // Drawn on the CPU and composited (#203, #234): no GPU
                // pipeline.
                DataRenderType::EnergyLevels(_) | DataRenderType::Feynman(_) => continue,
                // Refused at registration until its draw path lands (#210).
                DataRenderType::TriMesh(_) => continue,
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
        }
    };
}

/// A preparation's [`PrewarmLayouts`], borrowed field by field so that
/// `pipelines` stays free for the prewarm's `&mut`.
macro_rules! preparation_layouts {
    ($job:expr, $contour:expr) => {
        PrewarmLayouts {
            device: &$job.device,
            queue: &$job.queue,
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
            feature: Some($job.feature),
            surface_format: $job.format,
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
                observer,
            )
            .await
            .map_err(failure)?;
            if self.feature == PreparationFeature::Fields {
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
                .map_err(failure)?;
                parts.contour = Some(self.contour_pipelines());
            }
            let layouts = preparation_layouts!(self, parts.contour.as_ref());
            self.pipelines
                .prewarm_all_observed(&layouts, observer)
                .await;
            if self.feature == PreparationFeature::Basic {
                parts.arc = Some(
                    data_render::line_arc::create_arc_scan_pipelines_observed_async(
                        &self.device,
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
                parts.arc = Some(data_render::line_arc::create_arc_scan_pipelines_observed(
                    &self.device,
                    observer,
                ));
                parts.extent = Some(self.extent_engine());
            }
        }
        Ok(self.prepared(parts))
    }

    fn contour_pipelines(&self) -> crate::gpu_contour::ContourLabelPipelines {
        crate::gpu_contour::ContourLabelPipelines::new(&self.device, &self.field)
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
            PreparationFeature::Basic => {
                p.line.is_some()
                    && p.line_mapped.is_some()
                    && p.line_parallel.is_some()
                    && p.line_parallel_selection.is_some()
                    && p.scatter.is_some()
                    && p.scatter_mapped.is_some()
                    && p.pick_ring.is_some()
                    && p.pick_ring_mapped.is_some()
                    && p.ternary.iter().all(Option::is_some)
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
                    && p.bar_selection_mapped.is_some()
                    && p.bar_category.is_some()
                    && p.bar_category_selection.is_some()
                    && p.bar_box_plot.is_some()
                    && p.bar_box_plot_selection.is_some()
                    && p.bar_range.is_some()
                    && p.bar_range_selection.is_some()
                    && p.bar_violin.is_some()
                    && p.bar_violin_selection.is_some()
                    && p.bar_box_points.is_some()
                    && p.bar_box_point_selection.is_some()
                    && p.bar_value_label.is_some()
                    && p.bar_value_label_selection.is_some()
            }
            PreparationFeature::Fields => {
                p.field.is_some()
                    && p.field_selection.is_some()
                    && p.field_selection_labelled.is_some()
                    && p.contour.is_some()
                    && p.contour_labelled.is_some()
                    && p.contour_label.is_some()
                    && p.vector_points.is_some()
                    && p.vector_grid.is_some()
                    && p.vector_path.is_some()
                    && self.contour_label_pipelines.is_some()
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
            line_mapped: None,
            line_parallel: None,
            line_parallel_selection: None,
            area: None,
            scatter: None,
            scatter_mapped: None,
            pick_ring: None,
            pick_ring_mapped: None,
            ternary: Default::default(),
            view3d: Default::default(),
            surface_shader: source.surface_shader.clone(),
            surface: Default::default(),
            ternary_region_shader: source.ternary_region_shader.clone(),
            ternary_region: Default::default(),
            // Compute pipelines: the target's format does not enter them.
            ternary_region_scan: source.ternary_region_scan.clone(),
            errorbar: None,
            errorbar_mapped: None,
            errorbar_stem: None,
            bar: None,
            bar_envelope: None,
            bar_mapped: None,
            bar_selection: None,
            bar_selection_mapped: None,
            bar_category: None,
            bar_category_selection: None,
            bar_box_plot: None,
            bar_box_plot_selection: None,
            bar_range: None,
            bar_range_selection: None,
            bar_violin: None,
            bar_violin_selection: None,
            bar_box_points: None,
            bar_box_point_selection: None,
            bar_value_label: None,
            bar_value_label_selection: None,
            field: None,
            field_selection: None,
            field_selection_labelled: None,
            contour: None,
            contour_labelled: None,
            contour_label: None,
            vector_points: None,
            vector_grid: None,
            vector_path: None,
        }
    }
    fn merge_preparation(&mut self, other: Self) {
        if self.ternary_region_scan.is_none() {
            self.ternary_region_scan = other.ternary_region_scan.clone();
        }
        if self.line.is_none() {
            self.line = other.line;
        }
        if self.line_mapped.is_none() {
            self.line_mapped = other.line_mapped;
        }
        if self.line_parallel.is_none() {
            self.line_parallel = other.line_parallel;
        }
        if self.line_parallel_selection.is_none() {
            self.line_parallel_selection = other.line_parallel_selection;
        }
        if self.area.is_none() {
            self.area = other.area;
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
        if self.bar_selection_mapped.is_none() {
            self.bar_selection_mapped = other.bar_selection_mapped;
        }
        if self.bar_category.is_none() {
            self.bar_category = other.bar_category;
        }
        if self.bar_category_selection.is_none() {
            self.bar_category_selection = other.bar_category_selection;
        }
        if self.bar_box_plot.is_none() {
            self.bar_box_plot = other.bar_box_plot;
        }
        if self.bar_box_plot_selection.is_none() {
            self.bar_box_plot_selection = other.bar_box_plot_selection;
        }
        if self.bar_range.is_none() {
            self.bar_range = other.bar_range;
        }
        if self.bar_range_selection.is_none() {
            self.bar_range_selection = other.bar_range_selection;
        }
        if self.bar_violin.is_none() {
            self.bar_violin = other.bar_violin;
        }
        if self.bar_violin_selection.is_none() {
            self.bar_violin_selection = other.bar_violin_selection;
        }
        if self.bar_box_points.is_none() {
            self.bar_box_points = other.bar_box_points;
        }
        if self.bar_box_point_selection.is_none() {
            self.bar_box_point_selection = other.bar_box_point_selection;
        }
        if self.bar_value_label.is_none() {
            self.bar_value_label = other.bar_value_label;
        }
        if self.bar_value_label_selection.is_none() {
            self.bar_value_label_selection = other.bar_value_label_selection;
        }
        if self.vector_points.is_none() {
            self.vector_points = other.vector_points;
        }
        if self.vector_grid.is_none() {
            self.vector_grid = other.vector_grid;
        }
        if self.vector_path.is_none() {
            self.vector_path = other.vector_path;
        }
        if self.field.is_none() {
            self.field = other.field;
        }
        if self.field_selection.is_none() {
            self.field_selection = other.field_selection;
        }
        if self.field_selection_labelled.is_none() {
            self.field_selection_labelled = other.field_selection_labelled;
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
