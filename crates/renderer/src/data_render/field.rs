//! Field (heatmap) series: the grid columns, field parameters and tables,
//! their bind groups, the field pipelines and the field layers.

use super::*;

/// The assembled field (heatmap and contour) module.
pub(crate) const FIELD_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/contour_label_box.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/field_grid.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/field_paint.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/field_columnar.wgsl",
);

/// Vertices per field draw. The field is **one** quad for the whole grid, not
/// one per cell — `field_columnar.wgsl`'s header has the two reasons (25M
/// instances at matrix scale, and MSAA seams between adjacent quads).
pub const FIELD_VERTICES: u32 = 6;

/// Where one constituent grid column lives in the pool.
///
/// GPU twin: `common/field_grid.wgsl::GridColumn`. `base` is an **f32 lane** index,
/// matching the pool's `array<f32>` view — the same convention the arc scan and
/// the star pass use.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GridColumnGpu {
    pub base: u32,
    pub len: u32,
}

const _: () = assert!(std::mem::size_of::<GridColumnGpu>() == 8);

/// `FieldParams.flags` — the grid's declared shape, packed. Every one of these
/// comes from the *declaration*, never from the column lengths.
pub const FIELD_FLAG_COLUMNS_ARE_Y: u32 = 1;

pub const FIELD_FLAG_CENTERS: u32 = 2;

pub const FIELD_FLAG_INTERPOLATED: u32 = 4;

pub const FIELD_FLAG_BANDS: u32 = 8;

pub const FIELD_FLAG_LOG_Z: u32 = 16;

/// Everything the field shader needs beyond `Transform` and `Style`.
///
/// GPU twin: `common/field_grid.wgsl::FieldParams`, 64 B. `z_min` / `z_max` are the
/// colourbar's bounds as the pool's `(hi, lo)` f32 pair, **already
/// log-transformed** when [`FIELD_FLAG_LOG_Z`] is set: the bounds are f64 on the
/// host, so taking the logarithm here keeps precision the shader would lose.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FieldParamsGpu {
    pub x_base: u32,
    pub y_base: u32,
    pub x_len: u32,
    pub y_len: u32,
    pub cols: u32,
    pub rows: u32,
    /// Contour levels, and for a `Bands` fill the bands' boundaries. Binding 2
    /// preserves declaration order; binding 3 carries a block-sorted lookup copy
    /// so fragments binary-search reachable levels without changing positional
    /// `level_index`, duplicate, colour, or source-over semantics.
    pub level_count: u32,
    pub stop_count: u32,
    pub flags: u32,
    pub opacity: f32,
    /// Contour stroke width in pixels, already scaled. Read by `fs_contour` and
    /// ignored by the fill.
    pub line_width_px: f32,
    /// Entries in the `level_colors` table.
    pub level_color_count: u32,
    pub z_min: [f32; 2],
    pub z_max: [f32; 2],
}

impl FieldParamsGpu {
    /// Params for a field group that serves cell geometry only (a grid
    /// vector's selected-cell outline): the grid lattice and
    /// its flags are real; the z range, levels, stops and colours are unread
    /// by those passes and are left at neutral values.
    pub fn cell_geometry(
        x_base: u32,
        y_base: u32,
        x_len: u32,
        y_len: u32,
        cols: u32,
        rows: u32,
        flags: u32,
    ) -> Self {
        Self {
            x_base,
            y_base,
            x_len,
            y_len,
            cols,
            rows,
            level_count: 0,
            stop_count: 0,
            flags,
            opacity: 1.0,
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: [0.0, 0.0],
            z_max: [1.0, 0.0],
        }
    }
}

const _: () = assert!(std::mem::size_of::<FieldParamsGpu>() == 64);

/// Bind group layout for the field's own data (group 2).
///
/// No textures: the colour ramp travels as its control points and the shader
/// reimplements `model::colormap::sample` over them, so the GPU field and the
/// CPU-drawn colourbar strip agree by construction instead of by resampling a
/// quantized LUT. The pool is bound whole in both stages — the vertex stage
/// needs nothing from it, but a single layout for one shader is simpler than two.
///
/// **Compute-visible too.** `contour_anchor.wgsl` reads the very same grid
/// through the very same `locate`/`grid_value` (they are one SSoT block), so it
/// binds this group rather than owning a second copy of the table. A label
/// therefore cannot land on a grid the lines were not drawn from.
pub fn create_field_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let render = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let shared = render.union(wgpu::ShaderStages::COMPUTE);
    let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    // Only what the anchor pass actually reads is compute-visible. A storage
    // binding counts against `max_storage_buffers_per_shader_stage` (8) in every
    // stage it is visible to, and the anchor pipeline binds four of its own — so
    // marking the ramp tables compute-visible for symmetry would put the layout
    // one over the limit on a conformant device. The colour stops alone are
    // compute-visible: the cell pick evaluates the colour the fill paints
    // (`field_paint`), which brings the anchor pass to exactly eight.
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy field data bgl"),
            entries: &[
                storage(0, shared), // column pool (coordinate + grid bases in the uniform)
                storage(1, shared), // per-constituent-column (base, len)
                storage(2, shared), // contour levels, in data units
                storage(3, shared), // colourmap control points
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: shared,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(5, render), // per-level contour stroke colour, premultiplied
                storage(6, wgpu::ShaderStages::FRAGMENT), // contour lookup block metadata
            ],
        },
    )
}

/// Bind the field's pool, tables and params into the layout above.
///
/// Every slice must be non-empty — a zero-sized storage binding is invalid, so
/// the caller pads. `FieldParams` carries the real counts.
/// The field's group-2 table contents, resolved from the grid declaration.
///
/// One argument instead of four: they are always built together from the same
/// declaration and always uploaded together, so passing them separately only
/// creates an order to get wrong. Each slice must be non-empty — a zero-sized
/// storage binding is invalid — and the real counts live in `params`.
pub struct FieldTables<'a> {
    pub grid: &'a [GridColumnGpu],
    pub levels: &'a [f32],
    /// Binding 3: `max(params.stop_count, 1)` ramp/padding entries followed by one
    /// search record per real contour level. In each 32-record block the finite
    /// keys form a sorted prefix; x is the level value and y stores the original
    /// level index as a normal numeric f32. The remaining records are zero.
    pub stops: &'a [[f32; 4]],
    /// Per-level contour stroke colour, **premultiplied**. A fill-only series
    /// pads it to one entry; `FieldParams.level_color_count` carries the truth.
    pub level_colors: &'a [[f32; 4]],
    /// Binding 6: one `[finite_count, negative_infinity_count]` record per
    /// 32-level block. An empty level declaration still has one zero record.
    pub lookup_metadata: &'a [ContourLookupMetadataGpu],
    pub params: &'a FieldParamsGpu,
}

/// The group-2 tables a field bind group needs beyond its grid and params
/// when it serves cell geometry only: a grid vector's selected-cell outline
/// runs the field pipeline, which reads the grid table
/// and `FieldParams` and nothing of the levels, stops, level colours or
/// lookup metadata. Those four bindings must still exist, so one padding set
/// (an empty level list, one stop, one colour, one zero lookup record) is
/// built once per renderer and shared by every such group, the way the
/// neutral value-map group is shared by mapped scatter series.
pub struct FieldCellTables {
    pub(super) levels: wgpu::Buffer,
    pub(super) stops: wgpu::Buffer,
    pub(super) level_colors: wgpu::Buffer,
    pub(super) lookup_metadata: wgpu::Buffer,
    pub(super) _charge: crate::gpu_memory::SharedCharge,
}

pub fn create_field_cell_tables(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
) -> crate::Result<FieldCellTables> {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let lookup = build_field_lookup_tables(&[[0.0; 4]], &[])?;
    let tally = ChargeTally::new();
    let storage = |label: &'static str, contents: &[u8]| {
        charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: wgpu::BufferUsages::STORAGE,
            },
        )
    };
    let levels = storage(
        "figgy field cell levels (padding)",
        bytemuck::cast_slice(&[0.0f32]),
    );
    let stops = storage(
        "figgy field cell stops (padding)",
        bytemuck::cast_slice(&lookup.stops),
    );
    let level_colors = storage(
        "figgy field cell level colours (padding)",
        bytemuck::cast_slice(&[[0.0f32; 4]]),
    );
    let lookup_metadata = storage(
        "figgy field cell lookup metadata (padding)",
        bytemuck::cast_slice(&lookup.metadata),
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    Ok(FieldCellTables {
        levels,
        stops,
        level_colors,
        lookup_metadata,
        _charge: charge,
    })
}

/// A field bind group over an existing grid table buffer for cell geometry
/// only (see [`FieldCellTables`]): the grid and the pool are the caller's,
/// the padding tables are shared, and the params uniform is the one buffer
/// this group owns. `params` comes from [`FieldParamsGpu::cell_geometry`].
pub fn create_field_cell_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    grid: &wgpu::Buffer,
    tables: &FieldCellTables,
    params: &FieldParamsGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let tally = ChargeTally::new();
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field cell params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy field cell bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: tables.levels.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: tables.stops.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: param_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: tables.level_colors.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: tables.lookup_metadata.as_entire_binding(),
            },
        ],
    });
    (bind_group, charge)
}

pub(crate) struct FieldLookupTables {
    pub stops: Vec<[f32; 4]>,
    pub metadata: Vec<ContourLookupMetadataGpu>,
}

/// Build binding 3 and binding 6 from the same uploaded-f32 level slice.
///
/// Binding 3 keeps one record per declared level so block offsets remain source
/// positional. Each block compacts only its finite keys into a sorted prefix;
/// binding 6 is the bound that makes the zero padding unreachable. The source
/// `levels` remain untouched and continue to define every positional contract.
pub(crate) fn build_field_lookup_tables(
    stops: &[[f32; 4]],
    levels: &[f32],
) -> crate::Result<FieldLookupTables> {
    if levels.len() > CONTOUR_LEVEL_LOOKUP_CAPACITY {
        return Err(crate::FiggyError::StateAllocationFailed {
            resource: "contour level lookup",
            reason: format!(
                "{} levels exceed the {}-entry lookup capacity",
                levels.len(),
                CONTOUR_LEVEL_LOOKUP_CAPACITY
            ),
        });
    }

    let mut table = Vec::new();
    table
        .try_reserve_exact(stops.len().saturating_add(levels.len()))
        .map_err(|error| crate::FiggyError::StateAllocationFailed {
            resource: "field stop and contour lookup table",
            reason: error.to_string(),
        })?;
    table.extend_from_slice(stops);

    let block_count = levels.len().div_ceil(CONTOUR_LEVEL_BLOCK_SIZE);
    let mut metadata = Vec::new();
    metadata
        .try_reserve_exact(block_count.max(1))
        .map_err(|error| crate::FiggyError::StateAllocationFailed {
            resource: "contour lookup metadata table",
            reason: error.to_string(),
        })?;

    let mut order = [0u32; CONTOUR_LEVEL_BLOCK_SIZE];
    for block_start in (0..levels.len()).step_by(CONTOUR_LEVEL_BLOCK_SIZE) {
        let block_len = (levels.len() - block_start).min(CONTOUR_LEVEL_BLOCK_SIZE);
        let mut finite_count = 0usize;
        let mut negative_infinity_count = 0u32;
        for local in 0..block_len {
            let original = block_start + local;
            let value = levels[original];
            if value.is_finite() {
                order[finite_count] = u32::try_from(original).unwrap_or(u32::MAX);
                finite_count += 1;
            } else if value == f32::NEG_INFINITY {
                negative_infinity_count += 1;
            }
        }
        order[..finite_count].sort_unstable_by(|left, right| {
            let left_value = levels[*left as usize];
            let right_value = levels[*right as usize];
            left_value
                .partial_cmp(&right_value)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.cmp(right))
        });
        for original_index in &order[..finite_count] {
            table.push([
                levels[*original_index as usize],
                *original_index as f32,
                0.0,
                0.0,
            ]);
        }
        table.resize(table.len() + block_len - finite_count, [0.0; 4]);
        metadata.push(ContourLookupMetadataGpu {
            finite_count: u32::try_from(finite_count).unwrap_or(u32::MAX),
            negative_infinity_count,
        });
    }
    if metadata.is_empty() {
        metadata.push(ContourLookupMetadataGpu::default());
    }
    Ok(FieldLookupTables {
        stops: table,
        metadata,
    })
}

/// The six buffers end up owned only by the returned bind group, so no wrapper
/// can observe their lifetime. They are charged as one lump against the
/// [`crate::gpu_memory::GpuResourceKind::FieldTable`] row and the returned
/// [`crate::gpu_memory::SharedCharge`] credits
/// it back when the last clone of the layer dies — the same construction
/// `gpu_pick` uses for its bind-group-owned buffers. The grid table scales with
/// the column count, so leaving it uncharged would put a data-proportional term
/// outside the budget.
pub fn create_field_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    tables: FieldTables<'_>,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let FieldTables {
        grid,
        levels,
        stops,
        level_colors,
        lookup_metadata,
        params,
    } = tables;
    let tally = ChargeTally::new();
    let grid_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field grid table"),
            contents: bytemuck::cast_slice(grid),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let level_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field levels"),
            contents: bytemuck::cast_slice(levels),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let stop_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field ramp and contour lookup"),
            contents: bytemuck::cast_slice(stops),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let level_color_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field level colours"),
            contents: bytemuck::cast_slice(level_colors),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let lookup_metadata_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field contour lookup metadata"),
            contents: bytemuck::cast_slice(lookup_metadata),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy field params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy field data bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: grid_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: level_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: stop_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: param_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: level_color_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: lookup_metadata_buf.as_entire_binding(),
            },
        ],
    });
    (bind_group, charge)
}

/// Columnar field pipeline. No vertex buffers — the quad is emitted from
/// `vertex_index` and every data read goes through group 2's storage bindings.
pub fn create_field_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::field(device);
    create_field_columnar_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_field_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        target_format,
        sample_count,
        "fs_main",
        "figgy field columnar pipeline",
    )
}

pub(crate) fn create_field_selection_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        selection_bgl,
        field_bgl,
        target_format,
        sample_count,
        "fs_data_selection",
        "figgy selected field data pipeline",
    )
}

/// The field quad against one fragment entry point.
///
/// `fs_main` fills; `fs_contour` draws the isolines as the level set of the same
/// interpolation. Same vertex shader, same layouts, same quad — which is what
/// makes a filled band and the line over it agree by construction rather than by
/// two implementations happening to round the same way.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_field_columnar_pipeline_with_entry(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        None,
        target_format,
        sample_count,
        fragment_entry,
        label,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_field_columnar_pipeline_with_optional_extra_layout(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    extra_bgl: Option<&wgpu::BindGroupLayout>,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    fragment_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let layouts = [
        Some(transform_bgl),
        Some(style_bgl),
        Some(field_bgl),
        extra_bgl,
    ];
    let layout_count = if extra_bgl.is_some() { 4 } else { 3 };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy field columnar layout"),
            bind_group_layouts: &layouts[..layout_count],
            immediate_size: 0,
        },
    );
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
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
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// One columnar field (heatmap / band) series.
///
/// `field_bg` is group 2: the pool, the grid's `(base, len)` table, the contour
/// levels, the colourmap stops and `FieldParams`. It is built where the grid
/// declaration is resolved, so this layer carries no counts of its own — the
/// shader reads them from the uniform and the draw is always one quad.
pub struct ColumnFieldLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// Owned, not borrowed: the field's `Style` holds the *chart's*
    /// `colorbar.nan_color`, which the per-series style set does not know, so it
    /// is built where the grid declaration is resolved — the same reason the
    /// pick ring owns its style bind group.
    pub style_bg: wgpu::BindGroup,
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 buffers' lump charge alive for as long as anything can
    /// still draw with them.
    pub charge: crate::gpu_memory::SharedCharge,
    /// False when the declaration resolved to no cells; the draw is skipped
    /// rather than issued for an empty grid.
    pub drawable: bool,
}

/// One selected matrix cell or contour level. `field_bg` is the exact group-2
/// snapshot used by the underlying field/contour draw, so the overlay cannot
/// drift onto a different lattice or level set.
pub struct ColumnFieldSelectionLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub selection_bg: wgpu::BindGroup,
    pub selection_charge: crate::gpu_memory::SharedCharge,
    pub field_bg: wgpu::BindGroup,
    pub charge: crate::gpu_memory::SharedCharge,
    pub drawable: bool,
}
