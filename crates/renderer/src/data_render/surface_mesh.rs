//! The 3D surface: a heatmap on a 3D chart (`Config.view_3d`) drawn as a
//! mesh over its sample lattice, through the panel's camera, with the depth
//! test, inside the panel composite.
//!
//! One instance per quad between four neighbouring samples, six vertices,
//! no vertex buffers: the vertex stage reads every sample from the pool
//! through the field group 2 (`create_field_data_bind_group_layout`), the
//! table the flat heatmap reads, and colours through the heatmap's own
//! `common/field_paint.wgsl`. Group 3 holds the camera ([`View3dGpu`]) and
//! the surface's look ([`SurfaceParamsGpu`]). Nothing is stored per sample.
//!
//! Not `data_render/surface.rs`, which is the wgpu window surface.

use model::view3d::{Projection3d, View3d};

use crate::config::{AxisScale, Config};

/// The assembled surface module. The projection fragment is the layer a
/// later 3D vector glyph reuses (rule 9).
pub const SURFACE_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/grid_pair.wgsl",
    "data_render/common/field_grid.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/field_paint.wgsl",
    "data_render/common/material.wgsl",
    "data_render/common/project_3d.wgsl",
    "data_render/surface_mesh/surface_input_grid.wgsl",
    "data_render/surface_mesh/surface_entry.wgsl",
);

/// The depth-stencil format of a 3D panel's composite: depth for the
/// surfaces, stencil for a ternary chart's region fills (#49, #55).
/// Guaranteed by WebGPU, so no device feature is needed.
pub const DEPTH_STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// The ID pick target's format: `(series slot + 1, kind, c, r)`.
pub const SURFACE_PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;

/// Vertices per surface quad: two triangles.
pub const SURFACE_QUAD_VERTICES: u32 = 6;

/// Instances of the picked sample's patch: the up to four quads around it.
pub const SURFACE_SELECTED_INSTANCES: u32 = 4;

/// GPU twin: `common/project_3d.wgsl::View3d`.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct View3dGpu {
    pub view_proj: [[f32; 4]; 4],
    pub view_proj_live: [[f32; 4]; 4],
    pub z_min: [f32; 2],
    pub z_max: [f32; 2],
    /// x: logarithmic z; y: perspective; z: output pixels per box
    /// half-diagonal.
    pub flags: [f32; 4],
    pub half_extents: [f32; 4],
    pub eye: [f32; 4],
    pub light: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<View3dGpu>() == 208);

/// `SurfaceParamsGpu::flags` bits, twins of `surface_input_grid.wgsl`.
pub const SURFACE_FLAG_COLOR_GRID: u32 = 1;
pub const SURFACE_FLAG_WIREFRAME: u32 = 2;
pub const SURFACE_FLAG_LIT: u32 = 4;

/// No picked sample.
pub const SURFACE_NO_PICK: u32 = u32::MAX;

/// GPU twin: `surface_mesh/surface_input_grid.wgsl::SurfaceParams`.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SurfaceParamsGpu {
    pub color_base: u32,
    pub flags: u32,
    pub picked_c: u32,
    pub picked_r: u32,
    pub wire_width_px: f32,
    pub pick_id: u32,
    pub _pad: [u32; 2],
    pub wire_color: [f32; 4],
    pub(crate) material: super::MaterialGpu,
}

const _: () = assert!(std::mem::size_of::<SurfaceParamsGpu>() == 80);

/// The light, fixed relative to the camera (#49 D8): 45° to the right of
/// the eye and 45° above it.
const LIGHT_AZIMUTH_OFFSET_DEGREES: f64 = 45.0;
const LIGHT_ELEVATION_DEGREES: f64 = 45.0;

/// The map from chart pixels to the clip space of a `size` target whose
/// origin is chart pixel `origin`, as row-major rows.
fn pixel_to_clip(origin: (f64, f64), size: (f64, f64)) -> [[f64; 4]; 4] {
    let (w, h) = (size.0.max(1.0), size.1.max(1.0));
    [
        [2.0 / w, 0.0, 0.0, -1.0 - 2.0 * origin.0 / w],
        [0.0, -2.0 / h, 0.0, 1.0 + 2.0 * origin.1 / h],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mul(a: &[[f64; 4]; 4], b: &[[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Row-major to the column layout of a WGSL `mat4x4<f32>`.
fn columns(m: &[[f64; 4]; 4]) -> [[f32; 4]; 4] {
    let mut c = [[0.0f32; 4]; 4];
    for (j, col) in c.iter_mut().enumerate() {
        for (i, v) in col.iter_mut().enumerate() {
            *v = m[i][j] as f32;
        }
    }
    c
}

/// A box direction in box space from an azimuth and an elevation.
fn direction(azimuth_degrees: f64, elevation_degrees: f64) -> [f32; 4] {
    let (a, e) = (azimuth_degrees.to_radians(), elevation_degrees.to_radians());
    [
        (e.cos() * a.cos()) as f32,
        (e.cos() * a.sin()) as f32,
        e.sin() as f32,
        0.0,
    ]
}

/// The camera uniform of a 3D chart. `composite` is the chart-pixel
/// rectangle of the panel composite target (origin, size), `live` the
/// frame target's: the composite draws the surface, the live slot its
/// highlight. `None` for a chart that is not 3D.
pub fn view3d_gpu(
    config: &Config,
    composite: ((f64, f64), (f64, f64)),
    live: ((f64, f64), (f64, f64)),
) -> Option<View3dGpu> {
    let view: &View3d = config.view_3d.as_ref()?;
    let m = config.view_matrix()?.m;
    let axis = &config.right_y;
    let log = matches!(axis.scale, AxisScale::Logarithmic);
    let (lo, hi) = if log {
        let (a, b) = crate::chart::guarded_log_range(axis.min, axis.max);
        (a.log10(), b.log10())
    } else {
        axis.data_range()
    };
    let (lo, hi) = if axis.inverted { (hi, lo) } else { (lo, hi) };
    let (min_hi, min_lo) = crate::data::split_f64_to_f32_pair(lo);
    let (max_hi, max_lo) = crate::data::split_f64_to_f32_pair(hi);
    let h = view.half_extents();
    // Output pixels per box half-diagonal: the distance between the
    // projected centre and a projected unit step along the screen.
    let centre = config.project_3d([0.5, 0.5, 0.5]);
    let corner = config.project_3d([1.0, 1.0, 1.0]);
    let pixels = match (centre, corner) {
        (Some(c), Some(k)) => ((k[0] - c[0]).hypot(k[1] - c[1])).max(1.0),
        _ => 1.0,
    };
    Some(View3dGpu {
        view_proj: columns(&mul(&pixel_to_clip(composite.0, composite.1), &m)),
        view_proj_live: columns(&mul(&pixel_to_clip(live.0, live.1), &m)),
        z_min: [min_hi, min_lo],
        z_max: [max_hi, max_lo],
        flags: [
            if log { 1.0 } else { 0.0 },
            if view.projection == Projection3d::Perspective {
                1.0
            } else {
                0.0
            },
            pixels as f32,
            0.0,
        ],
        half_extents: [h[0] as f32, h[1] as f32, h[2] as f32, 0.0],
        eye: direction(view.azimuth_degrees, view.elevation_degrees),
        light: direction(
            view.azimuth_degrees + LIGHT_AZIMUTH_OFFSET_DEGREES,
            (view.elevation_degrees + LIGHT_ELEVATION_DEGREES).min(90.0),
        ),
    })
}

/// Group 3 of the surface: binding 4 the camera, binding 5 the surface's
/// look. Bindings 4 and 5 leave the field module's group 3 bindings (the
/// contour label gaps, 0, 1 and 3) free for the floor entries.
pub fn create_surface_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let uniform = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy surface data bgl"),
            entries: &[uniform(4), uniform(5)],
        },
    )
}

/// Which surface pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum SurfacePass {
    /// The surface in the panel composite, depth-tested.
    Draw,
    /// The picked sample's patch in the live highlight slot, no depth.
    Selected,
    /// The 1×1 ID pick, depth-tested.
    Pick,
}

impl SurfacePass {
    pub const ALL: [SurfacePass; 3] = [SurfacePass::Draw, SurfacePass::Selected, SurfacePass::Pick];

    fn entries(self) -> (&'static str, &'static str) {
        match self {
            SurfacePass::Draw => ("vs_main", "fs_main"),
            SurfacePass::Selected => ("vs_selected", "fs_selected"),
            SurfacePass::Pick => ("vs_main", "fs_pick"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SurfacePass::Draw => "figgy surface pipeline",
            SurfacePass::Selected => "figgy surface selection pipeline",
            SurfacePass::Pick => "figgy surface pick pipeline",
        }
    }

    /// The depth state every depth-tested surface pass shares: nearer wins.
    pub fn depth_stencil(self) -> Option<wgpu::DepthStencilState> {
        match self {
            SurfacePass::Selected => None,
            SurfacePass::Draw | SurfacePass::Pick => Some(wgpu::DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        }
    }
}

/// One surface pipeline. `target_format` and `sample_count` are the
/// composite's (or the frame's, for [`SurfacePass::Selected`]); the pick
/// renders one sample into [`SURFACE_PICK_FORMAT`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_surface_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    surface_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: SurfacePass,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy surface layout"),
            bind_group_layouts: &[
                Some(transform_bgl),
                Some(style_bgl),
                Some(field_bgl),
                Some(surface_bgl),
            ],
            immediate_size: 0,
        },
    );
    let (vertex, fragment) = pass.entries();
    let (format, samples, blend) = match pass {
        SurfacePass::Pick => (SURFACE_PICK_FORMAT, 1, None),
        _ => (
            target_format,
            sample_count,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        ),
    };
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(pass.label()),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vertex),
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
            depth_stencil: pass.depth_stencil(),
            multisample: super::multisample_state(samples),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fragment),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// One heatmap drawn as a surface: the field's group 2 snapshot, its own
/// group 3 and the quad count to instance.
pub struct ColumnSurfaceLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub field_bg: wgpu::BindGroup,
    pub surface_bg: wgpu::BindGroup,
    pub quads: u32,
    /// Keeps the group 2 and group 3 buffers' charges alive.
    pub charge: crate::gpu_memory::SharedCharge,
    pub surface_charge: crate::gpu_memory::SharedCharge,
}

/// Draw `layer` into a pass with the depth-stencil attachment.
pub fn issue_surface_layer(pass: &mut wgpu::RenderPass<'_>, layer: &ColumnSurfaceLayer<'_>) {
    if layer.quads == 0 {
        return;
    }
    pass.set_pipeline(layer.pipeline);
    pass.set_bind_group(0, layer.transform_bg, &[]);
    pass.set_bind_group(1, &layer.style_bg, &[]);
    pass.set_bind_group(2, &layer.field_bg, &[]);
    pass.set_bind_group(3, &layer.surface_bg, &[]);
    pass.draw(0..SURFACE_QUAD_VERTICES, 0..layer.quads);
}

/// The quads of a `cols × rows` sample lattice.
pub fn surface_quad_count(cols: u32, rows: u32) -> u32 {
    cols.saturating_sub(1)
        .saturating_mul(rows.saturating_sub(1))
}

/// The group 3 of one surface draw: the camera and the surface's look, in
/// two uniforms charged as one.
pub(crate) fn create_surface_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &View3dGpu,
    surface: &SurfaceParamsGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let tally = crate::gpu_memory::ChargeTally::new();
    let uniform = |label, contents: &[u8]| {
        crate::gpu_memory::charged_buffer_init(
            &tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        )
    };
    let view_buf = uniform("figgy surface camera uniform", bytemuck::bytes_of(view));
    let surface_buf = uniform("figgy surface params uniform", bytemuck::bytes_of(surface));
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy surface bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 4,
                resource: view_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: surface_buf.as_entire_binding(),
            },
        ],
    });
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// The surface look of `style` (the default when `None`) for the GPU:
/// colour grid at `color_base`, wireframe width at `scale`, the picked
/// sample and the ID the pick writes.
pub(crate) fn surface_params_gpu(
    style: Option<&model::view3d::SurfaceStyle>,
    color_base: u32,
    scale: f32,
    picked: Option<(u32, u32)>,
    pick_id: u32,
) -> SurfaceParamsGpu {
    let default = model::view3d::SurfaceStyle::default();
    let style = style.unwrap_or(&default);
    let mut flags = 0;
    if style.color_matrix.is_some() {
        flags |= SURFACE_FLAG_COLOR_GRID;
    }
    let (wire_width_px, wire_color) = match &style.wireframe {
        Some(wire) if wire.width > 0.0 => {
            flags |= SURFACE_FLAG_WIREFRAME;
            let c = wire.color;
            (wire.width * scale, [c.r * c.a, c.g * c.a, c.b * c.a, c.a])
        }
        _ => (0.0, [0.0; 4]),
    };
    if style.material != model::Material::Flat {
        flags |= SURFACE_FLAG_LIT;
    }
    let (picked_c, picked_r) = picked.unwrap_or((SURFACE_NO_PICK, SURFACE_NO_PICK));
    SurfaceParamsGpu {
        color_base,
        flags,
        picked_c,
        picked_r,
        wire_width_px,
        pick_id,
        _pad: [0; 2],
        wire_color,
        material: super::MaterialGpu::pack(
            style.material,
            0.5,
            style.texture_strength,
            style.texture_scale,
            style.gloss,
            0.0,
            0.0,
            style.emphasis_brightness,
        ),
    }
}
