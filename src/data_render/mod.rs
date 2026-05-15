//! GPU data rendering via wgpu.
//!
//! wgpu owns the window surface; skia rasterises axes / titles / labels on the
//! CPU and uploads them as a texture. A single render pass draws data
//! primitives followed by the skia texture overlay, then presents.

use crate::color::Color;
use crate::config::Config;
use crate::layout::Rect;

use wgpu::util::DeviceExt;

pub mod column_pool;
pub use column_pool::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, ColumnSlot, DefragPolicy, FreeRegion,
};

// Instance, adapter, surface, and device setup.

/// Create a `wgpu::Instance` with default settings (all native backends, no
/// pre-bound display handle). Synchronous and infallible.
pub fn create_instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor::default())
}

/// Pick any adapter, with no surface compatibility constraint.
/// Returns `Err` on headless / driver-less environments so callers can skip.
pub fn request_adapter(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: None,
    };
    pollster::block_on(instance.request_adapter(&options))
}

/// Create a wgpu `Surface` for any window-like target. figgy itself does not
/// depend on winit; the caller passes its own window handle (winit / egui /
/// iced / ...). Using an `Arc<Window>` yields a `Surface<'static>` since
/// ownership is shared into the surface.
pub fn create_surface_for_window<'a>(
    instance: &wgpu::Instance,
    target: impl Into<wgpu::SurfaceTarget<'a>>,
) -> Result<wgpu::Surface<'a>, wgpu::CreateSurfaceError> {
    instance.create_surface(target)
}

/// Pick an adapter that is guaranteed to present to the given surface. On
/// hybrid-GPU systems only one of the GPUs may be compatible, so this must be
/// called once the surface exists.
pub fn request_adapter_for_surface(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: Some(surface),
    };
    pollster::block_on(instance.request_adapter(&options))
}

/// Open a logical device and its queue with WebGPU baseline limits.
pub fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("figgy main device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    pollster::block_on(adapter.request_device(&descriptor))
}

/// Build and apply a `SurfaceConfiguration` for the given size. Callers should
/// keep the returned config so they can call [`reconfigure_surface`] on resize.
///
/// The format selection prefers a **non-sRGB** format. See `upload_rgba_texture`
/// for why: we want pixel-level parity with skia's gamma-incorrect blending,
/// which requires the GPU side to also blend bytes directly.
fn target_format_is_supported(
    device_features: wgpu::Features,
    format: wgpu::TextureFormat,
) -> bool {
    if !device_features.contains(format.required_features()) {
        return false;
    }
    let features = format.guaranteed_format_features(device_features);
    features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        && features
            .flags
            .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
}

fn is_rgba8_surface_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

fn choose_surface_format(
    device_features: wgpu::Features,
    caps: &wgpu::SurfaceCapabilities,
) -> Option<wgpu::TextureFormat> {
    let supported = || {
        caps.formats
            .iter()
            .copied()
            .filter(|f| target_format_is_supported(device_features, *f))
    };

    supported()
        .filter(|f| is_rgba8_surface_format(*f))
        .find(|f| !f.is_srgb())
        .or_else(|| supported().find(|f| is_rgba8_surface_format(*f)))
        .or_else(|| supported().find(|f| *f == wgpu::TextureFormat::Rgb10a2Unorm))
        .or_else(|| supported().find(|f| !f.is_srgb()))
        .or_else(|| {
            caps.formats
                .iter()
                .copied()
                .find(|f| target_format_is_supported(device_features, *f))
        })
}

fn choose_present_mode(caps: &wgpu::SurfaceCapabilities) -> Option<wgpu::PresentMode> {
    #[cfg(target_arch = "wasm32")]
    const PREFERRED: &[wgpu::PresentMode] = &[
        wgpu::PresentMode::Fifo,
        wgpu::PresentMode::AutoVsync,
    ];

    #[cfg(not(target_arch = "wasm32"))]
    const PREFERRED: &[wgpu::PresentMode] = &[
        wgpu::PresentMode::Fifo,
        wgpu::PresentMode::AutoVsync,
        wgpu::PresentMode::FifoRelaxed,
        wgpu::PresentMode::AutoNoVsync,
        wgpu::PresentMode::Mailbox,
        wgpu::PresentMode::Immediate,
    ];

    let preferred = PREFERRED
        .iter()
        .copied()
        .find(|mode| caps.present_modes.contains(mode));

    #[cfg(target_arch = "wasm32")]
    {
        preferred
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        preferred.or_else(|| caps.present_modes.first().copied())
    }
}

fn choose_alpha_mode(caps: &wgpu::SurfaceCapabilities) -> Option<wgpu::CompositeAlphaMode> {
    [
        wgpu::CompositeAlphaMode::Auto,
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
    ]
    .into_iter()
    .find(|mode| caps.alpha_modes.contains(mode))
    .or_else(|| caps.alpha_modes.first().copied())
}

pub fn try_configure_surface(
    surface: &wgpu::Surface<'_>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> crate::Result<wgpu::SurfaceConfiguration> {
    let caps = surface.get_capabilities(adapter);
    let device_features = device.features();

    // Prefer non-sRGB so GPU blending matches skia's gamma-incorrect path.
    let format = choose_surface_format(device_features, &caps).ok_or_else(|| {
        crate::FiggyError::SurfaceConfigurationFailed {
            reason: "surface reported no figgy-compatible renderable/blendable texture formats".into(),
        }
    })?;
    let present_mode = choose_present_mode(&caps).ok_or_else(|| {
        crate::FiggyError::SurfaceConfigurationFailed {
            reason: "surface reported no supported present modes".into(),
        }
    })?;
    let alpha_mode = choose_alpha_mode(&caps).ok_or_else(|| {
        crate::FiggyError::SurfaceConfigurationFailed {
            reason: "surface reported no supported alpha modes".into(),
        }
    })?;

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode,
        alpha_mode,
        view_formats: Vec::new(),
        desired_maximum_frame_latency: 2,
    };

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        surface.configure(device, &config);
    }))
    .map_err(|_| crate::FiggyError::SurfaceConfigurationFailed {
        reason: "wgpu Surface::configure panicked".into(),
    })?;

    Ok(config)
}

pub fn configure_surface(
    surface: &wgpu::Surface<'_>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> crate::Result<wgpu::SurfaceConfiguration> {
    try_configure_surface(surface, adapter, device, width, height)
}

/// Update only width/height on the existing config and reconfigure. Other
/// fields (format/present_mode/...) are preserved.
pub fn reconfigure_surface(
    surface: &wgpu::Surface<'_>,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    config: &mut wgpu::SurfaceConfiguration,
    width: u32,
    height: u32,
) -> crate::Result<()> {
    *config = try_configure_surface(surface, adapter, device, width, height)?;
    Ok(())
}

/// Result of a render call. The caller branches on this to decide whether to
/// reconfigure the surface or just retry the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Frame drawn and presented.
    Rendered,
    /// Swap chain invalidated — caller must reconfigure the surface.
    Reconfigure,
    /// Skip this frame (occluded / timeout); retry next frame.
    Skipped,
}

/// Clear the current frame to `clear_color` and present.
///
/// `clear_color` is in linear RGB (0..=1). On an sRGB surface the GPU applies
/// gamma encoding automatically; on a non-sRGB surface the bytes are written
/// as-is.
pub fn render_clear(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    clear_color: wgpu::Color,
) -> RenderOutcome {
    let frame = match surface.get_current_texture() {
        Ok(t) => t,
        Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
            return RenderOutcome::Reconfigure;
        }
        Err(wgpu::SurfaceError::Timeout | wgpu::SurfaceError::OutOfMemory) => {
            return RenderOutcome::Skipped;
        }
        Err(_) => return RenderOutcome::Skipped,
    };

    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("figgy clear encoder"),
    });

    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("figgy clear pass"),
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
        });
    }

    queue.submit(std::iter::once(encoder.finish()));
    frame.present();

    RenderOutcome::Rendered
}

// Texture upload, samplers, bind groups, and pipelines for the skia overlay.

/// Upload an RGBA8 pixel array to a 2D texture, usable as `TEXTURE_BINDING`
/// and `COPY_DST`. No mipmaps, no MSAA.
///
/// Format is `Rgba8Unorm` (non-sRGB) on purpose: paired with a non-sRGB
/// surface, the GPU blends bytes directly so the result matches skia's
/// gamma-incorrect blend path pixel-for-pixel. Switching either side to an
/// sRGB-aware format would break that parity (most visible at AA edges).
///
/// Returns an error if the texture exceeds the cached device 2D texture limit.
///
/// # Panics
/// If `rgba.len() != width * height * 4`; callers already own that CPU buffer
/// and this remains an internal shape invariant rather than a hardware check.
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
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "rgba buffer length mismatch: got {}, expected {} ({}x{} RGBA8)",
        rgba.len(),
        expected,
        width,
        height
    );

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
    let texture = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        device.create_texture(&texture_desc)
    }))
    .map_err(|_| crate::FiggyError::GpuResourceAllocationFailed {
        resource: "rgba texture",
        reason: "wgpu Device::create_texture panicked".into(),
    })?;

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
        mipmap_filter: wgpu::FilterMode::Nearest,
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
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    })
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

/// Build the fullscreen textured-quad pipeline. The shader emits its own
/// vertices via `vertex_index`, so no vertex buffers are needed.
///
/// Blend is `PREMULTIPLIED_ALPHA_BLENDING` because skia rasters with
/// `AlphaType::Premul`; using plain `ALPHA_BLENDING` would multiply by alpha
/// twice and darken AA edges.
pub fn create_fullscreen_textured_pipeline(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy fullscreen textured shader"),
        source: wgpu::ShaderSource::Wgsl(
            include_str!("fullscreen_textured.wgsl").into(),
        ),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy fullscreen textured pipeline layout"),
        bind_group_layouts: &[bind_group_layout],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy fullscreen textured pipeline"),
        layout: Some(&pipeline_layout),

        vertex: wgpu::VertexState {
            module: &shader,
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

        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },

        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),

        multiview: None,
        cache: None,
    })
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
    let frame = match surface.get_current_texture() {
        Ok(t) => t,
        Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
            return RenderOutcome::Reconfigure;
        }
        Err(_) => return RenderOutcome::Skipped,
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
        });

        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    queue.submit(std::iter::once(encoder.finish()));
    frame.present();

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
        -1.0,  1.0, // LT
         1.0,  1.0, // RT
    ];
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&vertices),
        )
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy unit-centered quad"),
        contents: bytes,
        usage: wgpu::BufferUsages::VERTEX,
    })
}

// Transform uniform (data-space -> NDC), shared by scatter / line / errorbar.
// Vertex buffers carry data-space values directly; resize/zoom only updates
// this uniform, never the instance data.

/// Shared transform uniform for scatter / line / errorbar shaders.
///
/// 48 bytes, 16-byte aligned (WGSL uniform std140-like layout). `_pad` keeps
/// the struct a multiple of 16 bytes — re-check if fields are added.
///
/// `point_size_ndc` is the scatter point radius in NDC, supplied per-axis so
/// non-square panels still draw circles (not ellipses):
/// `point_size_ndc = (pixel_radius / chart_w, pixel_radius / chart_h)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterTransform {
    pub data_min: [f32; 2],       // offset 0
    pub data_max: [f32; 2],       // offset 8
    pub point_size_ndc: [f32; 2], // offset 16
    /// Per-axis flag: 0.0 = linear, 1.0 = log10.
    pub scale_log: [f32; 2],      // offset 24
    /// `(2 / chart_w, 2 / chart_h)` — 1 pixel in NDC. Used by the line shader
    /// to convert pixel thickness into NDC.
    pub pixel_to_ndc: [f32; 2],   // offset 32
    pub _pad: [f32; 2],           // offset 40 → 48 byte
}

/// Allocate the transform uniform buffer with `COPY_DST` so subsequent
/// updates can use `queue.write_buffer` instead of recreating it.
pub fn create_scatter_transform_uniform_buffer(
    device: &wgpu::Device,
    transform: &ScatterTransform,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter transform uniform"),
        contents: bytemuck::bytes_of(transform),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Overwrite the uniform buffer in place. Call on resize or autoscale change.
pub fn update_scatter_transform(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    transform: &ScatterTransform,
) {
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(transform));
}

/// Bind-group layout for the transform uniform. Vertex-only — the data→NDC
/// mapping happens in the vertex stage.
pub fn create_scatter_transform_bind_group_layout(
    device: &wgpu::Device,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy scatter transform bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Bind the transform uniform buffer into the layout above.
pub fn create_scatter_transform_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy scatter transform bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

// Primitive style uniform — color and per-primitive options.
// Bind groups: group(0) = transform (shared), group(1) = style (per primitive).

/// Per-primitive style uniform. 32 bytes, 16-byte aligned.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimitiveStyle {
    pub color_premul: [f32; 4],   // 16
    pub line_width_px: f32,       // 4
    pub _pad: [f32; 3],           // 12 → 32 byte align
}

impl PrimitiveStyle {
    /// Convert a straight-RGBA `Color` to the premultiplied form expected by
    /// the `PREMULTIPLIED_ALPHA_BLENDING` pipeline.
    pub fn from_color(c: Color) -> Self {
        Self::from_color_with_width(c, 1.0)
    }

    /// `line_width_px` is the pixel thickness for line series; ignored by
    /// other primitives.
    pub fn from_color_with_width(c: Color, line_width_px: f32) -> Self {
        let a = c.a.clamp(0.0, 1.0);
        Self {
            color_premul: [c.r * a, c.g * a, c.b * a, a],
            line_width_px,
            _pad: [0.0; 3],
        }
    }
}

/// Bind-group layout for the style uniform. `VERTEX_FRAGMENT` because the
/// line vertex shader reads `line_width_px`.
pub fn create_style_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy primitive style bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub fn create_style_uniform_buffer(
    device: &wgpu::Device,
    style: &PrimitiveStyle,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy primitive style uniform"),
        contents: bytemuck::bytes_of(style),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

pub fn create_style_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy primitive style bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

pub fn update_style(queue: &wgpu::Queue, buffer: &wgpu::Buffer, style: &PrimitiveStyle) {
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(style));
}

pub struct AxisLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub bind_group: &'a wgpu::BindGroup,
}

/// Build a [`ScatterTransform`] from a `Config` and the desired pixel radius
/// for scatter points.
///
/// The transform encodes data-space ranges, log-axis flags, and pixel-to-NDC
/// scale factors. When `data_area` is smaller than `chart_area`, the data
/// range is **extended** so the same data extents land inside data_area only,
/// and the rest of the panel viewport is empty space.
#[allow(clippy::too_many_arguments)]
pub fn scatter_transform_from_config(
    config: &Config,
    point_pixel_radius: f32,
) -> ScatterTransform {
    let ca = &config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;

    use crate::config::AxisScale;
    let log_x = matches!(config.bottom_x.scale, AxisScale::Logarithmic);
    let log_y = matches!(config.left_y.scale, AxisScale::Logarithmic);

    // For log axes, convert min/max into log10 space. Non-positive data
    // becomes -inf/NaN in the shader — callers must supply positive values.
    let to_log = |v: f64| (v.max(f64::MIN_POSITIVE)).log10() as f32;

    let data_min_x = if log_x { to_log(config.bottom_x.min) } else { config.bottom_x.min as f32 };
    let data_max_x = if log_x { to_log(config.bottom_x.max) } else { config.bottom_x.max as f32 };
    let data_min_y = if log_y { to_log(config.left_y.min)   } else { config.left_y.min as f32 };
    let data_max_y = if log_y { to_log(config.left_y.max)   } else { config.left_y.max as f32 };

    let scale_log = [if log_x { 1.0 } else { 0.0 }, if log_y { 1.0 } else { 0.0 }];

    // Convert pixel radius to per-axis NDC so circles stay circular in
    // non-square panels.
    let point_size_ndc = [
        point_pixel_radius / chart_w,
        point_pixel_radius / chart_h,
    ];

    // 1 px in NDC: NDC spans 2 across chart_w pixels.
    let pixel_to_ndc = [2.0 / chart_w, 2.0 / chart_h];

    // No data_area → use the data range directly (no extension).
    let da: Rect = match config.data_area() {
        Ok(d) => d.0,
        Err(_) => {
            return ScatterTransform {
                data_min: [data_min_x, data_min_y],
                data_max: [data_max_x, data_max_y],
                point_size_ndc,
                scale_log,
                pixel_to_ndc,
                _pad: [0.0; 2],
            };
        }
    };

    // Relative to chart_area origin (drop the panel's global offset).
    let rel_x = da.x as i64 - ca.x as i64;
    let rel_y = da.y as i64 - ca.y as i64;
    let rel_x = rel_x as f32;
    let rel_y = rel_y as f32;

    // Fractions of chart_area covered by data_area, in NDC orientation
    // (X left/right, Y bottom/top — screen is Y-down, NDC is Y-up).
    let sx = rel_x / chart_w;
    let ex = (rel_x + da.width as f32) / chart_w;
    let sy = (chart_h - (rel_y + da.height as f32)) / chart_h;
    let ey = (chart_h - rel_y) / chart_h;

    let extend = |min: f32, max: f32, s: f32, e: f32| -> (f32, f32) {
        let span = e - s;
        if span.abs() < f32::EPSILON {
            return (min, max);
        }
        let range_ext = (max - min) / span;
        let min_ext = min - s * range_ext;
        let max_ext = min_ext + range_ext;
        (min_ext, max_ext)
    };

    let (min_x_ext, max_x_ext) = extend(data_min_x, data_max_x, sx, ex);
    let (min_y_ext, max_y_ext) = extend(data_min_y, data_max_y, sy, ey);

    ScatterTransform {
        data_min: [min_x_ext, min_y_ext],
        data_max: [max_x_ext, max_y_ext],
        point_size_ndc,
        scale_log,
        pixel_to_ndc,
        _pad: [0.0; 2],
    }
}

// Columnar pipelines backed by ColumnPool.
//
// Every column is uploaded once into a ColumnPool buffer; charts draw by
// binding `pool.buffer().slice(handle.byte_range())` into vertex slots.
// Shaders take a single column per slot; lengths and offsets are decided by
// the caller via the slice and draw range.

/// Columnar line pipeline. slots: 0=x_a, 1=y_a, 2=x_b, 3=y_b (per-instance).
pub fn create_line_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy line columnar shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("line_columnar.wgsl").into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy line columnar layout"),
        bind_group_layouts: &[transform_bgl, style_bgl],
        push_constant_ranges: &[],
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy line columnar pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            // 4 per-instance f32 slots: x_a, y_a, x_b, y_b. The same X/Y
            // columns are bound twice; the second pair starts 4 bytes (one
            // f32) later, so instance i sees points [i] and [i+1] together.
            // Each instance emits a 4-vertex quad strip for one segment.
            buffers: &[
                // slot 0: x_a (X column from offset 0)
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 0,
                    }],
                },
                // slot 1: y_a
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 1,
                    }],
                },
                // slot 2: x_b (X column from offset 4)
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 2,
                    }],
                },
                // slot 3: y_b
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 3,
                    }],
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            // 4 vertices per instance — a 2-triangle strip.
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Columnar SDF scatter pipeline.
/// slot 0 (per-vertex): unit quad, slot 1 (per-instance): X, slot 2: Y.
pub fn create_scatter_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy scatter columnar shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("scatter_columnar.wgsl").into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy scatter columnar layout"),
        bind_group_layouts: &[transform_bgl, style_bgl],
        push_constant_ranges: &[],
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;
    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy scatter columnar pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                // slot 0: unit quad (per-vertex, vec2)
                wgpu::VertexBufferLayout {
                    array_stride: vec2_stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                },
                // slot 1: X column (per-instance, f32)
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 1,
                    }],
                },
                // slot 2: Y column (per-instance, f32)
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 2,
                    }],
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Columnar errorbar pipeline.
/// slots 0..5 are per-instance f32: x, y, err_y_lo, err_y_hi, err_x_lo,
/// err_x_hi. Each instance emits 12 vertices on a `LineList`.
pub fn create_errorbar_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy errorbar columnar shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("errorbar_columnar.wgsl").into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy errorbar columnar layout"),
        bind_group_layouts: &[transform_bgl, style_bgl],
        push_constant_ranges: &[],
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;
    // Hold attributes in const arrays to avoid temporary-lifetime issues.
    const ATTR0: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 0,
    }];
    const ATTR1: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 1,
    }];
    const ATTR2: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 2,
    }];
    const ATTR3: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 3,
    }];
    const ATTR4: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 4,
    }];
    const ATTR5: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32, offset: 0, shader_location: 5,
    }];
    let buffers = [
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR0,
        },
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR1,
        },
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR2,
        },
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR3,
        },
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR4,
        },
        wgpu::VertexBufferLayout {
            array_stride: f32_stride, step_mode: wgpu::VertexStepMode::Instance, attributes: &ATTR5,
        },
    ];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy errorbar columnar pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Bundle of handles for drawing one columnar line series. `pool_buffer` is
/// passed in separately so it can be shared across series.
pub struct ColumnLineLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
}

pub struct ColumnScatterLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
}

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    pub errorbar: Option<ColumnErrorBarDraw<'a>>,
    pub line: Option<ColumnLineLayer<'a>>,
    pub scatter: Option<ColumnScatterLayer<'a>>,
}

pub struct ColumnErrorBarDraw<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub err_y_lo: ColumnHandle,
    pub err_y_hi: ColumnHandle,
    pub err_x_lo: ColumnHandle,
    pub err_x_hi: ColumnHandle,
}

/// Clamp a rect into `(0..target.0, 0..target.1)`. Returns `None` if the
/// clamped width or height is zero so callers can skip the draw entirely.
fn clamp_rect_to_target(r: Rect, target: (u32, u32)) -> Option<Rect> {
    let (tw, th) = target;
    let x0 = r.x.min(tw);
    let y0 = r.y.min(th);
    let x1 = r.x.saturating_add(r.width).min(tw);
    let y1 = r.y.saturating_add(r.height).min(th);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 { None } else { Some(Rect { x: x0, y: y0, width: w, height: h }) }
}

/// Issue draw calls for one series' data primitives. The caller must have
/// already set the viewport (panel) and scissor (data_area).
fn issue_series_data(
    pass: &mut wgpu::RenderPass<'_>,
    series: &SeriesLayers<'_>,
) {
    if let Some(eb) = series.errorbar.as_ref() {
        let count = [
            eb.x.len_values,
            eb.y.len_values,
            eb.err_y_lo.len_values,
            eb.err_y_hi.len_values,
            eb.err_x_lo.len_values,
            eb.err_x_hi.len_values,
        ]
        .into_iter()
        .min()
        .unwrap_or(0) as u32;
        if count > 0 {
            pass.set_pipeline(eb.pipeline);
            pass.set_bind_group(0, eb.transform_bg, &[]);
            pass.set_bind_group(1, eb.style_bg, &[]);
            pass.set_vertex_buffer(0, eb.pool_buffer.slice(eb.x.byte_range()));
            pass.set_vertex_buffer(1, eb.pool_buffer.slice(eb.y.byte_range()));
            pass.set_vertex_buffer(2, eb.pool_buffer.slice(eb.err_y_lo.byte_range()));
            pass.set_vertex_buffer(3, eb.pool_buffer.slice(eb.err_y_hi.byte_range()));
            pass.set_vertex_buffer(4, eb.pool_buffer.slice(eb.err_x_lo.byte_range()));
            pass.set_vertex_buffer(5, eb.pool_buffer.slice(eb.err_x_hi.byte_range()));
            pass.draw(0..12, 0..count);
        }
    }

    if let Some(l) = series.line.as_ref() {
        let count = l.x.len_values.min(l.y.len_values) as u32;
        if count >= 2 {
            pass.set_pipeline(l.pipeline);
            pass.set_bind_group(0, l.transform_bg, &[]);
            pass.set_bind_group(1, l.style_bg, &[]);
            let x_full = l.x.byte_range();
            let y_full = l.y.byte_range();
            let x_shift = (x_full.start + 4)..x_full.end;
            let y_shift = (y_full.start + 4)..y_full.end;
            pass.set_vertex_buffer(0, l.pool_buffer.slice(x_full));
            pass.set_vertex_buffer(1, l.pool_buffer.slice(y_full));
            pass.set_vertex_buffer(2, l.pool_buffer.slice(x_shift));
            pass.set_vertex_buffer(3, l.pool_buffer.slice(y_shift));
            pass.draw(0..4, 0..(count - 1));
        }
    }

    if let Some(s) = series.scatter.as_ref() {
        let count = s.x.len_values.min(s.y.len_values) as u32;
        if count > 0 {
            pass.set_pipeline(s.pipeline);
            pass.set_bind_group(0, s.transform_bg, &[]);
            pass.set_bind_group(1, s.style_bg, &[]);
            pass.set_vertex_buffer(0, s.quad_vb.slice(..));
            pass.set_vertex_buffer(1, s.pool_buffer.slice(s.x.byte_range()));
            pass.set_vertex_buffer(2, s.pool_buffer.slice(s.y.byte_range()));
            pass.draw(0..4, 0..count);
        }
    }
}

/// Draw one chart panel: grid → every series → decoration. The function
/// configures viewport and scissor itself, so callers only supply rects.
///
/// `target_size` is the pixel size of the current color attachment;
/// `panel_rect` / `data_area` are clamped to it to avoid wgpu validation
/// errors when a panel partially exits the surface.
#[allow(clippy::too_many_arguments)]
pub fn draw_chart_panel_columnar(
    pass: &mut wgpu::RenderPass<'_>,
    target_size: (u32, u32),
    panel_rect: Rect,
    data_area: Rect,
    grid: AxisLayer<'_>,
    series_list: &[SeriesLayers<'_>],
    decoration: AxisLayer<'_>,
) {
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else { return };
    let Some(data_clamped) = clamp_rect_to_target(data_area, target_size) else { return };

    pass.set_viewport(
        panel_clamped.x as f32,
        panel_clamped.y as f32,
        panel_clamped.width as f32,
        panel_clamped.height as f32,
        0.0, 1.0,
    );

    // 1) Grid layer (under data).
    pass.set_scissor_rect(
        panel_clamped.x, panel_clamped.y, panel_clamped.width, panel_clamped.height,
    );
    pass.set_pipeline(grid.pipeline);
    pass.set_bind_group(0, grid.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 2) Data primitives — scissor once to data_area, then issue every series.
    pass.set_scissor_rect(
        data_clamped.x, data_clamped.y, data_clamped.width, data_clamped.height,
    );
    for s in series_list {
        issue_series_data(pass, s);
    }

    // 3) Decoration layer (over data).
    pass.set_scissor_rect(
        panel_clamped.x, panel_clamped.y, panel_clamped.width, panel_clamped.height,
    );
    pass.set_pipeline(decoration.pipeline);
    pass.set_bind_group(0, decoration.bind_group, &[]);
    pass.draw(0..3, 0..1);
}

// Tests.

#[cfg(test)]
mod tests {
    use super::*;

    /// Instance creation must not panic, even on driver-less environments
    /// (it does not talk to any GPU yet).
    #[test]
    fn instance_creation_succeeds() {
        let _instance = create_instance();
    }

    /// Print adapter info when one is available; otherwise skip silently.
    /// Mostly useful locally with `cargo test -- --nocapture`.
    #[test]
    fn adapter_request_prints_info_when_available() {
        let instance = create_instance();
        match request_adapter(&instance) {
            Ok(adapter) => {
                let info = adapter.get_info();
                println!("adapter name    : {}", info.name);
                println!("adapter backend : {:?}", info.backend);
                println!("adapter type    : {:?}", info.device_type);
                println!("adapter driver  : {} / {}", info.driver, info.driver_info);
            }
            Err(e) => {
                println!("no adapter available in this environment: {e}");
            }
        }
    }

    /// Smoke-test the texture-upload API path (no readback). Validation
    /// would surface here if the call shape were wrong.
    #[test]
    fn rgba_texture_upload_roundtrips_api() {
        let instance = create_instance();
        let Ok(adapter) = request_adapter(&instance) else {
            println!("no adapter — skipping texture upload test");
            return;
        };
        let (device, queue) = request_device(&adapter).expect("device");

        // 2x2 checkerboard: 4 RGBA pixels = 16 bytes.
        let rgba: [u8; 16] = [
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
            0, 0, 255, 255, // blue
            255, 255, 0, 255, // yellow
        ];
        let tex = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        ).expect("upload texture");

        // Wait for the queued write_texture to complete; validation errors
        // surface during this poll.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        assert_eq!(tex.width(), 2);
        assert_eq!(tex.height(), 2);
        assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
    }


    /// Compile the WGSL and build the fullscreen textured pipeline. Shader
    /// or layout mismatches would panic during creation.
    #[test]
    fn fullscreen_textured_pipeline_compiles_and_creates() {
        let instance = create_instance();
        let Ok(adapter) = request_adapter(&instance) else {
            println!("no adapter — skipping pipeline test");
            return;
        };
        let (device, _queue) = request_device(&adapter).expect("device");

        let bgl = create_texture_bind_group_layout(&device);
        let pipeline = create_fullscreen_textured_pipeline(
            &device,
            &bgl,
            // Same non-sRGB format the surface picks at runtime.
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = pipeline;
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Wire up sampler + bind-group layout + bind group end-to-end; a
    /// slot-type mismatch would panic in `create_bind_group`.
    #[test]
    fn texture_sampler_bind_group_wires_up() {
        let instance = create_instance();
        let Ok(adapter) = request_adapter(&instance) else {
            println!("no adapter — skipping bind group test");
            return;
        };
        let (device, queue) = request_device(&adapter).expect("device");

        let rgba: [u8; 16] = [
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
        ];
        let texture = upload_rgba_texture(
            &device,
            &queue,
            device.limits().max_texture_dimension_2d,
            2,
            2,
            &rgba,
        ).expect("upload texture");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = create_linear_sampler(&device);
        let layout = create_texture_bind_group_layout(&device);
        let _bind_group = create_texture_bind_group(&device, &layout, &view, &sampler);

        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Open a device + queue without a surface and print a few limits.
    /// Skipped when no adapter is available.
    #[test]
    fn device_request_opens_device_and_queue() {
        let instance = create_instance();
        let Ok(adapter) = request_adapter(&instance) else {
            println!("no adapter — skipping device test");
            return;
        };
        match request_device(&adapter) {
            Ok((device, _queue)) => {
                let limits = device.limits();
                println!("device opened OK");
                println!("  max_texture_dim_2d     : {}", limits.max_texture_dimension_2d);
                println!("  max_buffer_size        : {}", limits.max_buffer_size);
                println!("  max_bind_groups        : {}", limits.max_bind_groups);
            }
            Err(e) => panic!("request_device failed on available adapter: {e}"),
        }
    }
}
