//! Point-picker shader assembly: the WGSL fragments, the exact-pass
//! variants and the compiled pipeline bundle.

use super::*;
use crate::shader_assembly::{Assembled, assembled};

pub(super) const INIT_SCOPE: &str = "renderer.gpu_pick";

/// The exact pass's `PICK_VARIANT` bits (`gpu_pick/core.wgsl`): which
/// optional paths one `pick_exact_candidates` pipeline carries.
pub(crate) mod exact_variant {
    pub(crate) const STYLED_MARKER: u32 = 2;
    /// Error-bar stems and caps with the markers (`gpu_pick/errorbar.wgsl`).
    pub(crate) const ERRORBAR: u32 = 4;
    /// A line's area fill (`gpu_pick/area.wgsl`).
    pub(crate) const AREA: u32 = 8;
    pub(crate) const DASH: u32 = 16;
    pub(crate) const STYLED_LINE: u32 = 32;
    pub(crate) const POLAR: u32 = 64;
    /// A streamline series' arrowheads with its line (`gpu_pick/line.wgsl`).
    pub(crate) const PATH_ARROWS: u32 = 128;
    /// A box plot's raw points, placed in their category slot and jittered
    /// (`gpu_pick/marker.wgsl`, `category_jitter.wgsl`).
    pub(crate) const CATEGORY_JITTER: u32 = 256;
    /// A ternary series: the third component read from the pool and the
    /// samples placed by `ternary_ndc` (`gpu_pick/core.wgsl`).
    pub(crate) const TERNARY: u32 = 512;
    /// Every path: the shader's default.
    #[cfg(test)]
    pub(crate) const ALL: u32 = STYLED_MARKER
        | ERRORBAR
        | AREA
        | DASH
        | STYLED_LINE
        | POLAR
        | PATH_ARROWS
        | CATEGORY_JITTER
        | TERNARY;
    /// The override's name in `gpu_pick/core.wgsl`.
    pub(crate) const CONSTANT: &str = "PICK_VARIANT";
}

/// The assembled point-picker shaders (rule 9's pattern, as in
/// `data_render::vector`): the gate module and one exact module per
/// primitive class, each the concatenation of its fragments, in the order
/// [`PICK_MODULES`] lists them. A pick compiles only the exact module (and
/// variant) its series need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickShader {
    /// The gate and reduction entries.
    Gate = 0,
    /// Exact: a marker series.
    Marker = 1,
    /// Exact: a series with markers and a line.
    MarkerLine = 2,
    /// Exact: a line series.
    Line = 3,
    /// Exact: a point vector series.
    Vector = 4,
}

/// The pick modules, indexed by [`PickShader`]: the shader module label and
/// the assembly. The exact modules follow the gate in [`PickShader::EXACT`]
/// order, so an exact module's slot in [`PickExactPipelines`] is its index
/// here minus one.
const PICK_MODULES: [(&str, Assembled); 5] = [
    (
        "figgy GPU pick gate shader",
        assembled!(
            "gpu_pick/core.wgsl",
            "data_render/common/projection.wgsl",
            "data_render/common/sample_validity.wgsl",
            "gpu_pick/gate_entries.wgsl",
        ),
    ),
    (
        "figgy GPU pick exact marker shader",
        assembled!(
            "gpu_pick/core.wgsl",
            "data_render/common/projection.wgsl",
            "data_render/common/sample_validity.wgsl",
            "data_render/common/ramp.wgsl",
            "data_render/common/grid_pair.wgsl",
            "data_render/common/sketch_noise.wgsl",
            "data_render/common/category_jitter.wgsl",
            "data_render/common/errorbar_segment.wgsl",
            "data_render/common/marker.wgsl",
            "gpu_pick/marker.wgsl",
            "gpu_pick/errorbar.wgsl",
            "gpu_pick/no_line.wgsl",
            "gpu_pick/exact_entry.wgsl",
        ),
    ),
    (
        "figgy GPU pick exact marker-line shader",
        assembled!(
            "gpu_pick/core.wgsl",
            "data_render/common/projection.wgsl",
            "data_render/common/sample_validity.wgsl",
            "data_render/common/ramp.wgsl",
            "data_render/common/grid_pair.wgsl",
            "data_render/common/arc_pair.wgsl",
            "data_render/common/sketch_noise.wgsl",
            "data_render/common/category_jitter.wgsl",
            "data_render/common/errorbar_segment.wgsl",
            "data_render/common/line_stroke.wgsl",
            "data_render/common/line_step.wgsl",
            "data_render/common/area_piece.wgsl",
            "data_render/common/arrow.wgsl",
            "data_render/common/path_arrow.wgsl",
            "data_render/common/marker.wgsl",
            "gpu_pick/marker.wgsl",
            "gpu_pick/errorbar.wgsl",
            "gpu_pick/line.wgsl",
            "gpu_pick/area.wgsl",
            "gpu_pick/exact_entry.wgsl",
        ),
    ),
    (
        "figgy GPU pick exact line shader",
        assembled!(
            "gpu_pick/core.wgsl",
            "data_render/common/projection.wgsl",
            "data_render/common/sample_validity.wgsl",
            "data_render/common/ramp.wgsl",
            "data_render/common/arc_pair.wgsl",
            "data_render/common/sketch_noise.wgsl",
            "data_render/common/line_stroke.wgsl",
            "data_render/common/line_step.wgsl",
            "data_render/common/area_piece.wgsl",
            "data_render/common/arrow.wgsl",
            "data_render/common/path_arrow.wgsl",
            "gpu_pick/no_scatter.wgsl",
            "gpu_pick/line.wgsl",
            "gpu_pick/area.wgsl",
            "gpu_pick/exact_entry.wgsl",
        ),
    ),
    (
        "figgy GPU pick exact vector shader",
        assembled!(
            "gpu_pick/core.wgsl",
            "data_render/common/projection.wgsl",
            "data_render/common/sample_validity.wgsl",
            "data_render/common/ramp.wgsl",
            "data_render/common/arrow.wgsl",
            "gpu_pick/vector.wgsl",
            "gpu_pick/no_line.wgsl",
            "gpu_pick/exact_entry.wgsl",
        ),
    ),
];

impl PickShader {
    #[cfg(test)]
    pub(crate) const EXACT: [PickShader; 4] = [
        PickShader::Marker,
        PickShader::MarkerLine,
        PickShader::Line,
        PickShader::Vector,
    ];

    /// The assembly: source and fragments (`shader_assembly::MODULES`).
    pub(crate) const fn assembled(self) -> Assembled {
        PICK_MODULES[self as usize].1
    }

    /// The fragment files, in assembly order, relative to `src/`.
    #[cfg(test)]
    pub(crate) const fn fragments(self) -> &'static [&'static str] {
        self.assembled().fragments
    }

    /// The assembled source: [`Self::fragments`] joined with one newline.
    pub(crate) const fn source(self) -> &'static str {
        self.assembled().source
    }

    pub(super) fn label(self) -> &'static str {
        PICK_MODULES[self as usize].0
    }

    pub(super) fn exact_index(self) -> usize {
        (self as usize)
            .checked_sub(1)
            .expect("the gate module has no exact entry")
    }
}

/// One exact pipeline: the module, the `PICK_VARIANT` bits and the
/// `PICK_SHAPE` the driver specialises it for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PickExactKey {
    pub(crate) shader: PickShader,
    pub(crate) bits: u32,
    /// A marker shape id, or [`PickExactKey::ANY_SHAPE`].
    pub(crate) shape: u32,
}

impl PickExactKey {
    /// `PICK_SHAPE_ANY`: the shape is read per point.
    pub(crate) const ANY_SHAPE: u32 = u32::MAX;

    /// The marker-and-line module with every path and any shape.
    #[cfg(test)]
    pub(crate) const ALL_PATHS: Self = Self {
        shader: PickShader::MarkerLine,
        bits: exact_variant::ALL,
        shape: Self::ANY_SHAPE,
    };
}

/// The exact pipelines created so far. Each is created on first use (the
/// first pick of a series that needs it) and kept for the renderer's
/// lifetime; owned by the renderer's picker and passed to each pick, so no
/// lock guards it. `pipelines` grows by one entry per distinct key a chart
/// has needed (a handful: module x draw-style paths x marker shape).
#[derive(Default)]
pub(crate) struct PickExactPipelines {
    pub(super) modules: [Option<wgpu::ShaderModule>; 4],
    pub(super) pipelines: Vec<(PickExactKey, wgpu::ComputePipeline)>,
}

impl PickExactPipelines {
    /// The exact pipeline for `key`, created if it is not yet.
    pub(crate) fn ensure(
        &mut self,
        bundle: &PickPipelineBundle,
        key: PickExactKey,
    ) -> Result<&wgpu::ComputePipeline, GpuPickError> {
        let index = match self.pipelines.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                self.pipelines
                    .try_reserve(1)
                    .map_err(|_| GpuPickError::AllocationFailed {
                        resource: "exact pick pipelines",
                    })?;
                let device = &bundle.device;
                let shader = key.shader;
                let module = self.modules[shader.exact_index()].get_or_insert_with(|| {
                    crate::gpu_compile::shader_module(
                        device,
                        wgpu::ShaderModuleDescriptor {
                            label: Some(shader.label()),
                            source: wgpu::ShaderSource::Wgsl(shader.source().into()),
                        },
                    )
                });
                let base = crate::gpu_compile::workgroup_options(device);
                let mut constants = [(exact_variant::CONSTANT, f64::from(key.bits)); 3];
                let mut count = 1;
                // Only the marker modules declare the shape override.
                if key.shape != PickExactKey::ANY_SHAPE
                    && matches!(shader, PickShader::Marker | PickShader::MarkerLine)
                {
                    constants[count] = ("PICK_SHAPE", f64::from(key.shape));
                    count += 1;
                }
                if let Some(&zero) = base.constants.first() {
                    constants[count] = zero;
                    count += 1;
                }
                let pipeline = crate::gpu_compile::compute_pipeline(
                    device,
                    &wgpu::ComputePipelineDescriptor {
                        label: Some("figgy GPU pick exact candidate pipeline"),
                        layout: Some(&bundle.exact_layout),
                        module,
                        entry_point: Some("pick_exact_candidates"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants[..count],
                            zero_initialize_workgroup_memory: base.zero_initialize_workgroup_memory,
                        },
                        cache: None,
                    },
                );
                self.pipelines.push((key, pipeline));
                self.pipelines.len() - 1
            }
        };
        Ok(&self.pipelines[index].1)
    }

    pub(super) fn get(&self, key: PickExactKey) -> Option<&wgpu::ComputePipeline> {
        self.pipelines
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, pipeline)| pipeline)
    }
}

impl PickPipelineBundle {
    pub(crate) fn begin_stream(
        self: &Arc<Self>,
        query: GpuPickQuery,
        display_scale: f32,
        identities: Vec<(Option<String>, String)>,
    ) -> Result<GpuStreamPick, GpuPickError> {
        let reduce = GpuPickEngine::create_reduce_resources(
            &self.device,
            &self.ledger,
            &self.reduce_bgl,
            2,
        )?;
        let [cx, cy] = query.canvas_position_px;
        let [x, y, w, h] = query.chart_rect_px;
        let enabled = display_scale.is_finite()
            && display_scale > 0.0
            && [cx, cy, x, y, w, h, query.max_distance_px]
                .into_iter()
                .all(f32::is_finite)
            && w > 0.0
            && h > 0.0
            && query.max_distance_px >= 0.0
            && query.data_area_px.is_none_or(|[x, y, w, h]| {
                [x, y, w, h].into_iter().all(f32::is_finite)
                    && cx >= x
                    && cx <= x + w
                    && cy >= y
                    && cy <= y + h
            });
        Ok(GpuStreamPick {
            engine: GpuPickEngine::from_bundle_and_reduce(Arc::clone(self), reduce),
            query,
            display_scale,
            identities: identities
                .into_iter()
                .map(|(source_id, series_id)| PickIdentity {
                    source_id,
                    series_id,
                })
                .collect(),
            has_chunks: false,
            enabled,
        })
    }
}

/// Immutable device resources shared by every picker registry on one renderer.
pub(crate) struct PickPipelineBundle {
    pub(super) device: Arc<wgpu::Device>,
    pub(super) queue: Arc<wgpu::Queue>,
    /// Where pick scratch is charged. Carried by the bundle so every
    /// `&self` query path can charge without a per-call argument.
    pub(super) ledger: Arc<GpuLedger>,
    pub(super) query_data_bgl: wgpu::BindGroupLayout,
    pub(super) query_work_bgl: wgpu::BindGroupLayout,
    pub(super) reduce_bgl: wgpu::BindGroupLayout,
    pub(super) gate_x: wgpu::ComputePipeline,
    pub(super) gate_y: wgpu::ComputePipeline,
    /// The layout every exact pipeline uses ([`PickExactPipelines`]).
    pub(super) exact_layout: wgpu::PipelineLayout,
    pub(super) reduce: wgpu::ComputePipeline,
    /// The exact pass's group 2: one series' arc prefix (`pick_arc`).
    pub(super) arc_bgl: wgpu::BindGroupLayout,
    /// Group 2 for a series picked without an arc prefix or ring starts:
    /// one zero pair in both bindings.
    pub(super) arc_placeholder: wgpu::BindGroup,
    /// That zero pair, bound in place of whichever buffer a series lacks.
    pub(super) arc_placeholder_buffer: wgpu::Buffer,
    pub(super) _arc_placeholder_charge: SharedCharge,
}

// Renderer GPU ownership is Arc-based on every target. WebGPU handles are
// intentionally !Send/!Sync on wasm because JavaScript confines them locally.
#[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
pub(super) fn create_pipeline_bundle_observed(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    observer: &mut dyn FnMut(InitEvent),
) -> Arc<PickPipelineBundle> {
    started(observer, INIT_SCOPE, "setup");
    // The gate module only: the exact modules compile on first use
    // (`PickExactPipelines`), the variant a series needs.
    let shader = crate::gpu_compile::shader_module(
        &device,
        wgpu::ShaderModuleDescriptor {
            label: Some(PickShader::Gate.label()),
            source: wgpu::ShaderSource::Wgsl(PickShader::Gate.source().into()),
        },
    );

    let query_data_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query data bgl"),
            entries: &QUERY_DATA_ENTRIES,
        },
    );
    let query_work_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query work bgl"),
            entries: &QUERY_WORK_ENTRIES,
        },
    );
    let reduce_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick reduction bgl"),
            entries: &[storage_entry(3, true), storage_entry(4, false)],
        },
    );

    let query_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick query layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl)],
            immediate_size: 0,
        },
    );
    let arc_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick arc prefix bgl"),
            entries: &ARC_ENTRIES,
        },
    );
    let exact_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick exact layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl), Some(&arc_bgl)],
            immediate_size: 0,
        },
    );
    let (arc_placeholder, arc_placeholder_buffer, arc_placeholder_charge) =
        create_arc_placeholder(&device, &ledger, &arc_bgl);
    let reduce_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick reduction layout"),
            bind_group_layouts: &[Some(&reduce_bgl)],
            immediate_size: 0,
        },
    );
    finished(observer, INIT_SCOPE, "setup");
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, label: &'static str| {
        crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: if entry == "pick_reduce_candidates" {
                    crate::gpu_compile::workgroup_options(&device)
                } else {
                    wgpu::PipelineCompilationOptions::default()
                },
                cache: None,
            },
        )
    };

    Arc::new(PickPipelineBundle {
        ledger,
        gate_x: observe_value(observer, INIT_SCOPE, "pick_gate_x", || {
            pipeline(
                &query_layout,
                "pick_gate_x",
                "figgy GPU pick X gate pipeline",
            )
        }),
        gate_y: observe_value(observer, INIT_SCOPE, "pick_gate_y", || {
            pipeline(
                &query_layout,
                "pick_gate_y",
                "figgy GPU pick Y gate pipeline",
            )
        }),
        exact_layout,
        reduce: observe_value(observer, INIT_SCOPE, "pick_reduce_candidates", || {
            pipeline(
                &reduce_layout,
                "pick_reduce_candidates",
                "figgy GPU pick candidate reduction pipeline",
            )
        }),
        query_data_bgl,
        query_work_bgl,
        reduce_bgl,
        arc_bgl,
        arc_placeholder,
        arc_placeholder_buffer,
        _arc_placeholder_charge: arc_placeholder_charge,
        device,
        queue,
    })
}

/// The exact pass's group 2 for a series without an arc prefix or ring
/// starts: a one-pair storage buffer in both bindings, which the shader
/// never reads (`line.y` is zero and `PICK_FLAG_REGION` is clear).
pub(super) fn create_arc_placeholder(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
    arc_bgl: &wgpu::BindGroupLayout,
) -> (wgpu::BindGroup, wgpu::Buffer, SharedCharge) {
    let tally = ChargeTally::new();
    // gpu-alloc: PickScratch
    let buffer = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy GPU pick arc placeholder"),
            contents: bytemuck::bytes_of(&[0u32; 2]),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy GPU pick arc placeholder bg"),
        layout: arc_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffer.as_entire_binding(),
            },
        ],
    });
    (
        bind_group,
        buffer,
        shared_charge(tally, ledger, GpuResourceKind::PickScratch),
    )
}

impl PickPipelineBundle {
    /// The exact pass's group 2 holding one series' arc prefix and ring
    /// starts, the placeholder pair standing in for either it lacks.
    pub(super) fn arc_bind_group(
        &self,
        arc: Option<&wgpu::Buffer>,
        ring: Option<&wgpu::Buffer>,
    ) -> wgpu::BindGroup {
        let placeholder = &self.arc_placeholder_buffer;
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy GPU pick arc prefix bg"),
            layout: &self.arc_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: arc.unwrap_or(placeholder).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ring.unwrap_or(placeholder).as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
pub(super) async fn create_pipeline_bundle_observed_async(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<Arc<PickPipelineBundle>, GpuPickError> {
    use crate::init::{observe_value_async, yield_init_frame};
    #[cfg(target_arch = "wasm32")]
    crate::init::prewarm_compute_entries_js(
        &device,
        "gpu.pick.async",
        PickShader::Gate.source(),
        &[
            ("pick_gate_x", "pick_gate_x"),
            ("pick_gate_y", "pick_gate_y"),
            ("pick_reduce_candidates", "pick_reduce_candidates"),
        ],
        observer,
    )
    .await
    .map_err(GpuPickError::AsyncCompileFailed)?;
    started(observer, INIT_SCOPE, "setup");
    // The gate module only: the exact modules compile on first use
    // (`PickExactPipelines`), the variant a series needs.
    let shader = crate::gpu_compile::shader_module(
        &device,
        wgpu::ShaderModuleDescriptor {
            label: Some(PickShader::Gate.label()),
            source: wgpu::ShaderSource::Wgsl(PickShader::Gate.source().into()),
        },
    );

    let query_data_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query data bgl"),
            entries: &QUERY_DATA_ENTRIES,
        },
    );
    let query_work_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query work bgl"),
            entries: &QUERY_WORK_ENTRIES,
        },
    );
    let reduce_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick reduction bgl"),
            entries: &[storage_entry(3, true), storage_entry(4, false)],
        },
    );

    let query_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick query layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl)],
            immediate_size: 0,
        },
    );
    let arc_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick arc prefix bgl"),
            entries: &ARC_ENTRIES,
        },
    );
    let exact_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick exact layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl), Some(&arc_bgl)],
            immediate_size: 0,
        },
    );
    let (arc_placeholder, arc_placeholder_buffer, arc_placeholder_charge) =
        create_arc_placeholder(&device, &ledger, &arc_bgl);
    let reduce_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick reduction layout"),
            bind_group_layouts: &[Some(&reduce_bgl)],
            immediate_size: 0,
        },
    );
    finished(observer, INIT_SCOPE, "setup");
    yield_init_frame().await;
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, label: &'static str| {
        crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: if entry == "pick_reduce_candidates" {
                    crate::gpu_compile::workgroup_options(&device)
                } else {
                    wgpu::PipelineCompilationOptions::default()
                },
                cache: None,
            },
        )
    };

    let gate_x = observe_value_async(observer, INIT_SCOPE, "pick_gate_x", || {
        pipeline(
            &query_layout,
            "pick_gate_x",
            "figgy GPU pick X gate pipeline",
        )
    })
    .await;
    let gate_y = observe_value_async(observer, INIT_SCOPE, "pick_gate_y", || {
        pipeline(
            &query_layout,
            "pick_gate_y",
            "figgy GPU pick Y gate pipeline",
        )
    })
    .await;
    let reduce = observe_value_async(observer, INIT_SCOPE, "pick_reduce_candidates", || {
        pipeline(
            &reduce_layout,
            "pick_reduce_candidates",
            "figgy GPU pick candidate reduction pipeline",
        )
    })
    .await;

    Ok(Arc::new(PickPipelineBundle {
        gate_x,
        gate_y,
        exact_layout,
        reduce,
        query_data_bgl,
        query_work_bgl,
        reduce_bgl,
        arc_bgl,
        arc_placeholder,
        arc_placeholder_buffer,
        _arc_placeholder_charge: arc_placeholder_charge,
        device,
        queue,
        ledger,
    }))
}

/// Group 0 of the gate and exact passes: the pool, the gate masks and the
/// workgroup candidates.
pub(super) const QUERY_DATA_ENTRIES: [wgpu::BindGroupLayoutEntry; 3] = [
    storage_entry(0, true),
    storage_entry(1, false),
    storage_entry(2, false),
];
/// Group 1: the query parameters, the style slots and overrides and the
/// series result.
pub(super) const QUERY_WORK_ENTRIES: [wgpu::BindGroupLayoutEntry; 4] = [
    uniform_entry(0),
    storage_entry(1, true),
    storage_entry(2, true),
    storage_entry(3, false),
];
/// Group 2 of the exact pass: the arc prefix and a region series' ring
/// starts (#49).
pub(super) const ARC_ENTRIES: [wgpu::BindGroupLayoutEntry; 2] =
    [storage_entry(0, true), storage_entry(1, true)];
/// The exact pass's layout (`exact_layout`), in group order: the widest
/// layout a pick pipeline uses, so the one the device limits are checked
/// against.
pub(super) const EXACT_LAYOUT: [&[wgpu::BindGroupLayoutEntry]; 3] =
    [&QUERY_DATA_ENTRIES, &QUERY_WORK_ENTRIES, &ARC_ENTRIES];

/// The storage buffers the exact layout binds per shader stage.
pub(super) fn exact_layout_storage_buffers() -> u32 {
    EXACT_LAYOUT
        .iter()
        .flat_map(|group| group.iter())
        .filter(|entry| {
            matches!(
                entry.ty,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { .. },
                    ..
                }
            )
        })
        .count() as u32
}

pub(super) fn validate_device_limits(device: &wgpu::Device) -> Result<(), GpuPickError> {
    let limits = device.limits();
    if limits.max_compute_workgroup_size_x < GPU_PICK_WORKGROUP_SIZE {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_workgroup_size_x",
            requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
            limit: u64::from(limits.max_compute_workgroup_size_x),
        });
    }
    if limits.max_compute_invocations_per_workgroup < GPU_PICK_WORKGROUP_SIZE {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_invocations_per_workgroup",
            requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
            limit: u64::from(limits.max_compute_invocations_per_workgroup),
        });
    }
    // The exact layout is the widest a pick pipeline binds.
    let groups = EXACT_LAYOUT.len() as u32;
    if limits.max_bind_groups < groups {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_bind_groups",
            requested: u64::from(groups),
            limit: u64::from(limits.max_bind_groups),
        });
    }
    let storage_buffers = exact_layout_storage_buffers();
    if limits.max_storage_buffers_per_shader_stage < storage_buffers {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_storage_buffers_per_shader_stage",
            requested: u64::from(storage_buffers),
            limit: u64::from(limits.max_storage_buffers_per_shader_stage),
        });
    }
    Ok(())
}

#[allow(dead_code)] // Consumed by renderer ownership integration in P-02.
impl PickPipelineBundle {
    pub(crate) fn new_observed(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, GpuPickError> {
        validate_device_limits(&device)?;
        Ok(create_pipeline_bundle_observed(
            device, queue, ledger, observer,
        ))
    }

    pub(crate) async fn new_observed_async(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, GpuPickError> {
        validate_device_limits(&device)?;
        create_pipeline_bundle_observed_async(device, queue, ledger, observer).await
    }
}
