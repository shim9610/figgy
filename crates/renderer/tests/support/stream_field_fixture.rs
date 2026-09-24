use super::renderer;
use bytemuck::Zeroable;
use renderer::data_render::{FieldParamsGpu, GridColumnGpu, PrimitiveStyle, ScatterTransform};
use wgpu::util::DeviceExt;

pub(super) const WIDTH: u32 = 19;
pub(super) const HEIGHT: u32 = 15;
pub(super) const PANEL: (u32, u32, u32, u32) = (2, 2, 15, 11);
pub(super) const BACKGROUND: wgpu::Color = wgpu::Color {
    r: 0.13,
    g: 0.23,
    b: 0.31,
    a: 0.63,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct Fixture {
    pub(super) centers: bool,
    pub(super) interpolated: bool,
    pub(super) columns_are_y: bool,
    pub(super) log: bool,
    pub(super) inverted: bool,
}

pub(super) struct SourceFixture {
    pub(super) transform: ScatterTransform,
    pub(super) style: PrimitiveStyle,
    pub(super) params: FieldParamsGpu,
    pub(super) axes: [Vec<[f32; 2]>; 2],
    pub(super) sources: Vec<Vec<[f32; 2]>>,
    pub(super) declarations: Vec<usize>,
    pub(super) oracle_pool: Vec<[f32; 2]>,
    pub(super) oracle_grid: Vec<GridColumnGpu>,
}

pub(super) fn fixture_data(case: Fixture) -> SourceFixture {
    use renderer::data_config::{GridLayout, MatrixOrientation, MatrixRef};
    let raw_x = if case.log {
        vec![1.0, 2.0, 4.0, 8.0, 16.0]
    } else {
        vec![0.0, 1.0, 2.0, 3.0, 4.0]
    };
    let raw_y = if case.log {
        vec![1.0, 2.0, 4.0, 8.0]
    } else {
        vec![0.0, 1.0, 2.0, 3.0]
    };
    let axes = [
        raw_x.iter().map(|&v| [v, 0.0]).collect::<Vec<_>>(),
        raw_y.iter().map(|&v| [v, 0.0]).collect::<Vec<_>>(),
    ];
    let along = if case.columns_are_y {
        raw_y.len()
    } else {
        raw_x.len()
    } - usize::from(!case.centers);
    let across = if case.columns_are_y {
        raw_x.len()
    } else {
        raw_y.len()
    } - usize::from(!case.centers);
    // A repeated source ID is still two distinct matrix column ordinals.
    let mut declarations = (0..along)
        .map(|c| if c == 2 { 0 } else { c })
        .collect::<Vec<_>>();
    let mut sources = (0..along)
        .map(|c| {
            (0..across + c % 2)
                .map(|r| {
                    if c == 1 && r == 1 {
                        [f32::NAN, 0.0]
                    } else {
                        [1_000_000_000.0, 0.1 + c as f32 * 0.13 + r as f32 * 0.07]
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    sources.push(Vec::new());
    declarations.push(sources.len() - 1); // surplus empty column cannot clip rows
    let matrix = MatrixRef {
        columns: declarations
            .iter()
            .map(|id| format!("z-{id}").into())
            .collect(),
        orientation: if case.columns_are_y {
            MatrixOrientation::ColumnsAreY
        } else {
            MatrixOrientation::ColumnsAreX
        },
        grid_layout: if case.centers {
            GridLayout::Centers
        } else {
            GridLayout::Edges
        },
    };
    let usable = matrix
        .coordinate_cells(axes[0].len(), axes[1].len())
        .0
        .min(matrix.columns.len());
    let shortest = declarations[..usable]
        .iter()
        .map(|&id| sources[id].len())
        .min()
        .unwrap();
    let (cols, rows) = matrix.effective_extent(axes[0].len(), axes[1].len(), shortest);
    assert_eq!((cols, rows), (along, across));
    let mut oracle_pool = axes[0].clone();
    let y_base = oracle_pool.len() as u32 * 2;
    oracle_pool.extend_from_slice(&axes[1]);
    let mut bases = Vec::new();
    for source in &sources {
        bases.push(oracle_pool.len() as u32 * 2);
        oracle_pool.extend_from_slice(source);
    }
    let oracle_grid = declarations[..cols]
        .iter()
        .map(|&id| GridColumnGpu {
            base: bases[id],
            len: sources[id].len() as u32,
        })
        .collect();
    let mut transform = ScatterTransform {
        data_min: [raw_x[0], raw_y[0]],
        data_max: [*raw_x.last().unwrap(), *raw_y.last().unwrap()],
        data_min_lo: [0.0; 2],
        data_max_lo: [0.0; 2],
        scale_log: [0.0; 2],
        pixel_to_ndc: [2.0 / PANEL.2 as f32, 2.0 / PANEL.3 as f32],
        data_to_panel_scale: [1.0; 2],
        data_to_panel_offset: [0.0; 2],
        style_params: [[0.0; 4]; 3],
    };
    if case.log {
        transform.data_min = transform.data_min.map(f32::log10);
        transform.data_max = transform.data_max.map(f32::log10);
        transform.scale_log = [1.0; 2];
    }
    if case.inverted {
        transform.data_to_panel_scale[0] = -1.0;
        transform.data_to_panel_offset[0] = 1.0;
    }
    let mut style = PrimitiveStyle::zeroed();
    style.color_premul = [0.08, 0.13, 0.19, 0.45];
    let params = FieldParamsGpu {
        x_base: 0,
        y_base,
        x_len: axes[0].len() as u32,
        y_len: axes[1].len() as u32,
        cols: cols as u32,
        rows: rows as u32,
        level_count: 0,
        stop_count: 3,
        flags: u32::from(case.columns_are_y)
            | u32::from(case.centers) * 2
            | u32::from(case.interpolated) * 4,
        opacity: 0.65,
        line_width_px: 0.0,
        level_color_count: 0,
        z_min: [1_000_000_000.0, 0.0],
        z_max: [1_000_000_000.0, 1.0],
    };
    SourceFixture {
        transform,
        style,
        params,
        axes,
        sources,
        declarations,
        oracle_pool,
        oracle_grid,
    }
}

pub(super) fn buffer(
    device: &wgpu::Device,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("field replay fixture"),
        contents: bytes,
        usage,
    })
}
pub(super) fn empty(device: &wgpu::Device, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounded field replay"),
        size,
        usage,
        mapped_at_creation: false,
    })
}
pub(super) fn bindings(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &[(u32, &wgpu::Buffer)],
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("field replay bindings"),
        layout,
        entries: &buffers
            .iter()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>(),
    })
}
pub(super) fn compute_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    entry: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: None,
        module: shader,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}
pub(super) fn render_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    entry: &str,
    samples: u32,
    init: bool,
) -> wgpu::RenderPipeline {
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
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
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
}
pub(super) fn texture(device: &wgpu::Device, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("field replay target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
pub(super) fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    target: &wgpu::TextureView,
    groups: &[(u32, &wgpu::BindGroup)],
    tile: (u32, u32, u32, u32),
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(BACKGROUND)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_viewport(
        PANEL.0 as f32,
        PANEL.1 as f32,
        PANEL.2 as f32,
        PANEL.3 as f32,
        0.0,
        1.0,
    );
    pass.set_scissor_rect(tile.0, tile.1, tile.2, tile.3);
    pass.set_pipeline(pipeline);
    for &(index, group) in groups {
        pass.set_bind_group(index, group, &[]);
    }
    pass.draw(0..6, 0..1);
}

pub(super) fn final_image(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    samples: u32,
) -> wgpu::Buffer {
    let kind = if samples == 1 {
        "texture_2d<f32>"
    } else {
        "texture_multisampled_2d<f32>"
    };
    let sample = if samples == 1 { "0" } else { "i32(id.z)" };
    let source = format!(
        r#"
@group(0) @binding(0) var image: {kind};
@group(0) @binding(1) var<storage, read_write> out: array<u32>;
@compute @workgroup_size(8, 8, 1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    if (id.x >= {WIDTH}u || id.y >= {HEIGHT}u || id.z >= {samples}u) {{ return; }}
    let v = vec4<u32>(round(textureLoad(image, vec2<i32>(id.xy), {sample}) * 255.0));
    out[(id.z * {HEIGHT}u + id.y) * {WIDTH}u + id.x] = v.x | v.y << 8u | v.z << 16u | v.w << 24u;
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test-only final image extraction"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipe = compute_pipeline(device, &module, "main");
    let size = u64::from(WIDTH * HEIGHT * samples * 4);
    let gpu = empty(
        device,
        size,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let cpu = empty(
        device,
        size,
        wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    );
    let view = target.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipe.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: gpu.as_entire_binding(),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), samples);
    }
    encoder.copy_buffer_to_buffer(&gpu, 0, &cpu, 0, size);
    cpu
}
pub(super) fn read_final_image(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .unwrap()
        .unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let image = bytemuck::cast_slice(&mapped).to_vec();
    drop(mapped);
    buffer.unmap();
    image
}
