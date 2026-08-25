//! Contour labels — where each label goes, and the atlas it is drawn from.
//!
//! There is no contour *tracing* here. The implicit form replaces marching
//! squares: `field_columnar.wgsl::fs_contour` draws the
//! isolines as the level set of the same bilinear interpolation the fill uses,
//! from the analytic gradient, so no segment list exists to trace, buffer, or
//! bound. What remains is the part that needs an *explicit* position — a label.
//!
//! Two halves, both one-directional (no GPU→CPU readback, no ticket):
//!
//! - **CPU**: bake each level's string into one cell of a 2D label atlas
//!   (`axis_render::bake_contour_label_atlas`). The content comes from `levels`,
//!   which is config, so it is known before the frame starts.
//! - **GPU**: `contour_anchor.wgsl` seeds a lattice over the data area and Newton
//!   projects each seed onto its level's isoline, then a single serial pass keeps
//!   a separated subset. A per-level fallback may keep a closer candidate rather
//!   than omit a level. `contour_label.wgsl` draws one quad per kept anchor.
//!
//! No atomic operation appears anywhere in the anchor pass. That is what makes
//! placement identical on every device and every run — the earlier design picked
//! bucket winners by `atomicMin` over `atomicAdd`-assigned segment indices, which
//! was only stable because a software rasterizer happens to run serially.

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
type GpuRef<T> = Arc<T>;
#[cfg(target_arch = "wasm32")]
type GpuRef<T> = Rc<T>;

use crate::error::{FiggyError, Result};
use crate::gpu_memory::{
    ChargeTally, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer, charged_buffer_init,
};

/// Vertices per label instance — one quad. GPU twin: the corner table in
/// `contour_label.wgsl::vs_main`, and `AnchorParams.label_vertices`.
pub const LABEL_VERTICES: u32 = 6;

/// Anchors `anchor_select` may keep, for the whole series.
///
/// A ceiling, and also the size of the workgroup arrays the select pass compares
/// against — GPU twin: `contour_anchor.wgsl::MAX_KEPT`. Automatic and explicit
/// placement share the model's full 1024-level capacity.
pub const MAX_CONTOUR_LABELS_TOTAL: u32 = 1024;

/// Candidate slots the project pass may write: `lattice cells x levels`.
///
/// The select pass walks them serially in one invocation, so this is what bounds
/// that walk. A declaration with many levels gets a coarser seed lattice rather
/// than an unbounded scan — see [`ContourLabelState::anchor_plan`], which is also
/// where the level count is capped so the product cannot overflow this buffer.
pub const MAX_ANCHOR_CANDIDATES: u32 = 4096;

const _: () = assert!(MAX_ANCHOR_CANDIDATES >= crate::data_config::MAX_CONTOUR_LEVELS as u32);

/// Seed lattice cells along one screen axis. A `spacing_px` finer than this gets
/// the finest lattice the candidate budget allows.
pub const ANCHOR_LATTICE_MAX: u32 = 32;

/// How many seed lattice cells fit across one `spacing_px`.
///
/// The lattice is **not** the spacing — it is the candidate supply. At one cell
/// per `spacing_px` a level's isoline offers only two or three positions, so when
/// the nearest one is too close to another level's label the select pass has
/// nothing else to fall back on and every level ends up stacked at the same place.
/// Oversampling gives it real choice. The normal selection sweep enforces the
/// target minimum distance; the per-level fallback may keep a closer candidate
/// rather than omit that level.
pub const ANCHOR_LATTICE_OVERSAMPLE: f32 = 3.0;

/// Anchor-pass parameters. GPU twin: `contour_anchor.wgsl::AnchorParams`, 64 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AnchorParamsGpu {
    pub lattice_x: u32,
    pub lattice_y: u32,
    pub level_count: u32,
    pub kept_capacity: u32,
    /// Chart area in the panel's pixel frame — what `axis_pair_to_t`'s `t` spans.
    pub area_x: f32,
    pub area_y: f32,
    pub area_w: f32,
    pub area_h: f32,
    /// Data area in the same frame — where the seeds go.
    pub clip_x: f32,
    pub clip_y: f32,
    pub clip_w: f32,
    pub clip_h: f32,
    pub spacing_px: f32,
    /// Atlas cell height in pixels, for the overlap test.
    pub label_h_px: f32,
    pub label_vertices: u32,
    pub _pad0: u32,
}

const _: () = assert!(std::mem::size_of::<AnchorParamsGpu>() == 64);

/// What the anchor pass may walk this frame: the seed lattice, and how many
/// levels fit the candidate buffer alongside it.
///
/// `lattice_x * lattice_y * levels <= MAX_ANCHOR_CANDIDATES` is the invariant the
/// project pass' slot arithmetic depends on, and
/// [`ContourLabelState::anchor_plan`] is the only place it is established.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AnchorPlan {
    pub lattice_x: u32,
    pub lattice_y: u32,
    pub levels: u32,
}

/// One label placement. GPU twin: `contour_anchor.wgsl::LabelAnchor` and the
/// instance attributes of `contour_label.wgsl::vs_main`, 32 B.
///
/// The SSoT twin is `model::data_config::ContourLabelAnchor`: a host override
/// lands in this same record, which is what keeps one draw path for both.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LabelAnchorGpu {
    /// Position as the pool's own `(hi, lo)` f32 pair.
    pub x: [f32; 2],
    pub y: [f32; 2],
    /// Data-space tangent. The render shader projects it through the live
    /// transform, so this record does not go stale on a pan.
    pub dir: [f32; 2],
    /// Index into the atlas' grid cells, i.e. into `ContourConfig::levels`.
    pub level: u32,
    /// Measured atlas-cell content width in pixels. Keeping it on the selected
    /// anchor makes the label quad and the contour gap consume one exact value.
    pub width_px: f32,
}

const _: () = assert!(std::mem::size_of::<LabelAnchorGpu>() == 32);

/// Vertex-input contract shared by the production wgpu pipeline and the
/// browser's asynchronous pipeline prewarm. Keeping the browser descriptor
/// derived from this array prevents WebGPU-only boot failures when the anchor
/// record gains another field.
pub(crate) const CONTOUR_LABEL_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 16,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 24,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 28,
        shader_location: 4,
    },
];

/// Label draw parameters. GPU twin: `contour_label.wgsl::LabelParams`, 32 B.
///
/// Every member is a 4 B scalar in both layouts. The Rust twin is explicitly
/// 16 B aligned to match WebGPU's uniform-structure requirement; 32 B contains
/// no implicit member or trailing padding.
#[repr(C, align(16))]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LabelParamsGpu {
    pub atlas_w: f32,
    pub atlas_h: f32,
    pub cell_stride_w: f32,
    pub cell_stride_h: f32,
    pub columns: u32,
    pub rows: u32,
    pub gutter: f32,
    pub level_count: u32,
}

const _: () = assert!(std::mem::size_of::<LabelParamsGpu>() == 32);
const _: () = assert!(std::mem::align_of::<LabelParamsGpu>() == 16);

/// Everything the label feature compiles once per device: the two anchor compute
/// pipelines, and the module and layouts the per-target label render pipeline is
/// built from.
///
/// One struct because the two halves are inseparable — the draw has nothing to
/// place without the anchor pass, and the anchor pass has nothing to feed without
/// the draw.
pub struct ContourLabelPipelines {
    anchor_project: wgpu::ComputePipeline,
    anchor_select: wgpu::ComputePipeline,
    /// `Transform`, visible to compute. The render side reuses the renderer's own
    /// transform layout instead, so the label draw binds the very `transform_bg`
    /// every other data primitive binds.
    anchor_transform_bgl: wgpu::BindGroupLayout,
    anchor_io_bgl: wgpu::BindGroupLayout,
    label_module: wgpu::ShaderModule,
    label_bgl: wgpu::BindGroupLayout,
    /// Group 3 of the labelled contour fragment entry. It reads the exact
    /// selected anchors and atlas cell geometry so the line is omitted under
    /// the quads that will be drawn immediately afterwards.
    label_gap_bgl: wgpu::BindGroupLayout,
}

impl ContourLabelPipelines {
    /// `field_bgl` is the field's group-2 layout, bound unchanged at index 2 of
    /// the anchor pipelines: the projection reads z through the same
    /// `locate`/`grid_value` the fragment shader does (one SSoT block), so a label
    /// cannot land on a grid the lines were not drawn from.
    pub fn new(device: &wgpu::Device, field_bgl: &wgpu::BindGroupLayout) -> Self {
        let anchor_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figgy contour anchor shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("contour_anchor.wgsl").into()),
        });
        let uniform = |binding: u32, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = |binding: u32, read_only: bool, visibility: wgpu::ShaderStages| {
            wgpu::BindGroupLayoutEntry {
                binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let anchor_transform_bgl =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy contour anchor transform bgl"),
                entries: &[uniform(0, compute)],
            });
        let anchor_io_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy contour anchor io bgl"),
            entries: &[
                uniform(0, compute),
                storage(1, false, compute), // candidates
                storage(2, false, compute), // kept anchors
                storage(3, false, compute), // label draw-indirect args
                storage(4, true, compute),  // per-level label widths
            ],
        });
        let anchor_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy contour anchor layout"),
            bind_group_layouts: &[
                Some(&anchor_transform_bgl),
                Some(&anchor_io_bgl),
                Some(field_bgl),
            ],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, label: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&anchor_layout),
                module: &anchor_module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let label_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy contour label bgl"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let label_gap_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy contour label gap bgl"),
            entries: &[
                storage(0, true, wgpu::ShaderStages::FRAGMENT), // selected anchors
                storage(1, true, wgpu::ShaderStages::FRAGMENT), // draw args / count
                uniform(3, wgpu::ShaderStages::FRAGMENT),       // atlas cell geometry
            ],
        });
        Self {
            anchor_project: pipeline("anchor_project", "figgy contour anchor project pipeline"),
            anchor_select: pipeline("anchor_select", "figgy contour anchor select pipeline"),
            anchor_transform_bgl,
            anchor_io_bgl,
            label_module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy contour label shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("contour_label.wgsl").into()),
            }),
            label_bgl,
            label_gap_bgl,
        }
    }

    pub(crate) fn label_gap_bgl(&self) -> &wgpu::BindGroupLayout {
        &self.label_gap_bgl
    }

    /// The label draw, compiled against one target format. Called only from the
    /// renderer's single pipeline-ensure site, so the export path picks it up for
    /// free after the submitted frame has finished using it.
    pub fn render_pipeline(
        &self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy contour label layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(&self.label_bgl)],
            immediate_size: 0,
        });
        // One instance per anchor: the 32 B record, read straight out of the
        // buffer the anchor pass (or a host override) wrote.
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("figgy contour label pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.label_module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<LabelAnchorGpu>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &CONTOUR_LABEL_VERTEX_ATTRIBUTES,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: crate::data_render::multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: &self.label_module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // The atlas is tiny-skia output, which is premultiplied, so
                    // the baked colour and background arrive as drawn.
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }
}

/// One contour series' immutable label atlas plus its placement policy.
///
/// Automatic placement caches immutable results by their complete dispatch
/// input. A different input always gets different buffers; this remains safe
/// even after a prepared token is dropped while a host-owned command buffer
/// that references its old placement has not been submitted yet. Explicit
/// placement is immutable and therefore has one shared result.
pub struct ContourLabelState {
    atlas: GpuRef<ContourLabelAtlasState>,
    placement: ContourLabelPlacementState,
}

struct ContourLabelAtlasState {
    bind_group: wgpu::BindGroup,
    cell_h_px: f32,
    cell_w_buf: wgpu::Buffer,
    params_buf: wgpu::Buffer,
    _atlas: crate::gpu_memory::TrackedTexture,
    _charge: SharedCharge,
}

enum ContourLabelPlacementState {
    Automatic(Vec<AutomaticPlacementCacheEntry>),
    Explicit(GpuRef<ContourLabelPlacementSlot>),
}

pub(crate) const AUTOMATIC_PLACEMENT_CACHE_LIMIT: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
struct AutomaticPlacementKey {
    transform_bits: [u32; std::mem::size_of::<crate::data_render::ScatterTransform>() / 4],
    params_bits: [u32; std::mem::size_of::<AnchorParamsGpu>() / 4],
}

impl AutomaticPlacementKey {
    fn new(transform: &crate::data_render::ScatterTransform, params: AnchorParamsGpu) -> Self {
        Self {
            transform_bits: bytemuck::cast(*transform),
            params_bits: bytemuck::cast(params),
        }
    }
}

struct AutomaticPlacementCacheEntry {
    key: AutomaticPlacementKey,
    placement: GpuRef<ContourLabelPlacementSlot>,
}

struct ContourLabelPlacementSlot {
    anchors: GpuRef<wgpu::Buffer>,
    indirect: GpuRef<wgpu::Buffer>,
    label_gap_bg: wgpu::BindGroup,
    automatic: Option<AutomaticPlacementResources>,
    _charge: SharedCharge,
}

struct AutomaticPlacementResources {
    anchor_params_buf: wgpu::Buffer,
    transform_buf: wgpu::Buffer,
    anchor_transform_bg: wgpu::BindGroup,
    anchor_io_bg: wgpu::BindGroup,
    _cand: wgpu::Buffer,
}

/// One occurrence's complete label draw snapshot. The atlas and placement slot
/// owners keep every bind-group dependency and its ledger charge alive until the
/// last prepared token drops.
#[derive(Clone)]
pub(crate) struct ContourLabelSnapshot {
    atlas: GpuRef<ContourLabelAtlasState>,
    placement: GpuRef<ContourLabelPlacementSlot>,
}

impl ContourLabelSnapshot {
    pub(crate) fn bind_group(&self) -> &wgpu::BindGroup {
        &self.atlas.bind_group
    }

    pub(crate) fn anchors(&self) -> &wgpu::Buffer {
        &self.placement.anchors
    }

    pub(crate) fn indirect(&self) -> &wgpu::Buffer {
        &self.placement.indirect
    }

    pub(crate) fn label_gap_bind_group(&self) -> &wgpu::BindGroup {
        &self.placement.label_gap_bg
    }

    #[cfg(test)]
    pub(crate) fn placement_identity(&self) -> *const () {
        GpuRef::as_ptr(&self.placement).cast()
    }
}

/// The CPU-baked label atlas: one uniform-stride grid cell per level, with a
/// transparent texel gutter and a per-level content width.
pub struct LabelAtlas<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
    /// Drawn label height inside each cell, excluding the gutter.
    pub cell_h: f32,
    pub cell_stride_w: u32,
    pub cell_stride_h: u32,
    pub columns: u32,
    pub rows: u32,
    pub gutter: u32,
    /// Content width of each level's cell, in texels. One entry per level.
    pub cell_w: &'a [f32],
}

impl ContourLabelState {
    /// Upload the immutable atlas. Explicit anchors are uploaded once into an
    /// immutable placement snapshot; automatic results are allocated lazily per
    /// distinct dispatch input.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        pipelines: &ContourLabelPipelines,
        sampler: &wgpu::Sampler,
        max_texture_dimension_2d: u32,
        atlas: LabelAtlas<'_>,
        explicit_anchors: Option<&[LabelAnchorGpu]>,
    ) -> Result<Self> {
        let tally = ChargeTally::new();
        let explicit_cell_w = atlas.cell_w;
        let cell_w_buf = charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy contour label cell widths"),
                contents: bytemuck::cast_slice(if atlas.cell_w.is_empty() {
                    &[0.0f32][..]
                } else {
                    atlas.cell_w
                }),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let params = LabelParamsGpu {
            atlas_w: atlas.width as f32,
            atlas_h: atlas.height as f32,
            cell_stride_w: atlas.cell_stride_w as f32,
            cell_stride_h: atlas.cell_stride_h as f32,
            columns: atlas.columns,
            rows: atlas.rows,
            gutter: atlas.gutter as f32,
            level_count: u32::try_from(atlas.cell_w.len()).unwrap_or(u32::MAX),
        };
        let params_buf = charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy contour label params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );

        let atlas_texture = crate::gpu_memory::TrackedTexture::new(
            ledger,
            GpuResourceKind::ContourScratch,
            crate::data_render::upload_rgba_texture(
                device,
                queue,
                max_texture_dimension_2d,
                atlas.width,
                atlas.height,
                atlas.rgba,
            )?,
        );
        let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy contour label bg"),
            layout: &pipelines.label_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let atlas = GpuRef::new(ContourLabelAtlasState {
            bind_group,
            cell_h_px: atlas.cell_h,
            cell_w_buf,
            params_buf,
            _atlas: atlas_texture,
            _charge: crate::gpu_memory::shared_charge(
                tally,
                ledger,
                GpuResourceKind::ContourScratch,
            ),
        });
        let placement = match explicit_anchors {
            Some(anchors) => {
                ContourLabelPlacementState::Explicit(ContourLabelPlacementSlot::build_explicit(
                    device,
                    queue,
                    ledger,
                    pipelines,
                    &atlas.params_buf,
                    explicit_cell_w,
                    anchors,
                ))
            }
            None => ContourLabelPlacementState::Automatic(Vec::new()),
        };
        Ok(Self { atlas, placement })
    }

    /// How many seed lattice cells and how many levels the anchor pass may use.
    ///
    /// The lattice saturates at [`ANCHOR_LATTICE_MAX`] and then shrinks so that
    /// `cells * levels` fits [`MAX_ANCHOR_CANDIDATES`] — the select pass walks
    /// every candidate serially in one invocation, so a thousand-level
    /// declaration must get a coarser lattice rather than an unbounded scan.
    ///
    /// **The level count is capped defensively too.** Shrinking the lattice bottoms out at
    /// 1 x 1, and past that the only remaining lever is how many levels are
    /// covered: `4096` levels each need a candidate slot even with one seed
    /// apiece. Without this the product overflowed the candidate buffer and the
    /// project pass wrote past its end (bounds-checked by the shader, so not
    /// unsound — but the surplus levels silently corrupted the ones that fit).
    /// The model's 1024-level contract is lower than this 4096-slot arithmetic
    /// ceiling, so a validated series never loses a level here.
    ///
    /// The 2D atlas independently validates that all label cells fit the
    /// device's texture dimension. This function only bounds the candidate pass.
    pub fn anchor_plan(w: f32, h: f32, pitch: f32, level_count: u32) -> AnchorPlan {
        let cell = pitch / ANCHOR_LATTICE_OVERSAMPLE;
        let axis = |extent: f32| {
            if cell.is_nan() || cell <= 0.0 || extent.is_nan() || extent <= 0.0 {
                return 1;
            }
            let wanted = (extent / cell).round();
            if !wanted.is_finite() || wanted < 1.0 {
                1
            } else {
                (wanted as u32).min(ANCHOR_LATTICE_MAX)
            }
        };
        let mut lattice_x = axis(w);
        let mut lattice_y = axis(h);
        let budget = (MAX_ANCHOR_CANDIDATES / level_count.max(1)).max(1);
        while lattice_x.saturating_mul(lattice_y) > budget {
            if lattice_x >= lattice_y && lattice_x > 1 {
                lattice_x -= 1;
            } else if lattice_y > 1 {
                lattice_y -= 1;
            } else {
                break;
            }
        }
        let cells = lattice_x.saturating_mul(lattice_y).max(1);
        AnchorPlan {
            lattice_x,
            lattice_y,
            levels: level_count.min(MAX_ANCHOR_CANDIDATES / cells),
        }
    }

    pub fn is_automatic(&self) -> bool {
        matches!(self.placement, ContourLabelPlacementState::Automatic(_))
    }

    pub fn cell_h_px(&self) -> f32 {
        self.atlas.cell_h_px
    }

    pub(crate) fn explicit_snapshot(&self) -> Option<ContourLabelSnapshot> {
        let ContourLabelPlacementState::Explicit(placement) = &self.placement else {
            return None;
        };
        Some(ContourLabelSnapshot {
            atlas: GpuRef::clone(&self.atlas),
            placement: GpuRef::clone(placement),
        })
    }

    /// Return the immutable result for this exact dispatch input, creating and
    /// dispatching it once on a miss. No existing placement buffer is rewritten:
    /// a host may have recorded it into a command buffer after dropping the
    /// corresponding [`ContourLabelSnapshot`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch_automatic(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &ContourLabelPipelines,
        transform: &crate::data_render::ScatterTransform,
        params: AnchorParamsGpu,
        field_bg: &wgpu::BindGroup,
    ) -> ContourLabelSnapshot {
        let ContourLabelPlacementState::Automatic(slots) = &mut self.placement else {
            unreachable!("explicit contour placement is never dispatched");
        };
        let key = AutomaticPlacementKey::new(transform, params);
        if let Some(hit) = slots.iter().find(|entry| entry.key == key) {
            return ContourLabelSnapshot {
                atlas: GpuRef::clone(&self.atlas),
                placement: GpuRef::clone(&hit.placement),
            };
        }

        let placement = ContourLabelPlacementSlot::build_automatic(
            device,
            ledger,
            pipelines,
            &self.atlas.cell_w_buf,
            &self.atlas.params_buf,
        );
        placement.dispatch(queue, encoder, pipelines, transform, params, field_bg);
        if slots.len() >= AUTOMATIC_PLACEMENT_CACHE_LIMIT {
            slots.remove(0);
        }
        slots.push(AutomaticPlacementCacheEntry {
            key,
            placement: GpuRef::clone(&placement),
        });
        ContourLabelSnapshot {
            atlas: GpuRef::clone(&self.atlas),
            placement,
        }
    }

    #[cfg(test)]
    pub(crate) fn automatic_result_count(&self) -> usize {
        match &self.placement {
            ContourLabelPlacementState::Automatic(slots) => slots.len(),
            ContourLabelPlacementState::Explicit(_) => 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn atlas_identity(&self) -> usize {
        GpuRef::as_ptr(&self.atlas) as usize
    }
}

impl ContourLabelPlacementSlot {
    fn build_explicit(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        pipelines: &ContourLabelPipelines,
        params_buf: &wgpu::Buffer,
        cell_w: &[f32],
        anchors: &[LabelAnchorGpu],
    ) -> GpuRef<Self> {
        let tally = ChargeTally::new();
        let (anchors_buf, indirect) = Self::build_draw_buffers(&tally, device);
        let n = anchors.len().min(MAX_CONTOUR_LABELS_TOTAL as usize);
        if n > 0 {
            let mut resolved = anchors[..n].to_vec();
            for anchor in &mut resolved {
                anchor.width_px = cell_w.get(anchor.level as usize).copied().unwrap_or(0.0);
            }
            queue.write_buffer(&anchors_buf, 0, bytemuck::cast_slice(&resolved));
        }
        queue.write_buffer(
            &indirect,
            0,
            bytemuck::cast_slice(&[LABEL_VERTICES, n as u32, 0, 0]),
        );
        let label_gap_bg = Self::build_label_gap_bind_group(
            device,
            pipelines,
            &anchors_buf,
            &indirect,
            params_buf,
        );
        GpuRef::new(Self {
            anchors: GpuRef::new(anchors_buf),
            indirect: GpuRef::new(indirect),
            label_gap_bg,
            automatic: None,
            _charge: crate::gpu_memory::shared_charge(
                tally,
                ledger,
                GpuResourceKind::ContourScratch,
            ),
        })
    }

    fn build_automatic(
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        pipelines: &ContourLabelPipelines,
        cell_w_buf: &wgpu::Buffer,
        params_buf: &wgpu::Buffer,
    ) -> GpuRef<Self> {
        let tally = ChargeTally::new();
        let record = std::mem::size_of::<LabelAnchorGpu>() as u64;
        // gpu-alloc: ContourScratch
        let cand = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy contour anchor candidates"),
                size: (MAX_ANCHOR_CANDIDATES as u64) * record,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
        );
        let (anchors, indirect) = Self::build_draw_buffers(&tally, device);
        // gpu-alloc: ContourScratch
        let anchor_params_buf = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy contour anchor params"),
                size: std::mem::size_of::<AnchorParamsGpu>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        // gpu-alloc: ContourScratch
        let transform_buf = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy contour anchor transform uniform"),
                size: std::mem::size_of::<crate::data_render::ScatterTransform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let anchor_transform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy contour anchor transform bg"),
            layout: &pipelines.anchor_transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buf.as_entire_binding(),
            }],
        });
        let anchor_io_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy contour anchor io bg"),
            layout: &pipelines.anchor_io_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: anchor_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cand.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: anchors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: indirect.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: cell_w_buf.as_entire_binding(),
                },
            ],
        });
        let label_gap_bg =
            Self::build_label_gap_bind_group(device, pipelines, &anchors, &indirect, params_buf);
        GpuRef::new(Self {
            anchors: GpuRef::new(anchors),
            indirect: GpuRef::new(indirect),
            label_gap_bg,
            automatic: Some(AutomaticPlacementResources {
                anchor_params_buf,
                transform_buf,
                anchor_transform_bg,
                anchor_io_bg,
                _cand: cand,
            }),
            _charge: crate::gpu_memory::shared_charge(
                tally,
                ledger,
                GpuResourceKind::ContourScratch,
            ),
        })
    }

    fn build_label_gap_bind_group(
        device: &wgpu::Device,
        pipelines: &ContourLabelPipelines,
        anchors: &wgpu::Buffer,
        indirect: &wgpu::Buffer,
        params_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy contour label gap bg"),
            layout: pipelines.label_gap_bgl(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: anchors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: indirect.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        })
    }

    fn build_draw_buffers(
        tally: &ChargeTally,
        device: &wgpu::Device,
    ) -> (wgpu::Buffer, wgpu::Buffer) {
        let record = std::mem::size_of::<LabelAnchorGpu>() as u64;
        // gpu-alloc: ContourScratch
        let anchors = charged_buffer(
            tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy contour label anchors"),
                size: (MAX_CONTOUR_LABELS_TOTAL as u64) * record,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        // gpu-alloc: ContourScratch
        let indirect = charged_buffer(
            tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy contour label indirect args"),
                size: 16,
                usage: wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        );
        (anchors, indirect)
    }

    fn dispatch(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &ContourLabelPipelines,
        transform: &crate::data_render::ScatterTransform,
        params: AnchorParamsGpu,
        field_bg: &wgpu::BindGroup,
    ) {
        let automatic = self
            .automatic
            .as_ref()
            .expect("automatic contour slot has compute resources");
        queue.write_buffer(&automatic.transform_buf, 0, bytemuck::bytes_of(transform));
        queue.write_buffer(&automatic.anchor_params_buf, 0, bytemuck::bytes_of(&params));
        let total = params
            .lattice_x
            .saturating_mul(params.lattice_y)
            .saturating_mul(params.level_count);
        if total == 0 {
            // Still clear the args, or a previous frame's count would draw labels
            // for anchors that are no longer there.
            queue.write_buffer(&self.indirect, 0, bytemuck::cast_slice(&[0u32; 4]));
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy contour label anchors"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &automatic.anchor_transform_bg, &[]);
        pass.set_bind_group(1, &automatic.anchor_io_bg, &[]);
        pass.set_bind_group(2, field_bg, &[]);
        // One pass, in order: `anchor_select` reads what `anchor_project` wrote.
        // `total` is capped by `MAX_ANCHOR_CANDIDATES`, so the workgroup count
        // never approaches the per-dimension limit.
        pass.set_pipeline(&pipelines.anchor_project);
        pass.dispatch_workgroups(total.div_ceil(64).max(1), 1, 1);
        pass.set_pipeline(&pipelines.anchor_select);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

/// A `Vec` sized by `try_reserve_exact`, for the small per-level tables.
pub(crate) fn try_collect<T: Copy>(
    resource: &'static str,
    source: impl ExactSizeIterator<Item = T>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(source.len().max(1))
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource,
            reason: error.to_string(),
        })?;
    out.extend(source);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contour_label_vertex_contract_matches_anchor_record_and_shader() {
        assert_eq!(std::mem::size_of::<LabelAnchorGpu>(), 32);
        assert_eq!(CONTOUR_LABEL_VERTEX_ATTRIBUTES.len(), 5);

        let expected = [
            (wgpu::VertexFormat::Float32x2, 0, 0),
            (wgpu::VertexFormat::Float32x2, 8, 1),
            (wgpu::VertexFormat::Float32x2, 16, 2),
            (wgpu::VertexFormat::Uint32, 24, 3),
            (wgpu::VertexFormat::Float32, 28, 4),
        ];
        for (attribute, (format, offset, location)) in
            CONTOUR_LABEL_VERTEX_ATTRIBUTES.iter().zip(expected)
        {
            assert_eq!(attribute.format, format);
            assert_eq!(attribute.offset, offset);
            assert_eq!(attribute.shader_location, location);
        }

        let shader = include_str!("contour_label.wgsl");
        for input in [
            "@location(0) a_x: vec2<f32>",
            "@location(1) a_y: vec2<f32>",
            "@location(2) a_dir: vec2<f32>",
            "@location(3) a_level: u32",
            "@location(4) a_width_px: f32",
        ] {
            assert!(
                shader.contains(input),
                "contour label shader lost vertex input `{input}`"
            );
        }
    }

    #[test]
    fn label_params_layout_matches_the_wgsl_uniform() {
        assert_eq!(std::mem::size_of::<LabelParamsGpu>(), 32);
        assert_eq!(std::mem::align_of::<LabelParamsGpu>(), 16);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, atlas_w), 0);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, atlas_h), 4);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, cell_stride_w), 8);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, cell_stride_h), 12);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, columns), 16);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, rows), 20);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, gutter), 24);
        assert_eq!(std::mem::offset_of!(LabelParamsGpu, level_count), 28);
    }

    #[test]
    fn label_shader_maps_levels_to_grid_cells_and_clamps_texel_centres() {
        let shader = include_str!("contour_label.wgsl");
        for contract in [
            "let column = a_level % lp.columns;",
            "let row = a_level / lp.columns;",
            "let sample_lo = content_origin + vec2<f32>(0.5);",
            "let sample_hi = min(content_sample_hi, cell_sample_hi);",
            "let sample_px = clamp(",
            "textureSampleLevel(atlas, atlas_samp, in.uv, 0.0)",
        ] {
            assert!(
                shader.contains(contract),
                "contour label shader lost atlas contract: {contract}"
            );
        }

        let params = LabelParamsGpu {
            atlas_w: 1024.0,
            atlas_h: 1024.0,
            cell_stride_w: 32.0,
            cell_stride_h: 24.0,
            columns: 32,
            rows: 32,
            gutter: 1.0,
            level_count: 1024,
        };
        let last = params.level_count - 1;
        assert_eq!(last % params.columns, 31);
        assert_eq!(last / params.columns, 31);

        let first_content_max_x = params.cell_stride_w - params.gutter - 0.5;
        let next_content_min_x = params.cell_stride_w + params.gutter + 0.5;
        assert!(
            first_content_max_x < next_content_min_x,
            "linear-filter sample intervals of adjacent cells must be disjoint"
        );
    }

    /// The candidate buffer's bound holds for **every** level count.
    ///
    /// The project pass writes `cand[level * cells + cell]`, so
    /// `cells * levels` past `MAX_ANCHOR_CANDIDATES` runs off the end. The
    /// lattice shrink alone does not get there: it bottoms out at 1 x 1, and a
    /// declaration with more levels than the whole buffer has slots overflows
    /// anyway — which is what this missed before `anchor_plan` capped the level
    /// count too.
    #[test]
    fn an_anchor_plan_never_outgrows_the_candidate_buffer() {
        let sizes = [(1.0f32, 1.0f32), (672.0, 270.0), (4096.0, 2160.0)];
        let pitches = [1.0f32, 60.0, 120.0, 4000.0];
        let levels = [
            0u32,
            1,
            4,
            100,
            1000,
            MAX_ANCHOR_CANDIDATES - 1,
            MAX_ANCHOR_CANDIDATES,
            MAX_ANCHOR_CANDIDATES + 1,
            20_000,
            u32::MAX,
        ];
        for (w, h) in sizes {
            for pitch in pitches {
                for level_count in levels {
                    let plan = ContourLabelState::anchor_plan(w, h, pitch, level_count);
                    let cells = plan.lattice_x as u64 * plan.lattice_y as u64;
                    assert!(
                        cells >= 1,
                        "{w}x{h} @ {pitch} with {level_count} levels planned an empty lattice"
                    );
                    assert!(
                        cells * plan.levels as u64 <= MAX_ANCHOR_CANDIDATES as u64,
                        "{w}x{h} @ {pitch} with {level_count} levels planned \
                         {cells} cells x {} levels, past the {MAX_ANCHOR_CANDIDATES} \
                         candidate slots",
                        plan.levels
                    );
                    assert!(
                        plan.levels <= level_count,
                        "the plan invented levels the declaration does not have"
                    );
                }
            }
        }
    }

    /// A declaration small enough to fit keeps every one of its levels — the cap
    /// is a ceiling, not a policy that quietly trims ordinary charts.
    #[test]
    fn an_ordinary_declaration_keeps_every_level() {
        for level_count in [1u32, 4, 12, 40] {
            let plan = ContourLabelState::anchor_plan(672.0, 270.0, 120.0, level_count);
            assert_eq!(
                plan.levels, level_count,
                "{level_count} levels should all be planned for"
            );
            assert!(
                plan.lattice_x > 1 && plan.lattice_y > 1,
                "an ordinary chart should still get a lattice with room to choose: \
                 {}x{}",
                plan.lattice_x,
                plan.lattice_y
            );
        }
    }
}
