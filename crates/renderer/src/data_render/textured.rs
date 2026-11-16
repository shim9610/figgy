//! Textured layers: RGBA texture upload, the linear sampler and texture
//! bind groups, the shared shader modules, the fullscreen textured pipeline,
//! the unit quad and the AxisLayer handle a composited layer is drawn from.

use super::*;
use crate::shader_assembly::ShaderSet;

/// Upload an RGBA8 pixel array to a 2D texture, usable as `TEXTURE_BINDING`
/// and `COPY_DST`. No mipmaps, no MSAA.
///
/// Format is `Rgba8Unorm` (non-sRGB) on purpose: paired with a non-sRGB
/// surface, the GPU blends bytes directly so the result matches the raster's
/// gamma-incorrect blend path pixel-for-pixel. Switching either side to an
/// sRGB-aware format would break that parity (most visible at AA edges).
///
/// Returns an error if the texture exceeds the cached device 2D texture limit,
/// or if `rgba.len() != width * height * 4` (a panel sized differently from
/// the raster it is given, for example).
pub fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    max_texture_dimension_2d: u32,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> crate::Result<wgpu::Texture> {
    if width == 0 || height == 0 {
        return Err(crate::FiggyError::InvalidChartArea { width, height });
    }
    let max_dim = width.max(height);
    if max_dim > max_texture_dimension_2d {
        return Err(crate::FiggyError::GpuResourceLimit {
            resource: "rgba texture dimension",
            requested: max_dim as u64,
            limit: max_texture_dimension_2d as u64,
        });
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if expected != Some(rgba.len()) {
        return Err(crate::FiggyError::InvalidConfig {
            field: "rgba texture",
            kind: crate::RuleKind::RgbaBufferLength,
        });
    }

    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    let texture_desc = wgpu::TextureDescriptor {
        label: Some("figgy rgba texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    // gpu-alloc: caller
    let texture = crate::renderer::create_texture_checked(device, &texture_desc, "rgba texture")?;

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        size,
    );

    Ok(texture)
}

/// Linear sampler for the overlay quad. `ClampToEdge` avoids edge fringing
/// when the quad's UVs touch 0/1.
pub fn create_linear_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("figgy linear sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        lod_min_clamp: 0.0,
        lod_max_clamp: 0.0,
        compare: None,
        anisotropy_clamp: 1,
        border_color: None,
    })
}

/// Bind-group layout: one 2D texture (binding 0) + one filtering sampler
/// (binding 1), both fragment-only.
pub fn create_texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy texture+sampler layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        },
    )
}

/// Build a bind group binding `view` + `sampler` into the layout above.
pub fn create_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy texture bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

pub(crate) fn multisample_state(sample_count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: sample_count,
        mask: !0,
        alpha_to_coverage_enabled: false,
    }
}

/// One compiled module per WGSL file. Precise, mapped, pick-ring, and styled
/// pipelines reuse these instead of re-running naga on the same source.
#[derive(Clone)]
pub(crate) struct ShaderModules {
    /// Which shader set every data module here was assembled as
    /// (#299): the set of the whole pipeline set built from them.
    pub set: ShaderSet,
    pub fullscreen: wgpu::ShaderModule,
    pub line: wgpu::ShaderModule,
    pub scatter: wgpu::ShaderModule,
    pub errorbar: wgpu::ShaderModule,
    pub bar: wgpu::ShaderModule,
    pub field: wgpu::ShaderModule,
    /// The two assembled vector modules (`vector.rs`): one per input form.
    pub vector_points: wgpu::ShaderModule,
    pub vector_grid: wgpu::ShaderModule,
    /// The streamline arrowheads' vector module (`VectorInput::Path`).
    pub vector_path: wgpu::ShaderModule,
    /// A 3D chart's arrows (`VectorInput::Points3d`, #211).
    pub vector_points_3d: wgpu::ShaderModule,
}

impl ShaderModules {
    pub(crate) fn new(device: &wgpu::Device, set: ShaderSet) -> Self {
        Self {
            set,
            fullscreen: Self::fullscreen(device),
            line: Self::line(device, set),
            scatter: Self::scatter(device, set),
            errorbar: Self::errorbar(device, set),
            bar: Self::bar(device, set),
            field: Self::field(device, set),
            vector_points: vector::shader_module(device, vector::VectorInput::Points, set),
            vector_grid: vector::shader_module(device, vector::VectorInput::Grid, set),
            vector_path: vector::shader_module(device, vector::VectorInput::Path, set),
            vector_points_3d: vector::shader_module(device, vector::VectorInput::Points3d, set),
        }
    }

    /// The assembled vector module for one input form.
    pub(crate) fn vector(&self, input: vector::VectorInput) -> &wgpu::ShaderModule {
        match input {
            vector::VectorInput::Points => &self.vector_points,
            vector::VectorInput::Grid => &self.vector_grid,
            vector::VectorInput::Path => &self.vector_path,
            vector::VectorInput::Points3d => &self.vector_points_3d,
        }
    }
    pub(super) fn fullscreen(device: &wgpu::Device) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy fullscreen textured shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("fullscreen_textured.wgsl").into()),
            },
        )
    }
    pub(super) fn line(device: &wgpu::Device, set: ShaderSet) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy line columnar shader"),
                source: wgpu::ShaderSource::Wgsl(super::LINE_SHADER_BY_SET.of(set).source.into()),
            },
        )
    }
    pub(super) fn scatter(device: &wgpu::Device, set: ShaderSet) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy scatter columnar shader"),
                source: wgpu::ShaderSource::Wgsl(
                    super::SCATTER_SHADER_BY_SET.of(set).source.into(),
                ),
            },
        )
    }
    pub(super) fn errorbar(device: &wgpu::Device, set: ShaderSet) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy errorbar columnar shader"),
                source: wgpu::ShaderSource::Wgsl(
                    super::ERRORBAR_SHADER_BY_SET.of(set).source.into(),
                ),
            },
        )
    }
    pub(super) fn bar(device: &wgpu::Device, set: ShaderSet) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy bar columnar shader"),
                source: wgpu::ShaderSource::Wgsl(super::BAR_SHADER_BY_SET.of(set).source.into()),
            },
        )
    }
    pub(super) fn field(device: &wgpu::Device, set: ShaderSet) -> wgpu::ShaderModule {
        crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some("figgy field columnar shader"),
                source: wgpu::ShaderSource::Wgsl(super::FIELD_SHADER_BY_SET.of(set).source.into()),
            },
        )
    }
}

/// The data modules' render pipelines a browser renderer of `set` compiles
/// in the background, by source, each with its `(stage, pipeline label)`
/// pairs in order (#299). The fullscreen pipeline every frame draws through
/// is not among them: a renderer creates it directly.
#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_render_sources(
    set: ShaderSet,
) -> [crate::browser_compile::RenderSource<'static>; 7] {
    [
        (
            super::LINE_SHADER_BY_SET.of(set).source,
            &[
                ("precise line", "figgy line columnar pipeline"),
                ("colour-mapped line", "figgy line mapped pipeline"),
                ("parallel rows", "figgy line parallel pipeline"),
                (
                    "selected parallel row",
                    "figgy selected parallel row pipeline",
                ),
                ("line area fill", "figgy line area pipeline"),
                ("hand-drawn line", "figgy line styled pipeline"),
                ("milkyway ribbon", "figgy milkyway ribbon pipeline"),
                ("milkyway stars", "figgy milkyway stars pipeline"),
                (
                    "constellation line",
                    "figgy point constellation line pipeline",
                ),
            ],
        ),
        (
            super::SCATTER_SHADER_BY_SET.of(set).source,
            &[
                ("precise scatter", "figgy scatter columnar pipeline"),
                ("mapped scatter", "figgy scatter mapped pipeline"),
                ("picked point ring", "figgy picked point ring pipeline"),
                (
                    "mapped picked point ring",
                    "figgy picked point mapped ring pipeline",
                ),
                ("hand-drawn scatter", "figgy scatter styled pipeline"),
                ("milkyway planets", "figgy milkyway planets pipeline"),
                (
                    "constellation stars",
                    "figgy point constellation stars pipeline",
                ),
            ],
        ),
        (
            super::ERRORBAR_SHADER_BY_SET.of(set).source,
            &[
                ("precise errorbar", "figgy errorbar columnar pipeline"),
                ("mapped errorbar", "figgy errorbar mapped pipeline"),
                ("hand-drawn errorbar", "figgy errorbar styled pipeline"),
                ("milkyway jets", "figgy milkyway jets pipeline"),
            ],
        ),
        (
            super::BAR_SHADER_BY_SET.of(set).source,
            &[
                ("histogram bars", "figgy bar columnar pipeline"),
                (
                    "mapped histogram bars",
                    "figgy mapped bar columnar pipeline",
                ),
                ("histogram pixel envelope", "histogram envelope fill"),
                (
                    "selected histogram bin",
                    "figgy selected histogram bin pipeline",
                ),
            ],
        ),
        (
            super::FIELD_SHADER_BY_SET.of(set).source,
            &[
                ("heatmap field", "figgy field columnar pipeline"),
                ("contour field", "figgy field contour pipeline"),
                (
                    "label-gapped contour field",
                    "figgy labelled field contour pipeline",
                ),
                ("selected field data", "figgy selected field data pipeline"),
            ],
        ),
        (
            super::tri_mesh::TRI_MESH_SHADER_BY_SET.of(set).source,
            &[
                ("triangle mesh", "figgy triangle mesh field pipeline"),
                (
                    "triangle mesh contours",
                    "figgy triangle mesh contour pipeline",
                ),
            ],
        ),
        (
            crate::gpu_contour::CONTOUR_LABEL_SHADER_BY_SET
                .of(set)
                .source,
            &[("contour labels", "figgy contour label pipeline")],
        ),
    ]
}

/// The module, with its `(stage, pipeline label)` pairs, whose render
/// pipelines a browser renderer of `set` schedules only when a frame first
/// needs the pipeline `label` (#299): the vector, 3D surface and ternary
/// region modules, which a chart without such a series never compiles.
/// `None` for a label of the modules [`browser_render_sources`] schedules
/// at creation, or of none.
#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_on_demand_source(
    set: ShaderSet,
    label: &str,
) -> Option<crate::browser_compile::RenderSource<'static>> {
    browser_on_demand_sources(set)
        .into_iter()
        .find(|(_, stages)| stages.iter().any(|(_, listed)| *listed == label))
}

/// The module whose compute pipelines include `label` and that a browser
/// renderer compiles on demand (#299): the arc prefix scan, the ternary
/// ring-start scan, the contour label anchors and the histogram envelope.
/// A frame's preparation creates them when a series first needs them.
#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_on_demand_compute_source(
    set: ShaderSet,
    label: &str,
) -> Option<crate::browser_compile::RenderSource<'static>> {
    const ANCHORS: [(&str, &str); 4] = [
        (
            "contour label anchors",
            crate::gpu_contour::CONTOUR_ANCHOR_LABELS[0],
        ),
        (
            "contour label anchors",
            crate::gpu_contour::CONTOUR_ANCHOR_LABELS[1],
        ),
        (
            "contour label anchors",
            crate::gpu_contour::CONTOUR_ANCHOR_LABELS[2],
        ),
        (
            "contour label anchors",
            crate::gpu_contour::CONTOUR_ANCHOR_LABELS[3],
        ),
    ];
    let sources: [crate::browser_compile::RenderSource<'static>; 4] = [
        (
            super::line_arc::LINE_ARC_SHADER_BY_SET.of(set).source,
            &[("arc prefix scan", super::line_arc::ARC_SCAN_LABEL)],
        ),
        (
            super::ternary_region::TERNARY_REGION_SHADER_BY_SET
                .of(set)
                .source,
            &[(
                "ternary ring-start scan",
                super::ternary_region::RING_SCAN_LABEL,
            )],
        ),
        (
            crate::gpu_contour::CONTOUR_ANCHOR_SHADER_BY_SET
                .of(set)
                .source,
            &ANCHORS,
        ),
        (
            super::BAR_SHADER_BY_SET.of(set).source,
            &[(
                "histogram pixel envelope",
                super::bar_envelope::ENVELOPE_SCAN_LABEL,
            )],
        ),
    ];
    sources
        .into_iter()
        .find(|(_, stages)| stages.iter().any(|(_, listed)| *listed == label))
}

/// Every module whose render pipelines a browser renderer compiles in the
/// background (#299): the ones [`browser_render_sources`] schedules at
/// creation, then the ones compiled on demand. An export schedules from
/// these the pipelines it draws with, for its own target.
#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_every_render_source(
    set: ShaderSet,
) -> [crate::browser_compile::RenderSource<'static>; 14] {
    let [a, b, c, d, e, f, g] = browser_render_sources(set);
    let [h, i, j, k, l, m, n] = browser_on_demand_sources(set);
    [a, b, c, d, e, f, g, h, i, j, k, l, m, n]
}

/// The modules compiled on demand (#299): vector, 3D surface and ternary
/// region.
#[cfg(target_arch = "wasm32")]
fn browser_on_demand_sources(set: ShaderSet) -> [crate::browser_compile::RenderSource<'static>; 7] {
    const SURFACE: [(&str, &str); 5] = [
        ("3D surface", "figgy surface pipeline"),
        ("selected 3D surface", "figgy surface selection pipeline"),
        ("3D surface pick", "figgy surface pick pipeline"),
        ("3D floor contour", "figgy floor contour pipeline"),
        ("3D floor contour pick", "figgy floor contour pick pipeline"),
    ];
    const TRI_SURFACE: [(&str, &str); 3] = [
        ("3D triangle surface", "figgy triangle surface pipeline"),
        (
            "selected 3D triangle surface",
            "figgy triangle surface selection pipeline",
        ),
        (
            "3D triangle surface pick",
            "figgy triangle surface pick pipeline",
        ),
    ];
    let sources: [crate::browser_compile::RenderSource<'static>; 6] = [
        (
            super::vector::VECTOR_POINTS_SHADER_BY_SET.of(set).source,
            &[("vector points", "figgy vector points pipeline")],
        ),
        (
            super::vector::VECTOR_GRID_SHADER_BY_SET.of(set).source,
            &[("vector grid", "figgy vector grid pipeline")],
        ),
        (
            super::vector::VECTOR_PATH_SHADER_BY_SET.of(set).source,
            &[("vector path", "figgy vector path pipeline")],
        ),
        (
            super::vector::VECTOR_POINTS_3D_SHADER_BY_SET.of(set).source,
            &[
                ("3D vector", "figgy vector 3D pipeline"),
                ("3D vector pick", "figgy vector 3D pick pipeline"),
            ],
        ),
        (
            super::surface_mesh::SURFACE_SHADER_BY_SET.of(set).source,
            &SURFACE,
        ),
        (
            super::surface_mesh::TRI_SURFACE_SHADER_BY_SET
                .of(set)
                .source,
            &TRI_SURFACE,
        ),
    ];
    let region: crate::browser_compile::RenderSource<'static> = (
        super::ternary_region::TERNARY_REGION_SHADER_BY_SET
            .of(set)
            .source,
        &[
            (
                "ternary region stencil",
                "figgy ternary region stencil pipeline",
            ),
            (
                "ternary region cover",
                "figgy ternary region cover pipeline",
            ),
        ],
    );
    let [a, b, c, d, e, f] = sources;
    [a, b, c, d, e, f, region]
}

/// Compile the render pipelines `feature` includes (all for `None`) through
/// the browser's asynchronous compiler. A renderer with a background queue
/// for this target (`queue`) waits for its compiles, issuing them ahead of
/// the rest, so nothing is compiled twice and no more than the queue's limit
/// is in flight (#299); any other issues them itself.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn prewarm_browser_render_feature(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    feature: Option<crate::PreparationFeature>,
    set: ShaderSet,
    queue: Option<&crate::browser_compile::CompileQueue>,
    observer: &mut dyn FnMut(crate::InitEvent),
) -> Result<(), String> {
    if let Some(queue) = queue.filter(|queue| queue.serves(target_format, sample_count)) {
        return queue
            .wait_for(
                set,
                |label| feature.is_none_or(|feature| feature.includes_label(label)),
                observer,
            )
            .await;
    }
    let data = browser_render_sources(set);
    let fullscreen: crate::browser_compile::RenderSource<'static> = (
        include_str!("fullscreen_textured.wgsl"),
        &[("fullscreen textured", "figgy fullscreen textured pipeline")],
    );
    let groups: [crate::browser_compile::RenderSource<'_>; 8] = [
        fullscreen, data[0], data[1], data[2], data[3], data[4], data[5], data[6],
    ];
    crate::browser_compile::render(
        device,
        &groups,
        target_format,
        sample_count,
        feature,
        observer,
    )
    .await
}

/// Build the fullscreen textured-quad pipeline. The shader emits its own
/// vertices via `vertex_index`, so no vertex buffers are needed.
///
/// Blend is `PREMULTIPLIED_ALPHA_BLENDING` because the CPU raster works with
/// `AlphaType::Premul`; using plain `ALPHA_BLENDING` would multiply by alpha
/// twice and darken AA edges.
pub fn create_fullscreen_textured_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::fullscreen(device);
    create_fullscreen_textured_pipeline_with_sample_count(
        device,
        &shader,
        bind_group_layout,
        target_format,
        1,
    )
}

pub(crate) fn create_fullscreen_textured_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let pipeline_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy fullscreen textured pipeline layout"),
            bind_group_layouts: &[Some(bind_group_layout)],
            immediate_size: 0,
        },
    );

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some("figgy fullscreen textured pipeline"),
            layout: Some(&pipeline_layout),

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

/// Clear with `clear_color` and draw a fullscreen textured quad on top.
pub fn render_textured(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    clear_color: wgpu::Color,
) -> RenderOutcome {
    let frame = match acquire_surface_frame(surface) {
        Ok(t) => t,
        Err(outcome) => return outcome,
    };

    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("figgy textured encoder"),
    });

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("figgy textured pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    queue.submit(std::iter::once(encoder.finish()));
    queue.present(frame);

    RenderOutcome::Rendered
}

/// Test-only UV gradient: R = u, G = v (top→bottom), B = 0. Useful as a
/// visual smoke test for UV/NDC orientation: top-left black, top-right red,
/// bottom-left green, bottom-right yellow.
pub fn make_uv_gradient(width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height {
        let v = y as f32 / (height - 1).max(1) as f32; // 0 at top, 1 at bottom
        let g = (v * 255.0).round() as u8;
        for x in 0..width {
            let u = x as f32 / (width - 1).max(1) as f32; // 0 at left, 1 at right
            let r = (u * 255.0).round() as u8;
            buf.extend_from_slice(&[r, g, 0, 255]);
        }
    }
    buf
}

pub fn create_unit_centered_quad_vertex_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    let vertices: [f32; 8] = [
        -1.0, -1.0, // LB
        1.0, -1.0, // RB
        -1.0, 1.0, // LT
        1.0, 1.0, // RT
    ];
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&vertices),
        )
    };
    // gpu-alloc: caller
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy unit-centered quad"),
        contents: bytes,
        usage: wgpu::BufferUsages::VERTEX,
    })
}

pub struct AxisLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub bind_group: &'a wgpu::BindGroup,
}
