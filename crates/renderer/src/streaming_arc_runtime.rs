//! Canonical scan leaves are independent of host IO tickets. Only one leaf's
//! original pairs and the bounded canonical sums tree live on the GPU.
use super::*;
use crate::data_render::line_arc::{ArcParams, ArcReplayChunk, ArcReplayLeaf, ArcReplayParams, WG};
use crate::gpu_memory::{
    ChargeTally, SharedCharge, charged_buffer, charged_buffer_init, shared_charge,
};

const LEAF_POINTS: u64 = WG as u64 + 1;
const Y_OFFSET: u64 = LEAF_POINTS * 8;

#[path = "streaming_arc_supply.rs"]
mod supply;
pub(super) fn needs_arc(style: &DrawStyle, series: &SeriesConfig) -> bool {
    style_variant(style).is_some_and(|variant| variant.needs_arc_prefix)
        || extract_line(&series.render_type)
            .is_some_and(|line| !matches!(line.line_style, LineStylePreset::Solid))
}

struct ArcBuffers {
    pool: wgpu::Buffer,
    arc: wgpu::Buffer,
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
    fn leaf(&self) -> Result<ArcReplayLeaf> {
        self.chunk
            .leaf(self.block)
            .ok_or(FiggyError::internal("checked canonical leaf"))
    }

    fn advance(&mut self, supplied: u32, max_workgroups: u32) -> Result<u64> {
        self.filled += supplied;
        let leaf = self.leaf()?;
        if self.filled != leaf.supply_len {
            return Ok(0);
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
                    .ok_or(FiggyError::internal("checked canonical successor"))?;
                    self.drawing = false;
                }
            }
        }
        Ok(drawn)
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
        // Every arc value is an arc pair (SHADER_COMMON.md §5.6, #107).
        let pair = data_render::line_arc::ARC_PAIR_BYTES;
        let top_bytes = u64::from(chunk.block_count()) * pair;
        let upper_bytes = u64::from(chunk.block_count().div_ceil(WG)) * pair;
        let sizes = [
            Y_OFFSET * 2,
            LEAF_POINTS * pair,
            pair,
            top_bytes,
            upper_bytes,
            pair,
            pair,
            pair,
            std::mem::size_of::<data_render::ScatterTransform>() as u64,
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
            > self.gpu_ledger.budget_limit()
        {
            return Err(StreamError::TooLarge.into());
        }
        if self.arc_pipelines.is_none() {
            // The renderer's one scan, shared with its resident charts: of
            // its shader set's variant (#299).
            self.arc_pipelines = Some(data_render::line_arc::create_arc_scan_pipelines_by_set(
                &self.device,
                self.pipelines.shaders.set,
                &mut |_| {},
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
        )?;
        let arc = buffer(
            "stream canonical arc leaf output",
            sizes[1],
            storage | copy | wgpu::BufferUsages::VERTEX,
        )?;
        let sums = buffer("stream canonical arc leaf sum", pair, storage)?;
        let top = buffer("stream canonical arc sums0", top_bytes, storage)?;
        let upper = buffer("stream canonical arc sums1", upper_bytes, storage)?;
        let sink = buffer("stream canonical arc sum sink", pair, storage)?;
        let carry = buffer("stream canonical arc carry", pair, storage)?;
        let predecessor = buffer("stream canonical arc predecessor", pair, copy)?;
        let transform = charged_buffer_init(
            &tally,
            &self.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream canonical arc transform"),
                contents: bytemuck::bytes_of(&data_render::scatter_transform_from_config(config)),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        )?;
        let transform = self
            .arc_pipelines
            .as_ref()
            .ok_or(FiggyError::internal("arc pipelines were created above"))?
            .replay_transform_bind_group(&self.device, &transform);
        if tally.bytes() != bytes {
            return Err(
                FiggyError::internal("stream arc buffers charge their planned bytes").into(),
            );
        }
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
        if live_runtime(&self.stream_runtime)?
            .live_draw(job)?
            .arc
            .is_none()
        {
            let arc = self.new_stream_arc(config, n)?;
            live_runtime_mut(&mut self.stream_runtime)?
                .live_draw_mut(job)?
                .arc = Some(arc);
        }
        let draw = live_runtime(&self.stream_runtime)?.live_draw(job)?;
        let arc = draw
            .arc
            .as_ref()
            .ok_or(FiggyError::internal("the stream arc was created above"))?;
        let leaf = arc.leaf()?;
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
                live_runtime_mut(&mut self.stream_runtime)?
                    .live_draw_mut(job)?
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
        let mut snapshot = self.auto_stream_snapshot(ticket.job);
        let view = snapshot
            .as_ref()
            .map(|s| &s.view)
            .or(explicit_view)
            .ok_or(StreamError::WrongState)?;
        let draw = live_runtime(&self.stream_runtime)?
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
        let leaf = arc.leaf()?;
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
        // Exact uniform bytes the completed leaf allocates below: its
        // `ArcParams` and `ArcReplayParams`, plus the top/upper `ArcParams`
        // pair on the last leaf of a non-drawing replay.
        let arc_params = std::mem::size_of::<ArcParams>() as u64;
        let replay_params = std::mem::size_of::<ArcReplayParams>() as u64;
        let headroom = if complete_leaf {
            if !drawing && last_leaf {
                arc_params * 3 + replay_params
            } else {
                arc_params + replay_params
            }
        } else {
            0
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stream canonical arc replay"),
            });
        let chunk = match self.accept_stream_supply_with_headroom(
            ticket,
            supply,
            &mut encoder,
            headroom,
            None,
        ) {
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
                drop(snapshot);
                if let Err(error) = self.update_auto_stream_fit(ticket.job, config, false) {
                    drop((encoder, chunk, buffers, auxiliary));
                    self.discard_stream_recording(ticket)?;
                    self.end_gpu_frame();
                    return Err(error);
                }
                snapshot = self.auto_stream_snapshot(ticket.job);
            }
            Err(error) => {
                drop((encoder, chunk, buffers));
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                return Err(error);
            }
            Ok(None) => {}
        }
        let view = snapshot
            .as_ref()
            .map(|snapshot| &snapshot.view)
            .or(explicit_view)
            .ok_or(StreamError::WrongState)?;
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
            let charge = self.record_stream_arc_leaf(
                &mut encoder,
                &buffers,
                leaf,
                drawing,
                last_leaf,
                canonical_chunk,
                headroom,
                save_carry,
            )?;
            if drawing {
                self.draw_stream_arc_prefix(
                    &mut encoder,
                    &buffers,
                    leaf,
                    &snapshot,
                    series_index,
                    ticket,
                    view,
                    target,
                    &auxiliary,
                )?;
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
                let draw = live_runtime_mut(&mut self.stream_runtime)?
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == ticket.job)
                    .ok_or(FiggyError::internal("a submitted ticket's cursor is live"))?;
                let drawn = draw
                    .arc
                    .as_mut()
                    .ok_or(FiggyError::internal("a submitted arc ticket keeps its arc"))?
                    .advance(
                        supplied,
                        self.device.limits().max_compute_workgroups_per_dimension,
                    )?;
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
