//! GPU data rendering via wgpu.
//!
//! wgpu owns the window surface; the CPU raster stack (tiny-skia + swash)
//! rasterises axes / titles / labels and uploads them as a texture. A single
//! render pass draws data primitives followed by the chrome texture overlay,
//! then presents.

use crate::color::Color;
use crate::config::Config;
use crate::data_config::ScatterShape;
use crate::layout::Rect;

use wgpu::util::DeviceExt;

pub mod column_pool;
pub mod line_arc;
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
pub async fn request_adapter_async(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: None,
    };
    instance.request_adapter(&options).await
}

/// Blocking convenience wrapper around [`request_adapter_async`]. Native
/// only — on wasm, blocking the single thread would deadlock; await the
/// async variant from the host's event loop instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn request_adapter(
    instance: &wgpu::Instance,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    pollster::block_on(request_adapter_async(instance))
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
pub async fn request_adapter_for_surface_async(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::None,
        force_fallback_adapter: false,
        compatible_surface: Some(surface),
    };
    instance.request_adapter(&options).await
}

/// Blocking convenience wrapper around [`request_adapter_for_surface_async`].
/// Native only — see [`request_adapter`].
#[cfg(not(target_arch = "wasm32"))]
pub fn request_adapter_for_surface(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
    pollster::block_on(request_adapter_for_surface_async(instance, surface))
}

/// Open a logical device and its queue with WebGPU baseline limits.
pub async fn request_device_async(
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
    adapter.request_device(&descriptor).await
}

/// Blocking convenience wrapper around [`request_device_async`]. Native only
/// — see [`request_adapter`].
#[cfg(not(target_arch = "wasm32"))]
pub fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    pollster::block_on(request_device_async(adapter))
}

/// Build and apply a `SurfaceConfiguration` for the given size. Callers should
/// keep the returned config so they can call [`reconfigure_surface`] on resize.
///
/// The format selection prefers a **non-sRGB** format. See `upload_rgba_texture`
/// for why: we want pixel-level parity with the CPU raster's gamma-incorrect
/// blending, which requires the GPU side to also blend bytes directly.
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

    // Prefer non-sRGB so GPU blending matches the CPU raster's gamma-incorrect path.
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

// Texture upload, samplers, bind groups, and pipelines for the chrome overlay.

/// Upload an RGBA8 pixel array to a 2D texture, usable as `TEXTURE_BINDING`
/// and `COPY_DST`. No mipmaps, no MSAA.
///
/// Format is `Rgba8Unorm` (non-sRGB) on purpose: paired with a non-sRGB
/// surface, the GPU blends bytes directly so the result matches the raster's
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
/// Blend is `PREMULTIPLIED_ALPHA_BLENDING` because the CPU raster works with
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
/// 64 bytes (four `vec2<f32>` fields plus one `array<vec4<f32>, 2>` at
/// offset 32, stride 16, WGSL uniform layout). Pixel sizes (point radius,
/// cap half-length) live in [`PrimitiveStyle`]; shaders convert them to NDC
/// via `pixel_to_ndc`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterTransform {
    pub data_min: [f32; 2],       // offset 0
    pub data_max: [f32; 2],       // offset 8
    /// Per-axis flag: 0.0 = linear, 1.0 = log10.
    pub scale_log: [f32; 2],      // offset 16
    /// `(2 / chart_w, 2 / chart_h)` — 1 pixel in NDC. Shaders multiply pixel
    /// sizes (line width, point radius, cap half-length) by this.
    pub pixel_to_ndc: [f32; 2],   // offset 24
    /// Generic per-panel style parameter slots (SHADER_COMMON.md §1), packed
    /// by the renderer's style table (`StyleVariant::pack_params`, flat
    /// `[f32; 12]` split into three vec4 slots). All zeros in precise mode —
    /// the precise entry points never read them. Sketch:
    /// `[0] = [amplitude_px, wavelength_px, seed as f32, 0.0]`, rest 0;
    /// constellation: `[0] = [star_density, ribbon_width_px,
    /// ribbon_intensity, seed as f32]`, `[1] = [star_scale, spread_px,
    /// faint_bias, planet_rim]`, `[2] = [structure_scale, 0, 0, 0]`. Seeds
    /// are stored as f32 (exact up to 2^24) and shaders recover them via
    /// `u32(...)`.
    pub style_params: [[f32; 4]; 3],   // offset 32 → 80 byte
}

// WGSL mirror size guards (SHADER_COMMON.md §1 / §2). Update both the doc and
// every shader's common block before touching these.
const _: () = assert!(std::mem::size_of::<ScatterTransform>() == 80);

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

/// Per-primitive style uniform. 80 bytes, 16-byte aligned.
///
/// One struct serves all three primitive shaders; each reads its own fields
/// and ignores the rest (field semantics in SHADER_COMMON.md §2).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimitiveStyle {
    pub color_premul: [f32; 4],   // offset 0
    /// Line / errorbar stem thickness in pixels.
    pub line_width_px: f32,       // offset 16
    /// Scatter point radius in pixels.
    pub point_radius_px: f32,     // offset 20
    /// Errorbar cap half-length in pixels.
    pub cap_half_px: f32,         // offset 24
    /// Errorbar cap stroke thickness in pixels.
    pub cap_width_px: f32,        // offset 28
    /// `ScatterShape` declaration-order index — see [`shape_id`].
    pub shape_id: u32,            // offset 32
    /// Number of valid scalars in `dash`; 0 = solid.
    pub dash_len: u32,            // offset 36
    /// Per-series decorrelation salt (FNV-1a of `series_id`, written by
    /// `Renderer::create_style_for_series*`). Sketch/constellation shader
    /// entries XOR it into their hash seeds so series with identical
    /// sampling don't share star/wobble patterns; precise entries ignore it.
    pub series_salt: u32,         // offset 40
    /// Keeps `dash` 16-byte aligned.
    pub _pad: u32,                // offset 44
    /// Up to 8 sequential `[on, off, ...]` pixel lengths: `dash[0]` first,
    /// then `dash[1]`.
    pub dash: [[f32; 4]; 2],      // offset 48 → 80 byte
}

const _: () = assert!(std::mem::size_of::<PrimitiveStyle>() == 80);

impl PrimitiveStyle {
    /// Convert a straight-RGBA `Color` to the premultiplied form expected by
    /// the `PREMULTIPLIED_ALPHA_BLENDING` pipeline.
    pub fn from_color(c: Color) -> Self {
        Self::from_color_with_width(c, 1.0)
    }

    /// `line_width_px` is the pixel thickness for line series. Every other
    /// option gets a neutral default: 4 px point radius, 3 px cap half-length,
    /// 1 px cap stroke, filled circle, solid line.
    pub fn from_color_with_width(c: Color, line_width_px: f32) -> Self {
        let a = c.a.clamp(0.0, 1.0);
        Self {
            color_premul: [c.r * a, c.g * a, c.b * a, a],
            line_width_px,
            point_radius_px: 4.0,
            cap_half_px: 3.0,
            cap_width_px: 1.0,
            shape_id: shape_id(&ScatterShape::CircleFilled),
            dash_len: 0,
            series_salt: 0,
            _pad: 0,
            dash: [[0.0; 4]; 2],
        }
    }
}

/// Map a [`ScatterShape`] to the `Style.shape_id` uniform value — the
/// declaration-order index of the variant in the model enum.
pub fn shape_id(shape: &ScatterShape) -> u32 {
    match shape {
        ScatterShape::Circle => 0,
        ScatterShape::Square => 1,
        ScatterShape::Triangle => 2,
        ScatterShape::Diamond => 3,
        ScatterShape::Cross => 4,
        ScatterShape::CircleFilled => 5,
        ScatterShape::SquareFilled => 6,
        ScatterShape::TriangleFilled => 7,
        ScatterShape::DiamondFilled => 8,
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

/// Build a [`ScatterTransform`] from a `Config`.
///
/// The transform encodes data-space ranges, log-axis flags, and pixel-to-NDC
/// scale factors. When `data_area` is smaller than `chart_area`, the data
/// range is **extended** so the same data extents land inside data_area only,
/// and the rest of the panel viewport is empty space.
pub fn scatter_transform_from_config(config: &Config) -> ScatterTransform {
    let ca = &config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;

    use crate::config::AxisScale;
    let log_x = matches!(config.bottom_x.scale, AxisScale::Logarithmic);
    let log_y = matches!(config.left_y.scale, AxisScale::Logarithmic);

    // For log axes, guard only the axis range. Data values still follow the
    // shader's NaN/non-positive handling.
    let to_log = |v: f64| v.log10() as f32;
    let (x_min, x_max) = if log_x {
        crate::chart::guarded_log_range(config.bottom_x.min, config.bottom_x.max)
    } else {
        (config.bottom_x.min, config.bottom_x.max)
    };
    let (y_min, y_max) = if log_y {
        crate::chart::guarded_log_range(config.left_y.min, config.left_y.max)
    } else {
        (config.left_y.min, config.left_y.max)
    };

    let data_min_x = if log_x { to_log(x_min) } else { x_min as f32 };
    let data_max_x = if log_x { to_log(x_max) } else { x_max as f32 };
    let data_min_y = if log_y { to_log(y_min) } else { y_min as f32 };
    let data_max_y = if log_y { to_log(y_max) } else { y_max as f32 };

    let scale_log = [if log_x { 1.0 } else { 0.0 }, if log_y { 1.0 } else { 0.0 }];

    // 1 px in NDC: NDC spans 2 across chart_w pixels.
    let pixel_to_ndc = [2.0 / chart_w, 2.0 / chart_h];

    // Per-style shader parameters, packed by the style table's `pack_params`
    // (renderer.rs). Precise mode writes zeros — the precise entry points
    // never read them, so the output is unaffected. The export path's
    // `Config::scaled` already multiplied the style's pixel dims (e.g. sketch
    // amplitude/wavelength) by the DPI scale; pack functions read them as-is.
    let packed = match crate::renderer::style_variant(&config.draw_style) {
        Some(v) => (v.pack_params)(&config.draw_style),
        None => [0.0; 12],
    };
    let style_params = [
        [packed[0], packed[1], packed[2], packed[3]],
        [packed[4], packed[5], packed[6], packed[7]],
        [packed[8], packed[9], packed[10], packed[11]],
    ];

    // No data_area → use the data range directly (no extension).
    let da: Rect = match config.data_area() {
        Ok(d) => d.0,
        Err(_) => {
            return ScatterTransform {
                data_min: [data_min_x, data_min_y],
                data_max: [data_max_x, data_max_y],
                scale_log,
                pixel_to_ndc,
                style_params,
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
        scale_log,
        pixel_to_ndc,
        style_params,
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
    create_line_columnar_pipeline_with_entries(
        device, transform_bgl, style_bgl, target_format,
        "vs_main", "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        None,
        "figgy line columnar pipeline",
    )
}

/// Strip vertex count per instance for the sketch line pipeline:
/// `2 * (S + 1)` with the subdivision constant `S = 8` — must match
/// `SKETCH_SUBDIV` in `line_columnar.wgsl`.
pub const LINE_SKETCH_VERTICES_PER_INSTANCE: u32 = 18;

/// Constellation ribbon strip vertices per instance — `2·(S+1)`, twin of
/// `CONS_RIBBON_SUBDIV` in `line_columnar.wgsl`.
pub const CONSTELLATION_RIBBON_VERTICES: u32 = 18;
/// Constellation star quads per segment instance — `6·K`, twin of
/// `CONS_STARS_PER_SEGMENT` in `line_columnar.wgsl`. K is a per-segment
/// budget: a single segment saturates once `density·arc/100 > K` (documented
/// Step-1 cap; typical charts sit far below it).
pub const CONSTELLATION_STAR_VERTICES: u32 = 144;

/// Constellation pipelines + baked style textures for one target format —
/// cached inside the renderer's lazy style set (docs/CONSTELLATION_DESIGN.MD
/// §3c/§3d). The bind group keeps the textures alive.
pub(crate) struct ConstellationSet {
    pub(crate) ribbon: wgpu::RenderPipeline,
    pub(crate) stars: wgpu::RenderPipeline,
    /// Ringed-planet scatter pass (Step 2) — premultiplied blend, occludes
    /// the additive star field behind it.
    pub(crate) planets: wgpu::RenderPipeline,
    /// Bipolar-jet errorbars — additive beams + terminal shock knots over
    /// the precise errorbar geometry.
    pub(crate) jets: wgpu::RenderPipeline,
    pub(crate) star_tex_bg: wgpu::BindGroup,
}

// ── Procedural bakes for the planet atlas (Step 2). Heavy math is fine —
// this runs once per style-set creation and the results are GPU-cached.

fn bake_hash2(ix: i64, iy: i64, seed: u32) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (seed as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// 2D value noise, [0,1], smoothstep-interpolated.
pub(crate) fn vnoise2(x: f64, y: f64, seed: u32) -> f64 {
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = bake_hash2(ix, iy, seed);
    let b = bake_hash2(ix + 1, iy, seed);
    let c = bake_hash2(ix, iy + 1, seed);
    let d = bake_hash2(ix + 1, iy + 1, seed);
    a + (b - a) * ux + (c - a) * uy + (a - b - c + d) * ux * uy
}

/// Fractal Brownian motion over `vnoise2`, [0,1]-ish.
pub(crate) fn fbm2(x: f64, y: f64, octaves: u32, seed: u32) -> f64 {
    let mut acc = 0.0;
    let mut amp = 0.5;
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        acc += amp * vnoise2(fx, fy, seed.wrapping_add(o * 131));
        amp *= 0.5;
        fx *= 2.03;
        fy *= 2.03;
    }
    acc
}

/// Planet albedo atlas: 2×2 archetype tiles (each `tile`² px, equirect).
/// Longitude-seamless: noise is sampled on the unit cylinder (cos θ, sin θ).
/// Archetypes: 0 gas giant (domain-warped bands), 1 ice giant, 2 rocky,
/// 3 cratered gray.
fn bake_planet_atlas(tile: u32) -> Vec<u8> {
    let size = tile * 2;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let mix3 = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] {
        let t = t.clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    for ty in 0..2u32 {
        for tx in 0..2u32 {
            let arch = ty * 2 + tx;
            for py in 0..tile {
                for px in 0..tile {
                    let u = px as f64 / (tile - 1) as f64; // longitude 0..1
                    let v = py as f64 / (tile - 1) as f64; // latitude 0..1
                    let th = u * std::f64::consts::TAU;
                    let (cx, sx) = (th.cos(), th.sin());

                    let rgb: [f64; 3] = match arch {
                        // Gas giant: latitude bands, domain-warped by
                        // cylinder-sampled fBm — the Jupiter look. Warp is
                        // kept mild so the bands stay BANDS (strong warp
                        // reads as marble, not a gas giant).
                        0 => {
                            let warp = fbm2(cx * 2.2 + 11.0, sx * 2.2 + v * 5.0, 5, 7) - 0.5;
                            let band_t = v * 9.0 + warp * 1.1;
                            let s = 0.5 + 0.5 * (band_t * std::f64::consts::TAU * 0.5).sin();
                            let turb = fbm2(cx * 5.0, sx * 5.0 + v * 14.0, 5, 23) - 0.5;
                            let cream = [0.88, 0.80, 0.66];
                            let rust = [0.58, 0.36, 0.24];
                            let mut c = mix3(cream, rust, s * 0.9 + turb * 0.18);
                            // One dark belt accent.
                            let belt = (-(v - 0.62).powi(2) / 0.002).exp();
                            c = mix3(c, [0.42, 0.26, 0.18], belt * 0.55);
                            c
                        }
                        // Ice giant: smooth teal with faint streaks.
                        1 => {
                            let s = fbm2(cx * 1.6, sx * 1.6 + v * 7.0, 4, 41) - 0.5;
                            let base = mix3([0.34, 0.52, 0.86], [0.55, 0.72, 0.95], v * 0.5 + s * 0.25);
                            let streak = (-(v - 0.35).powi(2) / 0.004).exp();
                            mix3(base, [0.85, 0.92, 1.0], streak * 0.35)
                        }
                        // Rocky: ochre terrain patches + polar caps.
                        2 => {
                            let t1 = fbm2(cx * 3.0, sx * 3.0 + v * 6.0, 6, 67);
                            let mut c =
                                mix3([0.72, 0.46, 0.28], [0.44, 0.27, 0.17], (t1 - 0.35) * 2.0);
                            let polar = ((v - 0.5).abs() * 2.0 - 0.78).max(0.0) / 0.22;
                            c = mix3(c, [0.92, 0.90, 0.86], polar.min(1.0) * 0.8);
                            c
                        }
                        // Cratered gray: maria blotches over regolith noise.
                        _ => {
                            let t1 = fbm2(cx * 3.4, sx * 3.4 + v * 7.0, 6, 97);
                            let t2 = fbm2(cx * 1.4 + 5.0, sx * 1.4 + v * 3.0, 4, 113);
                            let g = 0.58 + (t1 - 0.5) * 0.30 - if t2 > 0.62 { 0.18 } else { 0.0 };
                            [g, g, g * 1.02]
                        }
                    };

                    let x = tx * tile + px;
                    let y = ty * tile + py;
                    let i = ((y * size + x) * 4) as usize;
                    out[i] = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 1] = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 2] = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 3] = 255;
                }
            }
        }
    }
    out
}

/// Ring radial strip (256×1): C ring (faint) → B ring (bright) → Cassini
/// gap → A ring, with fine radial density noise. RGB is the straight ring
/// color; A is the density the shader composes with.
fn bake_ring_strip() -> Vec<u8> {
    let mut out = vec![0u8; 256 * 4];
    for i in 0..256usize {
        let u = i as f64 / 255.0;
        let base = if u < 0.16 {
            0.22
        } else if u < 0.52 {
            0.85
        } else if u < 0.60 {
            0.04
        } else if u < 0.93 {
            0.60 * (1.0 - (u - 0.60) / 0.33 * 0.35)
        } else {
            0.0
        };
        let fine = (fbm2(u * 60.0, 0.5, 4, 151) - 0.5) * 0.35;
        let a = (base * (1.0 + fine)).clamp(0.0, 1.0);
        let rgb: [f64; 3] = [0.80, 0.74, 0.63];
        out[i * 4] = (rgb[0] * 255.0).round() as u8;
        out[i * 4 + 1] = (rgb[1] * 255.0).round() as u8;
        out[i * 4 + 2] = (rgb[2] * 255.0).round() as u8;
        out[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    out
}

/// Bake the star PSF sprite (R = saturating core, G = halo wings + one faint
/// Airy-style ring) — runs once per style-set creation; expensive math is
/// fine here (CONSTELLATION_DESIGN.md §0 "bake, then sample").
fn bake_psf_rgba(size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let half = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - half) / half; // -1..1
            let dy = (y as f32 - half) / half;
            let r = (dx * dx + dy * dy).sqrt();
            // Flat saturated core with a steep gaussian shoulder.
            let core = (-(r / 0.11).powf(2.6)).exp().min(1.0);
            // Exponential halo wings + one faint ring at 0.45.
            let halo = 0.85 * (-r / 0.28).exp()
                + 0.08 * (-((r - 0.45) / 0.06).powi(2)).exp();
            let i = ((y * size + x) * 4) as usize;
            out[i] = (core.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i + 1] = (halo.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i + 2] = 0;
            out[i + 3] = 255;
        }
    }
    out
}

/// Bake the 256×1 blackbody LUT, 2,500 K → 12,000 K (Tanner Helland's
/// piecewise fit — visually faithful Planckian locus, never green).
fn bake_blackbody_lut() -> Vec<u8> {
    let mut out = vec![0u8; 256 * 4];
    for i in 0..256usize {
        let kelvin = 2500.0 + 9500.0 * (i as f64 / 255.0);
        let t = kelvin / 100.0;
        let r = if t <= 66.0 {
            255.0
        } else {
            329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
        };
        let g = if t <= 66.0 {
            99.470_802_586_1 * t.ln() - 161.119_568_166_1
        } else {
            288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
        };
        out[i * 4] = r.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 1] = g.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 2] = b.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 3] = 255;
    }
    out
}

/// Build the constellation style set: bake PSF + blackbody LUT, upload them,
/// and compile the additive ribbon/star pipelines (group 2 = the textures).
pub(crate) fn create_constellation_set(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> ConstellationSet {
    let make_tex = |label: &str, w: u32, h: u32, data: &[u8]| {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    const PSF_SIZE: u32 = 128;
    const ATLAS_TILE: u32 = 128;
    let psf_view = make_tex("figgy constellation psf", PSF_SIZE, PSF_SIZE, &bake_psf_rgba(PSF_SIZE));
    let lut_view = make_tex("figgy constellation blackbody lut", 256, 1, &bake_blackbody_lut());
    let atlas_view = make_tex(
        "figgy constellation planet atlas",
        ATLAS_TILE * 2,
        ATLAS_TILE * 2,
        &bake_planet_atlas(ATLAS_TILE),
    );
    let ring_view = make_tex("figgy constellation ring strip", 256, 1, &bake_ring_strip());

    let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy constellation texture bgl"),
        entries: &[
            tex_entry(0), // PSF (stars)
            tex_entry(1), // blackbody LUT (stars)
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            tex_entry(3), // planet atlas (planets)
            tex_entry(4), // ring strip (planets)
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("figgy constellation sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let star_tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy constellation texture bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&psf_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lut_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&atlas_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&ring_view) },
        ],
    });

    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    // MAX, not ADD: the ribbon seals curve joints by overlapping square-cap
    // extensions (vs_ribbon), and max() keeps that overlap from
    // double-brightening — the haze is a field, not an accumulation.
    let max_blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        },
    };
    let ribbon = create_line_columnar_pipeline_with_entries(
        device, transform_bgl, style_bgl, target_format,
        "vs_ribbon", "fs_ribbon", max_blend,
        wgpu::PrimitiveTopology::TriangleStrip,
        Some(&tex_bgl),
        "figgy constellation ribbon pipeline",
    );
    let stars = create_line_columnar_pipeline_with_entries(
        device, transform_bgl, style_bgl, target_format,
        "vs_stars", "fs_stars", additive,
        wgpu::PrimitiveTopology::TriangleList,
        Some(&tex_bgl),
        "figgy constellation stars pipeline",
    );
    // Planets keep the scatter builder's premultiplied blend — bodies
    // occlude the additive star field behind them.
    let planets = create_scatter_columnar_pipeline_full(
        device, transform_bgl, style_bgl, target_format,
        "vs_planet", "fs_planet",
        Some(&tex_bgl),
        "figgy constellation planets pipeline",
    );
    let jets = create_errorbar_columnar_pipeline_full(
        device, transform_bgl, style_bgl, target_format,
        "vs_jet", "fs_jet", additive,
        "figgy constellation jets pipeline",
    );

    ConstellationSet { ribbon, stars, planets, jets, star_tex_bg }
}

/// Entry-point-parameterized line pipeline builder. Styled variants share
/// the precise pipeline's six instance slots and differ in entry points,
/// blend state (constellation is additive), topology (star quads are a
/// TriangleList), and an optional third bind group (style textures) — the
/// renderer's style table supplies all of it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_line_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    topology: wgpu::PrimitiveTopology,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy line columnar shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("line_columnar.wgsl").into()),
    });

    let mut bgls: Vec<&wgpu::BindGroupLayout> = vec![transform_bgl, style_bgl];
    if let Some(t) = texture_bgl {
        bgls.push(t);
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy line columnar layout"),
        bind_group_layouts: &bgls,
        push_constant_ranges: &[],
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(vs_entry),
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
                // slots 4/5: cumulative arc length (px) at A and B — the
                // same prefix buffer bound twice with a one-f32 shift, like
                // x/y. Solid lines bind the X column here as inert filler
                // (the fragment stage ignores it when dash_len == 0).
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 4,
                    }],
                },
                wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 5,
                    }],
                },
            ],
        },
        primitive: wgpu::PrimitiveState {
            // Strip per instance for the line entries (4 vertices for
            // `vs_main`, 2·(S+1) for `vs_sketch`/`vs_ribbon`), TriangleList
            // for `vs_stars` quads — the caller picks.
            topology,
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
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
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
    create_scatter_columnar_pipeline_full(
        device, transform_bgl, style_bgl, target_format,
        "vs_main", "fs_main", None, "figgy scatter columnar pipeline",
    )
}

/// Two-entry convenience used by the sketch style (shared layout/state).
pub(crate) fn create_scatter_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_columnar_pipeline_full(
        device, transform_bgl, style_bgl, target_format, vs_entry, fs_entry, None, label,
    )
}

/// Entry-point-parameterized scatter pipeline builder. Styled variants share
/// the precise pipeline's three vertex slots; the constellation planet
/// variant additionally binds the style textures as group 2.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_pipeline_full(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    vs_entry: &str,
    fs_entry: &str,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("figgy scatter columnar shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("scatter_columnar.wgsl").into()),
    });

    let mut bgls: Vec<&wgpu::BindGroupLayout> = vec![transform_bgl, style_bgl];
    if let Some(t) = texture_bgl {
        bgls.push(t);
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy scatter columnar layout"),
        bind_group_layouts: &bgls,
        push_constant_ranges: &[],
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;
    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(vs_entry),
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
            entry_point: Some(fs_entry),
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
/// err_x_hi. Each instance emits 36 vertices on a `TriangleList`: six
/// axis-aligned quads (Y stem, caps @ y_lo/y_hi, X stem, caps @ x_lo/x_hi),
/// expanded in the vertex shader by half their pixel stroke width
/// (`Style::line_width_px` for stems, `cap_width_px` for caps; caps span
/// ±`cap_half_px`). A direction whose err columns sum to <= 0 collapses to
/// zero-area quads, so its stem and caps draw nothing.
pub fn create_errorbar_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_with_entries(
        device, transform_bgl, style_bgl, target_format,
        "vs_main", "figgy errorbar columnar pipeline",
    )
}

/// Entry-point-parameterized errorbar pipeline builder. Styled variants
/// (e.g. the sketch `vs_sketch` — fragment stage shared, vertex count
/// unchanged at 36 per instance) share the precise pipeline's layout and
/// state; the renderer's style table supplies the entry string.
pub(crate) fn create_errorbar_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    vs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_full(
        device, transform_bgl, style_bgl, target_format,
        vs_entry, "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        label,
    )
}

/// Full-control errorbar builder — the constellation jet variant needs its
/// own fragment entry and additive blending.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_errorbar_columnar_pipeline_full(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    label: &str,
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
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(vs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &buffers,
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
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
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
    /// Cumulative arc-length prefix (px) for dash phase — `(buffer, length
    /// in bytes)`. `None` for solid lines: the X column is bound as inert
    /// filler instead. `Arc` because the buffer lives in the renderer's
    /// per-series cache while the layer is a per-frame view.
    pub arc: Option<(std::sync::Arc<wgpu::Buffer>, u64)>,
    /// Strip vertices per instance — must match `pipeline`'s vertex entry:
    /// 4 for the precise `vs_main`, [`LINE_SKETCH_VERTICES_PER_INSTANCE`]
    /// for the sketch `vs_sketch`, [`CONSTELLATION_RIBBON_VERTICES`] /
    /// [`CONSTELLATION_STAR_VERTICES`] for the constellation entries.
    pub verts_per_instance: u32,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the constellation PSF/LUT bind group. `None` for precise/sketch.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
}

pub struct ColumnScatterLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the constellation planet atlas/ring bind group. `None` otherwise.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
}

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    pub errorbar: Option<ColumnErrorBarDraw<'a>>,
    pub line: Option<ColumnLineLayer<'a>>,
    /// Second line-slot draw over the same columns — the constellation style
    /// uses it for the star pass on top of the ribbon in `line`. Drawn right
    /// after `line`, before `scatter`. `None` everywhere else.
    pub line_extra: Option<ColumnLineLayer<'a>>,
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

/// Issue one line-slot draw (the shared 6-slot binding scheme). Used for the
/// main line layer and the constellation star pass.
fn draw_line_layer(pass: &mut wgpu::RenderPass<'_>, l: &ColumnLineLayer<'_>) {
    let count = l.x.len_values.min(l.y.len_values) as u32;
    if count < 2 {
        return;
    }
    pass.set_pipeline(l.pipeline);
    pass.set_bind_group(0, l.transform_bg, &[]);
    pass.set_bind_group(1, l.style_bg, &[]);
    if let Some(tex) = l.texture_bg {
        pass.set_bind_group(2, tex, &[]);
    }
    let x_full = l.x.byte_range();
    let y_full = l.y.byte_range();
    let x_shift = (x_full.start + 4)..x_full.end;
    let y_shift = (y_full.start + 4)..y_full.end;
    pass.set_vertex_buffer(0, l.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, l.pool_buffer.slice(y_full));
    pass.set_vertex_buffer(2, l.pool_buffer.slice(x_shift.clone()));
    pass.set_vertex_buffer(3, l.pool_buffer.slice(y_shift));
    // Arc-length prefix (dash phase / constellation arc); solid precise lines
    // reuse the X column as filler (read by the VS, ignored by the FS).
    match l.arc.as_ref() {
        Some((buf, len_bytes)) => {
            pass.set_vertex_buffer(4, buf.slice(0..*len_bytes));
            pass.set_vertex_buffer(5, buf.slice(4..*len_bytes));
        }
        None => {
            pass.set_vertex_buffer(4, l.pool_buffer.slice(x_full));
            pass.set_vertex_buffer(5, l.pool_buffer.slice(x_shift));
        }
    }
    // Per-instance vertex count decided where the pipeline variant was
    // picked: 4 (precise) / 18 (sketch, ribbon) / 144 (stars).
    pass.draw(0..l.verts_per_instance, 0..(count - 1));
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
            // 36 vertices = 6 quads × 2 triangles (see errorbar_columnar.wgsl).
            pass.draw(0..36, 0..count);
        }
    }

    if let Some(l) = series.line.as_ref() {
        draw_line_layer(pass, l);
    }
    if let Some(l) = series.line_extra.as_ref() {
        draw_line_layer(pass, l);
    }

    if let Some(s) = series.scatter.as_ref() {
        let count = s.x.len_values.min(s.y.len_values) as u32;
        if count > 0 {
            pass.set_pipeline(s.pipeline);
            pass.set_bind_group(0, s.transform_bg, &[]);
            pass.set_bind_group(1, s.style_bg, &[]);
            if let Some(tex) = s.texture_bg {
                pass.set_bind_group(2, tex, &[]);
            }
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

    #[test]
    fn log_transform_guards_manual_range_without_raising_tiny_positive_min() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 100, height: 100 });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }

        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 1000.0;
        let t = scatter_transform_from_config(&config);
        assert_eq!(t.scale_log[0], 1.0);
        assert!((t.data_min[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 3.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 1.0e-15;
        config.bottom_x.max = 1.0e-12;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] + 15.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] + 12.0).abs() < 1.0e-6, "{:?}", t.data_max);

        config.bottom_x.min = 10.0;
        config.bottom_x.max = 1.0;
        let t = scatter_transform_from_config(&config);
        assert!((t.data_min[0] - 1.0).abs() < 1.0e-6, "{:?}", t.data_min);
        assert!((t.data_max[0] - 2.0).abs() < 1.0e-6, "{:?}", t.data_max);
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
