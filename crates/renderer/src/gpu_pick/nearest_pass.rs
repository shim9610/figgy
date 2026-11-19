//! The nearest-datum pick (#381 prototype, the maintainer's decision:
//! "하나만 찾자 그럼 시리즈당 1"): one compute kernel per column kind that
//! places each datum with the draw's projection and reduces, with no pick
//! reach, to each series' nearest datum and the panel's nearest among them.
//! Whether the cursor hits is the drawn pixel's to decide (ink under the
//! cursor: the hit is the panel's nearest datum); without ink there is no
//! hit, and the per-series nearest data are still returned.
//!
//! The kernel (`gpu_pick/nearest_entry.wgsl`, the xy column kind) is
//! spliced from the point picker's core and the shared projection
//! fragments only: no marker shape, stroke, dash or style, so it has no
//! variant. It runs in the exact pass's slot (the registry's dispatch and
//! per-workgroup candidates) and its result goes through the point picker's
//! per-series and cross-series reductions into storage buffers: no render
//! target, so neither MSAA nor integer formats are involved. The readback
//! holds the panel's nearest datum and each series' own nearest one.

use super::engine::create_buffer_checked;
use super::*;
use crate::shader_assembly::{AssembledBySet, ShaderSet, assembled_by_set};

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

/// The nearest-datum pipeline of one shader set, created on first use.
pub struct NearestPickPipelines {
    set: ShaderSet,
    pipeline: Option<wgpu::ComputePipeline>,
}

impl NearestPickPipelines {
    pub(crate) fn new(set: ShaderSet) -> Self {
        Self {
            set,
            pipeline: None,
        }
    }

    pub(crate) fn set(&self) -> ShaderSet {
        self.set
    }

    /// How many pipelines have been created (0 or 1: one per column kind).
    pub fn created(&self) -> usize {
        usize::from(self.pipeline.is_some())
    }

    pub(crate) fn ensure(&mut self, bundle: &PickPipelineBundle) -> &wgpu::ComputePipeline {
        let set = self.set;
        self.pipeline.get_or_insert_with(|| {
            let device = &bundle.device;
            let module = crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some(NEAREST_PICK_MODULE.0),
                    source: wgpu::ShaderSource::Wgsl(NEAREST_PICK_MODULE.1.of(set).source.into()),
                },
            );
            let base = crate::gpu_compile::workgroup_options(device);
            crate::gpu_compile::compute_pipeline(
                device,
                &wgpu::ComputePipelineDescriptor {
                    label: Some("figgy GPU pick nearest-datum pipeline"),
                    layout: Some(&bundle.exact_layout),
                    module: &module,
                    entry_point: Some("pick_nearest_points"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[],
                        zero_initialize_workgroup_memory: base.zero_initialize_workgroup_memory,
                    },
                    cache: None,
                },
            )
        })
    }
}

/// One series' nearest datum within the reach.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestDatum {
    pub source_id: Option<String>,
    pub series_id: String,
    /// The logical index (a monotonic series' sample index).
    pub point_index: usize,
    pub distance_px: f32,
}

/// A resolved nearest-datum pick: the panel's nearest datum (ties to the
/// series drawn on top) and every series' own nearest one, in draw order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NearestPick {
    pub nearest: Option<NearestDatum>,
    pub per_series: Vec<NearestDatum>,
}

/// A pending nearest-datum pick.
pub struct NearestPickTicket {
    readback: Option<GpuReadback>,
    identities: Arc<[PickIdentity]>,
    series: usize,
    pub(crate) monotonic: Vec<crate::renderer::MonotonicPickRows>,
    _charge: Option<GpuByteCharge>,
}

impl NearestPickTicket {
    fn none() -> Self {
        Self {
            readback: None,
            identities: Arc::from([]),
            series: 0,
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
        let series = self.series;
        let Some(readback) = self.readback.as_mut() else {
            return Ok(NearestPick::default());
        };
        let size = CANDIDATE_BYTES as usize;
        let mut decoded = Vec::new();
        decoded
            .try_reserve_exact(series + 1)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick result",
            })?;
        readback.finish_checked(|mapped| {
            for index in 0..=series {
                decoded.push(*bytemuck::from_bytes::<PickCandidateGpu>(
                    &mapped[index * size..][..size],
                ));
            }
        })?;
        let datum = |candidate: &PickCandidateGpu| -> Result<Option<NearestDatum>, GpuPickError> {
            if candidate.valid != 1 {
                return Ok(None);
            }
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
                    None => return Ok(None),
                },
                None => candidate.point_index as usize,
            };
            Ok(Some(NearestDatum {
                source_id: identity.source_id.clone(),
                series_id: identity.series_id.clone(),
                point_index,
                distance_px: candidate.distance_px,
            }))
        };
        let nearest = datum(&decoded[0])?;
        let mut per_series = Vec::new();
        per_series
            .try_reserve_exact(series)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick result",
            })?;
        for candidate in &decoded[1..] {
            if let Some(found) = datum(candidate)? {
                per_series.push(found);
            }
        }
        Ok(NearestPick {
            nearest,
            per_series,
        })
    }
}

impl GpuPickEngine {
    /// The nearest-datum pick of the registered series at `query`'s cursor,
    /// with no reach (`query.max_distance_px` is not used): the checks and
    /// query parameters of
    /// [`Self::pick_with_arcs`], then the nearest-datum kernel in place of
    /// the gate and exact passes, and the same reductions.
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
        let pipeline = pipelines.ensure(&self.bundle).clone();
        let members = members.min(self.registry.slots.len());
        let first_member = self.registry.slots.len() - members;
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
        }
        let series_count = self.registry.slots.len();
        let tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick nearest-datum readback"),
                size: CANDIDATE_BYTES * (1 + series_count as u64),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "nearest pick readback",
        )?;
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy GPU pick nearest-datum encoder"),
                });
        encoder.clear_buffer(&self.registry.reduce.candidates, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum scan"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(2, &self.bundle.arc_placeholder, &[]);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum per-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            for slot in &self.registry.slots {
                pass.set_bind_group(0, &slot.gpu.reduce_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, slot) in self.registry.slots.iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                &slot.gpu.result,
                0,
                &self.registry.reduce.candidates,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
            encoder.copy_buffer_to_buffer(
                &slot.gpu.result,
                0,
                &readback,
                (1 + index as u64) * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum cross-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            pass.set_bind_group(0, &self.registry.reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        self.bundle.queue.submit(std::iter::once(encoder.finish()));
        Ok(NearestPickTicket {
            readback: Some(GpuReadback::map(
                &self.bundle.device,
                &self.bundle.health,
                &readback,
                None,
            )),
            identities: Arc::clone(&self.registry.identities),
            series: series_count,
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
                "cases": cases,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
