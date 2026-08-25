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
    charged_buffer_init,
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
        Arc::new(Self {
            device,
            queue,
            ledger,
            query_bgl,
            histogram,
            field,
            empty_bar_style_map,
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
/// picking uses the render shaders' compute entries. Both submissions are made
/// before this ticket is returned, then the two scalar answers are ranked by
/// distance and original chart paint order during resolution.
pub struct GpuDataPickTicket {
    point: crate::gpu_pick::GpuPickTicket,
    data: DataPickTicket,
    point_orders: Arc<[PointPaintOrder]>,
}

impl GpuDataPickTicket {
    pub(crate) fn new(
        point: crate::gpu_pick::GpuPickTicket,
        data: DataPickTicket,
        point_orders: Vec<PointPaintOrder>,
    ) -> Self {
        Self {
            point,
            data,
            point_orders: point_orders.into(),
        }
    }

    pub async fn resolve(self) -> Result<Option<PickedData>, crate::gpu_pick::GpuPickError> {
        let point = self.point.resolve().await?;
        let data = self.data.resolve().await?;
        let point = point.map(|point| {
            let paint_order = self
                .point_orders
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
