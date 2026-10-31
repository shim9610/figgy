//! Vector field (quiver) series: a layered shader assembled in Rust.
//!
//! The arrow drawing is split into WGSL fragments with fixed interfaces, so
//! the glyph does not depend on the projection:
//!
//! - the shared fragments under `data_render/common/`: `transform`, `style`,
//!   `projection` and `sample_validity` (as in every Cartesian data shader),
//!   `ramp` (the colourbar ramp), `arrow` (the arrow geometry and its
//!   analytic coverage) and, for the grid form, `grid_pair` and `data_pick`;
//! - `vector/project_cartesian2d.wgsl`: `project_arrow`, which maps a
//!   `VectorSample` to a tail and a tip in chart-area pixels under either
//!   scale mode;
//! - `vector/vector_input_common.wgsl`: the group-2 bindings (the column pool
//!   as storage, the grid table, the colourmap stops, `VectorParams`);
//! - `vector/vector_input_points.wgsl` / `vector/vector_input_grid.wgsl` /
//!   `vector/vector_input_path.wgsl`: `vector_sample`, one `VectorSample`
//!   per instance index from index-aligned columns, from a grid
//!   declaration, or from an arc slot along a streamline series' drawn path
//!   (with `common/arc_walk.wgsl`'s group 3 and `common/path_arrow.wgsl`);
//! - `vector/vector_entry.wgsl`: `vs_main` / `fs_main`.
//!
//! [`VECTOR_POINTS_SHADER`], [`VECTOR_GRID_SHADER`] and
//! [`VECTOR_PATH_SHADER`] splice the fragments
//! in that order at compile time (`assembled!`); the list is the module's
//! identity and there is no runtime templating. Every assembled combination
//! is listed in [`VectorInput::ALL`], compiled by the shader compilation
//! test, recorded in the pipeline contract and warmed in the browser like
//! any other shader.

use super::{GridColumnGpu, multisample_state};

/// Which input fragment an assembled vector module carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VectorInput {
    /// `VectorComponents::Points`: index-aligned `u` / `v` columns.
    Points,
    /// `VectorComponents::Grid`: `u` / `v` grids over the coordinate columns.
    Grid,
    /// `DataRenderType::Streamlines` arrowheads: one head per arc slot along
    /// the drawn path, group 3 the arc slot data (`common/arc_walk.wgsl`),
    /// drawn indirect.
    Path,
}

impl VectorInput {
    /// Every assembled combination, so coverage can enumerate them.
    pub const ALL: [VectorInput; 3] = [VectorInput::Points, VectorInput::Grid, VectorInput::Path];

    /// The shader module label, as the compile contract records it.
    pub fn label(self) -> &'static str {
        match self {
            VectorInput::Points => "figgy vector points shader",
            VectorInput::Grid => "figgy vector grid shader",
            VectorInput::Path => "figgy vector path shader",
        }
    }
}

/// The assembled vector module for [`VectorInput::Points`].
pub const VECTOR_POINTS_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/arrow.wgsl",
    "data_render/vector/project_cartesian2d.wgsl",
    "data_render/vector/vector_input_common.wgsl",
    "data_render/vector/vector_input_points.wgsl",
    "data_render/vector/vector_entry.wgsl",
);

/// The assembled vector module for [`VectorInput::Grid`].
pub const VECTOR_GRID_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/arrow.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/vector/project_cartesian2d.wgsl",
    "data_render/vector/vector_input_common.wgsl",
    "data_render/vector/vector_input_grid.wgsl",
    "data_render/vector/vector_entry.wgsl",
);

/// The assembled vector module for [`VectorInput::Path`]: the streamline
/// arrowheads. The glyph and projection fragments are the point form's.
pub const VECTOR_PATH_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/arrow.wgsl",
    "data_render/common/arc_pair.wgsl",
    "data_render/common/arc_walk.wgsl",
    "data_render/common/path_arrow.wgsl",
    "data_render/vector/project_cartesian2d.wgsl",
    "data_render/vector/vector_input_common.wgsl",
    "data_render/vector/vector_input_path.wgsl",
    "data_render/vector/vector_entry.wgsl",
);

/// The assembled WGSL source of one input form, spliced at compile time.
/// `shader_assembly`'s `every_module_is_its_fragments_concatenated` holds it
/// to its fragment list.
pub const fn assemble(input: VectorInput) -> &'static str {
    match input {
        VectorInput::Points => VECTOR_POINTS_SHADER.source,
        VectorInput::Grid => VECTOR_GRID_SHADER.source,
        VectorInput::Path => VECTOR_PATH_SHADER.source,
    }
}

pub(crate) fn shader_module(device: &wgpu::Device, input: VectorInput) -> wgpu::ShaderModule {
    crate::gpu_compile::shader_module(
        device,
        wgpu::ShaderModuleDescriptor {
            label: Some(input.label()),
            source: wgpu::ShaderSource::Wgsl(assemble(input).into()),
        },
    )
}

/// The pick radius of a vector series' anchor, in logical pixels: half the
/// head width, or half the shaft width when the shaft is wider. The GPU
/// picker and the selection ring use the same number.
pub(crate) fn vector_anchor_radius_px(style: &crate::data_config::VectorStyle) -> f32 {
    (style.head_width_px.max(style.width_px) * 0.5).max(0.0)
}

/// Vertices per arrow: one oriented quad (`common/arrow.wgsl::ARROW_VERTICES`).
pub const ARROW_VERTICES: u32 = 6;

/// `VectorParamsGpu::flags`: the components are grids, not point columns.
pub const VECTOR_FLAG_GRID: u32 = 1;
/// `VectorParamsGpu::flags`: grid columns run along Y (`MatrixOrientation`).
pub const VECTOR_FLAG_COLUMNS_ARE_Y: u32 = 2;
/// `VectorParamsGpu::flags`: grid coordinates are cell centres, not edges.
pub const VECTOR_FLAG_CENTERS: u32 = 4;
/// `VectorParamsGpu::flags`: `scale` is pixels per unit, not a data factor.
pub const VECTOR_FLAG_SCALE_PIXELS: u32 = 8;
/// `VectorParamsGpu::flags`: the colourbar is logarithmic; `z_min` / `z_max`
/// are log-transformed.
pub const VECTOR_FLAG_LOG_Z: u32 = 16;
/// `VectorParamsGpu::flags`: arrows are coloured through the colourbar.
pub const VECTOR_FLAG_COLOR_MAP: u32 = 32;

/// `VectorParamsGpu::pivot`: the tail sits on the anchor.
pub const VECTOR_PIVOT_TAIL: u32 = 0;
/// `VectorParamsGpu::pivot`: the arrow is centred on the anchor.
pub const VECTOR_PIVOT_MIDDLE: u32 = 1;
/// `VectorParamsGpu::pivot`: the tip sits on the anchor.
pub const VECTOR_PIVOT_TIP: u32 = 2;

/// Everything the vector shader needs beyond `Transform` and `Style`.
///
/// GPU twin: `vector_input_common.wgsl::VectorParams`, 128 B. Points form:
/// `*_base` are f32 lane indices into the pool, `*_len` logical lengths and
/// `count` the shortest of them. Grid form: `u_base` / `v_base` / `c_base`
/// index the grid table (one entry per constituent column), `cols` / `rows`
/// are the effective extent in constituent-column order and `count` is
/// `cols * rows`. `z_min` / `z_max` are the colourbar's bounds as `(hi, lo)`
/// pairs, already log-transformed under [`VECTOR_FLAG_LOG_Z`].
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VectorParamsGpu {
    /// Pool lane of the x anchor column.
    pub x_base: u32,
    /// Pool lane of the y anchor column.
    pub y_base: u32,
    /// Logical length of the x anchor column.
    pub x_len: u32,
    /// Logical length of the y anchor column.
    pub y_len: u32,
    /// Pool lane of the u column, or the u grid's first table entry.
    pub u_base: u32,
    /// Logical length of the u column (points form).
    pub u_len: u32,
    /// Pool lane of the v column, or the v grid's first table entry.
    pub v_base: u32,
    /// Logical length of the v column (points form).
    pub v_len: u32,
    /// Pool lane of the colour column, or the colour grid's first entry.
    pub c_base: u32,
    /// Logical length of the colour column (points form).
    pub c_len: u32,
    /// Grid columns in constituent-column order (grid form).
    pub cols: u32,
    /// Grid rows in constituent-column order (grid form).
    pub rows: u32,
    /// The `VECTOR_FLAG_*` bits.
    pub flags: u32,
    /// Arrows to draw.
    pub count: u32,
    /// Colour stops in the group-2 stop table.
    pub stop_count: u32,
    /// One of the `VECTOR_PIVOT_*` values.
    pub pivot: u32,
    /// `factor` or `px_per_unit`, by [`VECTOR_FLAG_SCALE_PIXELS`].
    pub scale: f32,
    /// Shaft width in render pixels (logical pixels times the render scale).
    pub width_px: f32,
    /// Head length along the shaft, in render pixels.
    pub head_length_px: f32,
    /// Full head width, in render pixels.
    pub head_width_px: f32,
    /// Arrows shorter than this are not drawn, in render pixels.
    pub min_length_px: f32,
    /// Anti-aliasing width: one logical pixel times the render scale.
    pub aa_px: f32,
    /// Path form: the arc length between two arrowheads along the drawn
    /// path, in render pixels; zero for the other forms.
    pub spacing_px: f32,
    /// Alignment padding.
    pub pad1: f32,
    /// The colourbar's minimum as a `(hi, lo)` pair.
    pub z_min: [f32; 2],
    /// The colourbar's maximum as a `(hi, lo)` pair.
    pub z_max: [f32; 2],
    /// Premultiplied colour for values the ramp cannot place.
    pub nan_color_premul: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<VectorParamsGpu>() == 128);

/// Bind group layout for the vector series' own data (group 2): the column
/// pool as read-only storage, the grid table, the colourmap stops and the
/// params uniform. The pool is bound whole, as the field shader binds it;
/// the params carry the lane bases.
pub fn create_vector_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    // The grid form's arrow pick (`pick_vector_cell`) binds the same group.
    let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    crate::gpu_compile::bind_group_layout(
        device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy vector data bgl"),
            entries: &[
                storage(0), // column pool
                storage(1), // per-constituent-column (base, len), grid form
                storage(2), // colourmap control points
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        },
    )
}

/// The group-2 table contents of one vector series. Each slice must be
/// non-empty (a zero-sized storage binding is invalid); the real counts live
/// in `params`.
pub struct VectorTables<'a> {
    /// The grid table: one entry per constituent column of each grid.
    pub grid: &'a [GridColumnGpu],
    /// The colourbar's colour stops, premultiplied.
    pub stops: &'a [[f32; 4]],
    /// The series' parameters.
    pub params: &'a VectorParamsGpu,
}

/// One vector series' group 2: the bind group, the grid table buffer it
/// binds (a grid vector's selected-cell outline binds the same buffer as a
/// field's grid, so the table is uploaded once; its arrow pick binds this
/// group itself) and the charge for all three buffers.
pub struct VectorDataBindGroup {
    /// Group 2 of the vector pipeline.
    pub bind_group: wgpu::BindGroup,
    /// The grid table buffer the bind group binds.
    pub grid: wgpu::Buffer,
    /// The charge for the three group-2 buffers.
    pub charge: crate::gpu_memory::SharedCharge,
}

/// Bind the pool, the tables and the params into the layout above. The three
/// buffers are owned by the returned bind group and charged as one lump
/// against the field-table row, which the returned charge credits back when
/// the last clone dies, as `create_field_data_bind_group` does.
pub fn create_vector_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    tables: VectorTables<'_>,
) -> VectorDataBindGroup {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let tally = ChargeTally::new();
    let grid_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy vector grid table"),
            contents: bytemuck::cast_slice(tables.grid),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let stop_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy vector colourmap stops"),
            contents: bytemuck::cast_slice(tables.stops),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy vector params"),
            contents: bytemuck::bytes_of(tables.params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::FieldTable,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy vector data bg"),
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
                resource: stop_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: param_buf.as_entire_binding(),
            },
        ],
    });
    VectorDataBindGroup {
        bind_group,
        grid: grid_buf,
        charge,
    }
}

/// The vector pipeline for one input form. No vertex buffers: the quad is
/// emitted from `vertex_index` and every data read goes through group 2
/// (and, for [`VectorInput::Path`], the arc slot data of group 3, whose
/// layout `stylized.rs::create_star_data_bind_group_layout` describes and
/// this creates an equal one of).
pub fn create_vector_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    vector_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    input: VectorInput,
) -> wgpu::RenderPipeline {
    let shader = shader_module(device, input);
    let star_data_bgl = super::create_star_data_bind_group_layout(device);
    create_vector_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        vector_bgl,
        &star_data_bgl,
        target_format,
        1,
        input,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_vector_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    vector_bgl: &wgpu::BindGroupLayout,
    star_data_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    input: VectorInput,
) -> wgpu::RenderPipeline {
    let layouts = [
        Some(transform_bgl),
        Some(style_bgl),
        Some(vector_bgl),
        Some(star_data_bgl),
    ];
    // Only the path form reads group 3, the arc slots.
    let groups = if input == VectorInput::Path { 4 } else { 3 };
    let layout = crate::gpu_compile::pipeline_layout(
        device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy vector layout"),
            bind_group_layouts: &layouts[..groups],
            immediate_size: 0,
        },
    );
    let label = match input {
        VectorInput::Points => "figgy vector points pipeline",
        VectorInput::Grid => "figgy vector grid pipeline",
        VectorInput::Path => "figgy vector path pipeline",
    };
    crate::gpu_compile::render_pipeline(
        device,
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
                entry_point: Some("fs_main"),
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

/// One vector series' draw: `draw(0..ARROW_VERTICES, 0..count)` with group 2
/// bound to the whole column pool, the series' tables and its params (which
/// carry the lane bases). The streamline arrowheads draw indirect instead:
/// one instance per arc slot, the count the arc scan wrote.
pub struct ColumnVectorLayer<'a> {
    /// The assembled pipeline for the series' input form.
    pub pipeline: &'a wgpu::RenderPipeline,
    /// The chart's transform bind group (group 0).
    pub transform_bg: &'a wgpu::BindGroup,
    /// Owned: built where the series is resolved, like the field's.
    pub style_bg: wgpu::BindGroup,
    /// The series' group 2: the whole column pool, the series' tables and its
    /// params.
    pub vector_bg: wgpu::BindGroup,
    /// Keeps the group-2 buffers' lump charge alive while anything can draw.
    pub charge: crate::gpu_memory::SharedCharge,
    /// Arrows to issue; zero skips the draw. Unread with `indirect`.
    pub count: u32,
    /// [`VectorInput::Path`]: the `DrawIndirect` arguments the arc scan's
    /// `star_indirect` kernel wrote (`(ARROW_VERTICES, slots, 0, 0)`), and
    /// group 3, the arc slot data the path input walks (the arc prefix, the
    /// pool and `StarVsParams`). `None` for the other forms.
    pub indirect: Option<(&'a wgpu::Buffer, &'a wgpu::BindGroup)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fragment list of one input form (the text is held to it by
    /// `shader_assembly`'s `every_module_is_its_fragments_concatenated`).
    fn fragments(input: VectorInput) -> &'static [&'static str] {
        match input {
            VectorInput::Points => VECTOR_POINTS_SHADER.fragments,
            VectorInput::Grid => VECTOR_GRID_SHADER.fragments,
            VectorInput::Path => VECTOR_PATH_SHADER.fragments,
        }
    }

    #[test]
    fn assembled_vector_shaders_end_in_the_entry_and_differ_by_input() {
        for input in VectorInput::ALL {
            // The entry is last: the interfaces the other layers declare are
            // in place before the entry calls them.
            assert_eq!(
                fragments(input).last(),
                Some(&"data_render/vector/vector_entry.wgsl")
            );
        }
        assert_ne!(assemble(VectorInput::Points), assemble(VectorInput::Grid));
        assert_ne!(assemble(VectorInput::Points), assemble(VectorInput::Path));
    }

    /// The path form reuses the point form's glyph, projection and entry
    /// unchanged (rule 9): it differs only by its input layer and the arc
    /// slot fragments the input reads.
    #[test]
    fn the_path_form_reuses_the_glyph_projection_and_entry() {
        let path = fragments(VectorInput::Path);
        let points = fragments(VectorInput::Points);
        for fragment in points {
            if *fragment != "data_render/vector/vector_input_points.wgsl" {
                assert!(path.contains(fragment), "{fragment}");
            }
        }
        let extra: Vec<_> = path.iter().filter(|f| !points.contains(f)).collect();
        assert_eq!(
            extra,
            [
                &"data_render/common/arc_pair.wgsl",
                &"data_render/common/arc_walk.wgsl",
                &"data_render/common/path_arrow.wgsl",
                &"data_render/vector/vector_input_path.wgsl",
            ]
        );
    }

    #[test]
    fn assembled_vector_shaders_parse() {
        for input in VectorInput::ALL {
            let source = assemble(input);
            if let Err(error) = wgpu::naga::front::wgsl::parse_str(source) {
                panic!("{input:?}: {}", error.emit_to_string(source));
            }
        }
    }
}
