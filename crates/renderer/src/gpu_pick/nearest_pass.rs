//! The nearest-datum pick (#381 prototype, the maintainer's decision): the
//! [`PICK_NEAREST_K`] data nearest to the cursor over every series of the
//! panel, with no pick reach.
//!
//! The kernel (`gpu_pick/nearest_entry.wgsl`, the xy column kind) is
//! spliced from the point picker's core and the shared projection
//! fragments only: no marker shape, stroke, dash or style, so it has no
//! variant. It runs in the exact pass's dispatch layout; each workgroup
//! keeps its K nearest (a per-workgroup top-K merged in shared memory) in a
//! per-pick scratch buffer of K candidates per workgroup, and one merge
//! dispatch folds every series' lists into the K nearest, sorted. Results
//! live in storage buffers: no render target, so neither MSAA nor integer
//! formats are involved, and no atomic serialises on one slot.
//!
//! Whether the cursor hits is not decided here: the maintainer's rule (#381)
//! is the drawn pixel under the cursor (ink means a hit, and the hit is the
//! first of the K); without ink there is no hit, and the K nearest are still
//! returned.

use super::engine::{create_bind_group_checked, create_buffer_checked};
use super::*;
use crate::shader_assembly::{AssembledBySet, ShaderSet, assembled_by_set};

/// How many nearest data a pick returns: the result size the maintainer
/// chose (#381), not a cap on any data count. Every datum is measured; fewer
/// come back only when fewer exist. `PICK_NEAREST_K` in
/// `gpu_pick/nearest_entry.wgsl`.
pub const PICK_NEAREST_K: usize = 10;

/// The xy column kind's nearest-datum module.
pub(crate) const NEAREST_PICK_MODULE: (&str, AssembledBySet) = (
    "figgy GPU pick nearest-datum shader",
    assembled_by_set!(
        "gpu_pick/core.wgsl",
        set,
        "data_render/common/projection.wgsl",
        "data_render/common/sample_validity.wgsl",
        "gpu_pick/nearest_entry.wgsl",
    ),
);

/// The nearest-datum pipelines of one shader set, created on first use:
/// the per-series scan and the merge.
pub struct NearestPickPipelines {
    set: ShaderSet,
    pipelines: Option<(wgpu::ComputePipeline, wgpu::ComputePipeline)>,
}

impl NearestPickPipelines {
    pub(crate) fn new(set: ShaderSet) -> Self {
        Self {
            set,
            pipelines: None,
        }
    }

    pub(crate) fn set(&self) -> ShaderSet {
        self.set
    }

    /// How many pipelines have been created: the scan and the merge of the
    /// one column kind.
    pub fn created(&self) -> usize {
        if self.pipelines.is_some() { 2 } else { 0 }
    }

    pub(crate) fn ensure(
        &mut self,
        bundle: &PickPipelineBundle,
    ) -> &(wgpu::ComputePipeline, wgpu::ComputePipeline) {
        let set = self.set;
        self.pipelines.get_or_insert_with(|| {
            let device = &bundle.device;
            let module = crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some(NEAREST_PICK_MODULE.0),
                    source: wgpu::ShaderSource::Wgsl(NEAREST_PICK_MODULE.1.of(set).source.into()),
                },
            );
            let base = crate::gpu_compile::workgroup_options(device);
            let options = || wgpu::PipelineCompilationOptions {
                constants: &[],
                zero_initialize_workgroup_memory: base.zero_initialize_workgroup_memory,
            };
            let scan = crate::gpu_compile::compute_pipeline(
                device,
                &wgpu::ComputePipelineDescriptor {
                    label: Some("figgy GPU pick nearest-datum pipeline"),
                    layout: Some(&bundle.exact_layout),
                    module: &module,
                    entry_point: Some("pick_nearest_points"),
                    compilation_options: options(),
                    cache: None,
                },
            );
            // The merge binds only the lists and the result (group 0,
            // bindings 2 and 3), so its layout is the module's own.
            let merge = crate::gpu_compile::compute_pipeline(
                device,
                &wgpu::ComputePipelineDescriptor {
                    label: Some("figgy GPU pick nearest-datum merge pipeline"),
                    layout: None,
                    module: &module,
                    entry_point: Some("pick_nearest_merge"),
                    compilation_options: options(),
                    cache: None,
                },
            );
            (scan, merge)
        })
    }
}

/// One of the nearest data.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestDatum {
    pub source_id: Option<String>,
    pub series_id: String,
    /// The logical index (a monotonic series' sample index).
    pub point_index: usize,
    pub distance_px: f32,
}

/// A resolved nearest-datum pick: up to [`PICK_NEAREST_K`] data, nearest
/// first (an exact tie to the series drawn later, then the later index).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NearestPick {
    pub nearest: Vec<NearestDatum>,
}

/// A pending nearest-datum pick.
pub struct NearestPickTicket {
    readback: Option<GpuReadback>,
    identities: Arc<[PickIdentity]>,
    pub(crate) monotonic: Vec<crate::renderer::MonotonicPickRows>,
    _charge: Option<GpuByteCharge>,
}

impl NearestPickTicket {
    fn none() -> Self {
        Self {
            readback: None,
            identities: Arc::from([]),
            monotonic: Vec::new(),
            _charge: None,
        }
    }

    /// Wait for the pick, then decode it.
    pub async fn resolve(mut self) -> Result<NearestPick, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait().await?;
        }
        self.finish()
    }

    /// [`Self::resolve`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(mut self) -> Result<NearestPick, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait_blocking()?;
        }
        self.finish()
    }

    fn finish(&mut self) -> Result<NearestPick, GpuPickError> {
        let Some(readback) = self.readback.as_mut() else {
            return Ok(NearestPick::default());
        };
        let size = CANDIDATE_BYTES as usize;
        let mut decoded = [<PickCandidateGpu as bytemuck::Zeroable>::zeroed(); PICK_NEAREST_K];
        readback.finish_checked(|mapped| {
            for (index, slot) in decoded.iter_mut().enumerate() {
                *slot = *bytemuck::from_bytes::<PickCandidateGpu>(&mapped[index * size..][..size]);
            }
        })?;
        let mut nearest = Vec::new();
        nearest
            .try_reserve_exact(PICK_NEAREST_K)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick result",
            })?;
        for candidate in decoded.iter().filter(|c| c.valid == 1) {
            let identity = self
                .identities
                .get(candidate.series_order as usize)
                .ok_or(GpuPickError::InvalidGpuResult)?;
            let point_index = match self
                .monotonic
                .iter()
                .find(|rows| rows.series_id == identity.series_id)
            {
                Some(rows) => match rows.logical(candidate.point_index as usize) {
                    Some(index) => index,
                    None => continue,
                },
                None => candidate.point_index as usize,
            };
            nearest.push(NearestDatum {
                source_id: identity.source_id.clone(),
                series_id: identity.series_id.clone(),
                point_index,
                distance_px: candidate.distance_px,
            });
        }
        Ok(NearestPick { nearest })
    }
}

impl GpuPickEngine {
    /// The [`PICK_NEAREST_K`] data nearest to `query`'s cursor over the
    /// registered series, with no reach: the checks and query parameters of
    /// [`Self::pick_with_arcs`], then the nearest-datum scan per series and
    /// one merge.
    pub(crate) fn pick_nearest(
        &self,
        pipelines: &mut NearestPickPipelines,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
        members: usize,
        member_transform: &dyn Fn(usize) -> ScatterTransform,
    ) -> Result<NearestPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        let [_, _, chart_width, chart_height] = query.chart_rect_px;
        if !display_scale.is_finite()
            || display_scale <= 0.0
            || !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !(chart_width > 0.0 && chart_height > 0.0)
        {
            return Ok(NearestPickTicket::none());
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(NearestPickTicket::none());
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }
        let device = &self.bundle.device;
        let (scan, merge) = pipelines.ensure(&self.bundle).clone();
        let members = members.min(self.registry.slots.len());
        let first_member = self.registry.slots.len() - members;
        // Each series' lists: K candidates per workgroup, at an offset the
        // device can bind.
        let align = u64::from(device.limits().min_storage_buffer_offset_alignment);
        let k = PICK_NEAREST_K as u64;
        let mut scratch_bytes = 0u64;
        for (series_order, slot) in self.registry.slots.iter().enumerate() {
            Self::validate_series_columns(pool, slot)?;
            let series = &slot.gpu;
            let mut params = query_params(
                series,
                &slot.identity.series_id,
                &query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &slot.identity.series_id)?,
                    lane_base(series.y_handle, &slot.identity.series_id)?,
                    series.style_index_base,
                ],
                [0, series.point_count, series.point_count, 0],
                false,
            );
            if let Some(member) = series_order.checked_sub(first_member) {
                params.transform = member_transform(member);
            }
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
            let groups = u64::from(series.gate_word_count.div_ceil(GPU_PICK_WORKGROUP_SIZE));
            scratch_bytes =
                scratch_bytes.div_ceil(align) * align + groups.max(1) * k * CANDIDATE_BYTES;
        }
        let tally = ChargeTally::new();
        // Zeroed at creation: the alignment gaps read as no candidate.
        let lists = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick nearest-datum lists"),
                size: scratch_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "nearest pick lists",
        )?;
        let result = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick nearest-datum result"),
                size: k * CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "nearest pick result",
        )?;
        let readback = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick nearest-datum readback"),
                size: k * CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "nearest pick readback",
        )?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy GPU pick nearest-datum encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum scan"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&scan);
            pass.set_bind_group(2, &self.bundle.arc_placeholder, &[]);
            let mut offset = 0u64;
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                offset = offset.div_ceil(align) * align;
                let groups = u64::from(series.gate_word_count.div_ceil(GPU_PICK_WORKGROUP_SIZE));
                let size = groups.max(1) * k * CANDIDATE_BYTES;
                let data_bg = create_bind_group_checked(
                    device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("figgy GPU pick nearest-datum data bg"),
                        layout: &self.bundle.query_data_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: pool.buffer().as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: series.gate_masks.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &lists,
                                    offset,
                                    size: std::num::NonZeroU64::new(size),
                                }),
                            },
                        ],
                    },
                    "nearest pick data bind group",
                )?;
                pass.set_bind_group(0, &data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
                offset += size;
            }
        }
        {
            let merge_bg = create_bind_group_checked(
                device,
                &wgpu::BindGroupDescriptor {
                    label: Some("figgy GPU pick nearest-datum merge bg"),
                    layout: &merge.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: result.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: lists.as_entire_binding(),
                        },
                    ],
                },
                "nearest pick merge bind group",
            )?;
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum merge"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&merge);
            pass.set_bind_group(0, &merge_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, k * CANDIDATE_BYTES);
        self.bundle.queue.submit(std::iter::once(encoder.finish()));
        Ok(NearestPickTicket {
            readback: Some(GpuReadback::map(
                device,
                &self.bundle.health,
                &readback,
                None,
            )),
            identities: Arc::clone(&self.registry.identities),
            monotonic: Vec::new(),
            _charge: Some(tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback)),
        })
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;
    use std::time::Instant;

    fn transform(max: f32) -> ScatterTransform {
        ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [max, max],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [2.0 / 800.0, 2.0 / 600.0],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
            polar_angle: [0.0; 4],
            polar_extent: [0.0; 4],
            ternary_lt: [0.0; 4],
            ternary_r: [0.0; 4],
            breaks: crate::data_render::NO_BREAKS,
            break_info: crate::data_render::NO_BREAK_INFO,
            scale_prob: crate::data_render::NO_PROBABILITY,
        }
    }

    /// Per-pick time of the exact picker and the nearest-datum kernel at
    /// `FIGGY_PICK_SCALE_POINTS` points (default 1M,10M,50M,80M) on a device
    /// with the adapter's own limits, in two layouts: points spread over the
    /// plot (few within the reach), and every point within the reach (the
    /// worst case: every datum survives the early rejection). One series of
    /// markers; one pick in flight at a time, so the sustained hover rate is
    /// one over the latency. Prints a table; never fails on a time.
    #[test]
    #[ignore = "measurement (#381): run in release with --ignored"]
    fn nearest_datum_scale() {
        let sizes: Vec<usize> = std::env::var("FIGGY_PICK_SCALE_POINTS")
            .unwrap_or_else(|_| "1000000,10000000,50000000,80000000".into())
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        let instance = crate::test_gpu::instance();
        let adapter = crate::test_gpu::adapter(&instance);
        let limits = adapter.limits();
        let (device, queue) = crate::test_gpu::request_device(
            &adapter,
            &wgpu::DeviceDescriptor {
                required_limits: limits.clone(),
                ..Default::default()
            },
        );
        println!(
            "adapter {:?}, max storage binding {} B, max buffer {} B",
            adapter.get_info().name,
            limits.max_storage_buffer_binding_size,
            limits.max_buffer_size
        );
        println!(
            "| points | layout | exact first ms | exact median ms | nearest first ms | nearest median ms | nearest hover picks/s | same answer |"
        );
        println!("|---:|---|---:|---:|---:|---:|---:|---|");
        for points in sizes {
            for all_inside in [false, true] {
                let ledger = Arc::new(GpuLedger::new());
                let ctx = || crate::data_render::GpuAllocCtx::new(&device, &queue, &ledger);
                let pool_bytes = points as u64 * 16 + 1024 * 1024;
                let mut pool = match ColumnPool::new(ctx(), pool_bytes) {
                    Ok(pool) => pool,
                    Err(error) => {
                        println!(
                            "| {points} | {} | pool refused: {error} | | | | | |",
                            if all_inside { "all inside" } else { "spread" }
                        );
                        continue;
                    }
                };
                // Spread: a hash over [0, 1000); all inside: within 0.4 data
                // units of the centre, so within one pixel of the cursor.
                let value = |i: usize, seed: u64| -> f32 {
                    let mut x = (i as u64 ^ seed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    x ^= x >> 29;
                    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    x ^= x >> 32;
                    let unit = (x >> 40) as f32 / (1u64 << 24) as f32;
                    if all_inside {
                        500.0 + (unit - 0.5) * 0.8
                    } else {
                        unit * 1000.0
                    }
                };
                let xs: Vec<f32> = (0..points).map(|i| value(i, 1)).collect();
                let ys: Vec<f32> = (0..points).map(|i| value(i, 2)).collect();
                let added = pool
                    .add_column("scale-x".into(), &crate::data::Column { data: xs }, ctx())
                    .and_then(|_| {
                        pool.add_column("scale-y".into(), &crate::data::Column { data: ys }, ctx())
                    });
                if let Err(error) = added {
                    println!("| {points} | upload refused: {error} | | | | | | |");
                    continue;
                }
                let mut engine = GpuPickEngine::new(device.clone(), queue.clone()).unwrap();
                if let Err(error) = engine.add_series(
                    &pool,
                    GpuPickSeriesDescriptor {
                        source_id: None,
                        series_id: "scale".into(),
                        x_column: "scale-x".into(),
                        y_column: "scale-y".into(),
                        scatter: Some(GpuPickScatter {
                            base_radius_px: 3.0,
                            base_shape_id: 5,
                            style_map: None,
                            size_map: None,
                        }),
                        line_width_px: None,
                        line_closed: false,
                        path_arrows: None,
                        line_interpolation: crate::data_config::LineInterpolation::Linear,
                        line_style: crate::line::LineStylePreset::Solid,
                        vector: None,
                        errorbar: None,
                        area: None,
                        category_jitter: None,
                        ternary_c: None,
                        ternary_region: false,
                        stem: None,
                    },
                ) {
                    println!("| {points} | series refused: {error} | | | | | | |");
                    continue;
                }
                let query = GpuPickQuery {
                    transform: transform(1000.0),
                    chart_rect_px: [0.0, 0.0, 800.0, 600.0],
                    data_area_px: Some([0.0, 0.0, 800.0, 600.0]),
                    canvas_position_px: [400.0, 300.0],
                    max_distance_px: 8.0,
                    draw_style: GpuPickDrawStyle::PRECISE,
                };
                let time = |f: &mut dyn FnMut()| {
                    let started = Instant::now();
                    f();
                    started.elapsed().as_secs_f64() * 1e3
                };
                let mut exact_answer = None;
                let mut exact_pipelines = PickExactPipelines::default();
                let mut exact = || {
                    // The pipelines compile on the first pick and are kept,
                    // as the renderer's picker keeps them.
                    for key in engine.exact_variants(&query) {
                        exact_pipelines.ensure(engine.bundle(), key).unwrap();
                    }
                    exact_answer = engine
                        .pick_with_arcs(
                            &pool,
                            query,
                            1.0,
                            &[],
                            &[],
                            0,
                            &|_| query.transform,
                            &exact_pipelines,
                        )
                        .unwrap()
                        .resolve_blocking()
                        .unwrap()
                        .map(|p| (p.point_index, p.distance_px));
                };
                let exact_first = time(&mut exact);
                let mut exact_times: Vec<f64> = (0..7).map(|_| time(&mut exact)).collect();
                let mut pipelines = NearestPickPipelines::new(ShaderSet::Full);
                let mut nearest_answer = None;
                let mut nearest = || {
                    nearest_answer = engine
                        .pick_nearest(&mut pipelines, &pool, query, 1.0, 0, &|_| query.transform)
                        .unwrap()
                        .resolve_blocking()
                        .unwrap()
                        .nearest
                        .first()
                        .map(|d| (d.point_index, d.distance_px));
                };
                let nearest_first = time(&mut nearest);
                let mut nearest_times: Vec<f64> = (0..7).map(|_| time(&mut nearest)).collect();
                exact_times.sort_by(f64::total_cmp);
                nearest_times.sort_by(f64::total_cmp);
                let median = nearest_times[nearest_times.len() / 2];
                println!(
                    "| {points} | {} | {exact_first:.1} | {:.2} | {nearest_first:.1} | {median:.2} | {:.1} | exact {exact_answer:?}, nearest {nearest_answer:?} |",
                    if all_inside { "all inside" } else { "spread" },
                    exact_times[exact_times.len() / 2],
                    1000.0 / median,
                );
                drop(engine);
                drop(pool);
                device.poll(wgpu::PollType::wait_indefinitely()).ok();
            }
        }
    }

    /// Writes the inputs of the browser run of the nearest-datum kernel
    /// (`ci/pick_nearest_browser.cjs`): the basic module's source and, per
    /// size, the query parameters of one marker series whose x pairs start
    /// at pool word 0 and y pairs at word `2 × points`, with the registry's
    /// dispatch for that size. The browser builds the pool itself.
    #[test]
    #[ignore = "measurement (#381): writes target/ci-results/pick_nearest_browser.json"]
    fn export_nearest_browser_inputs() {
        let (device, queue) = crate::test_gpu::device();
        let ledger = Arc::new(GpuLedger::new());
        let ctx = || crate::data_render::GpuAllocCtx::new(&device, &queue, &ledger);
        let mut pool = ColumnPool::new(ctx(), 1024 * 1024).unwrap();
        for id in ["bx", "by"] {
            pool.add_column(
                id.into(),
                &crate::data::Column {
                    data: vec![0.0f32; 4],
                },
                ctx(),
            )
            .unwrap();
        }
        let mut engine = GpuPickEngine::new(device.clone(), queue.clone()).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "b".into(),
                    x_column: "bx".into(),
                    y_column: "by".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 3.0,
                        base_shape_id: 5,
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_closed: false,
                    path_arrows: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                    errorbar: None,
                    area: None,
                    category_jitter: None,
                    ternary_c: None,
                    ternary_region: false,
                    stem: None,
                },
            )
            .unwrap();
        let slot = &engine.registry.slots[0];
        let query = GpuPickQuery {
            transform: transform(1000.0),
            chart_rect_px: [0.0, 0.0, 800.0, 600.0],
            data_area_px: Some([0.0, 0.0, 800.0, 600.0]),
            canvas_position_px: [400.0, 300.0],
            max_distance_px: 8.0,
            draw_style: GpuPickDrawStyle::PRECISE,
        };
        let limit = device.limits().max_compute_workgroups_per_dimension;
        let mut cases = Vec::new();
        for points in [
            1_000_000u32,
            4_000_000,
            8_000_000,
            16_000_000,
            50_000_000,
            80_000_000,
        ] {
            let (words, groups, dispatch_x, dispatch_y) =
                engine::gate_dispatch_layout(points, limit).unwrap();
            let mut params = query_params(
                &slot.gpu,
                "b",
                &query,
                1.0,
                0,
                [points, 0, 2 * points, 0],
                [0, points, points, 0],
                false,
            );
            params.series = [words, 0, params.series[2], dispatch_x];
            let words_u32: &[u32] = bytemuck::cast_slice(bytemuck::bytes_of(&params));
            cases.push(serde_json::json!({
                "points": points,
                "groups": groups,
                "dispatch": [dispatch_x, dispatch_y],
                "params": words_u32,
            }));
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ci-results");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(
            out.join("pick_nearest_browser.json"),
            serde_json::to_vec(&serde_json::json!({
                "source": NEAREST_PICK_MODULE.1.basic.source,
                "candidate_bytes": CANDIDATE_BYTES,
                "k": PICK_NEAREST_K,
                "cases": cases,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
