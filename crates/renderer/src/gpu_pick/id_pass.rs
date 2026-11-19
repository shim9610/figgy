//! The on-demand pick ID pass (#381 prototype, option b).
//!
//! A pick draws the panel's point and line series once more, only when it
//! is requested, into a 1×1 `Rgba32Uint` target with a 1×1 `Depth32Float`
//! target: the cursor's pixel. The draw pipelines are untouched. Each
//! primitive is one triangle covering the target; its fragment measures the
//! primitive at the cursor and writes `(kind + 1 | covering, series slot,
//! point index, primitive index)` through the shared pick ID output
//! (`data_render/common/pick_id_output.wgsl`), whose depth rule resolves
//! overlaps on the GPU: a covering hit beats every near one, the topmost
//! covering hit wins, then the nearest near one.
//!
//! There is one module per column kind and no variant: the module is the
//! exact marker-line module (`PickShader::MarkerLine`'s fragments) with the
//! ID entries in place of the exact compute entry, compiled with the default
//! `PICK_VARIANT` (every path). Two fragment geometries are built so the
//! prototype can measure both:
//! - [`IdPickGeometry::Exact`]: the exact module's own candidate functions,
//!   so the ID pass agrees with the exact picker by construction;
//! - [`IdPickGeometry::Simple`]: a disc of the resolved marker radius and a
//!   capsule of the stroke's half width, the style-free kernel per column
//!   kind.
//!
//! The registry, the per-series query parameters and the style rows are the
//! point picker's; the pass binds them through its own layouts, whose
//! entries are visible to the render stages.

use super::engine::create_bind_group_checked;
use super::*;
use crate::shader_assembly::{AssembledBySet, ShaderSet, assembled_by_set};

/// The ID target's format: the 3D surface pick's (`SURFACE_PICK_FORMAT`).
pub(crate) const ID_PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
/// The ID pass's depth format.
pub(crate) const ID_PICK_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// `PICK_ID_COVERING` in `pick_id_output.wgsl`.
const ID_PICK_COVERING: u32 = 0x100;

/// The ID pick module: the exact marker-line module's fragments, the shared
/// pick ID input and output, and the ID entries.
pub(crate) const ID_PICK_MODULE: (&str, AssembledBySet) = (
    "figgy GPU pick ID shader",
    assembled_by_set!(
        "gpu_pick/core.wgsl",
        set,
        "data_render/common/projection.wgsl",
        "data_render/common/sample_validity.wgsl",
        "data_render/common/ramp.wgsl",
        "data_render/common/grid_pair.wgsl",
        "data_render/common/arc_pair.wgsl",
        "data_render/common/sketch_noise.wgsl",
        "data_render/common/category_jitter.wgsl",
        "data_render/common/errorbar_segment.wgsl",
        "data_render/common/line_stroke.wgsl",
        "data_render/common/line_step.wgsl",
        "data_render/common/area_piece.wgsl",
        "data_render/common/arrow.wgsl",
        "data_render/common/path_arrow.wgsl",
        "data_render/common/marker.wgsl",
        "gpu_pick/marker.wgsl",
        "gpu_pick/errorbar.wgsl",
        "gpu_pick/line.wgsl",
        "gpu_pick/area.wgsl",
        "data_render/common/pick_id_input.wgsl",
        "data_render/common/pick_id_output.wgsl",
        "gpu_pick/id_entries.wgsl",
        "gpu_pick/id_segments.wgsl",
        "gpu_pick/id_simple.wgsl",
    ),
);

/// The ID pick module of point vector series: the exact vector module's
/// fragments (its `pick_scatter_candidate` measures the arrows), the shared
/// pick ID input and output, and the ID entries.
pub(crate) const ID_PICK_VECTOR_MODULE: (&str, AssembledBySet) = (
    "figgy GPU pick ID vector shader",
    assembled_by_set!(
        "gpu_pick/core.wgsl",
        set,
        "data_render/common/projection.wgsl",
        "data_render/common/sample_validity.wgsl",
        "data_render/common/ramp.wgsl",
        "data_render/common/grid_pair.wgsl",
        "data_render/common/arrow.wgsl",
        "gpu_pick/vector.wgsl",
        "gpu_pick/no_line.wgsl",
        "data_render/common/pick_id_input.wgsl",
        "data_render/common/pick_id_output.wgsl",
        "gpu_pick/id_entries.wgsl",
    ),
);

/// Which geometry the ID pass's fragments measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdPickGeometry {
    /// The exact picker's candidate functions: marker shapes, dashes, steps,
    /// styled strokes, error bars, stems, areas and arrowheads.
    Exact,
    /// A disc of the resolved marker radius and a capsule of the stroke's
    /// half width: the style-free kernel per column kind.
    Simple,
}

/// The column kinds the prototype draws: points (markers), polylines
/// (segments from a point to the next) and point vectors (an arrow per
/// anchor, which has one geometry: its exact arrow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IdPickKind {
    Points = 0,
    Segments = 1,
    Vectors = 2,
}

/// Pipeline slots: points and segments in both geometries, then vectors.
const ID_PICK_SLOTS: usize = 5;

impl IdPickKind {
    fn vertex_entry(self) -> &'static str {
        match self {
            IdPickKind::Points | IdPickKind::Vectors => "pick_id_vs_points",
            IdPickKind::Segments => "pick_id_vs_segments",
        }
    }

    fn fragment_entry(self, geometry: IdPickGeometry) -> &'static str {
        match (self, geometry) {
            (IdPickKind::Points | IdPickKind::Vectors, IdPickGeometry::Exact)
            | (IdPickKind::Vectors, IdPickGeometry::Simple) => "pick_id_fs_points",
            (IdPickKind::Segments, IdPickGeometry::Exact) => "pick_id_fs_segments",
            (IdPickKind::Points, IdPickGeometry::Simple) => "pick_id_fs_points_disc",
            (IdPickKind::Segments, IdPickGeometry::Simple) => "pick_id_fs_segments_capsule",
        }
    }

    /// The module (0 marker-line, 1 vector) and the pipeline slot.
    fn slot(self, geometry: IdPickGeometry) -> (usize, usize) {
        match self {
            IdPickKind::Vectors => (1, 4),
            _ => (
                0,
                self as usize * 2 + usize::from(geometry == IdPickGeometry::Simple),
            ),
        }
    }
}

const fn render_storage(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const VERTEX_FRAGMENT: wgpu::ShaderStages = wgpu::ShaderStages::VERTEX_FRAGMENT;

/// Group 0: the column pool.
const ID_DATA_ENTRIES: [wgpu::BindGroupLayoutEntry; 1] = [render_storage(0, VERTEX_FRAGMENT)];
/// Group 1: the query parameters and the style slot and override rows.
const ID_WORK_ENTRIES: [wgpu::BindGroupLayoutEntry; 3] = [
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    render_storage(1, wgpu::ShaderStages::FRAGMENT),
    render_storage(2, wgpu::ShaderStages::FRAGMENT),
];
/// Group 2: the arc prefix and the ring starts.
const ID_ARC_ENTRIES: [wgpu::BindGroupLayoutEntry; 2] = [
    render_storage(0, wgpu::ShaderStages::FRAGMENT),
    render_storage(1, wgpu::ShaderStages::FRAGMENT),
];

/// The ID pass's layouts and pipelines, one module per shader set and one
/// pipeline per column kind and geometry, each created on first use.
pub struct IdPickPipelines {
    set: ShaderSet,
    data_bgl: wgpu::BindGroupLayout,
    work_bgl: wgpu::BindGroupLayout,
    arc_bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    modules: [Option<wgpu::ShaderModule>; 2],
    pipelines: [Option<wgpu::RenderPipeline>; ID_PICK_SLOTS],
}

impl IdPickPipelines {
    pub(crate) fn new(device: &wgpu::Device, set: ShaderSet) -> Self {
        let bgl = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            crate::gpu_compile::bind_group_layout(
                device,
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                },
            )
        };
        let data_bgl = bgl("figgy GPU pick ID data bgl", &ID_DATA_ENTRIES);
        let work_bgl = bgl("figgy GPU pick ID work bgl", &ID_WORK_ENTRIES);
        let arc_bgl = bgl("figgy GPU pick ID arc bgl", &ID_ARC_ENTRIES);
        let layout = crate::gpu_compile::pipeline_layout(
            device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy GPU pick ID layout"),
                bind_group_layouts: &[Some(&data_bgl), Some(&work_bgl), Some(&arc_bgl)],
                immediate_size: 0,
            },
        );
        Self {
            set,
            data_bgl,
            work_bgl,
            arc_bgl,
            layout,
            modules: [None, None],
            pipelines: [None, None, None, None, None],
        }
    }

    /// The shader set these pipelines are built from.
    pub(crate) fn set(&self) -> ShaderSet {
        self.set
    }

    /// How many pipelines have been created.
    pub fn created(&self) -> usize {
        self.pipelines.iter().filter(|p| p.is_some()).count()
    }

    /// Create every pipeline: each kind in each geometry.
    #[cfg(test)]
    pub(crate) fn ensure_all(&mut self, device: &wgpu::Device) {
        for geometry in [IdPickGeometry::Exact, IdPickGeometry::Simple] {
            for kind in [
                IdPickKind::Points,
                IdPickKind::Segments,
                IdPickKind::Vectors,
            ] {
                self.ensure(device, kind, geometry);
            }
        }
    }

    /// Create the pipeline of `kind` in `geometry`, when missing.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        kind: IdPickKind,
        geometry: IdPickGeometry,
    ) {
        let (module_index, slot) = kind.slot(geometry);
        if self.pipelines[slot].is_some() {
            return;
        }
        let set = self.set;
        let (label, assembled) = [ID_PICK_MODULE, ID_PICK_VECTOR_MODULE][module_index];
        let module = self.modules[module_index].get_or_insert_with(|| {
            crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(assembled.of(set).source.into()),
                },
            )
        });
        self.pipelines[slot] = Some(crate::gpu_compile::render_pipeline(
            device,
            &wgpu::RenderPipelineDescriptor {
                label: Some("figgy GPU pick ID pipeline"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(kind.vertex_entry()),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::PointList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: ID_PICK_DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(kind.fragment_entry(geometry)),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ID_PICK_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            },
        ));
    }

    fn pipeline(
        &self,
        kind: IdPickKind,
        geometry: IdPickGeometry,
    ) -> Option<&wgpu::RenderPipeline> {
        self.pipelines[kind.slot(geometry).1].as_ref()
    }
}

/// One hit the ID pass resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdPickHit {
    pub source_id: Option<String>,
    pub series_id: String,
    /// The logical index the pick reports (a monotonic series' sample
    /// index, through its envelope rows).
    pub point_index: usize,
    /// The point picker's `PICK_KIND_*` of the hit primitive.
    pub primitive_kind: u32,
    /// The drawn element: a marker's point, a segment's first point.
    pub primitive_index: usize,
    /// Whether the cursor is inside the drawn shape (distance 0).
    pub covering: bool,
}

/// A pending ID pick: the 16 bytes the 1×1 target holds, read back.
pub struct IdPickTicket {
    readback: Option<GpuReadback>,
    identities: Arc<[PickIdentity]>,
    pub(crate) monotonic: Vec<crate::renderer::MonotonicPickRows>,
    _charge: Option<GpuByteCharge>,
}

impl IdPickTicket {
    fn none() -> Self {
        Self {
            readback: None,
            identities: Arc::from([]),
            monotonic: Vec::new(),
            _charge: None,
        }
    }

    /// Wait for the pick, then decode it.
    pub async fn resolve(mut self) -> Result<Option<IdPickHit>, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait().await?;
        }
        self.finish()
    }

    /// [`Self::resolve`], polling the device on this thread until the pick
    /// completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(mut self) -> Result<Option<IdPickHit>, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait_blocking()?;
        }
        self.finish()
    }

    /// After a wait: decode the 16 bytes the target held.
    fn finish(&mut self) -> Result<Option<IdPickHit>, GpuPickError> {
        let Some(readback) = self.readback.as_mut() else {
            return Ok(None);
        };
        let words = readback.finish_checked(|mapped| {
            let mut words = [0u32; 4];
            for (k, word) in words.iter_mut().enumerate() {
                *word = u32::from_le_bytes([
                    mapped[k * 4],
                    mapped[k * 4 + 1],
                    mapped[k * 4 + 2],
                    mapped[k * 4 + 3],
                ]);
            }
            words
        })?;
        let tag = words[0];
        if tag == 0 {
            return Ok(None);
        }
        let identity = self
            .identities
            .get(words[1] as usize)
            .ok_or(GpuPickError::InvalidGpuResult)?;
        let point_index = match self
            .monotonic
            .iter()
            .find(|rows| rows.series_id == identity.series_id)
        {
            Some(rows) => match rows.logical(words[2] as usize) {
                Some(index) => index,
                None => return Ok(None),
            },
            None => words[2] as usize,
        };
        Ok(Some(IdPickHit {
            source_id: identity.source_id.clone(),
            series_id: identity.series_id.clone(),
            point_index,
            primitive_kind: (tag & 0xff) - 1,
            primitive_index: words[3] as usize,
            covering: tag & ID_PICK_COVERING != 0,
        }))
    }
}

/// The kinds a series with point picker `flags` draws, in paint order: the
/// line under the markers; a point vector series' arrows alone.
fn series_kinds(flags: u32) -> impl Iterator<Item = IdPickKind> {
    let vector = flags & FLAG_VECTOR != 0;
    [
        (!vector && flags & FLAG_LINE != 0).then_some(IdPickKind::Segments),
        (!vector && flags & FLAG_SCATTER != 0).then_some(IdPickKind::Points),
        vector.then_some(IdPickKind::Vectors),
    ]
    .into_iter()
    .flatten()
}

impl GpuPickEngine {
    /// The ID pick of the registered series at `query`'s cursor: the same
    /// checks and query parameters as [`Self::pick_with_arcs`], then one
    /// render pass into the 1×1 ID target instead of the gate, exact and
    /// reduce passes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pick_id(
        &self,
        pipelines: &mut IdPickPipelines,
        geometry: IdPickGeometry,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
        arcs: &[Option<wgpu::Buffer>],
        rings: &[Option<wgpu::Buffer>],
        members: usize,
        member_transform: &dyn Fn(usize) -> ScatterTransform,
    ) -> Result<IdPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        let [_, _, chart_width, chart_height] = query.chart_rect_px;
        if !display_scale.is_finite()
            || display_scale <= 0.0
            || !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
            || !(chart_width > 0.0 && chart_height > 0.0)
        {
            return Ok(IdPickTicket::none());
        }
        if let Some([x, y, width, height]) = query.data_area_px
            && !(cursor_x >= x && cursor_x <= x + width && cursor_y >= y && cursor_y <= y + height)
        {
            return Ok(IdPickTicket::none());
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(IdPickTicket::none());
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }
        let device = &self.bundle.device;
        // The kinds the registered series draw, compiled on first use.
        for slot in &self.registry.slots {
            for kind in series_kinds(slot.gpu.flags) {
                pipelines.ensure(device, kind, geometry);
            }
        }
        let members = members.min(self.registry.slots.len());
        let first_member = self.registry.slots.len() - members;
        for (series_order, slot) in self.registry.slots.iter().enumerate() {
            Self::validate_series_columns(pool, slot)?;
            let series = &slot.gpu;
            let mut params = query_params(
                series,
                &slot.identity.series_id,
                &query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &slot.identity.series_id)?,
                    lane_base(series.y_handle, &slot.identity.series_id)?,
                    series.style_index_base,
                ],
                [
                    0,
                    series.point_count,
                    if series.line_closed {
                        series.point_count
                    } else {
                        series.point_count.saturating_sub(1)
                    },
                    0,
                ],
                series_arc(arcs, series_order, series.point_count).is_some(),
            );
            if let Some(member) = series_order.checked_sub(first_member) {
                params.transform = member_transform(member);
            }
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        }

        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            "figgy GPU pick ID target",
            ID_PICK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(
            "figgy GPU pick ID depth",
            ID_PICK_DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let readback_tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &readback_tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick ID readback"),
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "ID pick readback",
        )?;
        let data_bg = create_bind_group_checked(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick ID data bg"),
                layout: &pipelines.data_bgl,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.buffer().as_entire_binding(),
                }],
            },
            "ID pick data bind group",
        )?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy GPU pick ID encoder"),
        });
        {
            let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy GPU pick ID pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &data_bg, &[]);
            // Series in registry order, the order they paint; within a
            // series the line under the markers.
            for (series_order, slot) in self.registry.slots.iter().enumerate() {
                let series = &slot.gpu;
                let work_bg = create_bind_group_checked(
                    device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("figgy GPU pick ID work bg"),
                        layout: &pipelines.work_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: series.query_params.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: series.style_rows[0].as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: series.style_rows[1].as_entire_binding(),
                            },
                        ],
                    },
                    "ID pick work bind group",
                )?;
                let placeholder = &self.bundle.arc_placeholder_buffer;
                let arc = series_arc(arcs, series_order, series.point_count).unwrap_or(placeholder);
                let ring =
                    series_ring(rings, series_order, series.point_count).unwrap_or(placeholder);
                let arc_bg = create_bind_group_checked(
                    device,
                    &wgpu::BindGroupDescriptor {
                        label: Some("figgy GPU pick ID arc bg"),
                        layout: &pipelines.arc_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: arc.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: ring.as_entire_binding(),
                            },
                        ],
                    },
                    "ID pick arc bind group",
                )?;
                pass.set_bind_group(1, &work_bg, &[]);
                pass.set_bind_group(2, &arc_bg, &[]);
                for kind in series_kinds(series.flags) {
                    pass.set_pipeline(
                        pipelines
                            .pipeline(kind, geometry)
                            .ok_or(GpuPickError::InvalidGpuResult)?,
                    );
                    pass.draw(0..series.point_count, 0..1);
                }
            }
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: None,
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.bundle.queue.submit(std::iter::once(encoder.finish()));
        Ok(IdPickTicket {
            readback: Some(GpuReadback::map(
                device,
                &self.bundle.health,
                &readback,
                None,
            )),
            identities: Arc::clone(&self.registry.identities),
            monotonic: Vec::new(),
            _charge: Some(
                readback_tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
            ),
        })
    }
}
