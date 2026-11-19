//! The frame's globals and clear colour, the prepared frame's decorations,
//! picks and draw, and the pick ticket.

use super::*;

/// The globals of a frame of `chart` laid out as `l`, `width x height`
/// physical pixels, drawn into a target of `format`.
pub(super) fn frame_globals(
    chart: &RadialChart,
    l: &layout::Layout,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
) -> Globals {
    Globals {
        size: [width as f32, height as f32, l.sectors.len() as f32, 2.0],
        background: [
            chart.background.r,
            chart.background.g,
            chart.background.b,
            chart.background.a,
        ],
        light: [
            chart.style.light[0],
            chart.style.light[1],
            chart.style.light[2],
            0.0,
        ],
        shadow: [if chart.style.shadow { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        groups: l.groups,
        meta: [
            if chart.split.is_some() { 2.0 } else { 1.0 },
            if format.is_srgb() { 1.0 } else { 0.0 },
            l.holes[0],
            l.holes[1],
        ],
    }
}

/// The clear colour of an export over `background`: transparent for
/// [`RadialExportBackground::Transparent`], the options' own otherwise.
pub(super) fn export_clear(clear: crate::Color, background: RadialExportBackground) -> crate::Color {
    match background {
        RadialExportBackground::Transparent => crate::Color::new(0.0, 0.0, 0.0, 0.0),
        _ => clear,
    }
}

impl RadialFrame {
    /// The frame's decorations, the plot area, the title and every slice
    /// label with its leader line, on the common `HitMap` / `Selectable` /
    /// `Draggable` / `Resizable` contract with the chart as the state. Each
    /// element places its parts from the chart it is asked about, with the
    /// one placement the label raster draws with, so pass the chart this
    /// frame was prepared from; a drag edits that chart's `decorations`.
    /// The elements measure text themselves: any `MeasureText` may be
    /// passed. Hit-test through [`Self::decoration_at`], which names nothing
    /// outside the canvas.
    pub fn decorations(&self) -> &crate::HitMap<RadialChart> {
        &self.decorations
    }
    /// The topmost decoration at `position` (chart-local logical pixels) of
    /// `chart`, the chart this frame was prepared from; nothing outside the
    /// canvas.
    pub fn decoration_at(&self, chart: &RadialChart, position: [f32; 2]) -> Option<crate::HitId> {
        let canvas = self.space.panel;
        if position.iter().any(|v| !v.is_finite()) || !canvas.contains(position[0], position[1]) {
            return None;
        }
        self.decorations.hit_test(
            chart,
            &crate::CpuTextMeasure::default(),
            position[0],
            position[1],
        )
    }
    /// The hit-map entry of decoration `id`, when the frame draws it.
    pub fn decoration(&self, id: &model::DecorationId) -> Option<crate::HitId> {
        self.decorations.find(&id.element_id())
    }
    /// Every decoration this frame draws, with its hit-map entry: the plot
    /// area, the title, then the slice labels in slice order, main circle
    /// first.
    pub fn decoration_ids<'a>(
        &'a self,
        chart: &'a RadialChart,
    ) -> impl Iterator<Item = (crate::HitId, model::DecorationId)> + 'a {
        let labels = (0..chart.group_count())
            .flat_map(|group| {
                (0..chart.group_len(group)).map(move |index| RadialTarget { group, index })
            })
            .map(model::DecorationId::RadialLabel);
        [
            model::DecorationId::DataArea,
            model::DecorationId::ChartTitle,
        ]
        .into_iter()
        .chain(labels)
        .filter_map(|id| Some((self.decoration(&id)?, id)))
    }
    /// The id of decoration `hit` of `chart`.
    pub fn decoration_id(
        &self,
        chart: &RadialChart,
        hit: crate::HitId,
    ) -> Option<model::DecorationId> {
        self.decoration_ids(chart)
            .find(|(h, _)| *h == hit)
            .map(|(_, id)| id)
    }
    /// The drawn bounds of decoration `hit` of `chart`, its leader line
    /// included.
    pub fn decoration_bounds(
        &self,
        chart: &RadialChart,
        hit: crate::HitId,
    ) -> Option<crate::layout::RectF> {
        use model::select::SelectionGeometry;
        self.decorations
            .get(hit)?
            .bounds(chart, &crate::CpuTextMeasure::default())
    }
    /// Move decoration `hit` of `chart` by a pointer delta through the
    /// common `Draggable` policy, writing `chart.decorations`; `None` for an
    /// unknown hit. A rejected move leaves the chart unchanged. Prepare a
    /// new frame before drawing or hit-testing again.
    pub fn drag_decoration(
        &self,
        chart: &mut RadialChart,
        hit: crate::HitId,
        dx: f32,
        dy: f32,
    ) -> Option<crate::layout::NudgeResult> {
        Some(self.decorations.get(hit)?.as_draggable()?.drag_by(
            chart,
            &crate::CpuTextMeasure::default(),
            dx,
            dy,
        ))
    }
    /// The resize handle of decoration `hit` (the plot area) at `position`.
    pub fn decoration_resize_handle(
        &self,
        chart: &RadialChart,
        hit: crate::HitId,
        position: [f32; 2],
    ) -> Option<crate::resize::ResizeHandle> {
        self.decorations
            .get(hit)?
            .as_resizable()?
            .hit_resize_handle(
                chart,
                &crate::CpuTextMeasure::default(),
                position[0],
                position[1],
            )
    }
    /// Resize decoration `hit` (the plot area) of `chart` by dragging
    /// `handle`; both edges move or neither does.
    pub fn resize_decoration(
        &self,
        chart: &mut RadialChart,
        hit: crate::HitId,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> Option<crate::layout::NudgeResult> {
        Some(self.decorations.get(hit)?.as_resizable()?.resize_by(
            chart,
            &crate::CpuTextMeasure::default(),
            handle,
            dx,
            dy,
        ))
    }
    /// The selection box of decoration `hit` of `chart`, inside the canvas:
    /// a title wider than the canvas is selected as what the canvas shows.
    pub fn decoration_selection_box(
        &self,
        chart: &RadialChart,
        hit: crate::HitId,
    ) -> Option<crate::SelectionBox> {
        self.decorations
            .selection_box(hit, chart, &crate::CpuTextMeasure::default())
            .map(|b| b.clipped_to(self.space.panel))
    }
    /// True when annotation GPU storage is shared (e.g. hover/material-only edits).
    pub fn shares_annotations_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self._annotations, &other._annotations)
    }
    /// The anchor space of this frame for the annotation layer, in logical
    /// pixels: the canvas and the plot area, without value axes (ratio
    /// anchors only).
    pub fn annotation_space(&self) -> crate::annotation::PixelSpace {
        self.space
    }
    /// Request the slice drawn under `position` (chart-local logical
    /// pixels): the visible surface nearest the camera, with tilt, gap,
    /// rounding, explode and the hover lift, picked on the GPU by the draw's
    /// own sector ray-march and occlusion rule from this frame's sectors.
    /// Resolve the ticket with [`RadialPickTicket::resolve`]; a position
    /// outside the canvas, or not finite, resolves to `None` at once.
    pub fn pick(self: &Arc<Self>, position: [f32; 2]) -> Result<RadialPickTicket, RadialError> {
        self.request_pick(position, false)
    }
    /// [`Self::pick`] on the unlifted surfaces, for hover hysteresis: a host
    /// keeps the previous hover while the pointer is in its unraised
    /// footprint, even where the lifted surface moved away.
    pub fn pick_at_rest(
        self: &Arc<Self>,
        position: [f32; 2],
    ) -> Result<RadialPickTicket, RadialError> {
        self.request_pick(position, true)
    }
    fn request_pick(
        self: &Arc<Self>,
        position: [f32; 2],
        rest: bool,
    ) -> Result<RadialPickTicket, RadialError> {
        if position.iter().any(|v| !v.is_finite()) || self.targets.is_empty() {
            return Ok(RadialPickTicket::none());
        }
        let pixel = [position[0] * self.scale, position[1] * self.scale];
        if pixel[0] < 0.0
            || pixel[1] < 0.0
            || pixel[0] >= self.size.0 as f32
            || pixel[1] >= self.size.1 as f32
        {
            return Ok(RadialPickTicket::none());
        }
        self.gpu.health().check().map_err(radial_gpu_error)?;
        let device = self.gpu.device();
        // gpu-alloc: PickScratch
        let query = gpu_created(device, "radial pick query", || {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("radial pick query"),
                contents: bytemuck::bytes_of(&RadialPickQueryGpu {
                    pixel,
                    count: self.targets.len() as u32,
                    rest: u32::from(rest),
                }),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        })?;
        let query = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::PickScratch, query);
        // gpu-alloc: PickScratch
        let result = gpu_created(device, "radial pick result", || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("radial pick result"),
                size: RADIAL_PICK_RESULT_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        })?;
        let result = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::PickScratch, result);
        // gpu-alloc: Readback
        let readback = gpu_created(device, "radial pick readback", || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("radial pick readback"),
                size: RADIAL_PICK_RESULT_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        let readback = TrackedBuffer::new(self.gpu.ledger(), GpuResourceKind::Readback, readback);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radial pick"),
            layout: &self.pick_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: query.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.sectors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: result.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("radial pick"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("radial pick"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pick_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, RADIAL_PICK_RESULT_BYTES);
        self.gpu.queue().submit([encoder.finish()]);
        Ok(RadialPickTicket {
            pending: Some(PendingRadialPick {
                frame: Arc::clone(self),
                readback: GpuReadback::map(self.gpu.device(), self.gpu.health(), &readback, None),
                _readback: readback,
                _scratch: [query, result],
            }),
        })
    }
    /// The CPU twin's pick on this frame's sectors, for the GPU pick's
    /// agreement test only.
    #[cfg(feature = "cpu-twin")]
    #[doc(hidden)]
    pub fn cpu_twin_pick(&self, position: [f32; 2], rest: bool) -> Option<RadialTarget> {
        crate::pick::cpu_twin::radial_pick(
            &self.geometry,
            &self.targets,
            position,
            self.scale,
            self.size,
            rest,
        )
    }

    /// Physical target dimensions, rounded from logical size times scale.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    /// The render attachment must have this frame's size and renderer format.
    /// Requires sample count 1; sets viewport/scissor to the whole attachment.
    /// For an embedded UI region, render into a matching offscreen texture.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_bound(pass, &self.bind_group);
    }
    /// [`Self::draw`] with `bind_group` in place of the frame's own: an
    /// export's background override ([`RadialRenderer::export_rgba_with_background`]).
    pub(super) fn draw_bound(&self, pass: &mut wgpu::RenderPass<'_>, bind_group: &wgpu::BindGroup) {
        pass.set_viewport(0.0, 0.0, self.size.0 as f32, self.size.1 as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, self.size.0, self.size.1);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

impl RadialPickTicket {
    fn none() -> Self {
        Self { pending: None }
    }
    /// The slice the GPU picked, or `None` where no sector is drawn. The
    /// bundle's poll thread completes the wait on native (see
    /// [`RendererDevice::new`]).
    pub async fn resolve(mut self) -> Result<Option<RadialTarget>, RadialError> {
        if let Some(pending) = &mut self.pending {
            pending
                .readback
                .wait()
                .await
                .map_err(radial_readback_error)?;
        }
        self.finish()
    }

    /// [`Self::resolve`], polling the device on this thread until the pick
    /// completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(mut self) -> Result<Option<RadialTarget>, RadialError> {
        if let Some(pending) = &mut self.pending {
            pending
                .readback
                .wait_blocking()
                .map_err(radial_readback_error)?;
        }
        self.finish()
    }

    /// The pick if it has completed, or the ticket back, without blocking.
    pub fn try_resolve(
        mut self,
    ) -> Result<crate::TryResolve<Option<RadialTarget>, Self>, RadialError> {
        if let Some(pending) = &mut self.pending
            && !pending
                .readback
                .poll_ready()
                .map_err(radial_readback_error)?
        {
            return Ok(crate::TryResolve::Pending(self));
        }
        self.finish().map(crate::TryResolve::Ready)
    }

    /// After a wait: the sector the result names.
    fn finish(&mut self) -> Result<Option<RadialTarget>, RadialError> {
        let Some(mut pending) = self.pending.take() else {
            return Ok(None);
        };
        let result = pending
            .readback
            .finish(|mapped| *bytemuck::from_bytes::<RadialPickResultGpu>(mapped))
            .map_err(radial_readback_error)?;
        if result.hit == 0 {
            return Ok(None);
        }
        pending
            .frame
            .targets
            .get(result.index as usize)
            .copied()
            .map(Some)
            .ok_or_else(|| RadialError::Gpu("radial pick returned no sector of the frame".into()))
    }
}
