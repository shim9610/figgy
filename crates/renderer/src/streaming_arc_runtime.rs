//! Canonical scan leaves are independent of host IO tickets. Only one leaf's
//! original pairs and the bounded canonical sums tree live on the GPU.
use super::*;
use crate::data_render::line_arc::{ArcParams, ArcReplayChunk, ArcReplayLeaf, WG};
use crate::gpu_memory::{
    ChargeTally, SharedCharge, charged_buffer, charged_buffer_init, shared_charge,
};

const LEAF_POINTS: u64 = WG as u64 + 1;
const Y_OFFSET: u64 = LEAF_POINTS * 8;

pub(super) fn needs_arc(style: &DrawStyle, series: &SeriesConfig) -> bool {
    style_variant(style).is_some_and(|variant| variant.needs_arc_prefix)
        || extract_line(&series.render_type)
            .is_some_and(|line| !matches!(line.line_style, LineStylePreset::Solid))
}

struct ArcBuffers {
    pool: wgpu::Buffer,
    arc: Arc<wgpu::Buffer>,
    sums: wgpu::Buffer,
    top: wgpu::Buffer,
    upper: wgpu::Buffer,
    sink: wgpu::Buffer,
    carry: wgpu::Buffer,
    predecessor: wgpu::Buffer,
    transform: wgpu::BindGroup,
    charge: SharedCharge,
}

pub(super) struct StreamArcState {
    buffers: Arc<ArcBuffers>,
    n: u32,
    cap: u32,
    chunk: ArcReplayChunk,
    block: u32,
    filled: u32,
    drawing: bool,
}

impl StreamArcState {
    fn leaf(&self) -> ArcReplayLeaf {
        self.chunk.leaf(self.block).expect("checked canonical leaf")
    }

    fn advance(&mut self, supplied: u32, max_workgroups: u32) -> u64 {
        self.filled += supplied;
        let leaf = self.leaf();
        if self.filled != leaf.supply_len {
            return 0;
        }
        self.filled = 0;
        let drawn = if self.drawing {
            u64::from(leaf.supply_len - 1)
        } else {
            0
        };
        self.block += 1;
        if self.block == self.chunk.block_count() {
            self.block = 0;
            if !self.drawing {
                self.drawing = true;
            } else {
                let start = self.chunk.start + self.chunk.len;
                if start < self.n {
                    self.chunk = ArcReplayChunk::checked(
                        start,
                        (self.n - start).min(self.cap),
                        max_workgroups,
                    )
                    .expect("checked canonical successor");
                    self.drawing = false;
                }
            }
        }
        drawn
    }
}

impl Renderer {
    fn new_stream_arc(&mut self, config: &Config, n: u64) -> StreamResult<StreamArcState> {
        let n = u32::try_from(n).map_err(|_| StreamError::TooLarge)?;
        let max_workgroups = self.device.limits().max_compute_workgroups_per_dimension;
        let cap = data_render::line_arc::chunk_capacity(max_workgroups) as u32;
        #[cfg(test)]
        let cap = self.arc_chunk_override.map_or(cap, |limit| limit.min(cap));
        let chunk =
            ArcReplayChunk::checked(0, n.min(cap), max_workgroups).ok_or(StreamError::TooLarge)?;
        let top_bytes = u64::from(chunk.block_count()) * 4;
        let upper_bytes = u64::from(chunk.block_count().div_ceil(WG)) * 4;
        let sizes = [
            Y_OFFSET * 2,
            LEAF_POINTS * 4,
            4,
            top_bytes,
            upper_bytes,
            4,
            4,
            4,
            112,
        ];
        let bytes = sizes.iter().sum::<u64>();
        let limits = self.device.limits();
        if sizes.iter().any(|size| {
            *size > limits.max_buffer_size
                || *size > u64::from(limits.max_storage_buffer_binding_size)
        }) || self
            .gpu_memory_usage()
            .total_bytes()
            .checked_add(bytes)
            .ok_or(StreamError::Overflow)?
            > self.memory_budget.unwrap_or(u64::MAX)
        {
            return Err(StreamError::TooLarge.into());
        }
        if self.arc_pipelines.is_none() {
            self.arc_pipelines = Some(data_render::line_arc::create_arc_scan_pipelines(
                &self.device,
            ));
        }
        let tally = ChargeTally::new();
        let buffer = |label, size, usage| {
            charged_buffer(
                &tally,
                &self.device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                },
            )
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let copy = wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let pool = buffer(
            "stream canonical arc leaf pairs",
            sizes[0],
            storage | copy | wgpu::BufferUsages::VERTEX,
        );
        let arc = Arc::new(buffer(
            "stream canonical arc leaf output",
            sizes[1],
            storage | copy | wgpu::BufferUsages::VERTEX,
        ));
        let sums = buffer("stream canonical arc leaf sum", 4, storage);
        let top = buffer("stream canonical arc sums0", top_bytes, storage);
        let upper = buffer("stream canonical arc sums1", upper_bytes, storage);
        let sink = buffer("stream canonical arc sum sink", 4, storage);
        let carry = buffer("stream canonical arc carry", 4, storage);
        let predecessor = buffer("stream canonical arc predecessor", 4, copy);
        let transform = charged_buffer_init(
            &tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream canonical arc transform"),
                contents: bytemuck::bytes_of(&data_render::scatter_transform_from_config(config)),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let transform = self
            .arc_pipelines
            .as_ref()
            .unwrap()
            .replay_transform_bind_group(&self.device, &transform);
        debug_assert_eq!(tally.bytes(), bytes);
        let charge = shared_charge(tally, &self.gpu_ledger, GpuResourceKind::ArcScan);
        Ok(StreamArcState {
            buffers: Arc::new(ArcBuffers {
                pool,
                arc,
                sums,
                top,
                upper,
                sink,
                carry,
                predecessor,
                transform,
                charge,
            }),
            n,
            cap,
            chunk,
            block: 0,
            filled: 0,
            drawing: false,
        })
    }

    pub(super) fn request_stream_arc(
        &mut self,
        job: StreamJob,
        config: &Config,
        names: &[String; 2],
        n: u64,
    ) -> StreamResult<StreamDrawRequestStatus> {
        if self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap()
            .arc
            .is_none()
        {
            let arc = self.new_stream_arc(config, n)?;
            self.stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .arc = Some(arc);
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let arc = draw.arc.as_ref().unwrap();
        let leaf = arc.leaf();
        let offset = u64::from(leaf.supply_start + arc.filled);
        let len = u64::from(leaf.supply_len - arc.filled).min(draw.max_primitives);
        let ranges = names.each_ref().map(|column| StreamSourceRange {
            column,
            offset,
            len,
        });
        let count = if names[0] == names[1] { 1 } else { 2 };
        match self.request_stream_columns(job, &ranges[..count])? {
            StreamRequestStatus::Backpressure => Ok(StreamDrawRequestStatus::Backpressure),
            StreamRequestStatus::Ready(ticket) => {
                self.stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap()
                    .pending = Some(ticket);
                Ok(StreamDrawRequestStatus::Ready(ticket))
            }
        }
    }

    pub(super) fn submit_stream_arc_supply(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        explicit_view: Option<&ChartView>,
        target: &wgpu::Texture,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        let snapshot = self.auto_stream_snapshot(ticket.job);
        let view = snapshot
            .as_ref()
            .map(|s| &s.view)
            .or(explicit_view)
            .ok_or(StreamError::WrongState)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == ticket.job)
            .ok_or(StreamError::WrongState)?;
        if draw.pending != Some(ticket)
            || !Arc::ptr_eq(&view.stream_revision, &draw.view_revision)
            || view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
            || target != &draw.target
        {
            return Err(StreamError::Stale.into());
        }
        let auxiliary = draw.auxiliary.clone();
        let series_index = draw.series;
        let arc = draw.arc.as_ref().ok_or(StreamError::WrongState)?;
        let buffers = Arc::clone(&arc.buffers);
        let leaf = arc.leaf();
        let canonical_chunk = arc.chunk;
        let filled = arc.filled;
        let drawing = arc.drawing;
        let last_leaf = arc.block + 1 == canonical_chunk.block_count();
        let save_carry = last_leaf && canonical_chunk.start + canonical_chunk.len < arc.n;
        self.validate_stream_supply_kind(ticket, supply)?;
        let requested = self.stream_request_columns(ticket)?;
        let mut columns = [requested[0].range; 2];
        if requested.len() == 2 {
            columns[1] = requested[1].range;
        }
        let supplied = u32::try_from(columns[0].len).map_err(|_| StreamError::TooLarge)?;
        if columns.iter().any(|column| {
            column.offset != u64::from(leaf.supply_start + filled)
                || column.len != u64::from(supplied)
        }) || supplied == 0
            || filled + supplied > leaf.supply_len
        {
            return Err(StreamError::InvalidRange.into());
        }
        let complete_leaf = filled + supplied == leaf.supply_len;
        let headroom = if complete_leaf {
            if !drawing && last_leaf { 64 } else { 32 }
        } else {
            0
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stream canonical arc replay"),
            });
        let chunk =
            match self.accept_stream_supply_with_headroom(ticket, supply, &mut encoder, headroom) {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop((encoder, buffers));
                    self.end_gpu_frame();
                    return Err(error);
                }
            };
        let pending_fit = self.pending_auto_fit_config(ticket.job);
        match pending_fit {
            Ok(Some(config)) => {
                drop((encoder, chunk, buffers, auxiliary, snapshot));
                self.discard_stream_recording(ticket)?;
                let result = self.restart_auto_stream_with_config(ticket.job, config);
                self.end_gpu_frame();
                return result;
            }
            Err(error) => {
                drop((encoder, chunk, buffers));
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                return Err(error);
            }
            Ok(None) => {}
        }
        let encoded = (|| -> StreamResult<Option<SharedCharge>> {
            for (index, column) in columns.iter().enumerate() {
                let handle = chunk.column_handle(*column)?;
                encoder.copy_buffer_to_buffer(
                    &chunk.work,
                    handle.offset,
                    &buffers.pool,
                    index as u64 * Y_OFFSET + u64::from(filled) * 8,
                    u64::from(supplied) * 8,
                );
            }
            if !complete_leaf {
                return Ok(None);
            }
            let tally = ChargeTally::new();
            let params = |label, contents: &[u8]| {
                charged_buffer_init(
                    &tally,
                    &self.device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents,
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                )
            };
            let leaf_params = params(
                "stream arc leaf params",
                bytemuck::bytes_of(&leaf.arc_params(0, (Y_OFFSET / 4) as u32)),
            );
            let replay_params = params(
                "stream arc leaf identity",
                bytemuck::bytes_of(&leaf.replay_params()),
            );
            let top_params = (!drawing && last_leaf).then(|| {
                params(
                    "stream arc top params",
                    bytemuck::bytes_of(&ArcParams {
                        len: canonical_chunk.block_count(),
                        x_base: 0,
                        y_base: 0,
                        start: 0,
                    }),
                )
            });
            let upper_params = (!drawing && last_leaf).then(|| {
                params(
                    "stream arc upper params",
                    bytemuck::bytes_of(&ArcParams {
                        len: canonical_chunk.block_count().div_ceil(WG),
                        x_base: 0,
                        y_base: 0,
                        start: 0,
                    }),
                )
            });
            debug_assert_eq!(tally.bytes(), headroom);
            let charge = shared_charge(tally, &self.gpu_ledger, GpuResourceKind::ArcScan);
            let pipelines = self.arc_pipelines.as_ref().unwrap();
            let storage = pipelines.replay_storage_bind_group(
                &self.device,
                &buffers.pool,
                &buffers.arc,
                &buffers.sums,
                &leaf_params,
                &buffers.carry,
            );
            let replay =
                pipelines.replay_control_bind_group(&self.device, &buffers.top, &replay_params);
            if drawing && leaf.local_start != 0 {
                encoder.copy_buffer_to_buffer(&buffers.predecessor, 0, &buffers.arc, 0, 4);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stream canonical arc leaf"),
                    timestamp_writes: None,
                });
                if drawing {
                    pipelines.record_replay_leaf_output(
                        &mut pass,
                        &buffers.transform,
                        &storage,
                        &replay,
                        leaf,
                        canonical_chunk.start != 0,
                        save_carry,
                    );
                } else {
                    pipelines.record_replay_leaf_total(
                        &mut pass,
                        &buffers.transform,
                        &storage,
                        &replay,
                    );
                }
            }
            if !drawing && last_leaf {
                let top = pipelines.replay_storage_bind_group(
                    &self.device,
                    &buffers.pool,
                    &buffers.top,
                    &buffers.upper,
                    top_params.as_ref().unwrap(),
                    &buffers.carry,
                );
                let upper = pipelines.replay_storage_bind_group(
                    &self.device,
                    &buffers.pool,
                    &buffers.upper,
                    &buffers.sink,
                    upper_params.as_ref().unwrap(),
                    &buffers.carry,
                );
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("stream canonical arc upper tree"),
                    timestamp_writes: None,
                });
                pipelines.record_replay_top(
                    &mut pass,
                    &buffers.transform,
                    &top,
                    &upper,
                    canonical_chunk,
                );
            }
            if drawing {
                encoder.copy_buffer_to_buffer(
                    &buffers.arc,
                    u64::from(leaf.supply_len - 1) * 4,
                    &buffers.predecessor,
                    0,
                    4,
                );
                let (states, preparation) = self.preparation_parts();
                let (config, series, style) = snapshot.as_ref().map_or_else(
                    || {
                        let state = &states[&ticket.job.chart];
                        (
                            &state.config,
                            &state.series[series_index],
                            &state.prepared_styles.as_ref().unwrap().styles[series_index],
                        )
                    },
                    |snapshot| {
                        (
                            &snapshot.config,
                            &snapshot.series[series_index],
                            &snapshot.styles.styles[series_index],
                        )
                    },
                );
                let handle = |offset| ColumnHandle {
                    generation: 0,
                    offset,
                    byte_size: u64::from(leaf.supply_len) * 8,
                    len_values: leaf.supply_len as usize,
                };
                let packet = preparation.build_stream_arc_series(
                    view,
                    config,
                    &Series {
                        config: series,
                        style,
                    },
                    &buffers.pool,
                    handle(0),
                    handle(Y_OFFSET),
                    (Arc::clone(&buffers.arc), u64::from(leaf.supply_len) * 4),
                    Arc::clone(&buffers.charge),
                    auxiliary
                        .as_ref()
                        .and_then(|a| a.pipelines.as_ref())
                        .unwrap_or(preparation.pipelines),
                )?;
                let size = (target.width(), target.height());
                let panel = data_render::clamp_rect_to_target(view.panel_rect, size);
                let data = data_render::clamp_rect_to_target(
                    config.data_area().map_or(view.panel_rect, |area| area.0),
                    size,
                );
                if let (Some(panel), Some(data)) = (panel, data) {
                    let target_view = target.create_view(&wgpu::TextureViewDescriptor {
                        mip_level_count: Some(1),
                        ..Default::default()
                    });
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("stream canonical line prefix"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_viewport(
                        panel.x as f32,
                        panel.y as f32,
                        panel.width as f32,
                        panel.height as f32,
                        0.0,
                        1.0,
                    );
                    pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                    data_render::issue_series_data(&mut pass, &packet.layers());
                }
            }
            Ok(Some(charge))
        })();
        let scratch = match encoded {
            Ok(scratch) => scratch,
            Err(error) => {
                drop((encoder, chunk, buffers));
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop((scratch, chunk, buffers));
        match result {
            Ok(submission) => {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == ticket.job)
                    .unwrap();
                let drawn = draw.arc.as_mut().unwrap().advance(
                    supplied,
                    self.device.limits().max_compute_workgroups_per_dimension,
                );
                draw.offset += drawn;
                draw.pending = None;
                draw.display_dirty |= drawn != 0;
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "streaming_arc_runtime_tests.rs"]
mod tests;
