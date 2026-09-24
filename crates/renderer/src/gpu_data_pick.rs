//! Exact GPU picking for histogram bins and matrix-backed field primitives.
//!
//! The compute entry points live in the same WGSL modules as the render entry
//! points. Histogram picking calls the bar renderer's rectangle helper;
//! heatmap/contour picking calls the field renderer's `locate`,
//! `contour_sample`, and quadratic contour-distance helpers. The CPU submits
//! only transforms, lane bases, style scalars, and identity indices. It never
//! reads or mirrors source values from [`ColumnPool`](crate::data_render::ColumnPool).

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::data_render::{ColumnHandle, ColumnPool, ScatterTransform};
use crate::gpu_memory::{
    ChargeTally, GpuByteCharge, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer,
    charged_buffer_init, shared_charge,
};
use crate::init::{InitEvent, finished, observe_value, started};
use crate::pick::PickedData;
use futures_channel::oneshot;

const INIT_SCOPE: &str = "renderer.gpu_data_pick";
const DATA_PICK_CANDIDATE_BYTES: u64 = 32;
const DATA_PICK_FLAG_FILL: u32 = 1;
const DATA_PICK_FLAG_CONTOUR: u32 = 2;
const DATA_PICK_KIND_HISTOGRAM_BIN: u32 = 1;
const DATA_PICK_KIND_MATRIX_CELL: u32 = 2;
const DATA_PICK_KIND_CONTOUR_LEVEL: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DataPickQueryGpu {
    cursor_ndc_t: [f32; 4],
    limits: [f32; 4],
    data: [u32; 4],
    bases: [u32; 4],
    baseline: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DataPickCandidateGpu {
    valid: u32,
    paint_order: u32,
    kind: u32,
    index0: u32,
    index1: u32,
    index2: u32,
    distance_px: f32,
    primitive_order: u32,
}

const _: () = assert!(std::mem::size_of::<DataPickQueryGpu>() == 80);
const _: () = assert!(std::mem::size_of::<DataPickCandidateGpu>() == 32);

#[derive(Clone)]
struct DataPickIdentity {
    source_id: Option<String>,
    series_id: String,
    paint_order: u32,
}

pub(crate) enum GpuDataPickGeometry {
    Histogram {
        edges: ColumnHandle,
        values: ColumnHandle,
        bin_count: u32,
        baseline: [f32; 2],
        gap_px: f32,
        width_ratio: f32,
        horizontal: bool,
        style_map: Option<wgpu::BindGroup>,
    },
    Field {
        field_bg: wgpu::BindGroup,
        charge: SharedCharge,
        has_fill: bool,
        has_contour: bool,
        contour_width_px: f32,
    },
}

pub(crate) struct GpuDataPickSeries {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) paint_order: u32,
    pub(crate) geometry: GpuDataPickGeometry,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GpuDataPickQuery {
    pub(crate) transform: ScatterTransform,
    pub(crate) chart_rect_px: [f32; 4],
    pub(crate) data_area_px: Option<[f32; 4]>,
    pub(crate) canvas_position_px: [f32; 2],
    pub(crate) max_distance_px: f32,
}

pub(crate) struct GpuStreamDataPick {
    bundle: Arc<DataPickPipelineBundle>,
    query: GpuDataPickQuery,
    identities: Arc<[DataPickIdentity]>,
    transform_bg: wgpu::BindGroup,
    candidate: wgpu::Buffer,
    best: wgpu::Buffer,
    charge: SharedCharge,
    has_chunks: bool,
    enabled: bool,
}

pub(crate) struct GpuStreamFieldQuery {
    pub query: wgpu::Buffer,
    pub candidate: wgpu::Buffer,
    pub _query_charge: SharedCharge,
    pub _candidate_charge: SharedCharge,
}

impl DataPickPipelineBundle {
    pub(crate) fn begin_stream(
        self: &Arc<Self>,
        query: GpuDataPickQuery,
        identities: Vec<(Option<String>, String)>,
    ) -> Result<GpuStreamDataPick, crate::gpu_pick::GpuPickError> {
        let tally = ChargeTally::new();
        let transform = charged_buffer_init(
            &tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy stream typed pick transform"),
                contents: bytemuck::bytes_of(&query.transform),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let transform_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy stream typed pick transform bg"),
            layout: &self.histogram.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform.as_entire_binding(),
            }],
        });
        let make_scalar = |label| {
            charged_buffer(
                &tally,
                &self.device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size: DATA_PICK_CANDIDATE_BYTES,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )
        };
        let candidate = make_scalar("figgy stream typed pick candidate");
        let best = make_scalar("figgy stream typed pick best");
        let [cx, cy] = query.canvas_position_px;
        let [x, y, w, h] = query.chart_rect_px;
        let enabled = [cx, cy, x, y, w, h, query.max_distance_px]
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
        Ok(GpuStreamDataPick {
            bundle: Arc::clone(self),
            query,
            identities: identities
                .into_iter()
                .enumerate()
                .map(|(index, (source_id, series_id))| DataPickIdentity {
                    source_id,
                    series_id,
                    paint_order: index as u32,
                })
                .collect(),
            transform_bg,
            candidate,
            best,
            charge: shared_charge(tally, &self.ledger, GpuResourceKind::PickScratch),
            has_chunks: false,
            enabled,
        })
    }
}

impl GpuStreamDataPick {
    pub(crate) fn field_enabled(&self) -> bool { self.enabled }

    pub(crate) fn field_query(&self, paint_order: u32) -> Result<GpuStreamFieldQuery, crate::gpu_pick::GpuPickError> {
        if paint_order as usize >= self.identities.len() { return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult); }
        let [cx, cy] = self.query.canvas_position_px;
        let [x, y, w, h] = self.query.chart_rect_px;
        let nx = ((cx - x) / w) * 2.0 - 1.0;
        let ny = 1.0 - ((cy - y) / h) * 2.0;
        let params = DataPickQueryGpu {
            cursor_ndc_t: [nx, ny, (nx + 1.0) * 0.5, (ny + 1.0) * 0.5],
            limits: [self.query.max_distance_px, 0.0, 0.0, 0.0],
            data: [DATA_PICK_FLAG_FILL, paint_order, 0, 0], bases: [0; 4], baseline: [0.0; 4],
        };
        let tally = ChargeTally::new();
        let query = charged_buffer_init(&tally, &self.bundle.device, &wgpu::util::BufferInitDescriptor {
            label: Some("stream field pick query"), contents: bytemuck::bytes_of(&params), usage: wgpu::BufferUsages::UNIFORM,
        });
        Ok(GpuStreamFieldQuery { query, candidate: self.candidate.clone(),
            _query_charge: shared_charge(tally, &self.bundle.ledger, GpuResourceKind::PickScratch),
            _candidate_charge: Arc::clone(&self.charge) })
    }

    pub(crate) fn encode_field_candidate(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let submitted_chunks = self.has_chunks;
        self.accumulate(encoder, &self.candidate.clone(), false);
        self.has_chunks = submitted_chunks;
    }

    pub(crate) fn commit_field_candidate(&mut self) {
        self.has_chunks = true;
    }
    pub(crate) const PERSISTENT_BYTES: u64 =
        std::mem::size_of::<ScatterTransform>() as u64 + 2 * DATA_PICK_CANDIDATE_BYTES;
    pub(crate) const CHUNK_BYTES: u64 = std::mem::size_of::<DataPickQueryGpu>() as u64;
    pub(crate) const READBACK_BYTES: u64 = DATA_PICK_CANDIDATE_BYTES;

    pub(crate) fn charge(&self) -> SharedCharge {
        self.charge.clone()
    }

    fn accumulate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        point: bool,
    ) {
        if !self.has_chunks {
            encoder.clear_buffer(&self.best, 0, None);
        }
        let bg = self
            .bundle
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy stream typed pick running reduction bg"),
                layout: &self.bundle.stream_reduce_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.best.as_entire_binding(),
                    },
                ],
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy stream typed pick running reduction"),
            timestamp_writes: None,
        });
        pass.set_pipeline(if point {
            &self.bundle.stream_reduce_point
        } else {
            &self.bundle.stream_reduce_data
        });
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        self.has_chunks = true;
    }

    /// Reduce the final point/line answer on the GPU. Point and typed streams
    /// must use the same chart-paint-order identity table.
    pub(crate) fn encode_point_result(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        point: &crate::gpu_pick::GpuStreamPick,
    ) {
        if self.enabled
            && let Some(buffer) = point.result_buffer()
        {
            self.accumulate(encoder, buffer, true);
        }
    }

    /// Uses the resident histogram geometry function, with bounded local lane
    /// bases and global override/bin identities. Retain the returned charge
    /// and any supplied style-map charge until this submission completes.
    pub(crate) fn encode_chunk(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        series: GpuDataPickSeries,
        global_bin_start: u32,
    ) -> Result<Option<SharedCharge>, crate::gpu_pick::GpuPickError> {
        use crate::gpu_pick::GpuPickError;
        if !self.enabled {
            return Ok(None);
        }
        let identity = self.identities.get(series.paint_order as usize).ok_or(
            GpuPickError::InvalidSeriesIndex {
                index: series.paint_order as usize,
                len: self.identities.len(),
            },
        )?;
        if identity.source_id != series.source_id || identity.series_id != series.series_id {
            return Err(GpuPickError::InvalidGpuResult);
        }
        let GpuDataPickGeometry::Histogram {
            edges,
            values,
            bin_count,
            baseline,
            gap_px,
            width_ratio,
            horizontal,
            style_map,
        } = series.geometry
        else {
            return Err(GpuPickError::NoPickPrimitive(series.series_id));
        };
        if bin_count == 0 {
            return Ok(None);
        }
        if bin_count as usize > values.len_values
            || bin_count as usize >= edges.len_values
            || global_bin_start.checked_add(bin_count - 1).is_none()
        {
            return Err(GpuPickError::TooManyValues {
                series_id: series.series_id,
                count: bin_count as usize,
            });
        }
        for handle in [edges, values] {
            crate::gpu_pick::validate_stream_handle(buffer, handle)?;
        }
        if buffer.size() > self.bundle.device.limits().max_storage_buffer_binding_size {
            return Err(GpuPickError::DeviceLimit {
                resource: "stream typed pick storage",
                requested: buffer.size(),
                limit: self.bundle.device.limits().max_storage_buffer_binding_size,
            });
        }
        let [cx, cy] = self.query.canvas_position_px;
        let [x, y, w, h] = self.query.chart_rect_px;
        let tx = (cx - x) / w;
        let nx = tx * 2.0 - 1.0;
        let ny = 1.0 - ((cy - y) / h) * 2.0;
        let params = DataPickQueryGpu {
            cursor_ndc_t: [nx, ny, (nx + 1.0) * 0.5, (ny + 1.0) * 0.5],
            limits: [self.query.max_distance_px, gap_px, width_ratio, 0.0],
            data: [0, series.paint_order, bin_count, u32::from(horizontal)],
            bases: [lane_base(edges)?, lane_base(values)?, global_bin_start, 0],
            baseline: [baseline[0], baseline[1], 0.0, 0.0],
        };
        let tally = ChargeTally::new();
        let uniform = charged_buffer_init(
            &tally,
            &self.bundle.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy stream typed pick chunk params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let query_bg = self
            .bundle
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy stream typed pick query bg"),
                layout: &self.bundle.query_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.candidate.as_entire_binding(),
                    },
                ],
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream histogram pick"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.histogram);
            pass.set_bind_group(0, &self.transform_bg, &[]);
            pass.set_bind_group(1, &query_bg, &[]);
            pass.set_bind_group(
                2,
                style_map
                    .as_ref()
                    .unwrap_or(&self.bundle.empty_bar_style_map),
                &[],
            );
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.accumulate(encoder, &self.candidate.clone(), false);
        Ok(Some(shared_charge(
            tally,
            &self.bundle.ledger,
            GpuResourceKind::PickScratch,
        )))
    }

    /// All source chunks and the optional point merge must already be submitted.
    pub(crate) fn finish(self) -> Result<GpuDataPickTicket, crate::gpu_pick::GpuPickError> {
        if !self.has_chunks {
            return Ok(GpuDataPickTicket {
                state: GpuDataPickTicketState::Stream(DataPickTicket::ready_none()),
            });
        }
        let tally = ChargeTally::new();
        let readback = charged_buffer(
            &tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy stream typed pick readback"),
                size: DATA_PICK_CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy stream typed pick finish"),
                });
        encoder.copy_buffer_to_buffer(&self.best, 0, &readback, 0, DATA_PICK_CANDIDATE_BYTES);
        self.bundle.queue.submit([encoder.finish()]);
        let (sender, receiver) = oneshot::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        Ok(GpuDataPickTicket {
            state: GpuDataPickTicketState::Stream(DataPickTicket {
                state: DataPickTicketState::Pending {
                    device: Arc::clone(&self.bundle.device),
                    readback,
                    receiver,
                    identities: self.identities,
                    byte_len: DATA_PICK_CANDIDATE_BYTES,
                    gpu_reduced: true,
                    _field_charges: vec![self.charge],
                    _scratch_charge: ChargeTally::new()
                        .into_charge(&self.bundle.ledger, GpuResourceKind::PickScratch),
                    _readback_charge: tally
                        .into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
                },
            }),
        })
    }
}

fn compute_storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn compute_uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Immutable pipelines shared by every chart and pick query on one renderer.
pub(crate) struct DataPickPipelineBundle {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    query_bgl: wgpu::BindGroupLayout,
    histogram: wgpu::ComputePipeline,
    field: wgpu::ComputePipeline,
    empty_bar_style_map: wgpu::BindGroup,
    stream_reduce_bgl: wgpu::BindGroupLayout,
    stream_reduce_data: wgpu::ComputePipeline,
    stream_reduce_point: wgpu::ComputePipeline,
}

impl DataPickPipelineBundle {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_observed(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        bar_shader: &wgpu::ShaderModule,
        field_shader: &wgpu::ShaderModule,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Arc<Self> {
        started(observer, INIT_SCOPE, "setup");
        let query_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy typed data pick query bgl"),
            entries: &[
                compute_uniform_entry(1),
                compute_storage_entry(2, true),
                compute_storage_entry(3, false),
            ],
        });
        let histogram_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy histogram data pick layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(&query_bgl), Some(bar_style_bgl)],
            immediate_size: 0,
        });
        let field_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy field data pick layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(&query_bgl), Some(field_bgl)],
            immediate_size: 0,
        });
        finished(observer, INIT_SCOPE, "setup");

        let histogram = observe_value(observer, INIT_SCOPE, "pick_histogram_bin", || {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("figgy histogram data pick pipeline"),
                layout: Some(&histogram_layout),
                module: bar_shader,
                entry_point: Some("pick_histogram_bin"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        });
        let field = observe_value(observer, INIT_SCOPE, "pick_field_data", || {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("figgy field data pick pipeline"),
                layout: Some(&field_layout),
                module: field_shader,
                entry_point: Some("pick_field_data"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        });
        let empty_bar_style_map =
            crate::data_render::create_bar_style_map(&device, bar_style_bgl, &[]).bind_group;
        let stream_reduce_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy streamed typed pick reduction bgl"),
            entries: &[
                compute_storage_entry(0, true),
                compute_storage_entry(1, false),
            ],
        });
        let stream_reduce_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy streamed typed pick reduction layout"),
            bind_group_layouts: &[Some(&stream_reduce_bgl)],
            immediate_size: 0,
        });
        let stream_reduce_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figgy streamed typed pick reduction shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_stream_data_pick.wgsl").into()),
        });
        let make_reduce = |entry: &'static str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&stream_reduce_layout),
                module: &stream_reduce_shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let stream_reduce_data = observe_value(observer, INIT_SCOPE, "stream_reduce_data", || {
            make_reduce("accumulate_data")
        });
        let stream_reduce_point =
            observe_value(observer, INIT_SCOPE, "stream_reduce_point", || {
                make_reduce("accumulate_point")
            });
        Arc::new(Self {
            device,
            queue,
            ledger,
            query_bgl,
            histogram,
            field,
            empty_bar_style_map,
            stream_reduce_bgl,
            stream_reduce_data,
            stream_reduce_point,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn new_observed_async(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        bar_shader: &wgpu::ShaderModule,
        field_shader: &wgpu::ShaderModule,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, crate::gpu_pick::GpuPickError> {
        #[cfg(target_arch = "wasm32")]
        {
            crate::init::prewarm_compute_entries_js(
                &device,
                "gpu.data_pick.bar.async",
                include_str!("data_render/bar_columnar.wgsl"),
                &[("pick_histogram_bin", "pick_histogram_bin")],
                observer,
            )
            .await
            .map_err(crate::gpu_pick::GpuPickError::AsyncCompileFailed)?;
            crate::init::prewarm_compute_entries_js(
                &device,
                "gpu.data_pick.stream.async",
                include_str!("gpu_stream_data_pick.wgsl"),
                &[
                    ("accumulate_data", "accumulate_data"),
                    ("accumulate_point", "accumulate_point"),
                ],
                observer,
            )
            .await
            .map_err(crate::gpu_pick::GpuPickError::AsyncCompileFailed)?;
            crate::init::prewarm_compute_entries_js(
                &device,
                "gpu.data_pick.field.async",
                include_str!("data_render/field_columnar.wgsl"),
                &[("pick_field_data", "pick_field_data")],
                observer,
            )
            .await
            .map_err(crate::gpu_pick::GpuPickError::AsyncCompileFailed)?;
        }
        Ok(Self::new_observed(
            device,
            queue,
            ledger,
            bar_shader,
            field_shader,
            transform_bgl,
            field_bgl,
            bar_style_bgl,
            observer,
        ))
    }

    pub(crate) fn submit(
        &self,
        pool: &ColumnPool,
        query: GpuDataPickQuery,
        series: Vec<GpuDataPickSeries>,
    ) -> Result<DataPickTicket, crate::gpu_pick::GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(DataPickTicket::ready_none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(DataPickTicket::ready_none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(DataPickTicket::ready_none());
            }
        }
        if series.is_empty() {
            return Ok(DataPickTicket::ready_none());
        }

        let candidate_count = u64::try_from(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick candidate count",
            }
        })?;
        let candidate_bytes = candidate_count
            .checked_mul(DATA_PICK_CANDIDATE_BYTES)
            .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick candidate bytes",
            })?;
        let limits = self.device.limits();
        let candidate_stride =
            u64::from(limits.min_storage_buffer_offset_alignment).max(DATA_PICK_CANDIDATE_BYTES);
        let output_bytes = candidate_count.checked_mul(candidate_stride).ok_or(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick aligned output bytes",
            },
        )?;
        if candidate_bytes > limits.max_buffer_size || output_bytes > limits.max_buffer_size {
            return Err(crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "typed data pick candidate buffer",
                requested: candidate_bytes.max(output_bytes),
                limit: limits.max_buffer_size,
            });
        }

        let cursor_ndc_x = ((cursor_x - chart_x) / chart_width) * 2.0 - 1.0;
        let cursor_ndc_y = 1.0 - ((cursor_y - chart_y) / chart_height) * 2.0;
        let cursor = [
            cursor_ndc_x,
            cursor_ndc_y,
            (cursor_ndc_x + 1.0) * 0.5,
            (cursor_ndc_y + 1.0) * 0.5,
        ];

        let scratch_tally = ChargeTally::new();
        let readback_tally = ChargeTally::new();
        let transform_buffer = charged_buffer_init(
            &scratch_tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy typed data pick transform"),
                contents: bytemuck::bytes_of(&query.transform),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let transform_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy typed data pick transform bg"),
            layout: &self.histogram.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });
        let candidates = charged_buffer(
            &scratch_tally,
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick candidates"),
                size: output_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        );
        let readback = charged_buffer(
            &readback_tally,
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick readback"),
                size: candidate_bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );

        enum PreparedKind {
            Histogram(wgpu::BindGroup),
            Field(wgpu::BindGroup),
        }
        struct PreparedSeries {
            query_bg: wgpu::BindGroup,
            kind: PreparedKind,
        }

        let mut prepared = Vec::new();
        prepared.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick prepared series",
            }
        })?;
        let mut identities = Vec::new();
        identities.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick identities",
            }
        })?;
        let mut field_charges = Vec::new();

        for (index, item) in series.into_iter().enumerate() {
            let (params, kind) = match item.geometry {
                GpuDataPickGeometry::Histogram {
                    edges,
                    values,
                    bin_count,
                    baseline,
                    gap_px,
                    width_ratio,
                    horizontal,
                    style_map,
                } => (
                    DataPickQueryGpu {
                        cursor_ndc_t: cursor,
                        limits: [query.max_distance_px, gap_px, width_ratio, 0.0],
                        data: [0, item.paint_order, bin_count, u32::from(horizontal)],
                        bases: [lane_base(edges)?, lane_base(values)?, 0, 0],
                        baseline: [baseline[0], baseline[1], 0.0, 0.0],
                    },
                    PreparedKind::Histogram(
                        style_map.unwrap_or_else(|| self.empty_bar_style_map.clone()),
                    ),
                ),
                GpuDataPickGeometry::Field {
                    field_bg,
                    charge,
                    has_fill,
                    has_contour,
                    contour_width_px,
                } => {
                    let mut flags = 0;
                    if has_fill {
                        flags |= DATA_PICK_FLAG_FILL;
                    }
                    if has_contour {
                        flags |= DATA_PICK_FLAG_CONTOUR;
                    }
                    field_charges.push(charge);
                    (
                        DataPickQueryGpu {
                            cursor_ndc_t: cursor,
                            limits: [query.max_distance_px, 0.0, contour_width_px, 0.0],
                            data: [flags, item.paint_order, 0, 0],
                            bases: [0; 4],
                            baseline: [0.0; 4],
                        },
                        PreparedKind::Field(field_bg),
                    )
                }
            };
            let query_buffer = charged_buffer_init(
                &scratch_tally,
                &self.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some("figgy typed data pick query"),
                    contents: bytemuck::bytes_of(&params),
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            );
            let offset = u64::try_from(index)
                .ok()
                .and_then(|index| index.checked_mul(candidate_stride))
                .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                    resource: "typed data pick output offset",
                })?;
            let query_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy typed data pick query bg"),
                layout: &self.query_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: query_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: pool.buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &candidates,
                            offset,
                            size: NonZeroU64::new(DATA_PICK_CANDIDATE_BYTES),
                        }),
                    },
                ],
            });
            identities.push(DataPickIdentity {
                source_id: item.source_id,
                series_id: item.series_id,
                paint_order: item.paint_order,
            });
            prepared.push(PreparedSeries { query_bg, kind });
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy typed data pick encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy typed data pick pass"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &transform_bg, &[]);
            for item in &prepared {
                match &item.kind {
                    PreparedKind::Histogram(style_map) => {
                        pass.set_pipeline(&self.histogram);
                        pass.set_bind_group(2, style_map, &[]);
                    }
                    PreparedKind::Field(field_bg) => {
                        pass.set_pipeline(&self.field);
                        pass.set_bind_group(2, field_bg, &[]);
                    }
                }
                pass.set_bind_group(1, &item.query_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for index in 0..candidate_count {
            encoder.copy_buffer_to_buffer(
                &candidates,
                index * candidate_stride,
                &readback,
                index * DATA_PICK_CANDIDATE_BYTES,
                DATA_PICK_CANDIDATE_BYTES,
            );
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..candidate_bytes);
        let (sender, receiver) = oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        Ok(DataPickTicket {
            state: DataPickTicketState::Pending {
                device: Arc::clone(&self.device),
                readback,
                receiver,
                identities: identities.into(),
                byte_len: candidate_bytes,
                gpu_reduced: false,
                _field_charges: field_charges,
                _scratch_charge: scratch_tally
                    .into_charge(&self.ledger, GpuResourceKind::PickScratch),
                _readback_charge: readback_tally
                    .into_charge(&self.ledger, GpuResourceKind::Readback),
            },
        })
    }
}

fn lane_base(handle: ColumnHandle) -> Result<u32, crate::gpu_pick::GpuPickError> {
    u32::try_from(handle.offset / 4).map_err(|_| crate::gpu_pick::GpuPickError::DeviceLimit {
        resource: "typed data pick column lane base",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

enum DataPickTicketState {
    ReadyNone,
    Pending {
        device: Arc<wgpu::Device>,
        readback: wgpu::Buffer,
        receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
        identities: Arc<[DataPickIdentity]>,
        byte_len: u64,
        gpu_reduced: bool,
        _field_charges: Vec<SharedCharge>,
        _scratch_charge: GpuByteCharge,
        _readback_charge: GpuByteCharge,
    },
}

pub(crate) struct RankedPickedData {
    pub(crate) picked: PickedData,
    pub(crate) paint_order: u32,
}

pub(crate) struct DataPickTicket {
    state: DataPickTicketState,
}

impl DataPickTicket {
    fn ready_none() -> Self {
        Self {
            state: DataPickTicketState::ReadyNone,
        }
    }

    pub(crate) async fn resolve(
        self,
    ) -> Result<Option<RankedPickedData>, crate::gpu_pick::GpuPickError> {
        let DataPickTicketState::Pending {
            device: _poll_device,
            readback,
            receiver,
            identities,
            byte_len,
            gpu_reduced,
            _field_charges,
            _scratch_charge,
            _readback_charge,
        } = self.state
        else {
            return Ok(None);
        };

        #[cfg(not(target_arch = "wasm32"))]
        let _ = _poll_device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .await
            .map_err(|_| crate::gpu_pick::GpuPickError::MapChannelClosed)?
            .map_err(crate::gpu_pick::GpuPickError::MapFailed)?;
        let slice = readback.slice(..byte_len);
        let mapped = slice
            .get_mapped_range()
            .expect("typed data pick readback is mapped after map_async");
        let result = (|| {
            let records: &[DataPickCandidateGpu] = bytemuck::cast_slice(&mapped);
            if gpu_reduced {
                let [candidate] = records else {
                    return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                };
                if candidate.valid == 0 {
                    return Ok(None);
                }
                if candidate.valid != 1
                    || !candidate.distance_px.is_finite()
                    || candidate.distance_px < 0.0
                {
                    return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                }
                let identity = identities
                    .get(candidate.paint_order as usize)
                    .ok_or(crate::gpu_pick::GpuPickError::InvalidGpuResult)?;
                return decode_candidate(identity, *candidate).map(Some);
            }
            if records.len() != identities.len() {
                return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
            }

            let mut best: Option<(usize, DataPickCandidateGpu)> = None;
            for (index, candidate) in records.iter().copied().enumerate() {
                if candidate.valid == 0 {
                    continue;
                }
                if candidate.valid != 1
                    || !candidate.distance_px.is_finite()
                    || candidate.distance_px < 0.0
                    || identities[index].paint_order != candidate.paint_order
                {
                    return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                }
                let replace = best.as_ref().is_none_or(|(_, incumbent)| {
                    candidate.distance_px < incumbent.distance_px
                        || (candidate.distance_px == incumbent.distance_px
                            && candidate.paint_order > incumbent.paint_order)
                });
                if replace {
                    best = Some((index, candidate));
                }
            }
            best.map(|(index, candidate)| decode_candidate(&identities[index], candidate))
                .transpose()
        })();
        drop(mapped);
        readback.unmap();
        drop((_scratch_charge, _readback_charge));
        result
    }
}

fn decode_candidate(
    identity: &DataPickIdentity,
    candidate: DataPickCandidateGpu,
) -> Result<RankedPickedData, crate::gpu_pick::GpuPickError> {
    let source_id = identity.source_id.clone();
    let series_id = identity.series_id.clone();
    let distance_px = candidate.distance_px;
    let picked = match candidate.kind {
        0 => PickedData::Point {
            source_id,
            series_id,
            point_index: candidate.index0 as usize,
            distance_px,
        },
        DATA_PICK_KIND_HISTOGRAM_BIN => PickedData::HistogramBin {
            source_id,
            series_id,
            bin_index: candidate.index0 as usize,
            distance_px,
        },
        DATA_PICK_KIND_MATRIX_CELL => PickedData::MatrixCell {
            source_id,
            series_id,
            x_index: candidate.index0 as usize,
            y_index: candidate.index1 as usize,
            distance_px,
        },
        DATA_PICK_KIND_CONTOUR_LEVEL => PickedData::ContourLevel {
            source_id,
            series_id,
            level_index: candidate.index0 as usize,
            x_index: candidate.index1 as usize,
            y_index: candidate.index2 as usize,
            distance_px,
        },
        _ => return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult),
    };
    Ok(RankedPickedData {
        picked,
        paint_order: identity.paint_order,
    })
}

impl fmt::Debug for DataPickTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataPickTicket").finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct PointPaintOrder {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) paint_order: u32,
}

/// Owned async ticket for a typed data pick.
///
/// Point/line picking keeps its mature streaming gate engine; histogram/field
/// picking uses the render shaders' compute entries. Resident queries retain
/// their existing scalar merge at resolution. Streamed queries reduce every
/// chunk and primitive family on the GPU and map only the final scalar.
pub struct GpuDataPickTicket {
    state: GpuDataPickTicketState,
}

enum GpuDataPickTicketState {
    Resident {
        point: crate::gpu_pick::GpuPickTicket,
        data: DataPickTicket,
        point_orders: Arc<[PointPaintOrder]>,
    },
    Stream(DataPickTicket),
}

impl GpuDataPickTicket {
    pub(crate) fn new(
        point: crate::gpu_pick::GpuPickTicket,
        data: DataPickTicket,
        point_orders: Vec<PointPaintOrder>,
    ) -> Self {
        Self {
            state: GpuDataPickTicketState::Resident {
                point,
                data,
                point_orders: point_orders.into(),
            },
        }
    }

    pub async fn resolve(self) -> Result<Option<PickedData>, crate::gpu_pick::GpuPickError> {
        let (point, data, point_orders) = match self.state {
            GpuDataPickTicketState::Stream(ticket) => {
                return ticket
                    .resolve()
                    .await
                    .map(|result| result.map(|hit| hit.picked));
            }
            GpuDataPickTicketState::Resident {
                point,
                data,
                point_orders,
            } => (point, data, point_orders),
        };
        let point = point.resolve().await?;
        let data = data.resolve().await?;
        let point = point.map(|point| {
            let paint_order = point_orders
                .iter()
                .rev()
                .find(|identity| {
                    identity.series_id == point.series_id && identity.source_id == point.source_id
                })
                .map_or(0, |identity| identity.paint_order);
            RankedPickedData {
                picked: point.into(),
                paint_order,
            }
        });
        Ok(match (point, data) {
            (None, None) => None,
            (Some(point), None) => Some(point.picked),
            (None, Some(data)) => Some(data.picked),
            (Some(point), Some(data)) => {
                let point_distance = point.picked.distance_px();
                let data_distance = data.picked.distance_px();
                if data_distance < point_distance
                    || (data_distance == point_distance && data.paint_order > point.paint_order)
                {
                    Some(data.picked)
                } else {
                    Some(point.picked)
                }
            }
        })
    }
}

impl fmt::Debug for GpuDataPickTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuDataPickTicket").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use crate::data::{Column, split_f64_to_f32_pair};
    use crate::data_render::{self, BarStyleOverrideGpu, GpuAllocCtx};
    use wgpu::util::DeviceExt;

    fn bundle(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Arc<DataPickPipelineBundle> {
        let bar = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("data_render/bar_columnar.wgsl").into()),
        });
        let field = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                include_str!("data_render/field_columnar.wgsl").into(),
            ),
        });
        let transform = data_render::create_scatter_transform_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let style = data_render::create_per_point_style_map_bind_group_layout(&device);
        DataPickPipelineBundle::new_observed(
            device,
            queue,
            Arc::new(GpuLedger::new()),
            &bar,
            &field,
            &transform,
            &field_bgl,
            &style,
            &mut |_| {},
        )
    }

    fn config(logarithmic: bool) -> crate::config::Config {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        config.bottom_x.min = if logarithmic {
            1.0
        } else {
            1_000_000_000_000.0
        };
        config.bottom_x.max = if logarithmic {
            100.0
        } else {
            1_000_000_000_004.0
        };
        config.bottom_x.scale = if logarithmic {
            crate::config::AxisScale::Logarithmic
        } else {
            crate::config::AxisScale::Linear
        };
        config.bottom_x.inverted = logarithmic;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        config.left_y.inverted = logarithmic;
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }
        config
    }

    #[test]
    fn streamed_histogram_matches_resident_global_bin_ties_and_mapped_styles() {
        let Some((device, queue)) = data_render::shared_device() else {
            return;
        };
        let bundle = bundle(Arc::clone(&device), Arc::clone(&queue));
        for logarithmic in [false, true] {
            let origin = if logarithmic {
                0.0
            } else {
                1_000_000_000_000.0
            };
            let edges = if logarithmic {
                [1.0, 10.0, 5.0, 20.0, 100.0]
            } else {
                [0.0, 2.0, 1.0, 3.0, 4.0]
            }
            .map(|x| x + origin);
            let values = [f64::NAN, 6.0, 7.0, 8.0];
            let mut pool = ColumnPool::new(GpuAllocCtx::unbudgeted(&device, &queue), 4096).unwrap();
            pool.add_hilo_column(
                "edges".into(),
                &Column {
                    data: edges.to_vec(),
                    min: edges[0],
                    max: edges[4],
                },
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool.add_hilo_column(
                "values".into(),
                &Column {
                    data: values.to_vec(),
                    min: 6.0,
                    max: 8.0,
                },
                GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            let config = config(logarithmic);
            for mapped in [false, true] {
                let style_map = mapped.then(|| {
                    data_render::create_bar_style_map(
                        &device,
                        &bundle.histogram.get_bind_group_layout(2),
                        &[BarStyleOverrideGpu {
                            bin_index: 3,
                            _pad: [0; 3],
                            fill_color_premul: [0.0; 4],
                            border_color_premul: [0.0; 4],
                            params: [0.0, 0.0, 0.0, 16.0],
                        }],
                    )
                    .bind_group
                });
                let series = |order: u32, edges, values, bin_count| GpuDataPickSeries {
                    source_id: None,
                    series_id: format!("bars-{order}"),
                    paint_order: order,
                    geometry: GpuDataPickGeometry::Histogram {
                        edges,
                        values,
                        bin_count,
                        baseline: [0.0; 2],
                        gap_px: 0.0,
                        width_ratio: 1.0,
                        horizontal: false,
                        style_map: style_map.clone(),
                    },
                };
                for cursor in [[50.0, 50.0], [80.0, 50.0], [20.0, 50.0], [50.0, 15.0]] {
                    let query = GpuDataPickQuery {
                        transform: data_render::scatter_transform_from_config(&config),
                        chart_rect_px: [0.0, 0.0, 100.0, 100.0],
                        data_area_px: None,
                        canvas_position_px: cursor,
                        max_distance_px: 5.0,
                    };
                    let resident = bundle
                        .submit(
                            &pool,
                            query,
                            (0..2)
                                .map(|order| {
                                    series(
                                        order,
                                        pool.handle_for("edges").unwrap(),
                                        pool.handle_for("values").unwrap(),
                                        4,
                                    )
                                })
                                .collect(),
                        )
                        .unwrap();
                    let expected = pollster::block_on(resident.resolve())
                        .unwrap()
                        .map(|hit| hit.picked);
                    for chunk_size in [1usize, 2, 3] {
                        let mut stream = bundle
                            .begin_stream(
                                query,
                                (0..2)
                                    .map(|order| (None, format!("bars-{order}")))
                                    .collect(),
                            )
                            .unwrap();
                        let mut charges = vec![stream.charge()];
                        assert_eq!(
                            charges[0].charged_bytes(),
                            GpuStreamDataPick::PERSISTENT_BYTES
                        );
                        for order in (0..2).rev() {
                            let starts = (0..4).step_by(chunk_size).collect::<Vec<_>>();
                            for start in starts.into_iter().rev() {
                                let len = chunk_size.min(4 - start);
                                let mut pairs = Vec::new();
                                for value in &edges[start..=start + len] {
                                    let (hi, lo) = split_f64_to_f32_pair(*value);
                                    pairs.push([hi, lo]);
                                }
                                for value in &values[start..start + len] {
                                    let (hi, lo) = split_f64_to_f32_pair(*value);
                                    pairs.push([hi, lo]);
                                }
                                let buffer =
                                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                        label: None,
                                        contents: bytemuck::cast_slice(&pairs),
                                        usage: wgpu::BufferUsages::STORAGE,
                                    });
                                let edges = ColumnHandle {
                                    generation: 0,
                                    offset: 0,
                                    byte_size: (len as u64 + 1) * 8,
                                    len_values: len + 1,
                                };
                                let values = ColumnHandle {
                                    generation: 0,
                                    offset: edges.byte_size,
                                    byte_size: len as u64 * 8,
                                    len_values: len,
                                };
                                let mut encoder =
                                    device.create_command_encoder(&Default::default());
                                if let Some(charge) = stream
                                    .encode_chunk(
                                        &mut encoder,
                                        &buffer,
                                        series(order, edges, values, len as u32),
                                        start as u32,
                                    )
                                    .unwrap()
                                {
                                    assert_eq!(
                                        charge.charged_bytes(),
                                        GpuStreamDataPick::CHUNK_BYTES
                                    );
                                    charges.push(charge);
                                }
                                queue.submit([encoder.finish()]);
                            }
                        }
                        let actual =
                            pollster::block_on(stream.finish().unwrap().resolve()).unwrap();
                        assert_eq!(
                            expected, actual,
                            "histogram stream size={chunk_size}, mapped={mapped}, logarithmic={logarithmic}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn stream_typed_reducer_keeps_point_and_paint_order_ties_on_gpu() {
        let Some((device, queue)) = data_render::shared_device() else {
            return;
        };
        let bundle = bundle(Arc::clone(&device), Arc::clone(&queue));
        let query = GpuDataPickQuery {
            transform: data_render::scatter_transform_from_config(&config(false)),
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: None,
            canvas_position_px: [50.0; 2],
            max_distance_px: 5.0,
        };
        for point_order in [0u32, 1, 2] {
            for point_first in [false, true] {
                let mut stream = bundle
                    .begin_stream(
                        query,
                        (0..3)
                            .map(|order| (None, format!("series-{order}")))
                            .collect(),
                    )
                    .unwrap();
                let point = [
                    1u32,
                    point_order,
                    42,
                    1,
                    41,
                    0.0f32.to_bits(),
                    0.0f32.to_bits(),
                    0,
                ];
                let data = DataPickCandidateGpu {
                    valid: 1,
                    paint_order: 1,
                    kind: 1,
                    index0: 17,
                    index1: 0,
                    index2: 0,
                    distance_px: 0.0,
                    primitive_order: 17,
                };
                let point = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&point),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::bytes_of(&data),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                for is_point in [point_first, !point_first] {
                    stream.accumulate(
                        &mut encoder,
                        if is_point { &point } else { &data },
                        is_point,
                    );
                }
                queue.submit([encoder.finish()]);
                let actual = pollster::block_on(stream.finish().unwrap().resolve())
                    .unwrap()
                    .unwrap();
                if point_order >= 1 {
                    assert!(matches!(
                        actual,
                        PickedData::Point {
                            point_index: 42,
                            ..
                        }
                    ));
                } else {
                    assert!(matches!(
                        actual,
                        PickedData::HistogramBin { bin_index: 17, .. }
                    ));
                }
                assert_eq!(actual.series_id(), format!("series-{}", point_order.max(1)));
            }
        }
    }
}
