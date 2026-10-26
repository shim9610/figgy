#![doc = include_str!("../RADIAL.md")]
//! Resident categorical charts with analytic sector surfaces and procedural
//! materials. No Cartesian columns, auto-fit jobs, or stream replay are involved.
//! `prepare` returns an immutable frame; repeated identical snapshots reuse it.
use crate::gpu_memory::{
    GpuLedger, GpuMemoryUsage, GpuResourceKind, TrackedBuffer, TrackedTexture,
};
use crate::{RasterImage, RendererDevice};
pub use model::radial::*;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// The assembled radial module.
pub(crate) const RADIAL_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/material.wgsl",
    "data_render/common/radial_sector.wgsl",
    "radial.wgsl",
);
/// The radial pick module: the same sector fragment as the draw, with a
/// compute entry that picks the sector the draw shows under one pixel.
pub(crate) const RADIAL_PICK_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/material.wgsl",
    "data_render/common/radial_sector.wgsl",
    "radial_pick.wgsl",
);

#[path = "radial_layout.rs"]
mod layout;

#[derive(Debug)]
pub enum RadialError {
    Invalid(&'static str),
    Budget { needed: u64, limit: u64 },
    Gpu(String),
}
impl std::fmt::Display for RadialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Budget { needed, limit } => {
                write!(f, "radial GPU budget: {needed} bytes needed, limit {limit}")
            }
            Self::Gpu(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for RadialError {}
/// One sector as the radial modules read it (`radial_sector.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Sector {
    pub(crate) geometry: [f32; 4],
    pub(crate) arc: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) material: crate::data_render::MaterialGpu,
    pub(crate) detail: [f32; 4], // explode, angular gap, bevel, reserved (0, seed moved to `material`)
    pub(crate) rounding: [f32; 4], // inner corner, outer corner, vertical lift, reserved (0, gloss moved)
    pub(crate) outline: [f32; 4],
    pub(crate) outline_color: [f32; 4],
    // reserved (0, highlight moved), reserved (0, texture angle moved),
    // inside-label flag, inside-label lift: how far the lifted top face moves
    // up the screen in physical pixels (`rounding[2] * sin(arc[2]) *
    // geometry[2]`), computed once where the sector is built
    // (`radial_layout.rs`). The layout moves the inside labels or their hit
    // map by it and the shader moves the inside glyphs by it.
    pub(crate) effects: [f32; 4],
    pub(crate) light: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    size: [f32; 4],
    background: [f32; 4],
    light: [f32; 4],
    shadow: [f32; 4],
    groups: [[f32; 4]; 2],
    meta: [f32; 4],
}
const _: () =
    assert!(std::mem::size_of::<Sector>() == 176 && std::mem::size_of::<Globals>() == 112);

/// The radial pick's query (`RadialPickQuery` of `radial_pick.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RadialPickQueryGpu {
    pub(crate) pixel: [f32; 2],
    pub(crate) count: u32,
    pub(crate) rest: u32,
}
/// The radial pick's result (`RadialPickResult` of `radial_pick.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RadialPickResultGpu {
    pub(crate) index: u32,
    pub(crate) hit: u32,
    pub(crate) t: f32,
    pub(crate) reserved: u32,
}
const _: () = assert!(
    std::mem::size_of::<RadialPickQueryGpu>() == 16
        && std::mem::size_of::<RadialPickResultGpu>() == 16
);
const RADIAL_PICK_RESULT_BYTES: u64 = std::mem::size_of::<RadialPickResultGpu>() as u64;

/// Immutable GPU snapshot. Its shader draws straight into a same-format,
/// single-sample target; four analytic subpixel samples provide edge AA.
/// Labels are rasterized at the requested output resolution, never upscaled.
pub struct RadialFrame {
    decorations: crate::HitMap<RadialChart>,
    gpu: RendererDevice,
    ledger: Arc<GpuLedger>,
    size: (u32, u32),
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    pick_pipeline: wgpu::ComputePipeline,
    pick_layout: wgpu::BindGroupLayout,
    _uniform: TrackedBuffer,
    sectors: TrackedBuffer,
    _annotations: Arc<TrackedTexture>,
    /// The CPU sectors, kept only for the CPU twin's agreement test.
    #[cfg(feature = "cpu-twin")]
    geometry: Vec<Sector>,
    /// The slice each sector of the buffer draws, in buffer order.
    targets: Vec<RadialTarget>,
    scale: f32,
    space: crate::annotation::PixelSpace,
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
        let labels = chart
            .slices
            .iter()
            .enumerate()
            .map(|(index, _)| RadialTarget::main(index))
            .chain(
                chart
                    .split
                    .iter()
                    .flat_map(|s| s.children.iter().enumerate())
                    .map(|(index, _)| RadialTarget::detail(index)),
            )
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
        let device = self.gpu.device();
        // gpu-alloc: PickScratch
        let query = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radial pick query"),
            contents: bytemuck::bytes_of(&RadialPickQueryGpu {
                pixel,
                count: self.targets.len() as u32,
                rest: u32::from(rest),
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let query = TrackedBuffer::new(&self.ledger, GpuResourceKind::PickScratch, query);
        // gpu-alloc: PickScratch
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radial pick result"),
            size: RADIAL_PICK_RESULT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let result = TrackedBuffer::new(&self.ledger, GpuResourceKind::PickScratch, result);
        // gpu-alloc: Readback
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radial pick readback"),
            size: RADIAL_PICK_RESULT_BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback = TrackedBuffer::new(&self.ledger, GpuResourceKind::Readback, readback);
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
        let (sender, receiver) = futures_channel::oneshot::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = sender.send(r);
        });
        Ok(RadialPickTicket {
            pending: Some(PendingRadialPick {
                frame: Arc::clone(self),
                readback,
                receiver,
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
        pass.set_viewport(0.0, 0.0, self.size.0 as f32, self.size.1 as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, self.size.0, self.size.1);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
/// An asynchronous radial pick, from [`RadialFrame::pick`] or
/// [`RadialFrame::pick_at_rest`]: the request is submitted, and
/// [`Self::resolve`] reads the GPU's answer back. The ticket keeps its frame
/// alive until it resolves or is dropped.
pub struct RadialPickTicket {
    pending: Option<PendingRadialPick>,
}
struct PendingRadialPick {
    frame: Arc<RadialFrame>,
    readback: TrackedBuffer,
    receiver: futures_channel::oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
    /// The query and result buffers, credited when the pick resolves.
    _scratch: [TrackedBuffer; 2],
}
impl RadialPickTicket {
    fn none() -> Self {
        Self { pending: None }
    }
    /// The slice the GPU picked, or `None` where no sector is drawn.
    pub async fn resolve(self) -> Result<Option<RadialTarget>, RadialError> {
        let Some(pending) = self.pending else {
            return Ok(None);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pending
            .frame
            .gpu
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| RadialError::Gpu(e.to_string()))?;
        pending
            .receiver
            .await
            .map_err(|e| RadialError::Gpu(e.to_string()))?
            .map_err(|e| RadialError::Gpu(e.to_string()))?;
        let result = {
            let mapped = pending
                .readback
                .get_mapped_range(..)
                .map_err(|e| RadialError::Gpu(e.to_string()))?;
            *bytemuck::from_bytes::<RadialPickResultGpu>(&mapped)
        };
        pending.readback.unmap();
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
    #[cfg(not(target_arch = "wasm32"))]
    /// Native blocking wrapper for [`Self::resolve`].
    pub fn resolve_blocking(self) -> Result<Option<RadialTarget>, RadialError> {
        pollster::block_on(self.resolve())
    }
}

/// Shares the host device/queue. Owns one pipeline and at most one cached frame.
/// Kept separate from the large numeric-data/Cartesian renderer deliberately.
pub struct RadialRenderer {
    gpu: RendererDevice,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    pick_layout: wgpu::BindGroupLayout,
    pick_pipeline: wgpu::ComputePipeline,
    ledger: Arc<GpuLedger>,
    budget: u64,
    annotations_cached: Option<(FrameKey, (u32, u32), f32, u64, Arc<TrackedTexture>)>,
    /// The dedicated annotation layer, drawn over the frame by its own pass.
    annotation_layer: crate::annotation::RadialAnnotationCache,
    cached: Option<(FrameKey, (u32, u32), f32, u64, Arc<RadialFrame>)>,
}
impl RadialRenderer {
    /// Initialize browser pipelines through WebGPU's asynchronous compiler.
    /// Uses the checked production pipeline descriptors before publishing the
    /// wgpu-owned pipeline. Native hosts use the ordinary synchronous constructor.
    pub async fn new_async(
        gpu: RendererDevice,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RadialError> {
        #[cfg(target_arch = "wasm32")]
        crate::browser_compile::render(
            gpu.device(),
            RADIAL_SHADER.source,
            &[("radial surfaces", "radial surfaces")],
            format,
            1,
            None,
            &mut |_| {},
        )
        .await
        .map_err(RadialError::Gpu)?;
        #[cfg(target_arch = "wasm32")]
        crate::browser_compile::compute(
            gpu.device(),
            "renderer.radial_pick",
            RADIAL_PICK_SHADER.source,
            &[("radial pick", "pick")],
            &mut |_| {},
        )
        .await
        .map_err(RadialError::Gpu)?;
        Self::new(gpu, format)
    }

    /// Share the host device/queue. Supports RGBA8/BGRA8, linear or sRGB targets.
    /// Starts with a separate 256 MiB resource budget, not the Cartesian ledger.
    pub fn new(gpu: RendererDevice, format: wgpu::TextureFormat) -> Result<Self, RadialError> {
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(RadialError::Invalid("radial target must be RGBA8 or BGRA8"));
        }
        let device = gpu.device();
        let layout = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("radial chart"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(112),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<Sector>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            },
        );
        let pl = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("radial chart"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            },
        );
        let shader = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("radial surfaces"),
                source: wgpu::ShaderSource::Wgsl(RADIAL_SHADER.source.into()),
            },
        );
        let pipeline = crate::gpu_compile::render_pipeline(
            &device,
            &wgpu::RenderPipelineDescriptor {
                label: Some("radial surfaces"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            },
        );
        // The pick module's own bindings: its query, the frame's sectors and
        // its result.
        let pick_layout = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("radial pick"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<
                                RadialPickQueryGpu,
                            >()
                                as u64),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<Sector>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(RADIAL_PICK_RESULT_BYTES),
                        },
                        count: None,
                    },
                ],
            },
        );
        let pick_pipeline_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("radial pick"),
                bind_group_layouts: &[Some(&pick_layout)],
                immediate_size: 0,
            },
        );
        let pick_shader = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("radial pick"),
                source: wgpu::ShaderSource::Wgsl(RADIAL_PICK_SHADER.source.into()),
            },
        );
        let pick_pipeline = crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some("radial pick"),
                layout: Some(&pick_pipeline_layout),
                module: &pick_shader,
                entry_point: Some("pick"),
                compilation_options: Default::default(),
                cache: None,
            },
        );
        Ok(Self {
            gpu,
            format,
            layout,
            pipeline,
            pick_layout,
            pick_pipeline,
            // host-alloc: W1-a
            ledger: Arc::new(GpuLedger::new()),
            budget: 256 * 1024 * 1024,
            cached: None,
            annotations_cached: None,
            annotation_layer: crate::annotation::RadialAnnotationCache::new(format),
        })
    }
    /// The annotation layer of `chart` for the frame [`Self::prepare`]
    /// returns for the same arguments: its texture and its hit map, cached
    /// like the frame. Draw it over the frame (the frame, then
    /// `draw_overlay`). The radial chart has no value axes, so it draws no
    /// span: every item is drawn above the slices.
    pub fn prepare_annotations(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadialError> {
        let frame = self.prepare(chart, size, scale)?;
        self.prepare_annotations_for(&frame, chart, scale)
    }
    fn prepare_annotations_for(
        &mut self,
        frame: &RadialFrame,
        chart: &RadialChart,
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadialError> {
        let selected = crate::annotation::selected_index(chart.decorations.selected.as_ref());
        let space = frame.annotation_space();
        if self.annotation_layer.needs_raster(
            &chart.annotations,
            space,
            frame.size,
            scale,
            selected,
        ) {
            self.admit(
                self.annotation_layer
                    .bytes_needed(&chart.annotations, frame.size),
            )?;
        }
        Ok(self.annotation_layer.prepare(
            &self.gpu,
            &self.ledger,
            &chart.annotations,
            space,
            frame.size,
            scale,
            selected,
        ))
    }
    /// Accounted live resources plus resources awaiting submission completion.
    pub fn gpu_memory_usage(&self) -> GpuMemoryUsage {
        self.ledger.snapshot()
    }
    /// Set the admission limit in bytes. Does not evict existing frames/resources.
    pub fn set_memory_budget(&mut self, bytes: u64) {
        self.budget = bytes;
    }
    fn admit(&self, extra: u64) -> Result<(), RadialError> {
        let needed = self.ledger.total_bytes().saturating_add(extra);
        if needed > self.budget {
            Err(RadialError::Budget {
                needed,
                limit: self.budget,
            })
        } else {
            Ok(())
        }
    }
    /// `size` is logical pixels. `scale` (0.5..4) increases actual geometry AND
    /// glyph resolution without changing the layout. Invalid edits do not evict
    /// the last valid snapshot. All chart/model fields participate in reuse.
    /// Each logical dimension must be at least 160; physical output must fit the
    /// device texture limit. Model edits need no manual cache invalidation.
    pub fn prepare(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<RadialFrame>, RadialError> {
        let (width, height) = self.frame_size(chart, size, scale)?;
        let fonts = crate::text_render::font_generation();
        if let Some((_, _, _, _, frame)) =
            self.cached.as_ref().filter(|(c, s, k, generation, _)| {
                c.matches(chart) && *s == size && *k == scale && *generation == fonts
            })
        {
            return Ok(Arc::clone(frame));
        }
        let count = chart.slices.len() + chart.split.as_ref().map_or(0, |s| s.children.len());
        let key = annotation_key(chart);
        let reused = self
            .annotations_cached
            .as_ref()
            .filter(|(c, s, k, f, _)| c == &key && *s == size && *k == scale && *f == fonts)
            .map(|(_, _, _, _, t)| Arc::clone(t));
        self.admit(
            if reused.is_some() {
                0
            } else {
                u64::from(width) * u64::from(height) * 4
            } + 112
                + count as u64 * std::mem::size_of::<Sector>() as u64,
        )?;
        let labels = if reused.is_none() {
            layout::Labels::Raster
        } else {
            layout::Labels::None
        };
        let l = layout::layout(chart, size, scale, labels)?;
        let device = self.gpu.device();
        let annotations = if let Some(texture) = reused {
            texture
        } else {
            // gpu-alloc: PanelTexture
            let annotations = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("radial labels at output resolution"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let annotations =
                TrackedTexture::new(&self.ledger, GpuResourceKind::PanelTexture, annotations);
            self.gpu.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &annotations,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &l.annotations,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            Arc::new(annotations)
        };
        let (uniform, sectors, bind_group) =
            self.bind_frame(chart, &l, (width, height), &annotations);
        let frame = Arc::new(RadialFrame {
            decorations: l.decorations,
            gpu: self.gpu.clone(),
            ledger: Arc::clone(&self.ledger),
            size: (width, height),
            pipeline: self.pipeline.clone(),
            bind_group,
            pick_pipeline: self.pick_pipeline.clone(),
            pick_layout: self.pick_layout.clone(),
            _uniform: uniform,
            sectors,
            _annotations: Arc::clone(&annotations),
            #[cfg(feature = "cpu-twin")]
            geometry: l.sectors,
            targets: l.targets,
            scale,
            space: l.space,
        });
        self.annotations_cached = Some((key, size, scale, fonts, annotations));
        self.cached = Some((FrameKey::of(chart), size, scale, fonts, Arc::clone(&frame)));
        Ok(frame)
    }
    /// The vector export of `chart` at logical `size` and `scale` into
    /// `out` (#48): the frame's sectors and background as one image, from a
    /// frame laid out without labels and built uncached; then the title, the
    /// labels with their leader lines and the split connectors from the
    /// layout's own label emit, and the annotation overlay, as vector
    /// content. Every inside label is drawn above every sector at its lifted
    /// place, and no selection box is drawn.
    pub(crate) async fn write_vector(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
        out: &mut dyn crate::vector_export::VectorOut,
    ) -> Result<(), RadialError> {
        let (width, height) = self.frame_size(chart, size, scale)?;
        let count = chart.slices.len() + chart.split.as_ref().map_or(0, |s| s.children.len());
        self.admit(
            u64::from(width) * u64::from(height) * 4
                + 112
                + count as u64 * std::mem::size_of::<Sector>() as u64,
        )?;
        let l = layout::layout(chart, size, scale, layout::Labels::None)?;
        // A texture starts zeroed: the frame draws no label.
        // gpu-alloc: PanelTexture
        let labels = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("radial vector export without labels"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let labels = TrackedTexture::new(&self.ledger, GpuResourceKind::PanelTexture, labels);
        let (_uniform, _sectors, bind_group) = self.bind_frame(chart, &l, (width, height), &labels);
        let image = self
            .render_rgba((width, height), |pass| {
                pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                pass.set_scissor_rect(0, 0, width, height);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..3, 0..1);
            })
            .await?;
        let failed = |e: crate::FiggyError| RadialError::Gpu(e.to_string());
        out.image([0, 0], &image).map_err(failed)?;
        drop(image);
        let canvas = crate::layout::Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        out.push_clip(canvas);
        let mut sink = crate::vector_export::VectorSink::new(
            out,
            crate::sketch::DecoStroker::Precise,
            crate::text_render::FontPolicy::Standard,
            [0.0, 0.0],
            scale,
            canvas,
        );
        let placed = layout::layout(chart, size, scale, layout::Labels::Into(&mut sink));
        if placed.is_ok() && chart.annotations.visible {
            // The overlay the frame's annotation layer draws
            // (`RadialAnnotationCache`): the radial space has no value axes,
            // so nothing goes below the slices.
            let request = crate::annotation::RasterRequest {
                layer: &chart.annotations,
                space: l.space,
                origin: [0.0, 0.0],
                size,
                scale,
                selected: None,
                policy: crate::text_render::FontPolicy::Standard,
            };
            crate::annotation::emit_annotation_layer(
                &request,
                &mut crate::vector_export::SlotFilter {
                    slot: crate::sink::Slot::Above,
                    inner: &mut sink,
                },
                &mut |_| {},
            );
        }
        let error = sink.error.take();
        drop(sink);
        out.pop_clip();
        placed?;
        error.map_or(Ok(()), |e| Err(failed(e)))
    }

    /// The physical size of `chart`'s frame at logical `size` and `scale`,
    /// after the checks every frame passes: a valid chart whose shown
    /// labels have text, a canvas of at least 160x160 at a scale of 0.5..4,
    /// inside the device's texture limit.
    fn frame_size(
        &self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(u32, u32), RadialError> {
        chart.validate().map_err(RadialError::Invalid)?;
        if !chart.labels_have_text() {
            return Err(RadialError::Invalid(
                "a shown radial label has no text; call RadialChart::format_labels after editing slices, or normalize_labels on a loaded document",
            ));
        }
        if size.0 < 160 || size.1 < 160 || !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return Err(RadialError::Invalid(
                "radial canvas must be at least 160x160; scale must be 0.5..4",
            ));
        }
        let width = (size.0 as f64 * f64::from(scale)).round();
        let height = (size.1 as f64 * f64::from(scale)).round();
        let limit = f64::from(self.gpu.device().limits().max_texture_dimension_2d);
        if width > limit || height > limit {
            return Err(RadialError::Invalid(
                "radial output exceeds the device texture limit",
            ));
        }
        Ok((width as u32, height as u32))
    }

    /// The GPU inputs of one frame of layout `l`: its globals, its sectors
    /// and the bind group that reads them with the label texture `labels`.
    fn bind_frame(
        &self,
        chart: &RadialChart,
        l: &layout::Layout,
        (width, height): (u32, u32),
        labels: &TrackedTexture,
    ) -> (TrackedBuffer, TrackedBuffer, wgpu::BindGroup) {
        let globals = Globals {
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
                if self.format.is_srgb() { 1.0 } else { 0.0 },
                l.holes[0],
                l.holes[1],
            ],
        };
        let device = self.gpu.device();
        // gpu-alloc: Uniform
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radial globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uniform = TrackedBuffer::new(&self.ledger, GpuResourceKind::Uniform, uniform);
        // gpu-alloc: FieldTable
        let sectors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radial sectors"),
            contents: bytemuck::cast_slice(&l.sectors),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let sectors = TrackedBuffer::new(&self.ledger, GpuResourceKind::FieldTable, sectors);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radial frame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sectors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &labels.create_view(&Default::default()),
                    ),
                },
            ],
        });
        (uniform, sectors, bind_group)
    }

    /// Drop internal frame/annotation references. External frames stay valid.
    /// Retired allocation credits are processed through [`Self::end_frame`].
    pub fn clear_cache(&mut self) {
        self.cached = None;
        self.annotations_cached = None;
        self.annotation_layer.clear();
    }
    /// Call after submitting/discarding ALL recorded frames, including old
    /// snapshots. Completion credits are asynchronous, as in Renderer.
    pub fn end_frame(&self) {
        let batch = self.ledger.take_retirement();
        if batch.is_empty() {
            return;
        }
        let ledger = Arc::clone(&self.ledger);
        #[cfg(not(target_arch = "wasm32"))]
        self.gpu
            .queue()
            .on_submitted_work_done(move || ledger.complete_retirement(batch));
        #[cfg(target_arch = "wasm32")]
        {
            let (tx, rx) = futures_channel::oneshot::channel();
            self.gpu.queue().on_submitted_work_done(move || {
                let _ = tx.send(());
            });
            wasm_bindgen_futures::spawn_local(async move {
                if rx.await.is_ok() {
                    ledger.complete_retirement(batch);
                }
            });
        }
    }
    /// Render at the requested resolution and read back straight-alpha RGBA8.
    /// Available on native and WASM; includes the model's current hover/selection.
    /// Use [`crate::encode_png`] to encode the returned image separately.
    pub async fn export_rgba_async(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<RasterImage, RadialError> {
        let frame = self.prepare(chart, size, scale)?;
        let annotations = self.prepare_annotations_for(&frame, chart, scale)?;
        self.render_rgba(frame.size, |pass| {
            frame.draw(pass);
            annotations.draw_overlay(pass);
        })
        .await
    }

    /// Render `draw` into a cleared target of `width x height` physical
    /// pixels in the renderer's format and read it back as straight-alpha
    /// RGBA8 (linear, whatever the format's encoding).
    async fn render_rgba(
        &self,
        (width, height): (u32, u32),
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Result<RasterImage, RadialError> {
        let stride = (u64::from(width) * 4).div_ceil(256) * 256;
        let bytes = stride * u64::from(height);
        if bytes > self.gpu.device().limits().max_buffer_size {
            return Err(RadialError::Invalid(
                "radial readback exceeds the device buffer limit",
            ));
        }
        self.admit(u64::from(width) * u64::from(height) * 4 + bytes)?;
        let device = self.gpu.device();
        // gpu-alloc: ExportTarget
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radial export"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = TrackedTexture::new(&self.ledger, GpuResourceKind::ExportTarget, target);
        // gpu-alloc: Readback
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("radial readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buffer = TrackedBuffer::new(&self.ledger, GpuResourceKind::Readback, buffer);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("radial export"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue().submit([encoder.finish()]);
        let (tx, rx) = futures_channel::oneshot::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        #[cfg(not(target_arch = "wasm32"))]
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| RadialError::Gpu(e.to_string()))?;
        rx.await
            .map_err(|e| RadialError::Gpu(e.to_string()))?
            .map_err(|e| RadialError::Gpu(e.to_string()))?;
        let data = buffer
            .get_mapped_range(..)
            .map_err(|e| RadialError::Gpu(e.to_string()))?;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in data.chunks_exact(stride as usize) {
            rgba.extend_from_slice(&row[..width as usize * 4]);
        }
        drop(data);
        buffer.unmap();
        for pixel in rgba.chunks_exact_mut(4) {
            if matches!(
                self.format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            ) {
                pixel.swap(0, 2);
            }
            if pixel[3] > 0 && pixel[3] < 255 {
                for i in 0..3 {
                    let alpha = f32::from(pixel[3]) / 255.0;
                    let mut c = f32::from(pixel[i]) / 255.0;
                    if self.format.is_srgb() {
                        c = if c <= 0.04045 {
                            c / 12.92
                        } else {
                            ((c + 0.055) / 1.055).powf(2.4)
                        };
                    }
                    c = (c / alpha).clamp(0.0, 1.0);
                    if self.format.is_srgb() {
                        c = if c <= 0.0031308 {
                            c * 12.92
                        } else {
                            1.055 * c.powf(1.0 / 2.4) - 0.055
                        };
                    }
                    pixel[i] = (c * 255.0).round() as u8;
                }
            }
        }
        Ok(RasterImage {
            width,
            height,
            rgba,
        })
    }
    /// Export the chart as an SVG document at logical `size` and `scale`,
    /// with the same checks as [`Self::prepare`]: the slices and background
    /// as one image at `scale ×` resolution, the title, the labels with
    /// their leader lines, the split connectors and the annotation layer as
    /// vector content, glyphs as outlines. The SVG's `viewBox` is in physical
    /// pixels and its size the logical size in points (1 px = 0.75 pt). Every
    /// inside label is drawn above every slice, at its place in the PNG, and
    /// no selection box is drawn.
    pub async fn export_svg_async(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<String, RadialError> {
        self.export_vector::<crate::vector_export::SvgOut>(chart, size, scale)
            .await
    }

    /// [`Self::export_svg_async`] as a one-page PDF: the page is the logical
    /// size in points, the image a deflated RGB image with a soft mask.
    pub async fn export_pdf_async(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Vec<u8>, RadialError> {
        self.export_vector::<crate::vector_export::PdfOut>(chart, size, scale)
            .await
    }

    async fn export_vector<F: crate::vector_export::VectorFile>(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<F::Output, RadialError> {
        let physical = self.frame_size(chart, size, scale)?;
        let mut out = F::create(physical, crate::vector_export::page_points(size));
        self.write_vector(chart, size, scale, &mut out).await?;
        out.finish_file()
            .map_err(|e| RadialError::Gpu(e.to_string()))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Native blocking wrapper for [`Self::export_svg_async`].
    pub fn export_svg(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<String, RadialError> {
        pollster::block_on(self.export_svg_async(chart, size, scale))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Native blocking wrapper for [`Self::export_pdf_async`].
    pub fn export_pdf(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Vec<u8>, RadialError> {
        pollster::block_on(self.export_pdf_async(chart, size, scale))
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// Native blocking wrapper for [`Self::export_rgba_async`].
    pub fn export_rgba(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<RasterImage, RadialError> {
        pollster::block_on(self.export_rgba_async(chart, size, scale))
    }
}

// Only properties read by annotation/layout generation survive. Appearance and
// ordinary data selection do not invalidate the rasterized label texture.
// Edited/selected inside labels are rasterized at their current lifted position.
fn raster_moves_inside_labels(chart: &RadialChart) -> bool {
    let has_inside = chart
        .slices
        .iter()
        .chain(chart.split.iter().flat_map(|s| s.children.iter()))
        .any(|s| s.value > 0.0 && s.labels.unwrap_or(chart.labels) == RadialLabels::Inside);
    has_inside
        && (matches!(
            chart.decorations.selected,
            Some(model::DecorationId::RadialLabel(_))
        ) || chart
            .decorations
            .offsets
            .iter()
            .any(|o| matches!(o.element, model::DecorationId::RadialLabel(_))))
}

/// What a radial frame's pixels, sectors and hit map are built from: the
/// chart's fields that reach them, named one by one (#66). [`Self::of`] and
/// [`Self::matches`] destructure `RadialChart` without a rest pattern, so a
/// new chart field does not compile until it is classified there: taken
/// into the key, or named as not reaching the frame, with the reason. The
/// per-field mutation test (`frame_key_tests`) holds each classification.
#[derive(Clone, Debug, PartialEq)]
struct FrameKey {
    data_area: Option<crate::layout::RectF>,
    offsets: Vec<model::DecorationOffset>,
    /// The decoration selection the frame highlights: an annotation's is
    /// drawn on the annotation overlay instead.
    selected: Option<model::DecorationId>,
    title: String,
    chart_title: Option<model::config::ChartTitleOptions>,
    slices: Vec<RadialSlice>,
    kind: RadialKind,
    start_angle_degrees: f32,
    split: Option<RadialSplit>,
    style: RadialStyle,
    labels: RadialLabels,
    interaction: RadialInteraction,
    font_family: String,
    font_size: f32,
    label_color: crate::Color,
    background: crate::Color,
}
/// A decoration selection as the frame sees it.
fn frame_selection(selected: &Option<model::DecorationId>) -> Option<&model::DecorationId> {
    selected
        .as_ref()
        .filter(|s| crate::annotation::selected_index(Some(s)).is_none())
}
impl FrameKey {
    fn of(chart: &RadialChart) -> Self {
        let RadialChart {
            decorations,
            title,
            chart_title,
            // Routes pointer interaction only; no pixel depends on it.
            active_layer: _,
            // Drawn by its own pass (`RadialAnnotationCache`).
            annotations: _,
            slices,
            kind,
            start_angle_degrees,
            split,
            style,
            labels,
            // The frame draws each slice's `text`; these only write it, at
            // edit time (`RadialChart::format_labels`).
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction,
            font_family,
            font_size,
            label_color,
            background,
        } = chart;
        let model::DecorationLayout {
            data_area,
            offsets,
            selected,
        } = decorations;
        Self {
            data_area: *data_area,
            offsets: offsets.clone(),
            selected: frame_selection(selected).cloned(),
            title: title.clone(),
            chart_title: chart_title.clone(),
            slices: slices.clone(),
            kind: *kind,
            start_angle_degrees: *start_angle_degrees,
            split: split.clone(),
            style: style.clone(),
            labels: *labels,
            interaction: interaction.clone(),
            font_family: font_family.clone(),
            font_size: *font_size,
            label_color: *label_color,
            background: *background,
        }
    }
    /// `*self == Self::of(chart)` without building the key: the per-frame
    /// cache check.
    fn matches(&self, chart: &RadialChart) -> bool {
        let RadialChart {
            decorations,
            title,
            chart_title,
            active_layer: _,
            annotations: _,
            slices,
            kind,
            start_angle_degrees,
            split,
            style,
            labels,
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction,
            font_family,
            font_size,
            label_color,
            background,
        } = chart;
        let model::DecorationLayout {
            data_area,
            offsets,
            selected,
        } = decorations;
        self.data_area == *data_area
            && self.offsets == *offsets
            && self.selected.as_ref() == frame_selection(selected)
            && self.title == *title
            && self.chart_title == *chart_title
            && self.slices == *slices
            && self.kind == *kind
            && self.start_angle_degrees == *start_angle_degrees
            && self.split == *split
            && self.style == *style
            && self.labels == *labels
            && self.interaction == *interaction
            && self.font_family == *font_family
            && self.font_size == *font_size
            && self.label_color == *label_color
            && self.background == *background
    }
}
/// The key of a frame's label raster: its [`FrameKey`] with what only the
/// GPU draws put to fixed values, so appearance edits and ordinary hover
/// reuse the raster.
fn annotation_key(chart: &RadialChart) -> FrameKey {
    fn style(s: &RadialStyle) -> RadialStyle {
        let RadialStyle {
            tilt_degrees,
            depth,
            shadow,
            hover_lift,
            // Surface appearance: GPU work, never on the label raster.
            material: _,
            bevel: _,
            gap_degrees: _,
            roughness: _,
            texture_strength: _,
            texture_scale: _,
            light: _,
            inner_corner: _,
            outer_corner: _,
            outline: _,
            hover_brightness: _,
            gloss: _,
            texture_angle_degrees: _,
        } = s;
        let RadialStyle {
            tilt_degrees: _,
            depth: _,
            shadow: _,
            hover_lift: _,
            material,
            bevel,
            gap_degrees,
            roughness,
            texture_strength,
            texture_scale,
            light,
            inner_corner,
            outer_corner,
            outline,
            hover_brightness,
            gloss,
            texture_angle_degrees,
        } = RadialStyle::default();
        RadialStyle {
            material,
            tilt_degrees: *tilt_degrees,
            depth: *depth,
            bevel,
            gap_degrees,
            roughness,
            texture_strength,
            texture_scale,
            light,
            shadow: *shadow,
            inner_corner,
            outer_corner,
            outline,
            hover_lift: *hover_lift,
            hover_brightness,
            gloss,
            texture_angle_degrees,
        }
    }
    let mut c = FrameKey::of(chart);
    c.style = style(&c.style);
    if !raster_moves_inside_labels(chart) {
        c.interaction = RadialInteraction::default();
    }
    c.background = crate::Color::WHITE;
    for slice in c
        .slices
        .iter_mut()
        .chain(c.split.iter_mut().flat_map(|s| s.children.iter_mut()))
    {
        slice.style = slice.style.as_ref().map(style).filter(|s| s != &c.style);
        if slice.labels.unwrap_or(c.labels) != RadialLabels::Inside || slice.label_color.is_some() {
            slice.color = crate::Color::WHITE;
        }
    }
    c
}

#[cfg(test)]
mod pick_module_tests {
    use super::*;
    /// The draw and the pick splice one sector fragment after the material,
    /// so the pick answers with the draw's own ray-march.
    #[test]
    fn draw_and_pick_splice_the_same_sector_fragment() {
        let shared = [
            "data_render/common/material.wgsl",
            "data_render/common/radial_sector.wgsl",
        ];
        assert_eq!(RADIAL_SHADER.fragments[..2], shared);
        assert_eq!(RADIAL_PICK_SHADER.fragments[..2], shared);
        assert_eq!(RADIAL_SHADER.fragments.len(), 3);
        assert_eq!(RADIAL_PICK_SHADER.fragments.len(), 3);
        // The draw's own file keeps no sector definition, only the markers
        // of the blocks the fragment holds.
        let draw = include_str!("radial.wgsl");
        for definition in ["struct Sector", "fn hit_pixel", "fn footprint", "fn solid"] {
            assert!(
                !draw.lines().any(|line| line.starts_with(definition)),
                "{definition}"
            );
        }
        assert_eq!(
            draw.lines()
                .filter(|line| line.starts_with("// [data_render/common/radial_sector.wgsl:"))
                .count(),
            2
        );
    }
}

#[cfg(test)]
mod frame_key_tests {
    use super::*;
    use model::{DecorationId, DecorationOffset, LayerRef};
    fn base() -> RadialChart {
        let mut chart = RadialChart {
            slices: vec![
                RadialSlice::new("a", 1., crate::Color::WHITE),
                RadialSlice::new("b", 2., crate::Color::BLACK),
            ],
            ..Default::default()
        };
        chart.format_labels();
        chart
    }
    /// Every field of `RadialChart`, mutated once: the frame key moves
    /// exactly for the fields it names, and `matches` agrees with `of`.
    #[test]
    fn every_chart_field_is_in_the_frame_key_or_named_out_of_it() {
        // Exhaustive, like `FrameKey::of`: a new field must get a row here.
        let RadialChart {
            decorations: _,
            title: _,
            chart_title: _,
            active_layer: _,
            annotations: _,
            slices: _,
            kind: _,
            start_angle_degrees: _,
            split: _,
            style: _,
            labels: _,
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction: _,
            font_family: _,
            font_size: _,
            label_color: _,
            background: _,
        } = base();
        type Edit = fn(&mut RadialChart);
        let reaching: &[(&str, Edit)] = &[
            ("decorations.data_area", |c| {
                c.decorations.data_area = Some(crate::layout::RectF {
                    x: 30.,
                    y: 80.,
                    width: 200.,
                    height: 150.,
                })
            }),
            ("decorations.offsets", |c| {
                c.decorations.offsets.push(DecorationOffset {
                    element: DecorationId::ChartTitle,
                    offset: [1., 2.],
                })
            }),
            ("decorations.selected", |c| {
                c.decorations.selected = Some(DecorationId::ChartTitle)
            }),
            ("title", |c| c.title = "T".into()),
            ("chart_title", |c| c.normalize_title()),
            ("slices", |c| c.slices[0].explode = 0.1),
            ("slices.text", |c| c.slices[0].text = Some("x".into())),
            ("kind", |c| c.kind = RadialKind::Donut { inner_radius: 0.5 }),
            ("start_angle_degrees", |c| c.start_angle_degrees = 0.),
            ("split", |c| {
                c.split = Some(RadialSplit {
                    slice_index: 1,
                    kind: RadialKind::Pie,
                    children: vec![RadialSlice::new("c", 2., crate::Color::WHITE)],
                })
            }),
            ("style", |c| c.style.depth = 0.2),
            ("labels", |c| c.labels = RadialLabels::Inside),
            ("interaction", |c| {
                c.interaction.selected = Some(RadialTarget::main(0))
            }),
            ("font_family", |c| c.font_family = "Comic Neue".into()),
            ("font_size", |c| c.font_size = 20.),
            ("label_color", |c| c.label_color = crate::Color::BLACK),
            ("background", |c| c.background = crate::Color::BLACK),
        ];
        let outside: &[(&str, Edit)] = &[
            ("active_layer", |c| c.active_layer = LayerRef::Annotations),
            ("annotations", |c| c.annotations.visible = false),
            ("decorations.selected (an annotation)", |c| {
                c.decorations.selected = Some(DecorationId::Annotation(0))
            }),
            ("label_format", |c| {
                c.label_format = RadialLabelFormat::Value
            }),
            ("label_decimals", |c| c.label_decimals = 3),
            ("value_suffix", |c| c.value_suffix = " kg".into()),
        ];
        // A slice is keyed whole, as the one field `slices`: its name and
        // its own label format ride with it although the frame reads them
        // only through the `text` `format_labels` writes.
        let key = FrameKey::of(&base());
        for (name, edit) in reaching {
            let mut c = base();
            edit(&mut c);
            assert_ne!(FrameKey::of(&c), key, "{name} must reach the frame key");
            assert!(!key.matches(&c), "{name}: matches disagrees with of");
            assert!(FrameKey::of(&c).matches(&c), "{name}");
        }
        for (name, edit) in outside {
            let mut c = base();
            edit(&mut c);
            assert_eq!(FrameKey::of(&c), key, "{name} must not reach the frame key");
            assert!(key.matches(&c), "{name}: matches disagrees with of");
        }
    }
    #[test]
    fn the_label_raster_key_ignores_what_only_the_gpu_draws() {
        let key = annotation_key(&base());
        for edit in [
            |c: &mut RadialChart| c.style.material = crate::Material::Wood,
            |c: &mut RadialChart| c.background = crate::Color::BLACK,
            |c: &mut RadialChart| {
                c.interaction.hovered = Some(RadialTarget::main(0));
                c.interaction.hover_progress = 0.5;
            },
            |c: &mut RadialChart| c.active_layer = LayerRef::Annotations,
        ] {
            let mut c = base();
            edit(&mut c);
            assert_eq!(annotation_key(&c), key);
        }
        let mut c = base();
        c.slices[0].text = Some("x".into());
        assert_ne!(annotation_key(&c), key);
    }
}

#[cfg(test)]
mod layer_cache_key_tests {
    use super::*;
    #[test]
    fn frame_and_annotation_keys_ignore_layer_fields_only() {
        let base = RadialChart {
            slices: vec![RadialSlice::new("a", 1., crate::Color::WHITE)],
            ..Default::default()
        };
        let mut layered = base.clone();
        layered.active_layer = model::LayerRef::Annotations;
        layered.annotations.visible = false;
        assert_eq!(FrameKey::of(&base), FrameKey::of(&layered));
        assert!(FrameKey::of(&base).matches(&layered));
        assert_eq!(annotation_key(&base), annotation_key(&layered));
        let mut edited = base.clone();
        edited.title = "x".into();
        assert_ne!(FrameKey::of(&base), FrameKey::of(&edited));
        assert!(!FrameKey::of(&base).matches(&edited));
        assert_ne!(annotation_key(&base), annotation_key(&edited));
    }
}
