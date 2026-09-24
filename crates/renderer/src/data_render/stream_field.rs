//! Bounded GPU-only Heatmap continuation. Source identity/revision, queue
//! submission and target epochs remain the streaming runtime's responsibility.
//! A tile accepts exactly one recorded step at a time. Commit only after its
//! commands were submitted in order; discard only with the unsubmitted encoder.

use super::{
    ContourLookupMetadataGpu, FieldLookupTables, FieldParamsGpu, PrimitiveStyle, ScatterTransform,
};
use crate::gpu_memory::{GpuLedger, GpuResourceKind, SharedCharge, TrackedBuffer};
use crate::streaming::StreamError;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub(crate) const PIXEL_BYTES: u64 = 136;
pub(crate) const STEP_BYTES: u64 = 32;
const TILE_BYTES: u64 = 32;

#[derive(Clone, Copy)]
pub(crate) struct Budget {
    pub max_renderer_bytes: u64,
    pub pool_bytes: u64,
    /// Other simultaneously required allocations, excluding existing ledger
    /// owners and this call's own allocations. Recompute at each call.
    pub headroom_bytes: u64,
}

pub(crate) struct Pipelines {
    init: wgpu::RenderPipeline,
    axis: wgpu::ComputePipeline,
    z: wgpu::ComputePipeline,
    finish: wgpu::RenderPipeline,
    pick_init: wgpu::ComputePipeline,
    pick_final: wgpu::ComputePipeline,
    samples: u32,
}

impl Pipelines {
    pub(crate) fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Self, StreamError> {
        if !matches!(samples, 1 | 4) {
            return Err(StreamError::InvalidLimits);
        }
        let render = |entry, init| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: super::multisample_state(samples),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: if init {
                            wgpu::ColorWrites::empty()
                        } else {
                            wgpu::ColorWrites::ALL
                        },
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let compute = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Ok(Self {
            init: render("fs_stream_field_init", true),
            axis: compute("cs_stream_field_axis"),
            z: compute("cs_stream_field_z"),
            finish: render("fs_stream_field_final", false),
            pick_init: compute("cs_stream_field_pick_init"),
            pick_final: compute("cs_stream_field_pick_final"),
            samples,
        })
    }
}

/// Immutable, already-resolved draw metadata, never a second Config/Series or
/// column registry. Shared by independent tiles of this field/target revision.
pub(crate) struct Resources {
    pipelines: Arc<Pipelines>,
    ledger: Arc<GpuLedger>,
    params: FieldParamsGpu,
    _buffers: [TrackedBuffer; 5],
    init_field: wgpu::BindGroup,
    axis_transform: wgpu::BindGroup,
    axis_field: wgpu::BindGroup,
    z_field: wgpu::BindGroup,
    final_style: wgpu::BindGroup,
    final_field: wgpu::BindGroup,
}

impl Resources {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        device: &wgpu::Device,
        pipelines: Arc<Pipelines>,
        ledger: Arc<GpuLedger>,
        budget: Budget,
        transform: &ScatterTransform,
        style: &PrimitiveStyle,
        params: &FieldParamsGpu,
        tables: &FieldLookupTables,
    ) -> Result<Arc<Self>, StreamError> {
        let centers = params.flags & super::FIELD_FLAG_CENTERS != 0;
        let interpolated = params.flags & super::FIELD_FLAG_INTERPOLATED != 0;
        let cy = params.flags & super::FIELD_FLAG_COLUMNS_ARE_Y != 0;
        let cells = if cy {
            [params.rows, params.cols]
        } else {
            [params.cols, params.rows]
        };
        let min_cells = if interpolated { 2 } else { 1 };
        let axis_lengths = [params.x_len, params.y_len];
        let required_stops = usize::try_from(params.stop_count.max(1))
            .map_err(|_| StreamError::Overflow)?
            .checked_add(params.level_count as usize)
            .ok_or(StreamError::Overflow)?;
        if cells.into_iter().any(|n| n < min_cells)
            || axis_lengths
                .into_iter()
                .zip(cells)
                .any(|(n, cells)| n.saturating_sub(u32::from(!centers)) < cells)
            || params.level_count as usize > super::CONTOUR_LEVEL_LOOKUP_CAPACITY
            || tables.stops.len() < required_stops
            || tables.metadata.len() < (params.level_count as usize).div_ceil(32).max(1)
        {
            return Err(StreamError::InvalidRange);
        }
        let payloads = [
            bytemuck::bytes_of(transform),
            bytemuck::bytes_of(style),
            bytemuck::bytes_of(params),
            bytemuck::cast_slice(tables.stops.as_slice()),
            bytemuck::cast_slice(tables.metadata.as_slice()),
        ];
        let mut total = 0u64;
        let limits = device.limits();
        for (i, bytes) in payloads.iter().enumerate() {
            let len = bytes.len() as u64;
            let limit = if i < 3 {
                u64::from(limits.max_uniform_buffer_binding_size)
            } else {
                u64::from(limits.max_storage_buffer_binding_size)
            };
            if len == 0 || len > limit || len > limits.max_buffer_size {
                return Err(StreamError::TooLarge);
            }
            total = total.checked_add(len).ok_or(StreamError::Overflow)?;
        }
        check_budget(&ledger, budget, total)?;
        let alloc = |i| {
            allocate_init(
                device,
                &ledger,
                payloads[i],
                if i < 3 {
                    wgpu::BufferUsages::UNIFORM
                } else {
                    wgpu::BufferUsages::STORAGE
                },
            )
        };
        let buffers = [alloc(0)?, alloc(1)?, alloc(2)?, alloc(3)?, alloc(4)?];
        let init_field = bind(
            device,
            &pipelines.init.get_bind_group_layout(2),
            &[(4, &buffers[2])],
        );
        let axis_transform = bind(
            device,
            &pipelines.axis.get_bind_group_layout(0),
            &[(0, &buffers[0])],
        );
        let axis_field = bind(
            device,
            &pipelines.axis.get_bind_group_layout(2),
            &[(4, &buffers[2])],
        );
        let z_field = bind(
            device,
            &pipelines.z.get_bind_group_layout(2),
            &[(4, &buffers[2])],
        );
        let final_style = bind(
            device,
            &pipelines.finish.get_bind_group_layout(1),
            &[(0, &buffers[1])],
        );
        let final_field = bind(
            device,
            &pipelines.finish.get_bind_group_layout(2),
            &[(3, &buffers[3]), (4, &buffers[2]), (6, &buffers[4])],
        );
        Ok(Arc::new(Self {
            pipelines,
            ledger,
            params: *params,
            _buffers: buffers,
            init_field,
            axis_transform,
            axis_field,
            z_field,
            final_style,
            final_field,
        }))
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self._buffers.iter().map(TrackedBuffer::charged_bytes).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TileShape {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

impl TileShape {
    /// `max_bytes` bounds state + immutable tile and one step uniform. Source
    /// staging/work, field metadata and P/D targets are separate existing owners.
    pub(crate) fn new(
        limits: &wgpu::Limits,
        panel: [u32; 2],
        samples: u32,
        max_bytes: u64,
    ) -> Result<Self, StreamError> {
        if panel.contains(&0) || !matches!(samples, 1 | 4) {
            return Err(StreamError::InvalidLimits);
        }
        let available = max_bytes
            .checked_sub(TILE_BYTES + STEP_BYTES)
            .ok_or(StreamError::TooLarge)?
            .min(limits.max_buffer_size)
            .min(u64::from(limits.max_storage_buffer_binding_size));
        let pixels = available / PIXEL_BYTES / u64::from(samples);
        if pixels == 0 {
            return Err(StreamError::TooLarge);
        }
        let dispatch_limit = u64::from(limits.max_compute_workgroups_per_dimension) * 8;
        let width = u64::from(panel[0]).min(pixels).min(dispatch_limit) as u32;
        if width == 0 || samples > limits.max_compute_workgroups_per_dimension {
            return Err(StreamError::TooLarge);
        }
        let height = u64::from(panel[1])
            .min(pixels / u64::from(width))
            .min(dispatch_limit) as u32;
        let shape = Self {
            width,
            height,
            samples,
        };
        shape.state_bytes()?;
        Ok(shape)
    }

    pub(crate) fn state_bytes(self) -> Result<u64, StreamError> {
        let entries = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|n| n.checked_mul(u64::from(self.samples)))
            .ok_or(StreamError::Overflow)?;
        if entries == 0 || entries > u64::from(u32::MAX) {
            return Err(StreamError::TooLarge);
        }
        entries
            .checked_mul(PIXEL_BYTES)
            .ok_or(StreamError::Overflow)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceRole {
    Axis(u32),
    ZColumn(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceRequest {
    pub role: SourceRole,
    pub start: u32,
    pub len: u32,
    pub sweep: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Initialize,
    Source(SourceRequest),
    Finalize,
    Complete,
}

struct TileGpu {
    resources: Arc<Resources>,
    state: TrackedBuffer,
    meta: TrackedBuffer,
    init: wgpu::BindGroup,
    finish: wgpu::BindGroup,
    viewport: [f32; 4],
    rect: [u32; 4],
}

pub(crate) struct Tile {
    gpu: Arc<TileGpu>,
    action: Action,
    max_pairs: u32,
    pending: bool,
}

pub(crate) struct PickTile {
    tile: Tile,
    init_query: wgpu::BindGroup,
    init_field: wgpu::BindGroup,
    init_state: wgpu::BindGroup,
    final_query: wgpu::BindGroup,
    final_state: wgpu::BindGroup,
    _query: Arc<crate::gpu_data_pick::GpuStreamFieldQuery>,
}

impl PickTile {
    pub(crate) fn new(device: &wgpu::Device, resources: Arc<Resources>, budget: Budget, max_pairs: u32,
        query: crate::gpu_data_pick::GpuStreamFieldQuery) -> Result<Self, StreamError> {
        let tile = Tile::new(device, Arc::clone(&resources), budget, TileShape { width: 1, height: 1, samples: 1 },
            [0.0, 0.0, 1.0, 1.0], [0, 0, 1, 1], [0, 0, 1, 1], max_pairs)?;
        let pipelines = &resources.pipelines;
        let init_query = bind(device, &pipelines.pick_init.get_bind_group_layout(1), &[(1, &query.query)]);
        let init_field = bind(device, &pipelines.pick_init.get_bind_group_layout(2), &[(4, &resources._buffers[2])]);
        let init_state = bind(device, &pipelines.pick_init.get_bind_group_layout(3), &[(0, &tile.gpu.state)]);
        let final_query = bind(device, &pipelines.pick_final.get_bind_group_layout(1), &[(1, &query.query), (3, &query.candidate)]);
        let final_state = bind(device, &pipelines.pick_final.get_bind_group_layout(3), &[(0, &tile.gpu.state)]);
        Ok(Self { tile, init_query, init_field, init_state, final_query, final_state, _query: Arc::new(query) })
    }

    pub(crate) fn initialize(&mut self, encoder: &mut wgpu::CommandEncoder) -> Result<RecordedStep, StreamError> {
        if self.tile.action()? != Action::Initialize { return Err(StreamError::WrongState); }
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.tile.gpu.resources.pipelines.pick_init);
            pass.set_bind_group(1, &self.init_query, &[]);
            pass.set_bind_group(2, &self.init_field, &[]);
            pass.set_bind_group(3, &self.init_state, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.tile.pending = true;
        let mut step = self.tile.token(None, None, None);
        step._pick_query = Some(Arc::clone(&self._query));
        Ok(step)
    }

    pub(crate) fn action(&self) -> Result<Action, StreamError> { self.tile.action() }
    pub(crate) fn commit(&mut self, step: RecordedStep) -> Result<(), StreamError> { self.tile.commit(step) }
    pub(crate) fn discard(&mut self, step: RecordedStep) -> Result<(), StreamError> { self.tile.discard(step) }

    pub(crate) fn record_source(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, budget: Budget,
        request: SourceRequest, work: &wgpu::Buffer, base: u32, charge: SharedCharge) -> Result<(RecordedStep, bool), StreamError> {
        if !matches!(request.role, SourceRole::Axis(_)) { return Err(StreamError::WrongState); }
        let mut step = self.tile.record_source(device, encoder, budget, request, work, base, charge)?;
        step._pick_query = Some(Arc::clone(&self._query));
        let complete = matches!(self.tile.following_action(), Action::Source(SourceRequest { role: SourceRole::ZColumn(0), start: 0, .. }));
        if complete {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.tile.gpu.resources.pipelines.pick_final);
            pass.set_bind_group(1, &self.final_query, &[]);
            pass.set_bind_group(3, &self.final_state, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            step.completes_tile = true;
        }
        Ok((step, complete))
    }
}

/// Holding this token retains every buffer's accounting owner through command
/// recording. It must be committed after ordered submission or discarded with
/// those commands. Dropping it alone leaves the tile blocked (fail closed).
#[must_use = "commit after ordered submission, or discard with the unsubmitted commands"]
pub(crate) struct RecordedStep {
    owner: Arc<TileGpu>,
    action: Action,
    completes_tile: bool,
    _pick_query: Option<Arc<crate::gpu_data_pick::GpuStreamFieldQuery>>,
    _ticket: Option<TrackedBuffer>,
    _source_charge: Option<SharedCharge>,
    _binding: Option<wgpu::BindGroup>,
}

impl Tile {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        device: &wgpu::Device,
        resources: Arc<Resources>,
        budget: Budget,
        shape: TileShape,
        viewport: [f32; 4],
        panel_scissor: [u32; 4],
        rect: [u32; 4],
        max_pairs: u32,
    ) -> Result<Self, StreamError> {
        if shape.samples != resources.pipelines.samples
            || max_pairs < 2
            || viewport.into_iter().any(|n| !n.is_finite())
            || viewport[2] <= 0.0
            || viewport[3] <= 0.0
            || rect[2] == 0
            || rect[3] == 0
            || rect[2] > shape.width
            || rect[3] > shape.height
        {
            return Err(StreamError::InvalidRange);
        }
        for axis in 0..2 {
            let end = rect[axis]
                .checked_add(rect[axis + 2])
                .ok_or(StreamError::Overflow)?;
            let panel_end = panel_scissor[axis]
                .checked_add(panel_scissor[axis + 2])
                .ok_or(StreamError::Overflow)?;
            if rect[axis] < panel_scissor[axis] || end > panel_end {
                return Err(StreamError::InvalidRange);
            }
        }
        let shape = TileShape {
            width: rect[2],
            height: rect[3],
            samples: shape.samples,
        };
        let bytes = shape.state_bytes()?;
        let limits = device.limits();
        if bytes > limits.max_buffer_size
            || bytes > u64::from(limits.max_storage_buffer_binding_size)
            || rect[2].div_ceil(8) > limits.max_compute_workgroups_per_dimension
            || rect[3].div_ceil(8) > limits.max_compute_workgroups_per_dimension
        {
            return Err(StreamError::TooLarge);
        }
        check_budget(
            &resources.ledger,
            budget,
            bytes
                .checked_add(TILE_BYTES + STEP_BYTES)
                .ok_or(StreamError::Overflow)?,
        )?;
        let state = allocate(
            device,
            &resources.ledger,
            bytes,
            wgpu::BufferUsages::STORAGE,
        )?;
        let meta = allocate_init(
            device,
            &resources.ledger,
            bytemuck::cast_slice(&[rect[0], rect[1], rect[2], rect[3], shape.samples, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        )?;
        let init = bind(
            device,
            &resources.pipelines.init.get_bind_group_layout(3),
            &[(0, &state), (3, &meta)],
        );
        let finish = bind(
            device,
            &resources.pipelines.finish.get_bind_group_layout(3),
            &[(0, &state), (3, &meta)],
        );
        Ok(Self {
            gpu: Arc::new(TileGpu {
                resources,
                state,
                meta,
                init,
                finish,
                viewport,
                rect,
            }),
            action: Action::Initialize,
            max_pairs,
            pending: false,
        })
    }

    pub(crate) fn action(&self) -> Result<Action, StreamError> {
        if self.pending {
            Err(StreamError::WrongState)
        } else {
            Ok(self.action)
        }
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self.gpu.state.charged_bytes() + self.gpu.meta.charged_bytes()
    }

    pub(crate) fn record_init(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        initialized_target: &wgpu::TextureView,
    ) -> Result<RecordedStep, StreamError> {
        if self.action()? != Action::Initialize {
            return Err(StreamError::WrongState);
        }
        self.record_draw(encoder, initialized_target, true);
        self.pending = true;
        Ok(self.token(None, None, None))
    }

    /// The runtime validates the source revision and ticket before this call.
    /// `pair_base` addresses its already-uploaded work buffer, not a source
    /// index. `request.role` preserves the matrix declaration ordinal even when
    /// multiple ordinals refer to the same registered source ID.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_source(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        budget: Budget,
        request: SourceRequest,
        work: &wgpu::Buffer,
        pair_base: u32,
        source_charge: SharedCharge,
    ) -> Result<RecordedStep, StreamError> {
        if self.action()? != Action::Source(request) {
            return Err(StreamError::Stale);
        }
        let end = u64::from(pair_base)
            .checked_add(u64::from(request.len))
            .and_then(|n| n.checked_mul(8))
            .ok_or(StreamError::Overflow)?;
        if !work.usage().contains(wgpu::BufferUsages::STORAGE)
            || end > work.size()
            || work.size() > u64::from(device.limits().max_storage_buffer_binding_size)
            || source_charge.charged_bytes() < work.size()
        {
            return Err(StreamError::InvalidPayload);
        }
        check_budget(&self.gpu.resources.ledger, budget, STEP_BYTES)?;
        let params = &self.gpu.resources.params;
        let (pipe, axis, column, n, count) = match request.role {
            SourceRole::Axis(axis) => (
                &self.gpu.resources.pipelines.axis,
                axis,
                0,
                self.axis_len(axis),
                self.axis_count(axis),
            ),
            SourceRole::ZColumn(column) => {
                (&self.gpu.resources.pipelines.z, 0, column, params.rows, 0)
            }
        };
        let ticket = allocate_init(
            device,
            &self.gpu.resources.ledger,
            bytemuck::cast_slice(&[
                request.start,
                request.len,
                n,
                count,
                axis,
                column,
                pair_base,
                0,
            ]),
            wgpu::BufferUsages::UNIFORM,
        )?;
        let binding = bind(
            device,
            &pipe.get_bind_group_layout(3),
            &[
                (0, &self.gpu.state),
                (1, work),
                (2, &ticket),
                (3, &self.gpu.meta),
            ],
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("bounded field source replay"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipe);
            match request.role {
                SourceRole::Axis(_) => {
                    pass.set_bind_group(0, &self.gpu.resources.axis_transform, &[]);
                    pass.set_bind_group(2, &self.gpu.resources.axis_field, &[]);
                }
                SourceRole::ZColumn(_) => pass.set_bind_group(2, &self.gpu.resources.z_field, &[]),
            }
            pass.set_bind_group(3, &binding, &[]);
            pass.dispatch_workgroups(
                self.gpu.rect[2].div_ceil(8),
                self.gpu.rect[3].div_ceil(8),
                self.gpu.resources.pipelines.samples,
            );
        }
        self.pending = true;
        Ok(self.token(Some(ticket), Some(source_charge), Some(binding)))
    }

    /// No provisional tile is blended into P. This becomes legal only after
    /// all axis sweeps and every effective Z column/range were committed.
    pub(crate) fn record_final(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        initialized_target: &wgpu::TextureView,
    ) -> Result<RecordedStep, StreamError> {
        if self.action()? != Action::Finalize {
            return Err(StreamError::WrongState);
        }
        self.record_draw(encoder, initialized_target, false);
        self.pending = true;
        Ok(self.token(None, None, None))
    }

    pub(crate) fn commit(&mut self, step: RecordedStep) -> Result<(), StreamError> {
        self.validate_step(&step)?;
        self.action = if step.completes_tile { Action::Complete } else { self.following_action() };
        self.pending = false;
        Ok(())
    }

    /// Keep the last source compute and its final blend in the same scheduler
    /// receipt, so upload completion also proves completion of the visible tile.
    pub(crate) fn record_final_after_source(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        step: &mut RecordedStep,
    ) -> Result<bool, StreamError> {
        self.validate_step(step)?;
        if !matches!(step.action, Action::Source(_)) || step.completes_tile {
            return Err(StreamError::WrongState);
        }
        if self.following_action() != Action::Finalize {
            return Ok(false);
        }
        self.record_draw(encoder, target, false);
        step.completes_tile = true;
        Ok(true)
    }

    pub(crate) fn discard(&mut self, step: RecordedStep) -> Result<(), StreamError> {
        self.validate_step(&step)?;
        self.pending = false;
        Ok(())
    }

    fn validate_step(&self, step: &RecordedStep) -> Result<(), StreamError> {
        if !self.pending || !Arc::ptr_eq(&self.gpu, &step.owner) || self.action != step.action {
            return Err(StreamError::Stale);
        }
        Ok(())
    }

    fn token(
        &self,
        ticket: Option<TrackedBuffer>,
        source_charge: Option<SharedCharge>,
        binding: Option<wgpu::BindGroup>,
    ) -> RecordedStep {
        RecordedStep {
            owner: Arc::clone(&self.gpu),
            action: self.action,
            completes_tile: false,
            _pick_query: None,
            _ticket: ticket,
            _source_charge: source_charge,
            _binding: binding,
        }
    }

    fn axis_len(&self, axis: u32) -> u32 {
        if axis == 0 {
            self.gpu.resources.params.x_len
        } else {
            self.gpu.resources.params.y_len
        }
    }

    fn axis_count(&self, axis: u32) -> u32 {
        let p = &self.gpu.resources.params;
        let cy = p.flags & super::FIELD_FLAG_COLUMNS_ARE_Y != 0;
        let cells = if (axis == 0) != cy { p.cols } else { p.rows };
        cells - u32::from(p.flags & super::FIELD_FLAG_INTERPOLATED != 0)
    }

    fn axis_request(&self, axis: u32, start: u32, sweep: u32) -> Action {
        Action::Source(SourceRequest {
            role: SourceRole::Axis(axis),
            start,
            len: self.max_pairs.min(self.axis_len(axis) - start),
            sweep,
        })
    }

    fn z_request(&self, column: u32, start: u32) -> Action {
        Action::Source(SourceRequest {
            role: SourceRole::ZColumn(column),
            start,
            len: self.max_pairs.min(self.gpu.resources.params.rows - start),
            sweep: 0,
        })
    }

    fn following_action(&self) -> Action {
        match self.action {
            Action::Initialize => self.axis_request(0, 0, 0),
            Action::Source(r) => match r.role {
                SourceRole::Axis(axis) => {
                    let end = r.start + r.len;
                    if end < self.axis_len(axis) {
                        self.axis_request(axis, end - 1, r.sweep)
                    } else if r.sweep + 1 < sweep_bound(self.axis_count(axis)) {
                        self.axis_request(axis, 0, r.sweep + 1)
                    } else if axis == 0 {
                        self.axis_request(1, 0, 0)
                    } else {
                        self.z_request(0, 0)
                    }
                }
                SourceRole::ZColumn(column) => {
                    let end = r.start + r.len;
                    if end < self.gpu.resources.params.rows {
                        self.z_request(column, end)
                    } else if column + 1 < self.gpu.resources.params.cols {
                        self.z_request(column + 1, 0)
                    } else {
                        Action::Finalize
                    }
                }
            },
            Action::Finalize | Action::Complete => Action::Complete,
        }
    }

    fn record_draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        init: bool,
    ) {
        let r = &self.gpu.resources;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if init {
                "stream field raster initialization"
            } else {
                "stream field completed tile"
            }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let v = self.gpu.viewport;
        pass.set_viewport(v[0], v[1], v[2], v[3], 0.0, 1.0);
        let tile = self.gpu.rect;
        pass.set_scissor_rect(tile[0], tile[1], tile[2], tile[3]);
        if init {
            pass.set_pipeline(&r.pipelines.init);
            pass.set_bind_group(2, &r.init_field, &[]);
            pass.set_bind_group(3, &self.gpu.init, &[]);
        } else {
            pass.set_pipeline(&r.pipelines.finish);
            pass.set_bind_group(1, &r.final_style, &[]);
            pass.set_bind_group(2, &r.final_field, &[]);
            pass.set_bind_group(3, &self.gpu.finish, &[]);
        }
        pass.draw(0..6, 0..1);
    }
}

/// Each binary step leaves at most ceil(width/2). First/last and final a/b
/// require four more successful transitions; every full overlapping sweep
/// makes at least one transition. This is at most 36 for a u32 count.
pub(crate) fn sweep_bound(count: u32) -> u32 {
    if count == 0 {
        1
    } else {
        u32::BITS - (count - 1).leading_zeros() + 4
    }
}

fn check_budget(ledger: &GpuLedger, budget: Budget, additional: u64) -> Result<(), StreamError> {
    let peak = ledger
        .total_bytes()
        .checked_add(budget.pool_bytes)
        .and_then(|n| n.checked_add(budget.headroom_bytes))
        .and_then(|n| n.checked_add(additional))
        .ok_or(StreamError::Overflow)?;
    if peak > budget.max_renderer_bytes {
        Err(StreamError::TooLarge)
    } else {
        Ok(())
    }
}

fn allocate(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
    size: u64,
    usage: wgpu::BufferUsages,
) -> Result<TrackedBuffer, StreamError> {
    let buffer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // gpu-alloc: StreamingUpload
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream field bounded state"),
            size,
            usage,
            mapped_at_creation: false,
        })
    }))
    .map_err(|_| StreamError::AllocationFailed)?;
    Ok(TrackedBuffer::new(
        ledger,
        GpuResourceKind::StreamingUpload,
        buffer,
    ))
}

fn allocate_init(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> Result<TrackedBuffer, StreamError> {
    let buffer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // gpu-alloc: StreamingUpload
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("stream field immutable metadata"),
            contents: bytes,
            usage,
        })
    }))
    .map_err(|_| StreamError::AllocationFailed)?;
    Ok(TrackedBuffer::new(
        ledger,
        GpuResourceKind::StreamingUpload,
        buffer,
    ))
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &[(u32, &wgpu::Buffer)],
) -> wgpu::BindGroup {
    let entries: [wgpu::BindGroupEntry<'_>; 4] = std::array::from_fn(|index| {
        let &(binding, buffer) = buffers.get(index).unwrap_or(&buffers[0]);
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("stream field bindings"),
        layout,
        entries: &entries[..buffers.len()],
    })
}

const _: () = assert!(std::mem::size_of::<ContourLookupMetadataGpu>() == 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate as renderer;
    mod fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/stream_field_fixture.rs"
        ));
    }
    use fixture::*;

    fn gpu() -> (wgpu::Device, wgpu::Queue) {
        let instance = super::super::create_instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("stream field GPU test cannot skip");
        eprintln!("stream field helper adapter: {:?}", adapter.get_info());
        pollster::block_on(adapter.request_device(&Default::default())).unwrap()
    }

    fn submit_step(
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        tile: &mut Tile,
        step: RecordedStep,
    ) {
        queue.submit([encoder.finish()]);
        tile.commit(step).unwrap();
        let ledger = Arc::clone(&tile.gpu.resources.ledger);
        let retired = ledger.take_retirement();
        queue.on_submitted_work_done(move || ledger.complete_retirement(retired));
    }

    fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(BACKGROUND),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }

    #[test]
    fn production_helper_matches_bounded_proof_fixtures() {
        let (device, queue) = gpu();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("field_columnar.wgsl").into()),
        });
        let budget = Budget {
            max_renderer_bytes: 128 * 1024,
            pool_bytes: 0,
            headroom_bytes: 0,
        };
        let mut comparisons = 0;
        for centers in [false, true] {
            for interpolated in [false, true] {
                for columns_are_y in [false, true] {
                    let case = Fixture {
                        centers,
                        interpolated,
                        columns_are_y,
                        log: centers != interpolated,
                        inverted: columns_are_y,
                    };
                    let data = fixture_data(case);
                    let stops = [
                        [0.9f32, 0.1, 0.15, 0.55],
                        [0.15, 0.8, 0.2, 0.9],
                        [0.1, 0.2, 0.95, 0.3],
                    ];
                    let tables = super::super::build_field_lookup_tables(&stops, &[]).unwrap();
                    for samples in [1, 4] {
                        let ledger = Arc::new(GpuLedger::new());
                        let pipelines = Arc::new(
                            Pipelines::new(
                                &device,
                                &shader,
                                wgpu::TextureFormat::Rgba8Unorm,
                                samples,
                            )
                            .unwrap(),
                        );
                        let resources = Resources::new(
                            &device,
                            pipelines,
                            Arc::clone(&ledger),
                            budget,
                            &data.transform,
                            &data.style,
                            &data.params,
                            &tables,
                        )
                        .unwrap();
                        assert_eq!(ledger.total_bytes(), resources.charged_bytes());
                        let shape = TileShape::new(
                            &device.limits(),
                            [PANEL.2, PANEL.3],
                            samples,
                            16 * 1024,
                        )
                        .unwrap();
                        assert!(shape.width * shape.height < PANEL.2 * PANEL.3);
                        // Prefix padding exercises a nonzero base within an uploaded
                        // work buffer; no resident coordinate buffer enters the helper.
                        let work = allocate(
                            &device,
                            &ledger,
                            24,
                            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        )
                        .unwrap();
                        let target = texture(&device, samples);
                        let view = target.create_view(&Default::default());
                        let mut begin = device.create_command_encoder(&Default::default());
                        clear(&mut begin, &view);
                        queue.submit([begin.finish()]);
                        let mut tile_count = 0;
                        for y in (PANEL.1..PANEL.1 + PANEL.3).step_by(shape.height as usize) {
                            for x in (PANEL.0..PANEL.0 + PANEL.2).step_by(shape.width as usize) {
                                let rect = [
                                    x,
                                    y,
                                    shape.width.min(PANEL.0 + PANEL.2 - x),
                                    shape.height.min(PANEL.1 + PANEL.3 - y),
                                ];
                                let mut tile = Tile::new(
                                    &device,
                                    Arc::clone(&resources),
                                    budget,
                                    shape,
                                    [
                                        PANEL.0 as f32,
                                        PANEL.1 as f32,
                                        PANEL.2 as f32,
                                        PANEL.3 as f32,
                                    ],
                                    [PANEL.0, PANEL.1, PANEL.2, PANEL.3],
                                    rect,
                                    2,
                                )
                                .unwrap();
                                assert_eq!(
                                    tile.charged_bytes(),
                                    u64::from(rect[2] * rect[3] * samples) * PIXEL_BYTES
                                        + TILE_BYTES
                                );
                                let mut encoder =
                                    device.create_command_encoder(&Default::default());
                                let init = tile.record_init(&mut encoder, &view).unwrap();
                                assert_eq!(tile.action(), Err(StreamError::WrongState));
                                submit_step(&queue, encoder, &mut tile, init);
                                while let Action::Source(request) = tile.action().unwrap() {
                                    let source = match request.role {
                                        SourceRole::Axis(axis) => &data.axes[axis as usize],
                                        SourceRole::ZColumn(column) => {
                                            &data.sources[data.declarations[column as usize]]
                                        }
                                    };
                                    queue.write_buffer(
                                        &work,
                                        8,
                                        bytemuck::cast_slice(
                                            &source[request.start as usize
                                                ..(request.start + request.len) as usize],
                                        ),
                                    );
                                    let mut encoder =
                                        device.create_command_encoder(&Default::default());
                                    let step = tile
                                        .record_source(
                                            &device,
                                            &mut encoder,
                                            budget,
                                            request,
                                            &work,
                                            1,
                                            work.shared_charge(),
                                        )
                                        .unwrap();
                                    submit_step(&queue, encoder, &mut tile, step);
                                }
                                assert_eq!(tile.action().unwrap(), Action::Finalize);
                                let mut encoder =
                                    device.create_command_encoder(&Default::default());
                                let final_step = tile.record_final(&mut encoder, &view).unwrap();
                                submit_step(&queue, encoder, &mut tile, final_step);
                                assert_eq!(tile.action().unwrap(), Action::Complete);
                                let mut rejected =
                                    device.create_command_encoder(&Default::default());
                                assert!(matches!(
                                    tile.record_final(&mut rejected, &view),
                                    Err(StreamError::WrongState)
                                ));
                                tile_count += 1;
                            }
                        }
                        assert!(tile_count > 1);
                        let oracle_pipe =
                            render_pipeline(&device, &shader, "fs_main", samples, false);
                        let transform = buffer(
                            &device,
                            bytemuck::bytes_of(&data.transform),
                            wgpu::BufferUsages::UNIFORM,
                        );
                        let style = buffer(
                            &device,
                            bytemuck::bytes_of(&data.style),
                            wgpu::BufferUsages::UNIFORM,
                        );
                        let params = buffer(
                            &device,
                            bytemuck::bytes_of(&data.params),
                            wgpu::BufferUsages::UNIFORM,
                        );
                        let pool = buffer(
                            &device,
                            bytemuck::cast_slice(&data.oracle_pool),
                            wgpu::BufferUsages::STORAGE,
                        );
                        let grid = buffer(
                            &device,
                            bytemuck::cast_slice(&data.oracle_grid),
                            wgpu::BufferUsages::STORAGE,
                        );
                        let stops = buffer(
                            &device,
                            bytemuck::cast_slice(&tables.stops),
                            wgpu::BufferUsages::STORAGE,
                        );
                        let metadata = buffer(
                            &device,
                            bytemuck::cast_slice(&tables.metadata),
                            wgpu::BufferUsages::STORAGE,
                        );
                        let g0 = bindings(
                            &device,
                            &oracle_pipe.get_bind_group_layout(0),
                            &[(0, &transform)],
                        );
                        let g1 = bindings(
                            &device,
                            &oracle_pipe.get_bind_group_layout(1),
                            &[(0, &style)],
                        );
                        let g2 = bindings(
                            &device,
                            &oracle_pipe.get_bind_group_layout(2),
                            &[
                                (0, &pool),
                                (1, &grid),
                                (3, &stops),
                                (4, &params),
                                (6, &metadata),
                            ],
                        );
                        let oracle = texture(&device, samples);
                        let oracle_view = oracle.create_view(&Default::default());
                        let mut encoder = device.create_command_encoder(&Default::default());
                        draw(
                            &mut encoder,
                            &oracle_pipe,
                            &oracle_view,
                            &[(0, &g0), (1, &g1), (2, &g2)],
                            PANEL,
                            true,
                        );
                        let expected = final_image(&device, &mut encoder, &oracle, samples);
                        let actual = final_image(&device, &mut encoder, &target, samples);
                        queue.submit([encoder.finish()]);
                        let expected = read_final_image(&device, &expected);
                        let actual = read_final_image(&device, &actual);
                        assert_eq!(
                            actual, expected,
                            "production field helper {case:?} {samples}x"
                        );
                        comparisons += actual.len();
                        assert!(ledger.peak_bytes() <= budget.max_renderer_bytes);
                        drop(work);
                        drop(resources);
                        device
                            .poll(wgpu::PollType::Wait {
                                submission_index: None,
                                timeout: Some(std::time::Duration::from_secs(30)),
                            })
                            .unwrap();
                        ledger.end_submission();
                        assert_eq!(ledger.total_bytes(), 0);
                    }
                }
            }
        }
        assert_eq!(comparisons, (WIDTH * HEIGHT * 5 * 8) as usize);
    }

    #[test]
    fn tile_budget_and_recorded_ownership_fail_closed() {
        let (device, queue) = gpu();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("field_columnar.wgsl").into()),
        });
        let ledger = Arc::new(GpuLedger::new());
        let budget = Budget {
            max_renderer_bytes: 64 * 1024,
            pool_bytes: 0,
            headroom_bytes: 0,
        };
        let pipelines =
            Arc::new(Pipelines::new(&device, &shader, wgpu::TextureFormat::Rgba8Unorm, 1).unwrap());
        let data = fixture_data(Fixture {
            centers: false,
            interpolated: false,
            columns_are_y: false,
            log: false,
            inverted: false,
        });
        let tables = super::super::build_field_lookup_tables(&[[1.0; 4]; 3], &[]).unwrap();
        assert!(matches!(
            Resources::new(
                &device,
                Arc::clone(&pipelines),
                Arc::clone(&ledger),
                Budget {
                    max_renderer_bytes: 1,
                    ..budget
                },
                &data.transform,
                &data.style,
                &data.params,
                &tables
            ),
            Err(StreamError::TooLarge)
        ));
        assert_eq!(ledger.total_bytes(), 0);
        let resources = Resources::new(
            &device,
            pipelines,
            Arc::clone(&ledger),
            budget,
            &data.transform,
            &data.style,
            &data.params,
            &tables,
        )
        .unwrap();
        assert!(matches!(
            TileShape::new(&device.limits(), [15, 11], 4, 63),
            Err(StreamError::TooLarge)
        ));
        assert_eq!(sweep_bound(u32::MAX), 36);
        let shape = TileShape::new(&device.limits(), [15, 11], 1, 1024).unwrap();
        let make = || {
            Tile::new(
                &device,
                Arc::clone(&resources),
                budget,
                shape,
                [2.0, 2.0, 15.0, 11.0],
                [2, 2, 15, 11],
                [2, 2, shape.width, shape.height],
                2,
            )
            .unwrap()
        };
        let mut tile = make();
        let target = texture(&device, 1);
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view);
        let recorded = tile.record_init(&mut encoder, &view).unwrap();
        assert_eq!(tile.action(), Err(StreamError::WrongState));
        drop(encoder);
        tile.discard(recorded).unwrap();
        assert_eq!(tile.action().unwrap(), Action::Initialize);
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view);
        let recorded = tile.record_init(&mut encoder, &view).unwrap();
        submit_step(&queue, encoder, &mut tile, recorded);
        let Action::Source(request) = tile.action().unwrap() else {
            panic!("first source request");
        };
        let work = allocate_init(
            &device,
            &ledger,
            bytemuck::cast_slice(&data.axes[0][..2]),
            wgpu::BufferUsages::STORAGE,
        )
        .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        let prior_bytes = ledger.total_bytes();
        let tight = Budget {
            max_renderer_bytes: prior_bytes + STEP_BYTES - 1,
            ..budget
        };
        assert!(matches!(
            tile.record_source(
                &device,
                &mut encoder,
                tight,
                request,
                &work,
                0,
                work.shared_charge()
            ),
            Err(StreamError::TooLarge)
        ));
        assert_eq!(ledger.total_bytes(), prior_bytes);
        assert_eq!(tile.action().unwrap(), Action::Source(request));
        assert!(matches!(
            tile.record_source(
                &device,
                &mut encoder,
                budget,
                SourceRequest {
                    start: request.start + 1,
                    ..request
                },
                &work,
                0,
                work.shared_charge()
            ),
            Err(StreamError::Stale)
        ));
        assert!(matches!(
            tile.record_source(
                &device,
                &mut encoder,
                budget,
                request,
                &work,
                100,
                work.shared_charge()
            ),
            Err(StreamError::InvalidPayload)
        ));
        assert!(matches!(
            tile.record_final(&mut encoder, &view),
            Err(StreamError::WrongState)
        ));
        let recorded = tile
            .record_source(
                &device,
                &mut encoder,
                budget,
                request,
                &work,
                0,
                work.shared_charge(),
            )
            .unwrap();
        let before = ledger.snapshot();
        drop(tile);
        drop(resources);
        drop(work);
        assert_eq!(
            ledger.snapshot().live_bytes(),
            before.live_bytes(),
            "recorded token must retain cancelled tile/field owners"
        );
        queue.submit([encoder.finish()]);
        drop(recorded);
        assert!(ledger.total_bytes() > 0, "submission is not completion");
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        ledger.end_submission();
        assert_eq!(ledger.total_bytes(), 0);
    }
}
