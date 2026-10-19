#![doc = include_str!("../RADAR.md")]
//! Resident radar (spider) charts: a few series over a fixed set of
//! categories, drawn as closed polygons over one shared value axis. No
//! Cartesian columns, auto-fit jobs, or stream replay are involved.
//! `prepare` returns an immutable frame; repeated identical snapshots reuse it.
use crate::gpu_memory::{
    GpuLedger, GpuMemoryUsage, GpuResourceKind, TrackedBuffer, TrackedTexture,
};
use crate::{RasterImage, RendererDevice};
pub use model::radar::*;
use std::sync::Arc;
use wgpu::util::DeviceExt;
#[path = "radar_layout.rs"]
mod layout;

/// Why a radar chart could not be prepared or exported.
#[derive(Debug)]
pub enum RadarError {
    /// `RadarChart::validate` failed with this reason.
    Invalid(&'static str),
    /// The frame's GPU resources would exceed the renderer's budget.
    Budget {
        /// Bytes the frame needs.
        needed: u64,
        /// The budget's limit in bytes.
        limit: u64,
    },
    /// A GPU error, such as a failed readback.
    Gpu(String),
}
impl std::fmt::Display for RadarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Budget { needed, limit } => {
                write!(f, "radar GPU budget: {needed} bytes needed, limit {limit}")
            }
            Self::Gpu(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for RadarError {}

/// One quad of the radar shader: a polygon fill, an outline segment or a
/// vertex marker, in physical pixels. Twin of `radar.wgsl::Prim`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Prim {
    kind: [f32; 4],   // kind, vertex offset, vertex count, emphasis
    rect: [f32; 4],   // left, top, right, bottom
    points: [f32; 4], // segment a.xy b.xy | marker center.xy radius 0
    color: [f32; 4],  // straight RGBA
    stroke: [f32; 4], // half width (segment), 0, 0, 0
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    size: [f32; 4],
    background: [f32; 4],
    clip: [f32; 4],
}
const KIND_POLYGON: f32 = 0.0;
const KIND_SEGMENT: f32 = 1.0;
const KIND_CIRCLE: f32 = 2.0;
const KIND_SQUARE: f32 = 3.0;
const EMPTY_PRIM: Prim = Prim {
    kind: [0.0; 4],
    rect: [0.0; 4],
    points: [0.0; 4],
    color: [0.0; 4],
    stroke: [0.0; 4],
};
const _: () = assert!(std::mem::size_of::<Prim>() == 80 && std::mem::size_of::<Globals>() == 48);

/// Which drawn part of a series a hit landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RadarPart {
    /// The polygon interior.
    Fill,
    /// The outline between two vertices.
    Outline,
    /// A vertex marker; the target names the vertex.
    Marker,
}
/// What `RadarFrame::hit_test` found: the vertex or series and the part
/// the pointer landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadarPick {
    /// The vertex (`category_id` set) or series under the pointer.
    pub target: RadarTarget,
    /// The drawn part that was hit.
    pub part: RadarPart,
}

/// Signed distance to the closed polygon `vertices`: negative inside. Twin of
/// `radar.wgsl::polygon_sdf`.
fn polygon_distance(vertices: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = vertices.len();
    let mut inside = false;
    let mut d = f32::INFINITY;
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        let e = [b[0] - a[0], b[1] - a[1]];
        let q = [p[0] - a[0], p[1] - a[1]];
        let t =
            ((q[0] * e[0] + q[1] * e[1]) / (e[0] * e[0] + e[1] * e[1]).max(1e-20)).clamp(0.0, 1.0);
        d = d.min((q[0] - t * e[0]).hypot(q[1] - t * e[1]));
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * e[0] / e[1] {
            inside = !inside;
        }
    }
    if inside { -d } else { d }
}
fn distance(prim: &Prim, vertices: &[[f32; 2]], p: [f32; 2]) -> f32 {
    if prim.kind[0] == KIND_POLYGON {
        let offset = prim.kind[1] as usize;
        let count = prim.kind[2] as usize;
        return polygon_distance(&vertices[offset..offset + count], p);
    }
    if prim.kind[0] == KIND_SEGMENT {
        let [ax, ay, bx, by] = prim.points;
        let e = [bx - ax, by - ay];
        let q = [p[0] - ax, p[1] - ay];
        let t =
            ((q[0] * e[0] + q[1] * e[1]) / (e[0] * e[0] + e[1] * e[1]).max(1e-20)).clamp(0.0, 1.0);
        return (q[0] - t * e[0]).hypot(q[1] - t * e[1]) - prim.stroke[0];
    }
    let d = [p[0] - prim.points[0], p[1] - prim.points[1]];
    if prim.kind[0] == KIND_SQUARE {
        return d[0].abs().max(d[1].abs()) - prim.points[2];
    }
    d[0].hypot(d[1]) - prim.points[2]
}

/// Immutable GPU snapshot. Its shader draws straight into a same-format,
/// single-sample target; four analytic subpixel samples provide edge AA.
/// Labels are rasterized at the requested output resolution, never upscaled.
pub struct RadarFrame {
    decorations: crate::decoration::DecorationMap,
    _gpu: RendererDevice,
    size: (u32, u32),
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    _uniform: TrackedBuffer,
    _prims: TrackedBuffer,
    _vertices: TrackedBuffer,
    _annotations: Arc<TrackedTexture>,
    geometry: Vec<Prim>,
    vertices: Vec<[f32; 2]>,
    targets: Vec<RadarPick>,
    scale: f32,
    clip: [f32; 4],
    center: [f32; 2],
    radius: f32,
    space: crate::annotation::PixelSpace,
    background: crate::annotation::FrameBackground,
}
impl RadarFrame {
    /// Renderer-measured decorations using the common HitMap/Draggable contract.
    pub fn decorations(&self) -> &crate::decoration::DecorationMap {
        &self.decorations
    }
    /// True when annotation GPU storage is shared (e.g. hover-only edits).
    pub fn shares_annotations_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self._annotations, &other._annotations)
    }
    /// The anchor space of this frame for the annotation layer, in logical
    /// pixels: the square around the outer ring, ratio-anchored only.
    pub fn annotation_space(&self) -> crate::annotation::PixelSpace {
        self.space
    }
    /// How this frame treats the chart background.
    pub fn background(&self) -> crate::annotation::FrameBackground {
        self.background
    }
    /// Pick a visible component in chart-local logical pixels: vertex
    /// markers first, then outlines, then polygon interiors, each in reverse
    /// draw order so the series drawn last wins a tie.
    pub fn hit_test(&self, position: [f32; 2]) -> Option<RadarPick> {
        if position.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let p = [position[0] * self.scale, position[1] * self.scale];
        if p[0] < self.clip[0]
            || p[1] < self.clip[1]
            || p[0] >= self.clip[2]
            || p[1] >= self.clip[3]
        {
            return None;
        }
        for part in [RadarPart::Marker, RadarPart::Outline, RadarPart::Fill] {
            let slack = if part == RadarPart::Fill {
                0.0
            } else {
                1.5 * self.scale
            };
            if let Some((_, target)) = self
                .geometry
                .iter()
                .zip(&self.targets)
                .rev()
                .filter(|(prim, t)| t.part == part && prim.color[3] > 0.0)
                .find(|(prim, _)| distance(prim, &self.vertices, p) <= slack)
            {
                return Some(target.clone());
            }
        }
        None
    }
    /// Physical bounds of one rendered component.
    pub fn part_rect(&self, target: &RadarPick) -> Option<[f32; 4]> {
        self.targets
            .iter()
            .position(|t| t == target)
            .map(|i| self.geometry[i].rect)
    }
    /// Physical position of one vertex marker.
    pub fn vertex_position(&self, target: &RadarTarget) -> Option<[f32; 2]> {
        self.targets
            .iter()
            .position(|t| t.part == RadarPart::Marker && &t.target == target)
            .map(|i| [self.geometry[i].points[0], self.geometry[i].points[1]])
    }
    /// The plot area (physical pixels) the series are clipped to.
    pub fn plot_rect(&self) -> [f32; 4] {
        self.clip
    }
    /// The outer ring: centre and radius in physical pixels.
    pub fn plot_circle(&self) -> ([f32; 2], f32) {
        (self.center, self.radius)
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
        pass.draw(0..6, 0..self.geometry.len() as u32 + 2);
    }
}
/// Shares the host device/queue. Owns one pipeline and at most one cached frame.
/// Kept separate from the large numeric-data/Cartesian renderer deliberately.
pub struct RadarRenderer {
    gpu: RendererDevice,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    ledger: Arc<GpuLedger>,
    budget: u64,
    annotations_cached: Option<(RadarChart, (u32, u32), f32, u64, Arc<TrackedTexture>)>,
    /// The dedicated annotation layer, drawn around the frame by its own pass.
    annotation_layer: crate::annotation::BoundedAnnotationCache,
    background: crate::annotation::FrameBackground,
    /// One frame per background mode, so an export that needs `HostClear`
    /// beside an interactive `Opaque` frame does not evict it.
    cached: Vec<(RadarChart, (u32, u32), f32, u64, Arc<RadarFrame>)>,
}
impl RadarRenderer {
    /// Initialize browser pipelines through WebGPU's asynchronous compiler.
    /// Uses the checked production pipeline descriptors before publishing the
    /// wgpu-owned pipeline. Native hosts use the ordinary synchronous constructor.
    pub async fn new_async(
        gpu: RendererDevice,
        format: wgpu::TextureFormat,
    ) -> Result<Self, RadarError> {
        #[cfg(target_arch = "wasm32")]
        crate::browser_compile::render(
            gpu.device(),
            include_str!("radar.wgsl"),
            &[("radar surfaces", "radar surfaces")],
            format,
            1,
            None,
            &mut |_| {},
        )
        .await
        .map_err(RadarError::Gpu)?;
        Self::new(gpu, format)
    }

    /// Share the host device/queue. Supports RGBA8/BGRA8, linear or sRGB targets.
    /// Starts with a separate 256 MiB resource budget, not the Cartesian ledger.
    pub fn new(gpu: RendererDevice, format: wgpu::TextureFormat) -> Result<Self, RadarError> {
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(RadarError::Invalid("radar target must be RGBA8 or BGRA8"));
        }
        let device = gpu.device();
        let storage = |binding, min: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(min),
            },
            count: None,
        };
        let layout = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("radar chart"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(48),
                        },
                        count: None,
                    },
                    storage(1, 80),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    storage(3, 8),
                ],
            },
        );
        let pl = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("radar chart"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            },
        );
        let shader = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("radar surfaces"),
                source: wgpu::ShaderSource::Wgsl(include_str!("radar.wgsl").into()),
            },
        );
        let pipeline = crate::gpu_compile::render_pipeline(
            &device,
            &wgpu::RenderPipelineDescriptor {
                label: Some("radar surfaces"),
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
    /// How the next frame's background is cleared; see [`Self::set_background`].
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
        chart: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadarError> {
        let frame = self.prepare(chart, size, scale)?;
        self.prepare_annotations_for(&frame, chart, scale)
    }
    fn prepare_annotations_for(
        &mut self,
        frame: &RadarFrame,
        chart: &RadarChart,
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadarError> {
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
    fn admit(&self, extra: u64) -> Result<(), RadarError> {
        let needed = self.ledger.total_bytes().saturating_add(extra);
        if needed > self.budget {
            Err(RadarError::Budget {
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
    /// Logical dimensions must be at least 240 by 240; physical output must fit the
    /// device texture limit. Model edits need no manual cache invalidation.
    pub fn prepare(
        &mut self,
        chart: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<RadarFrame>, RadarError> {
        self.prepare_frame(chart, size, scale, self.background)
    }
    fn prepare_frame(
        &mut self,
        chart: &RadarChart,
        size: (u32, u32),
        scale: f32,
        background: crate::annotation::FrameBackground,
    ) -> Result<Arc<RadarFrame>, RadarError> {
        chart.validate().map_err(RadarError::Invalid)?;
        if size.0 < 240 || size.1 < 240 || !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return Err(RadarError::Invalid(
                "radar canvas must be at least 240x240; scale must be 0.5..4",
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
        if width > limit || height * 2.0 > limit {
            return Err(RadarError::Invalid(
                "radar output exceeds the device texture limit",
            ));
        }
        let (width, height) = (width as u32, height as u32);
        // Per series: one fill, one segment and one marker per category.
        let count = (chart.series.len() * (2 * chart.categories.len() + 1)).max(1);
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
                u64::from(width) * u64::from(height) * 8
            } + 48
                + count as u64 * 80
                + (chart.series.len() * chart.categories.len()).max(1) as u64 * 8,
        )?;
        let l = layout::layout(chart, size, scale, reused.is_none())?;
        let globals = Globals {
            size: [
                width as f32,
                height as f32,
                l.prims.len() as f32,
                if self.format.is_srgb() { 1.0 } else { 0.0 },
            ],
            clip: l.clip,
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
        };
        let device = self.gpu.device();
        // gpu-alloc: Uniform
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uniform = TrackedBuffer::new(&self.ledger, GpuResourceKind::Uniform, uniform);
        // gpu-alloc: FieldTable
        let prims = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar primitives"),
            contents: bytemuck::cast_slice(if l.prims.is_empty() {
                std::slice::from_ref(&EMPTY_PRIM)
            } else {
                &l.prims
            }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let prims = TrackedBuffer::new(&self.ledger, GpuResourceKind::FieldTable, prims);
        // gpu-alloc: FieldTable
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar polygon vertices"),
            contents: bytemuck::cast_slice(if l.vertices.is_empty() {
                &[[0.0f32; 2]]
            } else {
                l.vertices.as_slice()
            }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let vertices = TrackedBuffer::new(&self.ledger, GpuResourceKind::FieldTable, vertices);
        let annotations = if let Some(texture) = reused {
            texture
        } else {
            // gpu-alloc: PanelTexture
            let annotations = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("radar labels at output resolution"),
                size: wgpu::Extent3d {
                    width,
                    height: height * 2,
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
                    rows_per_image: Some(height * 2),
                },
                wgpu::Extent3d {
                    width,
                    height: height * 2,
                    depth_or_array_layers: 1,
                },
            );
            Arc::new(annotations)
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radar frame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: prims.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        &annotations.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: vertices.as_entire_binding(),
                },
            ],
        });
        let frame = Arc::new(RadarFrame {
            decorations: l.decorations,
            _gpu: self.gpu.clone(),
            size: (width, height),
            pipeline: self.pipeline.clone(),
            bind_group,
            _uniform: uniform,
            _prims: prims,
            _vertices: vertices,
            _annotations: Arc::clone(&annotations),
            geometry: l.prims,
            vertices: l.vertices,
            targets: l.targets,
            scale,
            clip: l.clip,
            center: l.center,
            radius: l.radius,
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
        chart: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<RasterImage, RadarError> {
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
            return Err(RadarError::Invalid(
                "radar readback exceeds the device buffer limit",
            ));
        }
        self.admit(u64::from(width) * u64::from(height) * 4 + bytes)?;
        let device = self.gpu.device();
        // gpu-alloc: ExportTarget
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("radar export"),
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
            label: Some("radar readback"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buffer = TrackedBuffer::new(&self.ledger, GpuResourceKind::Readback, buffer);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("radar export"),
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
            .map_err(|e| RadarError::Gpu(e.to_string()))?;
        rx.await
            .map_err(|e| RadarError::Gpu(e.to_string()))?
            .map_err(|e| RadarError::Gpu(e.to_string()))?;
        let data = buffer
            .get_mapped_range(..)
            .map_err(|e| RadarError::Gpu(e.to_string()))?;
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
        chart: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<RasterImage, RadarError> {
        pollster::block_on(self.export_rgba_async(chart, size, scale))
    }
}

/// The part of the chart this frame's pixels, geometry and hit map depend on.
/// `active_layer` only routes interaction and the annotation layer is drawn
/// by its own pass, so editing either must reuse the cached frame and its
/// label raster rather than rebuild them.
fn frame_key(chart: &RadarChart) -> RadarChart {
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
fn frame_eq(a: &RadarChart, b: &RadarChart) -> bool {
    let RadarChart {
        decorations,
        title,
        chart_title,
        active_layer,
        annotations,
        categories,
        series,
        style,
        hovered,
        selected,
        labels,
        label_decimals,
        value_suffix,
        value_range,
        ring_spacing,
        grid,
        legend,
        font_family,
        font_size,
        text_color,
        background,
    } = a;
    let _ = (active_layer, annotations);
    decorations_eq(decorations, &b.decorations)
        && title == &b.title
        && chart_title == &b.chart_title
        && categories == &b.categories
        && series == &b.series
        && style == &b.style
        && hovered == &b.hovered
        && selected == &b.selected
        && labels == &b.labels
        && label_decimals == &b.label_decimals
        && value_suffix == &b.value_suffix
        && value_range == &b.value_range
        && ring_spacing == &b.ring_spacing
        && grid == &b.grid
        && legend == &b.legend
        && font_family == &b.font_family
        && font_size == &b.font_size
        && text_color == &b.text_color
        && background == &b.background
}
/// The part of the chart the label raster depends on: everything the frame
/// depends on except hover, selection, the background and the series
/// appearance, none of which moves a glyph. `RadarLabels::Emphasized` labels
/// follow the emphasized series, so hover and selection stay in the key
/// then.
fn annotation_key(chart: &RadarChart) -> RadarChart {
    let mut c = frame_key(chart);
    if c.labels != RadarLabels::Emphasized {
        c.hovered = None;
        c.selected = None;
    }
    c.background = crate::Color::WHITE;
    let legend_colors: Vec<_> = c.series.iter().map(|s| s.color).collect();
    // The raster holds the rings and spokes; the series primitives are GPU
    // work and their style never changes the raster.
    c.style = RadarStyle {
        start_angle_degrees: c.style.start_angle_degrees,
        clockwise: c.style.clockwise,
        grid_shape: c.style.grid_shape,
        spoke_color: c.style.spoke_color,
        ring_color: c.style.ring_color,
        ..RadarStyle::default()
    };
    for (s, color) in c.series.iter_mut().zip(legend_colors) {
        s.style = None;
        // The legend swatch is painted on the raster.
        s.color = color;
    }
    c
}

#[cfg(test)]
mod layer_cache_key_tests {
    use super::*;
    #[test]
    fn frame_and_annotation_keys_ignore_layer_fields_only() {
        let base = RadarChart::default();
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
    #[test]
    fn polygon_distance_is_negative_inside_and_positive_outside() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(polygon_distance(&square, [5.0, 5.0]) < 0.0);
        assert!((polygon_distance(&square, [5.0, 5.0]) + 5.0).abs() < 1e-5);
        assert!((polygon_distance(&square, [15.0, 5.0]) - 5.0).abs() < 1e-5);
        assert!(polygon_distance(&square, [-3.0, -4.0]) > 4.9);
    }
}
