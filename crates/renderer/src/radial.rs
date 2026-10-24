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
pub(crate) const RADIAL_SHADER: crate::shader_assembly::Assembled =
    crate::shader_assembly::assembled!("data_render/common/material.wgsl", "radial.wgsl",);

#[path = "radial_geometry.rs"]
mod geometry;
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
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Sector {
    geometry: [f32; 4],
    arc: [f32; 4],
    color: [f32; 4],
    material: crate::data_render::MaterialGpu,
    detail: [f32; 4], // explode, angular gap, bevel, reserved (0, seed moved to `material`)
    rounding: [f32; 4], // inner corner, outer corner, vertical lift, reserved (0, gloss moved)
    outline: [f32; 4],
    outline_color: [f32; 4],
    // reserved (0, highlight moved), reserved (0, texture angle moved),
    // inside-label flag, inside-label lift: how far the lifted top face moves
    // up the screen in physical pixels (`rounding[2] * sin(arc[2]) *
    // geometry[2]`), computed once where the sector is built
    // (`radial_layout.rs`). The layout moves the inside labels or their hit
    // map by it and the shader moves the inside glyphs by it.
    effects: [f32; 4],
    light: [f32; 4],
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

/// Immutable GPU snapshot. Its shader draws straight into a same-format,
/// single-sample target; four analytic subpixel samples provide edge AA.
/// Labels are rasterized at the requested output resolution, never upscaled.
pub struct RadialFrame {
    decorations: crate::HitMap<RadialChart>,
    _gpu: RendererDevice,
    size: (u32, u32),
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    _uniform: TrackedBuffer,
    _sectors: TrackedBuffer,
    _annotations: Arc<TrackedTexture>,
    geometry: Vec<Sector>,
    targets: Vec<RadialTarget>,
    scale: f32,
    space: crate::annotation::PixelSpace,
    background: crate::annotation::FrameBackground,
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
    /// pixels: the plot area and the two document axes.
    pub fn annotation_space(&self) -> crate::annotation::PixelSpace {
        self.space
    }
    /// How this frame treats the chart background.
    pub fn background(&self) -> crate::annotation::FrameBackground {
        self.background
    }
    /// Original footprint for hover hysteresis: retain the previous hover while
    /// the pointer is in its unraised footprint, even if the lifted surface moved away.
    pub fn hit_test_at_rest(&self, position: [f32; 2]) -> Option<RadialTarget> {
        self.pick(position, true)
    }
    /// Visible surface picking in logical pixels, using this immutable snapshot.
    pub fn hit_test(&self, position: [f32; 2]) -> Option<RadialTarget> {
        self.pick(position, false)
    }
    fn pick(&self, position: [f32; 2], rest: bool) -> Option<RadialTarget> {
        if position.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let pixel = [position[0] * self.scale, position[1] * self.scale];
        if pixel[0] < 0.0
            || pixel[1] < 0.0
            || pixel[0] >= self.size.0 as f32
            || pixel[1] >= self.size.1 as f32
        {
            return None;
        }
        self.geometry
            .iter()
            .zip(&self.targets)
            .filter_map(|(s, t)| {
                let mut s = *s;
                if rest {
                    s.rounding[2] = 0.0;
                }
                geometry::hit(&s, pixel).map(|depth| (depth, *t))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t)| t)
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
/// Shares the host device/queue. Owns one pipeline and at most one cached frame.
/// Kept separate from the large numeric-data/Cartesian renderer deliberately.
pub struct RadialRenderer {
    gpu: RendererDevice,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    ledger: Arc<GpuLedger>,
    budget: u64,
    annotations_cached: Option<(RadialChart, (u32, u32), f32, u64, Arc<TrackedTexture>)>,
    /// The dedicated annotation layer, drawn around the frame by its own pass.
    annotation_layer: crate::annotation::BoundedAnnotationCache,
    background: crate::annotation::FrameBackground,
    /// One frame per background mode, so an export that needs `HostClear`
    /// beside an interactive `Opaque` frame does not evict it.
    cached: Vec<(RadialChart, (u32, u32), f32, u64, Arc<RadialFrame>)>,
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
        Ok(Self {
            gpu,
            format,
            layout,
            pipeline,
            // host-alloc: W1-a
            ledger: Arc::new(GpuLedger::new()),
            budget: 256 * 1024 * 1024,
            cached: Vec::new(),
            annotations_cached: None,
            annotation_layer: crate::annotation::BoundedAnnotationCache::new(format),
            background: Default::default(),
        })
    }
    /// How frames treat the chart background. `Opaque` (default) paints it in
    /// the frame's own draw; `HostClear` leaves it to the host's clear colour
    /// so an annotation underlay drawn before the frame shows through. A
    /// host that switches to `HostClear` clears with `chart.background`.
    pub fn set_background(&mut self, background: crate::annotation::FrameBackground) {
        self.background = background;
    }
    pub fn background(&self) -> crate::annotation::FrameBackground {
        self.background
    }
    /// The annotation layer of `chart` for the frame [`Self::prepare`]
    /// returns for the same arguments: two textures and their hit map, cached
    /// like the frame. Draw it around the frame (`draw_underlay`, the frame,
    /// `draw_overlay`); when it draws an underlay the frame must be prepared
    /// with [`crate::annotation::FrameBackground::HostClear`].
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
        self.prepare_frame(chart, size, scale, self.background)
    }
    fn prepare_frame(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
        background: crate::annotation::FrameBackground,
    ) -> Result<Arc<RadialFrame>, RadialError> {
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
        let fonts = crate::text_render::font_generation();
        if let Some((_, _, _, _, frame)) =
            self.cached.iter().find(|(c, s, k, generation, frame)| {
                frame_eq(c, chart)
                    && *s == size
                    && *k == scale
                    && *generation == fonts
                    && frame.background == background
            })
        {
            return Ok(Arc::clone(frame));
        }
        let width = (size.0 as f64 * f64::from(scale)).round();
        let height = (size.1 as f64 * f64::from(scale)).round();
        let limit = f64::from(self.gpu.device().limits().max_texture_dimension_2d);
        if width > limit || height > limit {
            return Err(RadialError::Invalid(
                "radial output exceeds the device texture limit",
            ));
        }
        let (width, height) = (width as u32, height as u32);
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
        let l = layout::layout(chart, size, scale, reused.is_none())?;
        let globals = Globals {
            size: [width as f32, height as f32, l.sectors.len() as f32, 2.0],
            background: match background {
                crate::annotation::FrameBackground::Opaque => [
                    chart.background.r,
                    chart.background.g,
                    chart.background.b,
                    chart.background.a,
                ],
                // Written as transparent: the frame leaves the host clear.
                crate::annotation::FrameBackground::HostClear => [0.0; 4],
            },
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
                        &annotations.create_view(&Default::default()),
                    ),
                },
            ],
        });
        let frame = Arc::new(RadialFrame {
            decorations: l.decorations,
            _gpu: self.gpu.clone(),
            size: (width, height),
            pipeline: self.pipeline.clone(),
            bind_group,
            _uniform: uniform,
            _sectors: sectors,
            _annotations: Arc::clone(&annotations),
            geometry: l.sectors,
            targets: l.targets,
            scale,
            space: l.space,
            background,
        });
        self.annotations_cached = Some((key, size, scale, fonts, annotations));
        self.cached
            .retain(|(_, _, _, _, cached)| cached.background != background);
        self.cached
            .push((frame_key(chart), size, scale, fonts, Arc::clone(&frame)));
        Ok(frame)
    }
    /// Drop internal frame/annotation references. External frames stay valid.
    /// Retired allocation credits are processed through [`Self::end_frame`].
    pub fn clear_cache(&mut self) {
        self.cached.clear();
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
        // An underlay only shows through a frame that leaves the background to
        // the clear colour; the export clears with the chart background then.
        let host_clear = annotations.draws_underlay()
            || frame.background == crate::annotation::FrameBackground::HostClear;
        let frame =
            if host_clear && frame.background != crate::annotation::FrameBackground::HostClear {
                self.prepare_frame(
                    chart,
                    size,
                    scale,
                    crate::annotation::FrameBackground::HostClear,
                )?
            } else {
                frame
            };
        let clear = if host_clear {
            wgpu::Color {
                r: f64::from(chart.background.r),
                g: f64::from(chart.background.g),
                b: f64::from(chart.background.b),
                a: f64::from(chart.background.a),
            }
        } else {
            wgpu::Color::TRANSPARENT
        };
        let (width, height) = frame.size;
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
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            annotations.draw_underlay(&mut pass);
            frame.draw(&mut pass);
            annotations.draw_overlay(&mut pass);
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

/// The part of the chart this frame's pixels, geometry and hit map depend on.
/// `active_layer` only routes interaction and the annotation layer is drawn
/// by its own pass, so editing either must reuse the cached frame and its
/// label raster rather than rebuild them.
fn frame_key(chart: &RadialChart) -> RadialChart {
    let mut c = chart.clone();
    c.active_layer = Default::default();
    c.annotations = Default::default();
    // Selecting an annotation draws its box on the annotation overlay.
    if crate::annotation::selected_index(c.decorations.selected.as_ref()).is_some() {
        c.decorations.selected = None;
    }
    c
}
/// `decorations` compared as `frame_key` normalizes them.
fn decorations_eq(a: &model::DecorationLayout, b: &model::DecorationLayout) -> bool {
    let base = |d: &model::DecorationLayout| {
        d.selected
            .clone()
            .filter(|s| crate::annotation::selected_index(Some(s)).is_none())
    };
    a.data_area == b.data_area && a.offsets == b.offsets && base(a) == base(b)
}
/// `frame_key(a) == frame_key(b)` without cloning, for the per-frame cache
/// hit check. Exhaustive so a new field must be classified here.
fn frame_eq(a: &RadialChart, b: &RadialChart) -> bool {
    let RadialChart {
        decorations,
        title,
        chart_title,
        active_layer,
        annotations,
        slices,
        kind,
        start_angle_degrees,
        split,
        style,
        labels,
        label_format,
        label_decimals,
        value_suffix,
        interaction,
        font_family,
        font_size,
        label_color,
        background,
    } = a;
    let _ = (active_layer, annotations);
    decorations_eq(decorations, &b.decorations)
        && title == &b.title
        && chart_title == &b.chart_title
        && slices == &b.slices
        && kind == &b.kind
        && start_angle_degrees == &b.start_angle_degrees
        && split == &b.split
        && style == &b.style
        && labels == &b.labels
        && label_format == &b.label_format
        && label_decimals == &b.label_decimals
        && value_suffix == &b.value_suffix
        && interaction == &b.interaction
        && font_family == &b.font_family
        && font_size == &b.font_size
        && label_color == &b.label_color
        && background == &b.background
}
fn annotation_key(chart: &RadialChart) -> RadialChart {
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
    let mut c = frame_key(chart);
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
        assert_eq!(frame_key(&base), frame_key(&layered));
        assert!(frame_eq(&frame_key(&base), &layered));
        assert_eq!(annotation_key(&base), annotation_key(&layered));
        let mut edited = base.clone();
        edited.title = "x".into();
        assert_ne!(frame_key(&base), frame_key(&edited));
        assert!(!frame_eq(&frame_key(&base), &edited));
        assert_ne!(annotation_key(&base), annotation_key(&edited));
    }
}
