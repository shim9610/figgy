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

pub mod bar_envelope;
pub mod column_pool;
pub mod line_arc;
pub(crate) mod stream_field;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod stream_point_style_tests;
pub use column_pool::{
    AllocError, ColumnHandle, ColumnId, ColumnPool, ColumnSlot, DefragPolicy, FreeRegion,
    GpuAllocCtx, GpuBudget, GrowthPolicy,
};

// Instance, adapter, surface, and device setup.

/// Create a `wgpu::Instance` with default settings (all native backends, no
/// pre-bound display handle). Synchronous and infallible.
pub fn create_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
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
        apply_limit_buckets: false,
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
        apply_limit_buckets: false,
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

/// One wgpu device+queue shared by every unit test in this binary.
///
/// libtest runs tests on up to `num_cpus` threads; if each GPU test built its
/// own instance/adapter/device (as they used to), a many-core machine spun up
/// 20+ live devices on one physical GPU at once, and under that contention a
/// submission would occasionally stall — leaving the test's indefinite
/// `poll(Wait)` spinning forever. Sharing a single device (wgpu resources are
/// `Send + Sync` and safe to use concurrently) removes the device-creation
/// storm while keeping test parallelism. Built once, lazily; `None` on a
/// machine with no usable adapter so tests skip exactly as before.
#[cfg(test)]
pub(crate) fn shared_device() -> Option<(std::sync::Arc<wgpu::Device>, std::sync::Arc<wgpu::Queue>)>
{
    use std::sync::{Arc, OnceLock};
    static SHARED: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let inst = create_instance();
            let adapter = request_adapter(&inst).ok()?;
            let (device, queue) = request_device(&adapter).ok()?;
            Some((Arc::new(device), Arc::new(queue)))
        })
        .clone()
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
    const PREFERRED: &[wgpu::PresentMode] =
        &[wgpu::PresentMode::Fifo, wgpu::PresentMode::AutoVsync];

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
            reason: "surface reported no figgy-compatible renderable/blendable texture formats"
                .into(),
        }
    })?;
    let present_mode = choose_present_mode(&caps).ok_or_else(|| {
        crate::FiggyError::SurfaceConfigurationFailed {
            reason: "surface reported no supported present modes".into(),
        }
    })?;
    let alpha_mode =
        choose_alpha_mode(&caps).ok_or_else(|| crate::FiggyError::SurfaceConfigurationFailed {
            reason: "surface reported no supported alpha modes".into(),
        })?;

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
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

fn acquire_surface_frame(
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::SurfaceTexture, RenderOutcome> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame)
        | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Ok(frame),
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            Err(RenderOutcome::Reconfigure)
        }
        wgpu::CurrentSurfaceTexture::Timeout
        | wgpu::CurrentSurfaceTexture::Occluded
        | wgpu::CurrentSurfaceTexture::Validation => Err(RenderOutcome::Skipped),
    }
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
    let frame = match acquire_surface_frame(surface) {
        Ok(t) => t,
        Err(outcome) => return outcome,
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
            multiview_mask: None,
        });
    }

    queue.submit(std::iter::once(encoder.finish()));
    queue.present(frame);

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
        // gpu-alloc: caller
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

pub(crate) fn multisample_state(sample_count: u32) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: sample_count,
        mask: !0,
        alpha_to_coverage_enabled: false,
    }
}

/// One compiled module per WGSL file. Precise, mapped, pick-ring, and styled
/// pipelines reuse these instead of re-running naga on the same source.
pub(crate) struct ShaderModules {
    pub fullscreen: wgpu::ShaderModule,
    pub line: wgpu::ShaderModule,
    pub scatter: wgpu::ShaderModule,
    pub errorbar: wgpu::ShaderModule,
    pub bar: wgpu::ShaderModule,
    pub field: wgpu::ShaderModule,
}

impl ShaderModules {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            fullscreen: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy fullscreen textured shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("fullscreen_textured.wgsl").into()),
            }),
            line: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy line columnar shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("line_columnar.wgsl").into()),
            }),
            scatter: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy scatter columnar shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("scatter_columnar.wgsl").into()),
            }),
            errorbar: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy errorbar columnar shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("errorbar_columnar.wgsl").into()),
            }),
            bar: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy bar columnar shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("bar_columnar.wgsl").into()),
            }),
            field: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("figgy field columnar shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("field_columnar.wgsl").into()),
            }),
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct BrowserVertexAttributeSpec {
    format: &'static str,
    offset: u32,
    location: u32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct BrowserVertexBufferSpec {
    stride: u32,
    step_mode: &'static str,
    attributes: &'static [BrowserVertexAttributeSpec],
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
enum BrowserBlend {
    Premultiplied,
    Additive,
    Maximum,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct BrowserRenderPipelineSpec {
    label: &'static str,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    topology: &'static str,
    buffers: &'static [BrowserVertexBufferSpec],
    blend: BrowserBlend,
}

#[cfg(target_arch = "wasm32")]
const fn browser_attr(format: &'static str, location: u32) -> BrowserVertexAttributeSpec {
    BrowserVertexAttributeSpec {
        format,
        offset: 0,
        location,
    }
}

#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_0: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 0)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_1: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 1)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_2: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 2)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_3: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 3)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_4: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 4)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32X2_5: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32x2", 5)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32_3: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32", 3)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32_4: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32", 4)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32_5: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32", 5)];
#[cfg(target_arch = "wasm32")]
const ATTR_F32_6: [BrowserVertexAttributeSpec; 1] = [browser_attr("float32", 6)];
#[cfg(target_arch = "wasm32")]
const fn contour_label_browser_attr(index: usize) -> BrowserVertexAttributeSpec {
    let attribute = crate::gpu_contour::CONTOUR_LABEL_VERTEX_ATTRIBUTES[index];
    let format = match attribute.format {
        wgpu::VertexFormat::Float32x2 => "float32x2",
        wgpu::VertexFormat::Uint32 => "uint32",
        wgpu::VertexFormat::Float32 => "float32",
        _ => panic!("unsupported contour label vertex format"),
    };
    BrowserVertexAttributeSpec {
        format,
        offset: attribute.offset as u32,
        location: attribute.shader_location,
    }
}

#[cfg(target_arch = "wasm32")]
const CONTOUR_LABEL_ATTRIBUTES: [BrowserVertexAttributeSpec;
    crate::gpu_contour::CONTOUR_LABEL_VERTEX_ATTRIBUTES.len()] = [
    contour_label_browser_attr(0),
    contour_label_browser_attr(1),
    contour_label_browser_attr(2),
    contour_label_browser_attr(3),
    contour_label_browser_attr(4),
];

#[cfg(target_arch = "wasm32")]
const LINE_BUFFERS: [BrowserVertexBufferSpec; 6] = [
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_0,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_1,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_2,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_3,
    },
    BrowserVertexBufferSpec {
        stride: 4,
        step_mode: "instance",
        attributes: &ATTR_F32_4,
    },
    BrowserVertexBufferSpec {
        stride: 4,
        step_mode: "instance",
        attributes: &ATTR_F32_5,
    },
];

#[cfg(target_arch = "wasm32")]
const SCATTER_BUFFERS: [BrowserVertexBufferSpec; 3] = [
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "vertex",
        attributes: &ATTR_F32X2_0,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_1,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_2,
    },
];

#[cfg(target_arch = "wasm32")]
const SCATTER_MAPPED_BUFFERS: [BrowserVertexBufferSpec; 4] = [
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "vertex",
        attributes: &ATTR_F32X2_0,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_1,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_2,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32_3,
    },
];

#[cfg(target_arch = "wasm32")]
const ERRORBAR_BUFFERS: [BrowserVertexBufferSpec; 6] = [
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_0,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_1,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_2,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_3,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_4,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_5,
    },
];

#[cfg(target_arch = "wasm32")]
const ERRORBAR_MAPPED_BUFFERS: [BrowserVertexBufferSpec; 7] = [
    ERRORBAR_BUFFERS[0],
    ERRORBAR_BUFFERS[1],
    ERRORBAR_BUFFERS[2],
    ERRORBAR_BUFFERS[3],
    ERRORBAR_BUFFERS[4],
    ERRORBAR_BUFFERS[5],
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32_6,
    },
];

#[cfg(target_arch = "wasm32")]
const BAR_BUFFERS: [BrowserVertexBufferSpec; 3] = [
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_0,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_1,
    },
    BrowserVertexBufferSpec {
        stride: 8,
        step_mode: "instance",
        attributes: &ATTR_F32X2_2,
    },
];

#[cfg(target_arch = "wasm32")]
const CONTOUR_LABEL_BUFFERS: [BrowserVertexBufferSpec; 1] = [BrowserVertexBufferSpec {
    stride: std::mem::size_of::<crate::gpu_contour::LabelAnchorGpu>() as u32,
    step_mode: "instance",
    attributes: &CONTOUR_LABEL_ATTRIBUTES,
}];

/// Warm every render shader through WebGPU's genuinely asynchronous pipeline
/// API. The returned JS pipelines are intentionally discarded; the subsequent
/// wgpu pipeline creation reuses the same device's shader/driver cache while
/// retaining wgpu as the sole owner of production pipeline objects.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn prewarm_browser_render_pipelines(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    observer: &mut dyn FnMut(crate::InitEvent),
) -> Result<(), String> {
    use js_sys::{Array, Function, Object, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    let gpu_device = device
        .as_webgpu()
        .ok_or_else(|| "wgpu device is not backed by a browser GPUDevice".to_owned())?;
    let device_js = JsValue::from(gpu_device.clone());
    let js_error = |error: JsValue| {
        error
            .as_string()
            .unwrap_or_else(|| format!("WebGPU pipeline compile failed: {error:?}"))
    };
    let method = |name: &str| -> Result<Function, String> {
        Reflect::get(&device_js, &JsValue::from_str(name))
            .map_err(js_error)?
            .dyn_into()
            .map_err(|_| format!("GPUDevice.{name} is unavailable"))
    };
    let create_shader = method("createShaderModule")?;
    let create_pipeline = method("createRenderPipelineAsync")?;
    let set = |object: &Object, key: &str, value: &JsValue| -> Result<(), String> {
        Reflect::set(object, &JsValue::from_str(key), value).map_err(js_error)?;
        Ok(())
    };
    let format = match target_format {
        wgpu::TextureFormat::Bgra8Unorm => "bgra8unorm",
        wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8unorm-srgb",
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        other => {
            return Err(format!(
                "unsupported browser prewarm target format: {other:?}"
            ));
        }
    };

    let groups: [(&str, &str, &[BrowserRenderPipelineSpec]); 7] = [
        (
            "fullscreen",
            include_str!("fullscreen_textured.wgsl"),
            &[BrowserRenderPipelineSpec {
                label: "fullscreen textured",
                vertex_entry: "vs_main",
                fragment_entry: "fs_main",
                topology: "triangle-list",
                buffers: &[],
                blend: BrowserBlend::Premultiplied,
            }],
        ),
        (
            "line",
            include_str!("line_columnar.wgsl"),
            &[
                BrowserRenderPipelineSpec {
                    label: "precise line",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_main",
                    topology: "triangle-strip",
                    buffers: &LINE_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "hand-drawn line",
                    vertex_entry: "vs_sketch",
                    fragment_entry: "fs_main",
                    topology: "triangle-strip",
                    buffers: &LINE_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "milkyway ribbon",
                    vertex_entry: "vs_ribbon",
                    fragment_entry: "fs_ribbon",
                    topology: "triangle-strip",
                    buffers: &LINE_BUFFERS,
                    blend: BrowserBlend::Maximum,
                },
                BrowserRenderPipelineSpec {
                    label: "milkyway stars",
                    vertex_entry: "vs_stars",
                    fragment_entry: "fs_stars",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Additive,
                },
                BrowserRenderPipelineSpec {
                    label: "constellation line",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_constellation_line",
                    topology: "triangle-strip",
                    buffers: &LINE_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
            ],
        ),
        (
            "scatter",
            include_str!("scatter_columnar.wgsl"),
            &[
                BrowserRenderPipelineSpec {
                    label: "precise scatter",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_main",
                    topology: "triangle-strip",
                    buffers: &SCATTER_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "mapped scatter",
                    vertex_entry: "vs_mapped",
                    fragment_entry: "fs_mapped",
                    topology: "triangle-strip",
                    buffers: &SCATTER_MAPPED_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "picked point ring",
                    vertex_entry: "vs_pick_ring",
                    fragment_entry: "fs_pick_ring",
                    topology: "triangle-strip",
                    buffers: &SCATTER_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "mapped picked point ring",
                    vertex_entry: "vs_pick_ring_mapped",
                    fragment_entry: "fs_pick_ring",
                    topology: "triangle-strip",
                    buffers: &SCATTER_MAPPED_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "hand-drawn scatter",
                    vertex_entry: "vs_sketch",
                    fragment_entry: "fs_sketch",
                    topology: "triangle-strip",
                    buffers: &SCATTER_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "milkyway planets",
                    vertex_entry: "vs_planet",
                    fragment_entry: "fs_planet",
                    topology: "triangle-strip",
                    buffers: &SCATTER_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "constellation stars",
                    vertex_entry: "vs_constellation_star",
                    fragment_entry: "fs_constellation_star",
                    topology: "triangle-strip",
                    buffers: &SCATTER_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
            ],
        ),
        (
            "errorbar",
            include_str!("errorbar_columnar.wgsl"),
            &[
                BrowserRenderPipelineSpec {
                    label: "precise errorbar",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &ERRORBAR_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "mapped errorbar",
                    vertex_entry: "vs_mapped",
                    fragment_entry: "fs_mapped",
                    topology: "triangle-list",
                    buffers: &ERRORBAR_MAPPED_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "hand-drawn errorbar",
                    vertex_entry: "vs_sketch",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &ERRORBAR_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "milkyway jets",
                    vertex_entry: "vs_jet",
                    fragment_entry: "fs_jet",
                    topology: "triangle-list",
                    buffers: &ERRORBAR_BUFFERS,
                    blend: BrowserBlend::Additive,
                },
            ],
        ),
        (
            "bar",
            include_str!("bar_columnar.wgsl"),
            &[
                BrowserRenderPipelineSpec {
                    label: "histogram bars",
                    vertex_entry: "vs_envelope_bars",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &BAR_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "mapped histogram bars",
                    vertex_entry: "vs_envelope_mapped_bars",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &BAR_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "histogram pixel envelope",
                    vertex_entry: "vs_bar_envelope",
                    fragment_entry: "fs_bar_envelope",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "selected histogram bin",
                    vertex_entry: "vs_bar_selection",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &BAR_BUFFERS,
                    blend: BrowserBlend::Premultiplied,
                },
            ],
        ),
        (
            "field",
            include_str!("field_columnar.wgsl"),
            &[
                BrowserRenderPipelineSpec {
                    label: "heatmap field",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_main",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "contour field",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_contour",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "label-gapped contour field",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_contour_labelled",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Premultiplied,
                },
                BrowserRenderPipelineSpec {
                    label: "selected field data",
                    vertex_entry: "vs_main",
                    fragment_entry: "fs_data_selection",
                    topology: "triangle-list",
                    buffers: &[],
                    blend: BrowserBlend::Premultiplied,
                },
            ],
        ),
        (
            "contour labels",
            include_str!("../contour_label.wgsl"),
            &[BrowserRenderPipelineSpec {
                label: "contour labels",
                vertex_entry: "vs_main",
                fragment_entry: "fs_main",
                topology: "triangle-list",
                buffers: &CONTOUR_LABEL_BUFFERS,
                blend: BrowserBlend::Premultiplied,
            }],
        ),
    ];

    for (module_label, source, specs) in groups {
        let shader_desc = Object::new();
        set(&shader_desc, "label", &JsValue::from_str(module_label))?;
        set(&shader_desc, "code", &JsValue::from_str(source))?;
        let shader = create_shader
            .call1(&device_js, shader_desc.as_ref())
            .map_err(js_error)?;

        for spec in specs {
            crate::init::started(observer, "renderer.prewarm.async", spec.label);
            let vertex = Object::new();
            set(&vertex, "module", &shader)?;
            set(&vertex, "entryPoint", &JsValue::from_str(spec.vertex_entry))?;
            let buffers = Array::new();
            for buffer_spec in spec.buffers {
                let attributes = Array::new();
                for attribute_spec in buffer_spec.attributes {
                    let attribute = Object::new();
                    set(
                        &attribute,
                        "format",
                        &JsValue::from_str(attribute_spec.format),
                    )?;
                    set(
                        &attribute,
                        "offset",
                        &JsValue::from_f64(attribute_spec.offset.into()),
                    )?;
                    set(
                        &attribute,
                        "shaderLocation",
                        &JsValue::from_f64(attribute_spec.location.into()),
                    )?;
                    attributes.push(attribute.as_ref());
                }
                let buffer = Object::new();
                set(
                    &buffer,
                    "arrayStride",
                    &JsValue::from_f64(buffer_spec.stride.into()),
                )?;
                set(
                    &buffer,
                    "stepMode",
                    &JsValue::from_str(buffer_spec.step_mode),
                )?;
                set(&buffer, "attributes", attributes.as_ref())?;
                buffers.push(buffer.as_ref());
            }
            set(&vertex, "buffers", buffers.as_ref())?;

            let fragment = Object::new();
            set(&fragment, "module", &shader)?;
            set(
                &fragment,
                "entryPoint",
                &JsValue::from_str(spec.fragment_entry),
            )?;
            let color = Object::new();
            set(&color, "srcFactor", &JsValue::from_str("one"))?;
            set(
                &color,
                "dstFactor",
                &JsValue::from_str(match spec.blend {
                    BrowserBlend::Premultiplied => "one-minus-src-alpha",
                    BrowserBlend::Additive | BrowserBlend::Maximum => "one",
                }),
            )?;
            set(
                &color,
                "operation",
                &JsValue::from_str(match spec.blend {
                    BrowserBlend::Maximum => "max",
                    BrowserBlend::Premultiplied | BrowserBlend::Additive => "add",
                }),
            )?;
            let alpha = color.clone();
            let blend = Object::new();
            set(&blend, "color", color.as_ref())?;
            set(&blend, "alpha", alpha.as_ref())?;
            let target = Object::new();
            set(&target, "format", &JsValue::from_str(format))?;
            set(&target, "blend", blend.as_ref())?;
            set(&target, "writeMask", &JsValue::from_f64(15.0))?;
            set(&fragment, "targets", Array::of1(target.as_ref()).as_ref())?;

            let primitive = Object::new();
            set(&primitive, "topology", &JsValue::from_str(spec.topology))?;
            let multisample = Object::new();
            set(
                &multisample,
                "count",
                &JsValue::from_f64(sample_count.into()),
            )?;
            set(&multisample, "mask", &JsValue::from_f64(u32::MAX.into()))?;
            set(&multisample, "alphaToCoverageEnabled", &JsValue::FALSE)?;
            let desc = Object::new();
            set(&desc, "label", &JsValue::from_str(spec.label))?;
            set(&desc, "layout", &JsValue::from_str("auto"))?;
            set(&desc, "vertex", vertex.as_ref())?;
            set(&desc, "fragment", fragment.as_ref())?;
            set(&desc, "primitive", primitive.as_ref())?;
            set(&desc, "multisample", multisample.as_ref())?;
            let promise = create_pipeline
                .call1(&device_js, desc.as_ref())
                .map_err(js_error)?;
            JsFuture::from(Promise::from(promise))
                .await
                .map_err(js_error)?;
            crate::init::finished(observer, "renderer.prewarm.async", spec.label);
            crate::init::yield_init_frame().await;
        }
    }
    Ok(())
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
    let shaders = ShaderModules::new(device);
    create_fullscreen_textured_pipeline_with_sample_count(
        device,
        &shaders.fullscreen,
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
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy fullscreen textured pipeline layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

// Transform uniform (data-space -> NDC), shared by scatter / line / errorbar.
// Vertex buffers carry data-space values directly; resize/zoom only updates
// this uniform, never the instance data.

/// Shared transform uniform for scatter / line / errorbar shaders.
///
/// 112 bytes (eight `vec2<f32>` fields plus one `array<vec4<f32>, 3>` at
/// offset 64, stride 16, WGSL uniform layout). Pixel sizes (point radius,
/// cap half-length) live in [`PrimitiveStyle`]; shaders convert them to NDC
/// via `pixel_to_ndc`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterTransform {
    pub data_min: [f32; 2],    // offset 0
    pub data_max: [f32; 2],    // offset 8
    pub data_min_lo: [f32; 2], // offset 16
    pub data_max_lo: [f32; 2], // offset 24
    /// Per-axis flag: 0.0 = linear, 1.0 = log10.
    pub scale_log: [f32; 2], // offset 32
    /// `(2 / chart_w, 2 / chart_h)` — 1 pixel in NDC. Shaders multiply pixel
    /// sizes (line width, point radius, cap half-length) by this.
    pub pixel_to_ndc: [f32; 2], // offset 40
    /// Affine from an axis-normalized SSoT coordinate into panel `t`.
    /// Layout margins belong here, never in `data_min` / `data_max`.
    pub data_to_panel_scale: [f32; 2], // offset 48
    pub data_to_panel_offset: [f32; 2], // offset 56
    /// Generic per-panel style parameter slots mirrored by WGSL `Transform`,
    /// packed by the renderer's style table (`StyleVariant::pack_params`, flat
    /// `[f32; 12]` split into three vec4 slots). All zeros in precise mode —
    /// the precise entry points never read them. Sketch:
    /// `[0] = [amplitude_px, wavelength_px, seed as f32, 0.0]`, rest 0;
    /// milkyway: `[0] = [star_density, ribbon_width_px,
    /// ribbon_intensity, seed as f32]`, `[1] = [star_scale, spread_px,
    /// faint_bias, planet_rim]`, `[2] = [structure_scale, star_brightness,
    /// 0, 0]`; constellation: `[0] = [star_opacity, line_opacity, 0.0,
    /// 0.0]`, rest 0. Seeds are stored as f32 (exact up to 2^24) and shaders
    /// recover them via `u32(...)`. Independently of style, `[2][2]` is
    /// reserved for the global point-base u32 bit pattern (resident: zero).
    /// Streamed point/errorbar entries recover it with `bitcast<u32>`, not
    /// numeric conversion, preserving global identities above 2^24.
    pub style_params: [[f32; 4]; 3], // offset 64 → 112 byte
}

// WGSL mirror size guards. Field order and size must remain byte-identical to
// every shader common block before either CPU structure changes.
const _: () = assert!(std::mem::size_of::<ScatterTransform>() == 112);

/// Admission headroom for one immutable streamed point/errorbar transform.
pub(crate) const STREAM_POINT_TRANSFORM_BYTES: u64 =
    std::mem::size_of::<ScatterTransform>() as u64;

/// Snapshot transform metadata with an exact global point identity. The
/// caller must admit [`STREAM_POINT_TRANSFORM_BYTES`] before allocation and
/// validate that its local draw range fits the global u32 index domain.
///
/// The bind group owns the uniform's GPU handle; its matching charge must be
/// retained by the same prepared packet. No source payload is copied, and the
/// shared view transform is never rewritten. Arc-driven line/star identities
/// have their own replay state and must not use this point-base field.
pub(crate) fn create_stream_point_transform_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    transform: &ScatterTransform,
    global_base: u32,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let mut chunk_transform = *transform;
    chunk_transform.style_params[2][2] = f32::from_bits(global_base);
    let tally = crate::gpu_memory::ChargeTally::new();
    let buffer = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy streamed point transform"),
            contents: bytemuck::bytes_of(&chunk_transform),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = create_scatter_transform_bind_group(device, layout, &buffer);
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// Allocate the transform uniform buffer with `COPY_DST` so subsequent
/// updates can use `queue.write_buffer` instead of recreating it.
pub fn create_scatter_transform_uniform_buffer(
    device: &wgpu::Device,
    transform: &ScatterTransform,
) -> wgpu::Buffer {
    // gpu-alloc: caller
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

/// Bind-group layout for the transform uniform. Render entries use it in the
/// vertex/fragment stages; exact histogram/field picking uses the same bytes in
/// compute so hit geometry cannot diverge from the draw transform.
pub fn create_scatter_transform_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy scatter transform bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT.union(wgpu::ShaderStages::COMPUTE),
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
/// and ignores the rest. Field order mirrors WGSL `Style` byte-for-byte.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimitiveStyle {
    pub color_premul: [f32; 4], // offset 0
    /// Line / errorbar stem thickness in pixels.
    pub line_width_px: f32, // offset 16
    /// Scatter point radius in pixels.
    pub point_radius_px: f32, // offset 20
    /// Errorbar cap half-length in pixels.
    pub cap_half_px: f32, // offset 24
    /// Errorbar cap stroke thickness in pixels.
    pub cap_width_px: f32, // offset 28
    /// Stable GPU marker-shape code — see [`shape_id`].
    pub shape_id: u32, // offset 32
    /// Number of valid scalars in `dash`; 0 = solid.
    pub dash_len: u32, // offset 36
    /// Per-series decorrelation salt (FNV-1a of `series_id`, written by
    /// `Renderer::create_style_for_series*`). Sketch/constellation shader
    /// entries XOR it into their hash seeds so series with identical
    /// sampling don't share star/wobble patterns; precise entries ignore it.
    pub series_salt: u32, // offset 40
    /// Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    /// for X; other primitive shaders ignore this field.
    pub primitive_flags: u32, // offset 44
    /// Up to 8 sequential `[on, off, ...]` pixel lengths: `dash[0]` first,
    /// then `dash[1]`.
    pub dash: [[f32; 4]; 2], // offset 48 → 80 byte
}

const _: () = assert!(std::mem::size_of::<PrimitiveStyle>() == 80);

pub(crate) const ERRORBAR_HAS_Y: u32 = 1 << 0;
pub(crate) const ERRORBAR_HAS_X: u32 = 1 << 1;

/// Renderer policy for histogram width ratios. Invalid host values inherit the
/// full-bin default instead of poisoning bar geometry with NaN.
pub(crate) fn sanitize_bar_width_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

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
            primitive_flags: 0,
            dash: [[0.0; 4]; 2],
        }
    }

    /// A bar's style — the `Style` reinterpretation documented in
    /// **SHADER_COMMON.md**'s bar `Style` reinterpretation table, which is the
    /// SSoT for this mapping. Change
    /// one without the other and a bar's colour, border, or baseline goes
    /// silently wrong.
    ///
    /// `scale` multiplies the pixel dimensions for high-DPI export, exactly as
    /// the other `create_style_for_series_scaled` paths do.
    pub fn from_bar(bar: &crate::data_config::DataBarStyleConfig, scale: f32) -> Self {
        let premul = |c: Color| {
            let a = c.a.clamp(0.0, 1.0);
            [c.r * a, c.g * a, c.b * a, a]
        };
        // The baseline crosses as the pool's (hi, lo) f32 pair so a large
        // absolute base keeps its small deltas — a single f32 would not.
        let (base_hi, base_lo) = crate::data::split_f64_to_f32_pair(bar.baseline);
        Self {
            color_premul: premul(bar.fill_color),
            line_width_px: bar.border_width.max(0.0) * scale,
            point_radius_px: 0.0,
            cap_half_px: bar.gap_px.max(0.0) * scale,
            cap_width_px: sanitize_bar_width_ratio(bar.width_ratio),
            shape_id: match bar.orientation {
                crate::data_config::BarOrientation::Vertical => 0,
                crate::data_config::BarOrientation::Horizontal => 1,
            },
            dash_len: 0,
            series_salt: 0,
            primitive_flags: 0,
            dash: [premul(bar.border_color), [base_hi, base_lo, 0.0, 0.0]],
        }
    }

    pub(crate) fn pack_dash_pattern(&mut self, pattern: &[f32], scale: f32) {
        let capacity: usize = self.dash.iter().map(|lane| lane.len()).sum();
        self.dash.iter_mut().flatten().for_each(|slot| *slot = 0.0);
        for (slot, length) in self.dash.iter_mut().flatten().zip(pattern.iter()) {
            *slot = *length * scale;
        }
        self.dash_len = pattern.len().min(capacity) as u32;
    }
}

pub(crate) const DATA_SELECTION_KIND_HISTOGRAM_BIN: u32 = 1;
pub(crate) const DATA_SELECTION_KIND_MATRIX_CELL: u32 = 2;
pub(crate) const DATA_SELECTION_KIND_CONTOUR_LEVEL: u32 = 3;

/// Small typed-selection uniform shared by the bar and field overlay entries.
///
/// GPU twins: `bar_columnar.wgsl::DataSelection` and
/// `field_columnar.wgsl::DataSelection`, both 48 bytes. It contains provenance
/// indices plus visual scalars, never reconstructed data coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DataSelectionGpu {
    pub color_premul: [f32; 4],
    pub metrics: [f32; 4],
    pub indices: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<DataSelectionGpu>() == 48);

impl DataSelectionGpu {
    pub(crate) fn from_color(color: Color) -> Self {
        let alpha = color.a.clamp(0.0, 1.0);
        Self {
            color_premul: [color.r * alpha, color.g * alpha, color.b * alpha, alpha],
            metrics: [0.0; 4],
            indices: [0; 4],
        }
    }
}

pub(crate) fn create_data_selection_bind_group_layout(
    device: &wgpu::Device,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy typed data selection bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 4,
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

pub(crate) fn create_data_selection_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    selection: &DataSelectionGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let tally = crate::gpu_memory::ChargeTally::new();
    let buffer = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy typed data selection uniform"),
            contents: bytemuck::bytes_of(selection),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy typed data selection bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 4,
            resource: buffer.as_entire_binding(),
        }],
    });
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}

/// Map a [`ScatterShape`] to the stable `Style.shape_id` uniform value.
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
        ScatterShape::TriangleDown => 9,
        ScatterShape::TriangleLeft => 10,
        ScatterShape::TriangleRight => 11,
        ScatterShape::Plus => 12,
        ScatterShape::Pentagon => 13,
        ScatterShape::Hexagon => 14,
        ScatterShape::Octagon => 15,
        ScatterShape::Star => 16,
        ScatterShape::TriangleDownFilled => 17,
        ScatterShape::TriangleLeftFilled => 18,
        ScatterShape::TriangleRightFilled => 19,
        ScatterShape::PlusFilled => 20,
        ScatterShape::CrossFilled => 21,
        ScatterShape::PentagonFilled => 22,
        ScatterShape::HexagonFilled => 23,
        ScatterShape::OctagonFilled => 24,
        ScatterShape::StarFilled => 25,
    }
}

/// Bind-group layout for the style uniform. `VERTEX_FRAGMENT` because the
/// line vertex shader reads `line_width_px`.
pub fn create_style_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy primitive style bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

pub fn create_style_uniform_buffer(device: &wgpu::Device, style: &PrimitiveStyle) -> wgpu::Buffer {
    // gpu-alloc: uncharged(style uniforms are owned by their bind groups)
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

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleSlotGpu {
    pub color_premul: [f32; 4],
    /// `(radius_px, shape_id as f32, mask_bits as f32, 0)`.
    pub meta: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ScatterStyleSlotGpu>() == 32);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleOverrideGpu {
    pub point_index: u32,
    pub _pad: [u32; 3],
    pub color_premul: [f32; 4],
    /// `(radius_px, shape_id as f32, mask_bits as f32, 0)`.
    pub meta: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ScatterStyleOverrideGpu>() == 48);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleMapMeta {
    pub style_count: u32,
    pub override_count: u32,
    pub has_index: u32,
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<ScatterStyleMapMeta>() == 16);

/// GPU-side per-point style map. The bind group keeps all backing buffers alive.
///
/// Layout (group 2 in the mapped precise scatter/errorbar pipelines). Bindings
/// 0..4 are already used by the constellation scatter entries in the same WGSL
/// module, so mapped precise entries use bindings 5..7.
/// - binding 5: [`ScatterStyleSlotGpu`] array.
/// - binding 6: [`ScatterStyleOverrideGpu`] array.
/// - binding 7: [`ScatterStyleMapMeta`].
pub struct ScatterStyleMap {
    pub bind_group: wgpu::BindGroup,
    pub has_index: bool,
    style_buf: wgpu::Buffer,
    override_buf: wgpu::Buffer,
    meta: ScatterStyleMapMeta,
    stream_base_charge: Option<crate::gpu_memory::SharedCharge>,
}

impl ScatterStyleMap {
    pub(crate) fn stream_base_bytes(&self) -> u64 {
        self.style_buf.size() + self.override_buf.size() + 16
    }

    pub(crate) fn stream_base_is_charged(&self) -> bool {
        self.stream_base_charge.is_some()
    }

    pub(crate) fn charge_stream_base(&mut self, ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>) {
        if self.stream_base_charge.is_none() {
            let tally = crate::gpu_memory::ChargeTally::new();
            tally.add(self.stream_base_bytes());
            self.stream_base_charge = Some(crate::gpu_memory::shared_charge(
                tally, ledger, crate::gpu_memory::GpuResourceKind::Uniform,
            ));
        }
    }

    pub(crate) fn stream_bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        global_base: u32,
        tally: &crate::gpu_memory::ChargeTally,
    ) -> wgpu::BindGroup {
        let stream_meta = ScatterStyleMapMeta {
            _pad: global_base,
            ..self.meta
        };
        let meta_buf = crate::gpu_memory::charged_buffer_init(
            tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy streamed point style map meta"),
                contents: bytemuck::bytes_of(&stream_meta),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy streamed point style map bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.style_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.override_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: meta_buf.as_entire_binding(),
                },
            ],
        })
    }
}

pub type ErrorBarStyleSlotGpu = ScatterStyleSlotGpu;
pub type ErrorBarStyleOverrideGpu = ScatterStyleOverrideGpu;
pub type ErrorBarStyleMapMeta = ScatterStyleMapMeta;
pub type ErrorBarStyleMap = ScatterStyleMap;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BarStyleSlotGpu {
    pub fill_color_premul: [f32; 4],
    pub border_color_premul: [f32; 4],
    /// `(border_width_px, gap_px, width_ratio, mask_bits as f32)`.
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<BarStyleSlotGpu>() == 48);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BarStyleOverrideGpu {
    pub bin_index: u32,
    pub _pad: [u32; 3],
    pub fill_color_premul: [f32; 4],
    pub border_color_premul: [f32; 4],
    /// `(border_width_px, gap_px, width_ratio, mask_bits as f32)`.
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<BarStyleOverrideGpu>() == 64);

pub type BarStyleMapMeta = ScatterStyleMapMeta;

/// GPU-side sparse histogram-bin style map. It deliberately shares the
/// generic mapped-style bind-group layout (bindings 5..7), while its record
/// bytes are bar-specific and consumed only by `bar_columnar.wgsl`.
pub struct BarStyleMap {
    pub bind_group: wgpu::BindGroup,
}

pub fn create_per_point_style_map_bind_group_layout(
    device: &wgpu::Device,
) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy per-point style map bgl"),
        entries: &[
            storage(5),
            storage(6),
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

pub fn create_scatter_style_map_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    create_per_point_style_map_bind_group_layout(device)
}

pub fn create_scatter_style_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    style_slots: &[ScatterStyleSlotGpu],
    overrides: &[ScatterStyleOverrideGpu],
    meta: ScatterStyleMapMeta,
) -> ScatterStyleMap {
    let dummy_style = [ScatterStyleSlotGpu {
        color_premul: [0.0; 4],
        meta: [0.0; 4],
    }];
    let dummy_override = [ScatterStyleOverrideGpu {
        point_index: 0,
        _pad: [0; 3],
        color_premul: [0.0; 4],
        meta: [0.0; 4],
    }];
    let style_slots = if style_slots.is_empty() {
        &dummy_style[..]
    } else {
        style_slots
    };
    let overrides = if overrides.is_empty() {
        &dummy_override[..]
    } else {
        overrides
    };
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let style_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style rows"),
        contents: bytemuck::cast_slice(style_slots),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let override_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style override rows"),
        contents: bytemuck::cast_slice(overrides),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let meta_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style map meta"),
        contents: bytemuck::bytes_of(&meta),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy scatter style map bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: style_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: override_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: meta_buf.as_entire_binding(),
            },
        ],
    });
    ScatterStyleMap {
        bind_group,
        has_index: meta.has_index != 0,
        style_buf,
        override_buf,
        meta,
        stream_base_charge: None,
    }
}

pub fn create_errorbar_style_map(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    style_slots: &[ErrorBarStyleSlotGpu],
    overrides: &[ErrorBarStyleOverrideGpu],
    meta: ErrorBarStyleMapMeta,
) -> ErrorBarStyleMap {
    create_scatter_style_map(device, bgl, style_slots, overrides, meta)
}

pub fn create_bar_style_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    overrides: &[BarStyleOverrideGpu],
) -> BarStyleMap {
    let dummy_style = [BarStyleSlotGpu {
        fill_color_premul: [0.0; 4],
        border_color_premul: [0.0; 4],
        params: [0.0; 4],
    }];
    let dummy_override = [BarStyleOverrideGpu {
        bin_index: 0,
        _pad: [0; 3],
        fill_color_premul: [0.0; 4],
        border_color_premul: [0.0; 4],
        params: [0.0; 4],
    }];
    let override_rows = if overrides.is_empty() {
        &dummy_override[..]
    } else {
        overrides
    };
    let meta = BarStyleMapMeta {
        style_count: 0,
        override_count: overrides.len().min(u32::MAX as usize) as u32,
        has_index: 0,
        _pad: 0,
    };
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let style_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style padding row"),
        contents: bytemuck::cast_slice(&dummy_style),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let override_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style override rows"),
        contents: bytemuck::cast_slice(override_rows),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: uncharged(style rows are rebuilt per prepare and owned by the bind group)
    let meta_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style map meta"),
        contents: bytemuck::bytes_of(&meta),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy histogram style map bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: style_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: override_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: meta_buf.as_entire_binding(),
            },
        ],
    });
    BarStyleMap { bind_group }
}

/// Bind group layout for the constellation star pass's per-series data
/// (group 3 of the stars pipeline): the arc-length prefix and the column
/// pool as read-only storage plus a small offsets uniform. Read-only storage
/// in the vertex stage is core WebGPU; the GL-downlevel adapters that lack
/// it are already rejected at renderer construction.
pub fn create_star_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy star data bgl"),
        entries: &[
            storage(0), // arc-length prefix
            storage(1), // column pool (x/y bases in the uniform)
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

pub struct AxisLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub bind_group: &'a wgpu::BindGroup,
}

/// Build a [`ScatterTransform`] from a `Config`.
///
/// The transform copies the Config SSoT axis range into `(hi, lo)` fields.
/// Data-area placement is a separate affine, so layout margins can never
/// silently replace the range the GPU draws against.
pub fn scatter_transform_from_config(config: &Config) -> ScatterTransform {
    let ca = &config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;

    use crate::config::AxisScale;
    let log_x = matches!(config.bottom_x.scale, AxisScale::Logarithmic);
    let log_y = matches!(config.left_y.scale, AxisScale::Logarithmic);

    // For log axes, guard only the axis range. Data values still follow the
    // shader's NaN/non-positive handling.
    let to_log = |v: f64| v.log10();
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

    let data_min_x = if log_x { to_log(x_min) } else { x_min };
    let data_max_x = if log_x { to_log(x_max) } else { x_max };
    let data_min_y = if log_y { to_log(y_min) } else { y_min };
    let data_max_y = if log_y { to_log(y_max) } else { y_max };

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
    let orient_axis = |min: f64, max: f64, inverted: bool| {
        if inverted { (max, min) } else { (min, max) }
    };
    let (data_to_panel_scale, data_to_panel_offset) = config.data_area().map_or_else(
        |_| ([1.0, 1.0], [0.0, 0.0]),
        |area| {
            let da = area.0;
            let rel_x = (da.x as i64 - ca.x as i64) as f64;
            let rel_y = (da.y as i64 - ca.y as i64) as f64;
            let chart_w64 = f64::from(chart_w);
            let chart_h64 = f64::from(chart_h);
            let sx = rel_x / chart_w64;
            let ex = (rel_x + f64::from(da.width)) / chart_w64;
            let sy = (chart_h64 - (rel_y + f64::from(da.height))) / chart_h64;
            let ey = (chart_h64 - rel_y) / chart_h64;
            ([(ex - sx) as f32, (ey - sy) as f32], [sx as f32, sy as f32])
        },
    );

    let (data_min_x, data_max_x) = orient_axis(data_min_x, data_max_x, config.bottom_x.inverted);
    let (data_min_y, data_max_y) = orient_axis(data_min_y, data_max_y, config.left_y.inverted);
    let (min_x_hi, min_x_lo) = crate::data::split_f64_to_f32_pair(data_min_x);
    let (max_x_hi, max_x_lo) = crate::data::split_f64_to_f32_pair(data_max_x);
    let (min_y_hi, min_y_lo) = crate::data::split_f64_to_f32_pair(data_min_y);
    let (max_y_hi, max_y_lo) = crate::data::split_f64_to_f32_pair(data_max_y);
    ScatterTransform {
        data_min: [min_x_hi, min_y_hi],
        data_max: [max_x_hi, max_y_hi],
        data_min_lo: [min_x_lo, min_y_lo],
        data_max_lo: [max_x_lo, max_y_lo],
        scale_log,
        pixel_to_ndc,
        data_to_panel_scale,
        data_to_panel_offset,
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
    let shaders = ShaderModules::new(device);
    create_line_columnar_pipeline_with_sample_count(
        device,
        &shaders.line,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_line_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_line_columnar_pipeline_with_entries(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_main",
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

/// Milkyway ribbon strip vertices per instance — `2·(S+1)`, twin of
/// `CONS_RIBBON_SUBDIV` in `line_columnar.wgsl`.
pub const MILKYWAY_RIBBON_VERTICES: u32 = 18;

const PSF_SIZE: u32 = 128;
const ATLAS_TILE: u32 = 128;
const STYLE_STRIP_SIZE: u32 = 256;

/// Exact Rgba8Unorm payload bytes of either textured style set: one PSF,
/// one 2x2 planet atlas, and the blackbody and ring strips (one mip each).
pub(crate) const STYLED_TEXTURE_BYTES: u64 = 4
    * (PSF_SIZE as u64 * PSF_SIZE as u64
        + (ATLAS_TILE as u64 * 2) * (ATLAS_TILE as u64 * 2)
        + 2 * STYLE_STRIP_SIZE as u64);

/// Milkyway pipelines and baked style textures for one target format, cached
/// inside the renderer's lazy style set. The bind group keeps the textures
/// alive for as long as the pipelines can sample them.
pub(crate) struct MilkywaySet {
    pub(crate) ribbon: wgpu::RenderPipeline,
    pub(crate) stars: wgpu::RenderPipeline,
    /// Ringed-planet scatter pass uses premultiplied blending so planet bodies
    /// occlude the additive star field behind them.
    pub(crate) planets: wgpu::RenderPipeline,
    /// Bipolar-jet errorbars — additive beams + terminal shock knots over
    /// the precise errorbar geometry.
    pub(crate) jets: wgpu::RenderPipeline,
    pub(crate) star_tex_bg: wgpu::BindGroup,
}

/// Point-constellation pipelines + baked star textures for one target format.
/// The style intentionally supports only `ScatterLine`: `line` connects the
/// data points and `stars` renders PSF sprites at the scatter positions.
pub(crate) struct PointConstellationSet {
    pub(crate) line: wgpu::RenderPipeline,
    pub(crate) stars: wgpu::RenderPipeline,
    pub(crate) star_tex_bg: wgpu::BindGroup,
}

// Procedural planet-atlas bakes run once per style-set creation; results are
// cached on the GPU and never recomputed per frame.

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
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
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
                            let base =
                                mix3([0.34, 0.52, 0.86], [0.55, 0.72, 0.95], v * 0.5 + s * 0.25);
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
    let mut out = vec![0u8; STYLE_STRIP_SIZE as usize * 4];
    for i in 0..STYLE_STRIP_SIZE as usize {
        let u = i as f64 / (STYLE_STRIP_SIZE - 1) as f64;
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
/// Airy-style ring). This runs once per style-set creation, so the render loop
/// only samples the baked texture.
fn bake_psf_rgba(size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let half = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - half) / half; // -1..1
            let dy = (y as f32 - half) / half;
            let r = (dx * dx + dy * dy).sqrt();
            let edge_t = ((r - 0.82) / (0.98 - 0.82)).clamp(0.0, 1.0);
            let aperture = 1.0 - edge_t * edge_t * (3.0 - 2.0 * edge_t);
            // Flat saturated core with a steep gaussian shoulder.
            let core = (-(r / 0.11).powf(2.6)).exp().min(1.0) * aperture;
            // Exponential halo wings + one faint ring at 0.45.
            let halo =
                (0.85 * (-r / 0.28).exp() + 0.08 * (-((r - 0.45) / 0.06).powi(2)).exp()) * aperture;
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
    let mut out = vec![0u8; STYLE_STRIP_SIZE as usize * 4];
    for i in 0..STYLE_STRIP_SIZE as usize {
        let kelvin = 2500.0 + 9500.0 * (i as f64 / (STYLE_STRIP_SIZE - 1) as f64);
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

/// Build the milkyway style set: bake PSF + blackbody LUT, upload them,
/// and compile the additive ribbon/star pipelines (group 2 = the textures).
// Milkyway assembly combines both texture resources and both line/star layouts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_milkyway_set(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &ShaderModules,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    star_data_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> MilkywaySet {
    let make_tex = |label: &str, w: u32, h: u32, data: &[u8]| {
        // gpu-alloc: uncharged(baked inside a lazily-compiled style set)
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let psf_view = make_tex(
        "figgy milkyway psf",
        PSF_SIZE,
        PSF_SIZE,
        &bake_psf_rgba(PSF_SIZE),
    );
    let lut_view = make_tex(
        "figgy milkyway blackbody lut",
        STYLE_STRIP_SIZE,
        1,
        &bake_blackbody_lut(),
    );
    let atlas_view = make_tex(
        "figgy milkyway planet atlas",
        ATLAS_TILE * 2,
        ATLAS_TILE * 2,
        &bake_planet_atlas(ATLAS_TILE),
    );
    let ring_view = make_tex(
        "figgy milkyway ring strip",
        STYLE_STRIP_SIZE,
        1,
        &bake_ring_strip(),
    );

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
        label: Some("figgy milkyway texture bgl"),
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
        label: Some("figgy milkyway sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let star_tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy milkyway texture bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&psf_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&lut_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&ring_view),
            },
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
        device,
        &shaders.line,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_ribbon",
        "fs_ribbon",
        max_blend,
        wgpu::PrimitiveTopology::TriangleStrip,
        Some(&tex_bgl),
        "figgy milkyway ribbon pipeline",
    );
    // Arc-driven star pass: NO vertex buffers — the VS reads the arc-length
    // prefix and the column pool as storage (group 3) and is drawn via
    // DrawIndirect args computed on the GPU from the polyline's total arc.
    let star_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy milkyway star layout"),
        bind_group_layouts: &[
            Some(transform_bgl),
            Some(style_bgl),
            Some(&tex_bgl),
            Some(star_data_bgl),
        ],
        immediate_size: 0,
    });
    let stars = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy milkyway stars pipeline"),
        layout: Some(&star_layout),
        vertex: wgpu::VertexState {
            module: &shaders.line,
            entry_point: Some("vs_stars"),
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
            module: &shaders.line,
            entry_point: Some("fs_stars"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(additive),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // Planets keep the scatter builder's premultiplied blend — bodies
    // occlude the additive star field behind them.
    let planets = create_scatter_columnar_pipeline_full(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_planet",
        "fs_planet",
        Some(&tex_bgl),
        "figgy milkyway planets pipeline",
    );
    let jets = create_errorbar_columnar_pipeline_full(
        device,
        &shaders.errorbar,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_jet",
        "fs_jet",
        additive,
        "figgy milkyway jets pipeline",
    );

    MilkywaySet {
        ribbon,
        stars,
        planets,
        jets,
        star_tex_bg,
    }
}

/// Build the lightweight constellation style set: PSF stars at scatter
/// positions plus a translucent connecting line. No arc-prefix star pass,
/// planets, or jets are compiled for this mode.
pub(crate) fn create_point_constellation_set(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &ShaderModules,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> PointConstellationSet {
    let make_tex = |label: &str, w: u32, h: u32, data: &[u8]| {
        // gpu-alloc: uncharged(baked inside a lazily-compiled style set)
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let psf_view = make_tex(
        "figgy point constellation psf",
        PSF_SIZE,
        PSF_SIZE,
        &bake_psf_rgba(PSF_SIZE),
    );
    let lut_view = make_tex(
        "figgy point constellation blackbody lut",
        STYLE_STRIP_SIZE,
        1,
        &bake_blackbody_lut(),
    );
    // These two are unused by point constellation entries, but keeping the
    // five-binding texture layout identical lets the shared scatter WGSL
    // module declare both star and planet resources.
    let atlas_view = make_tex(
        "figgy point constellation planet atlas",
        ATLAS_TILE * 2,
        ATLAS_TILE * 2,
        &bake_planet_atlas(ATLAS_TILE),
    );
    let ring_view = make_tex(
        "figgy point constellation ring strip",
        STYLE_STRIP_SIZE,
        1,
        &bake_ring_strip(),
    );

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
        label: Some("figgy point constellation texture bgl"),
        entries: &[
            tex_entry(0),
            tex_entry(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            tex_entry(3),
            tex_entry(4),
        ],
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("figgy point constellation sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let star_tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy point constellation texture bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&psf_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&lut_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&ring_view),
            },
        ],
    });

    let line = create_line_columnar_pipeline_with_entries(
        device,
        &shaders.line,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_constellation_line",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        None,
        "figgy point constellation line pipeline",
    );
    let stars = create_scatter_columnar_pipeline_full(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_constellation_star",
        "fs_constellation_star",
        Some(&tex_bgl),
        "figgy point constellation stars pipeline",
    );

    PointConstellationSet {
        line,
        stars,
        star_tex_bg,
    }
}

/// Entry-point-parameterized line pipeline builder. Styled variants share
/// the precise pipeline's six instance slots and differ in entry points,
/// blend state (constellation is additive), topology (star quads are a
/// TriangleList), and an optional third bind group (style textures) — the
/// renderer's style table supplies all of it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_line_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    topology: wgpu::PrimitiveTopology,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    let mut bgls: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(transform_bgl), Some(style_bgl)];
    if let Some(t) = texture_bgl {
        bgls.push(Some(t));
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy line columnar layout"),
        bind_group_layouts: &bgls,
        immediate_size: 0,
    });

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            // 4 per-instance f32 slots: x_a, y_a, x_b, y_b. The same X/Y
            // columns are bound twice; the second pair starts one logical
            // value later, so instance i sees points [i] and [i+1] together.
            // Each instance emits a 4-vertex quad strip for one segment.
            buffers: &[
                // slot 0: x_a (X column from offset 0)
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }),
                // slot 1: y_a
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 1,
                    }],
                }),
                // slot 2: x_b (X column from the next value)
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 2,
                    }],
                }),
                // slot 3: y_b
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 3,
                    }],
                }),
                // slots 4/5: cumulative arc length (px) at A and B — the
                // same prefix buffer bound twice with a one-f32 shift, like
                // x/y. Solid lines bind the X column here as inert filler
                // (the fragment stage ignores it when dash_len == 0).
                Some(wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 4,
                    }],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: f32_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 5,
                    }],
                }),
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
        multisample: multisample_state(sample_count),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
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
    let shaders = ShaderModules::new(device);
    create_scatter_columnar_pipeline_with_sample_count(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_scatter_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_scatter_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_main",
        None,
        "figgy scatter columnar pipeline",
    )
}

/// Precise scatter pipeline variant that resolves per-point style slots.
/// It keeps the common WGSL block untouched by using separate `vs_mapped` /
/// `fs_mapped` entry points and a fourth per-instance f32 vertex slot for
/// `point_style_index_column` when present. Sparse-only mappings bind the X
/// column into that slot as inert filler.
pub fn create_scatter_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let shaders = ShaderModules::new(device);
    create_scatter_columnar_mapped_pipeline_with_entries(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        style_map_bgl,
        target_format,
        sample_count,
        "vs_mapped",
        "fs_mapped",
        "figgy scatter mapped pipeline",
    )
}

// Mapping adds index/table layouts to the shared scatter pipeline state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_mapped_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(transform_bgl), Some(style_bgl), Some(style_map_bgl)],
        immediate_size: 0,
    });

    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: vec2_stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 1,
                    }],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 2,
                    }],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32,
                        offset: 0,
                        shader_location: 3,
                    }],
                }),
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
        multisample: multisample_state(sample_count),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Two-entry convenience used by the sketch style (shared layout/state).
// This wrapper forwards both shader entries and the shared scatter state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_scatter_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        vs_entry,
        fs_entry,
        None,
        label,
    )
}

/// Entry-point-parameterized scatter pipeline builder. Styled variants share
/// the precise pipeline's three vertex slots; the constellation planet
/// variant additionally binds the style textures as group 2.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scatter_columnar_pipeline_full(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    let mut bgls: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(transform_bgl), Some(style_bgl)];
    if let Some(t) = texture_bgl {
        bgls.push(Some(t));
    }
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy scatter columnar layout"),
        bind_group_layouts: &bgls,
        immediate_size: 0,
    });

    let vec2_stride = (std::mem::size_of::<f32>() * 2) as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                // slot 0: unit quad (per-vertex, vec2)
                Some(wgpu::VertexBufferLayout {
                    array_stride: vec2_stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }),
                // slot 1: X column (per-instance, f32)
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 1,
                    }],
                }),
                // slot 2: Y column (per-instance, f32)
                Some(wgpu::VertexBufferLayout {
                    array_stride: column_stride,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 2,
                    }],
                }),
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
        multisample: multisample_state(sample_count),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
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
    let shaders = ShaderModules::new(device);
    create_errorbar_columnar_pipeline_with_sample_count(
        device,
        &shaders.errorbar,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_errorbar_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_with_entries(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "figgy errorbar columnar pipeline",
    )
}

pub fn create_errorbar_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let shaders = ShaderModules::new(device);
    create_errorbar_columnar_mapped_pipeline_from_shader(
        device,
        &shaders.errorbar,
        transform_bgl,
        style_bgl,
        style_map_bgl,
        target_format,
        sample_count,
    )
}

pub(crate) fn create_errorbar_columnar_mapped_pipeline_from_shader(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy errorbar mapped layout"),
        bind_group_layouts: &[Some(transform_bgl), Some(style_bgl), Some(style_map_bgl)],
        immediate_size: 0,
    });

    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    const ATTR0: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }];
    const ATTR1: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 1,
    }];
    const ATTR2: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 2,
    }];
    const ATTR3: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 3,
    }];
    const ATTR4: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 4,
    }];
    const ATTR5: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 5,
    }];
    const ATTR6: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 0,
        shader_location: 6,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR3,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR4,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR5,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR6,
        }),
    ];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("figgy errorbar mapped pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_mapped"),
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
        multisample: multisample_state(sample_count),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_mapped"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Entry-point-parameterized errorbar pipeline builder. Styled variants
/// (e.g. the sketch `vs_sketch` — fragment stage shared, vertex count
/// unchanged at 36 per instance) share the precise pipeline's layout and
/// state; the renderer's style table supplies the entry string.
// Styled error bars vary shader entries while retaining shared pipeline state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_errorbar_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    create_errorbar_columnar_pipeline_full(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        vs_entry,
        "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        label,
    )
}

/// Full-control errorbar builder — the constellation jet variant needs its
/// own fragment entry and additive blending.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_errorbar_columnar_pipeline_full(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    label: &str,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy errorbar columnar layout"),
        bind_group_layouts: &[Some(transform_bgl), Some(style_bgl)],
        immediate_size: 0,
    });

    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    // Hold attributes in const arrays to avoid temporary-lifetime issues.
    const ATTR0: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }];
    const ATTR1: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 1,
    }];
    const ATTR2: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 2,
    }];
    const ATTR3: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 3,
    }];
    const ATTR4: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 4,
    }];
    const ATTR5: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 5,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR3,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR4,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR5,
        }),
    ];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
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
        multisample: multisample_state(sample_count),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs_entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Vertices per bar instance: five axis-aligned quads (fill + four border
/// edges) as a TriangleList. Must match `bar_columnar.wgsl`'s `BAR_SEGMENTS`.
pub const BAR_VERTICES_PER_INSTANCE: u32 = 30;
/// Four axis-aligned outline quads for one selected histogram bin.
/// Must match `vs_bar_selection` in `bar_columnar.wgsl`.
pub const BAR_SELECTION_VERTICES: u32 = 24;

/// Columnar bar pipeline. Slots (all per-instance): 0 = edge_lo, 1 = edge_hi,
/// 2 = value. The caller binds the edge column twice, the second time shifted
/// by one logical value, and picks which pool column is edges vs values from
/// the bar's orientation.
pub fn create_bar_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shaders = ShaderModules::new(device);
    create_bar_columnar_pipeline_with_entry(
        device,
        &shaders.bar,
        transform_bgl,
        style_bgl,
        None,
        target_format,
        1,
        "vs_main",
        "figgy bar columnar pipeline",
    )
}

pub(crate) fn create_bar_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        None,
        target_format,
        sample_count,
        "vs_envelope_bars",
        "figgy bar columnar pipeline",
    )
}

pub fn create_bar_columnar_mapped_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shaders = ShaderModules::new(device);
    create_bar_columnar_pipeline_with_entry(
        device,
        &shaders.bar,
        transform_bgl,
        style_bgl,
        Some(style_map_bgl),
        target_format,
        1,
        "vs_mapped",
        "figgy mapped bar columnar pipeline",
    )
}

pub(crate) fn create_bar_columnar_mapped_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        Some(style_map_bgl),
        target_format,
        sample_count,
        "vs_envelope_mapped_bars",
        "figgy mapped bar columnar pipeline",
    )
}

pub(crate) fn create_bar_selection_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_bar_columnar_pipeline_with_entry(
        device,
        shader,
        transform_bgl,
        style_bgl,
        Some(selection_bgl),
        target_format,
        sample_count,
        "vs_bar_selection",
        "figgy selected histogram bin pipeline",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_bar_columnar_pipeline_with_entry(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    group_one_bgl: &wgpu::BindGroupLayout,
    group_two_bgl: Option<&wgpu::BindGroupLayout>,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vertex_entry: &str,
    label: &str,
) -> wgpu::RenderPipeline {
    let base_layouts = [Some(transform_bgl), Some(group_one_bgl)];
    let selection_layouts = [Some(transform_bgl), Some(group_one_bgl), group_two_bgl];
    let bind_group_layouts = if group_two_bgl.is_some() {
        &selection_layouts[..]
    } else {
        &base_layouts[..]
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy bar columnar layout"),
        bind_group_layouts,
        immediate_size: 0,
    });

    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    const ATTR0: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }];
    const ATTR1: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 1,
    }];
    const ATTR2: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 2,
    }];
    let buffers = [
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR0,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR1,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: column_stride,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTR2,
        }),
    ];

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vertex_entry),
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
    })
}

/// Vertices per field draw. The field is **one** quad for the whole grid, not
/// one per cell — `field_columnar.wgsl`'s header has the two reasons (25M
/// instances at matrix scale, and MSAA seams between adjacent quads).
pub const FIELD_VERTICES: u32 = 6;

/// Where one constituent grid column lives in the pool.
///
/// GPU twin: `field_columnar.wgsl::GridColumn`. `base` is an **f32 lane** index,
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

/// Contour lookup granularity. Each block fits in one shader `u32` candidate
/// mask, so the original declaration order can be restored without a second
/// per-fragment table or an O(level_count) scan.
pub(crate) const CONTOUR_LEVEL_BLOCK_SIZE: usize = 32;
pub(crate) const CONTOUR_LEVEL_BLOCK_COUNT: usize = 32;
pub(crate) const CONTOUR_LEVEL_LOOKUP_CAPACITY: usize =
    CONTOUR_LEVEL_BLOCK_SIZE * CONTOUR_LEVEL_BLOCK_COUNT;
const _: () = assert!(crate::data_config::MAX_CONTOUR_LEVELS == CONTOUR_LEVEL_LOOKUP_CAPACITY);

/// Per-32-level lookup bounds uploaded at field group-2 binding 6.
///
/// GPU twin: `field_columnar.wgsl::ContourLookupMetadata`, 8 B. Counts are
/// derived from the actual f32 search keys, after the declared f64 levels have
/// crossed the renderer upload boundary.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContourLookupMetadataGpu {
    pub finite_count: u32,
    pub negative_infinity_count: u32,
}

const _: () = assert!(std::mem::size_of::<ContourLookupMetadataGpu>() == 8);

/// Everything the field shader needs beyond `Transform` and `Style`.
///
/// GPU twin: `field_columnar.wgsl::FieldParams`, 64 B. `z_min` / `z_max` are the
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
    // one over the limit on a conformant device.
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("figgy field data bgl"),
        entries: &[
            storage(0, shared), // column pool (coordinate + grid bases in the uniform)
            storage(1, shared), // per-constituent-column (base, len)
            storage(2, shared), // contour levels, in data units
            storage(3, render), // colourmap control points
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
    })
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
    let shaders = ShaderModules::new(device);
    create_field_columnar_pipeline_with_sample_count(
        device,
        &shaders.field,
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

/// Labelled contours add group 3 containing the exact selected-anchor buffers.
/// No-label contours keep the original layout and entry point, so merely adding
/// contour support does not compile or bind label machinery.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_labelled_contour_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_contour_labelled",
        "figgy labelled field contour pipeline",
    )
}

#[allow(clippy::too_many_arguments)]
fn create_field_columnar_pipeline_with_optional_extra_layout(
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
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("figgy field columnar layout"),
        bind_group_layouts: &layouts[..layout_count],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
    /// for the sketch `vs_sketch`, [`MILKYWAY_RIBBON_VERTICES`] for
    /// the milkyway ribbon.
    pub verts_per_instance: u32,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the milkyway/constellation PSF/LUT bind group. `None` for precise/sketch.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
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

/// One contour series' draw.
///
/// The **same quad as the field**, through `fs_contour`: the isolines are the
/// level set of the same bilinear interpolation, so there is no segment list, no
/// indirect draw and no per-level draw call. Per-level colour comes from the
/// group-2 `level_colors` table. Stroke coverage comes from the quadratic root
/// of the current cell's bilinear polynomial along the fragment's gradient
/// normal; it is not a claim of globally shortest Euclidean distance.
pub struct ColumnContourLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// group(1): the fallback colour for an unplaceable z. Unread by
    /// `fs_contour`, but the layout is shared with the fill.
    pub style_bg: wgpu::BindGroup,
    /// group(2): pool, grid table, levels, stops, params, level colours.
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 lump charge alive for as long as this layer can draw.
    pub charge: std::sync::Arc<crate::gpu_memory::GpuByteCharge>,
    /// False when the grid has no cell to interpolate across.
    pub drawable: bool,
    /// `None` when the series draws no labels.
    pub label: Option<ColumnContourLabel<'a>>,
}

/// The inline level labels for one contour series.
///
/// One quad per anchor, textured from the CPU-baked atlas. The serial
/// `anchor_select` pass writes the indirect instance count; the CPU neither
/// learns it nor needs to. A host override writes the same records and argument
/// quad, so there is exactly one draw path.
pub struct ColumnContourLabel<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    /// Immutable atlas plus this occurrence's explicit or exact-key automatic
    /// placement resources and their ledger charges.
    pub(crate) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}

/// One columnar bar (histogram) series.
///
/// `edges` and `values` are already role-resolved by the caller from the bar's
/// orientation: `edges` is the bin-bound column, `values` the magnitudes. The
/// shader is told which screen axis the edges run along through
/// `Style.shape_id`.
pub struct ColumnBarLayer<'a> {
    pub envelope: Option<bar_envelope::Snapshot>,
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub pool_buffer: &'a wgpu::Buffer,
    pub edges: ColumnHandle,
    pub values: ColumnHandle,
}

pub struct ColumnScatterLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the constellation planet atlas/ring bind group. `None` otherwise.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
}

pub struct ColumnPickRingLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub quad_vb: &'a wgpu::Buffer,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
    pub instance: u32,
}

/// One selected histogram bin. The instance index is the only geometric
/// selection state: the vertex entry reads the exact edge/value pairs from the
/// same pool columns as the normal bar draw.
pub struct ColumnBarSelectionLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: wgpu::BindGroup,
    pub selection_bg: wgpu::BindGroup,
    pub selection_charge: crate::gpu_memory::SharedCharge,
    pub pool_buffer: &'a wgpu::Buffer,
    pub edges: ColumnHandle,
    pub values: ColumnHandle,
    pub instance: u32,
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

/// One series' data primitives. A panel can hold multiple of these.
pub struct SeriesLayers<'a> {
    /// The field covers the whole grid, so it goes under everything else —
    /// see `issue_series_data` for the full order.
    pub field: Option<ColumnFieldLayer<'a>>,
    /// Bars are area primitives, so they are issued before the stroked ones.
    pub bar: Option<ColumnBarLayer<'a>>,
    /// Contour lines go over the field they describe and under the markers.
    pub contour: Option<ColumnContourLayer<'a>>,
    pub errorbar: Option<ColumnErrorBarDraw<'a>>,
    pub line: Option<ColumnLineLayer<'a>>,
    /// Constellation star pass over the same polyline as `line` — drawn
    /// right after it, before `scatter`. `None` everywhere else.
    pub line_extra: Option<ColumnStarLayer<'a>>,
    pub scatter: Option<ColumnScatterLayer<'a>>,
    pub selected_bars: Vec<ColumnBarSelectionLayer<'a>>,
    pub selected_fields: Vec<ColumnFieldSelectionLayer<'a>>,
    pub picked: Vec<ColumnPickRingLayer<'a>>,
}

/// Constellation star pass — an arc-driven indirect draw. No vertex
/// buffers: the vertex shader walks the arc-length prefix (binary search)
/// and fetches segment endpoints from the pool, both bound as read-only
/// storage in `star_bg`; `indirect` holds the GPU-computed DrawIndirect
/// args (instance count = candidate slots over the total arc).
pub struct ColumnStarLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub texture_bg: &'a wgpu::BindGroup,
    pub star_bg: &'a wgpu::BindGroup,
    pub indirect: &'a wgpu::Buffer,
}

pub struct ColumnErrorBarDraw<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub style_map_bg: Option<&'a wgpu::BindGroup>,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    pub err_y_lo: ColumnHandle,
    pub err_y_hi: ColumnHandle,
    pub err_x_lo: ColumnHandle,
    pub err_x_hi: ColumnHandle,
    pub style_index: Option<ColumnHandle>,
}

/// Clamp a rect into `(0..target.0, 0..target.1)`. Returns `None` if the
/// clamped width or height is zero so callers can skip the draw entirely.
pub(crate) fn clamp_rect_to_target(r: Rect, target: (u32, u32)) -> Option<Rect> {
    let (tw, th) = target;
    let x0 = r.x.min(tw);
    let y0 = r.y.min(th);
    let x1 = r.x.saturating_add(r.width).min(tw);
    let y1 = r.y.saturating_add(r.height).min(th);
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        None
    } else {
        Some(Rect {
            x: x0,
            y: y0,
            width: w,
            height: h,
        })
    }
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
    let x_next = (x_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..x_full.end;
    let y_next = (y_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..y_full.end;
    let x_f32_shift = (x_full.start + 4)..x_full.end;
    pass.set_vertex_buffer(0, l.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, l.pool_buffer.slice(y_full));
    pass.set_vertex_buffer(2, l.pool_buffer.slice(x_next));
    pass.set_vertex_buffer(3, l.pool_buffer.slice(y_next));
    // Arc-length prefix (dash phase / constellation arc); solid precise lines
    // reuse the X column as filler (read by the VS, ignored by the FS).
    match l.arc.as_ref() {
        Some((buf, len_bytes)) => {
            pass.set_vertex_buffer(4, buf.slice(0..*len_bytes));
            pass.set_vertex_buffer(5, buf.slice(4..*len_bytes));
        }
        None => {
            pass.set_vertex_buffer(4, l.pool_buffer.slice(x_full));
            pass.set_vertex_buffer(5, l.pool_buffer.slice(x_f32_shift));
        }
    }
    // Per-instance vertex count decided where the pipeline variant was
    // picked: 4 (precise) / 18 (sketch, constellation ribbon).
    pass.draw(0..l.verts_per_instance, 0..(count - 1));
}

/// Bars drawable from an `(edges, values)` pair.
///
/// `n` bins need `n + 1` edges, so the count is `min(edges - 1, values)` — and
/// a mismatch is **not** an error: the smallest common extent is drawn and the
/// caller reports the truncation. This is also why bars cannot
/// borrow the line draw call, which computes `min(x, y) - 1` and would drop the
/// last bar of a correctly-shaped histogram.
pub fn bar_instance_count(edge_values: usize, value_values: usize) -> u32 {
    edge_values.saturating_sub(1).min(value_values) as u32
}

/// Issue draw calls for one series' data primitives. The caller must have
/// already set the viewport (panel) and scissor (data_area).
pub(crate) fn issue_series_data(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    // The field is the backdrop: it paints every cell of the grid, so bars,
    // contour lines and markers all have to land on top of it.
    if let Some(f) = series.field.as_ref().filter(|f| f.drawable) {
        pass.set_pipeline(f.pipeline);
        pass.set_bind_group(0, f.transform_bg, &[]);
        pass.set_bind_group(1, &f.style_bg, &[]);
        pass.set_bind_group(2, &f.field_bg, &[]);
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    // Bars next: they are filled areas, and the stroked primitives below must
    // land on top of them rather than under.
    if let Some(b) = series.bar.as_ref() {
        let count = bar_instance_count(b.edges.len_values, b.values.len_values);
        if count > 0 {
            pass.set_pipeline(b.pipeline);
            pass.set_bind_group(0, b.transform_bg, &[]);
            pass.set_bind_group(1, b.style_bg, &[]);
            if let Some(map) = b.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            let edges = b.edges.byte_range();
            // The same column one logical value along — instance i then sees
            // `edges[i]` and `edges[i + 1]`.
            let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
            pass.set_vertex_buffer(0, b.pool_buffer.slice(edges.clone()));
            pass.set_vertex_buffer(1, b.pool_buffer.slice(edges_next));
            pass.set_vertex_buffer(2, b.pool_buffer.slice(b.values.byte_range()));
            pass.draw(0..BAR_VERTICES_PER_INSTANCE, 0..count);
            if let Some(envelope) = &b.envelope {
                envelope.draw(pass, b.transform_bg, b.style_bg);
            }
        }
    }

    // Contour lines: over the field they describe, under everything stroked for
    // the data itself. One draw of the same quad the fill uses — `fs_contour`
    // finds every level in one fragment pass, so the cost is the data area's
    // pixels rather than the grid's cells.
    if let Some(c) = series.contour.as_ref().filter(|c| c.drawable) {
        pass.set_pipeline(c.pipeline);
        pass.set_bind_group(0, c.transform_bg, &[]);
        pass.set_bind_group(1, &c.style_bg, &[]);
        pass.set_bind_group(2, &c.field_bg, &[]);
        if let Some(label) = c.label.as_ref() {
            pass.set_bind_group(3, label.snapshot.label_gap_bind_group(), &[]);
        }
        pass.draw(0..FIELD_VERTICES, 0..1);
        // Labels over the lines they name, still inside the data pass — so the
        // `data_area` scissor trims them and the decoration layer (axes, legend,
        // colourbar) composites on top, which is the order a legend box covering
        // a label needs.
        if let Some(l) = c.label.as_ref() {
            pass.set_pipeline(l.pipeline);
            pass.set_bind_group(0, c.transform_bg, &[]);
            pass.set_bind_group(1, l.snapshot.bind_group(), &[]);
            pass.set_vertex_buffer(0, l.snapshot.anchors().slice(..));
            pass.draw_indirect(l.snapshot.indirect(), 0);
        }
    }

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
            if let Some(map) = eb.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            }
            pass.set_vertex_buffer(0, eb.pool_buffer.slice(eb.x.byte_range()));
            pass.set_vertex_buffer(1, eb.pool_buffer.slice(eb.y.byte_range()));
            pass.set_vertex_buffer(2, eb.pool_buffer.slice(eb.err_y_lo.byte_range()));
            pass.set_vertex_buffer(3, eb.pool_buffer.slice(eb.err_y_hi.byte_range()));
            pass.set_vertex_buffer(4, eb.pool_buffer.slice(eb.err_x_lo.byte_range()));
            pass.set_vertex_buffer(5, eb.pool_buffer.slice(eb.err_x_hi.byte_range()));
            if eb.style_map_bg.is_some() {
                let style_index = eb.style_index.as_ref().unwrap_or(&eb.x);
                pass.set_vertex_buffer(6, eb.pool_buffer.slice(style_index.byte_range()));
            }
            // 36 vertices = 6 quads × 2 triangles (see errorbar_columnar.wgsl).
            pass.draw(0..36, 0..count);
        }
    }

    if let Some(l) = series.line.as_ref() {
        draw_line_layer(pass, l);
    }
    if let Some(s) = series.line_extra.as_ref() {
        // Constellation star pass: indirect draw — the instance count was
        // computed on the GPU from the polyline's total arc (line_arc.wgsl
        // star_indirect), so the CPU never sees, nor caps, the star count.
        pass.set_pipeline(s.pipeline);
        pass.set_bind_group(0, s.transform_bg, &[]);
        pass.set_bind_group(1, s.style_bg, &[]);
        pass.set_bind_group(2, s.texture_bg, &[]);
        pass.set_bind_group(3, s.star_bg, &[]);
        pass.draw_indirect(s.indirect, 0);
    }

    if let Some(s) = series.scatter.as_ref() {
        let count = s.x.len_values.min(s.y.len_values) as u32;
        if count > 0 {
            pass.set_pipeline(s.pipeline);
            pass.set_bind_group(0, s.transform_bg, &[]);
            pass.set_bind_group(1, s.style_bg, &[]);
            if let Some(map) = s.style_map_bg {
                pass.set_bind_group(2, map, &[]);
            } else if let Some(tex) = s.texture_bg {
                pass.set_bind_group(2, tex, &[]);
            }
            pass.set_vertex_buffer(0, s.quad_vb.slice(..));
            pass.set_vertex_buffer(1, s.pool_buffer.slice(s.x.byte_range()));
            pass.set_vertex_buffer(2, s.pool_buffer.slice(s.y.byte_range()));
            if s.style_map_bg.is_some() {
                let style_index = s.style_index.as_ref().unwrap_or(&s.x);
                pass.set_vertex_buffer(3, s.pool_buffer.slice(style_index.byte_range()));
            }
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
pub(crate) fn issue_series_picked(pass: &mut wgpu::RenderPass<'_>, series: &SeriesLayers<'_>) {
    for selection in &series.selected_bars {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.style_bg, &[]);
        pass.set_bind_group(2, &selection.selection_bg, &[]);
        let edges = selection.edges.byte_range();
        let edges_next = (edges.start + crate::data::COLUMN_VALUE_BYTES as u64)..edges.end;
        pass.set_vertex_buffer(0, selection.pool_buffer.slice(edges.clone()));
        pass.set_vertex_buffer(1, selection.pool_buffer.slice(edges_next));
        pass.set_vertex_buffer(
            2,
            selection.pool_buffer.slice(selection.values.byte_range()),
        );
        pass.draw(
            0..BAR_SELECTION_VERTICES,
            selection.instance..selection.instance + 1,
        );
    }

    for selection in series.selected_fields.iter().filter(|layer| layer.drawable) {
        pass.set_pipeline(selection.pipeline);
        pass.set_bind_group(0, selection.transform_bg, &[]);
        pass.set_bind_group(1, &selection.selection_bg, &[]);
        pass.set_bind_group(2, &selection.field_bg, &[]);
        pass.draw(0..FIELD_VERTICES, 0..1);
    }

    for p in &series.picked {
        pass.set_pipeline(p.pipeline);
        pass.set_bind_group(0, p.transform_bg, &[]);
        pass.set_bind_group(1, &p.style_bg, &[]);
        if let Some(map) = p.style_map_bg {
            pass.set_bind_group(2, map, &[]);
        }
        pass.set_vertex_buffer(0, p.quad_vb.slice(..));
        pass.set_vertex_buffer(1, p.pool_buffer.slice(p.x.byte_range()));
        pass.set_vertex_buffer(2, p.pool_buffer.slice(p.y.byte_range()));
        if p.style_map_bg.is_some() {
            let style_index = p.style_index.as_ref().unwrap_or(&p.x);
            pass.set_vertex_buffer(3, p.pool_buffer.slice(style_index.byte_range()));
        }
        pass.draw(0..4, p.instance..p.instance + 1);
    }
}

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
    let Some(panel_clamped) = clamp_rect_to_target(panel_rect, target_size) else {
        return;
    };
    let Some(data_clamped) = clamp_rect_to_target(data_area, target_size) else {
        return;
    };

    pass.set_viewport(
        panel_clamped.x as f32,
        panel_clamped.y as f32,
        panel_clamped.width as f32,
        panel_clamped.height as f32,
        0.0,
        1.0,
    );

    // 1) Grid layer (under data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(grid.pipeline);
    pass.set_bind_group(0, grid.bind_group, &[]);
    pass.draw(0..3, 0..1);

    // 2) Data primitives — scissor once to data_area, then issue every series.
    pass.set_scissor_rect(
        data_clamped.x,
        data_clamped.y,
        data_clamped.width,
        data_clamped.height,
    );
    for s in series_list {
        issue_series_data(pass, s);
    }
    for s in series_list {
        issue_series_picked(pass, s);
    }

    // 3) Decoration layer (over data).
    pass.set_scissor_rect(
        panel_clamped.x,
        panel_clamped.y,
        panel_clamped.width,
        panel_clamped.height,
    );
    pass.set_pipeline(decoration.pipeline);
    pass.set_bind_group(0, decoration.bind_group, &[]);
    pass.draw(0..3, 0..1);
}

// Tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contour_lookup_uses_finite_prefixes_numeric_indices_and_block_metadata() {
        let mut levels: Vec<f32> = (0..35).map(|index| 100.0 - index as f32).collect();
        levels[2] = 50.0;
        levels[3] = 50.0;
        levels[4] = f32::NAN;
        levels[30] = f32::NEG_INFINITY;
        levels[32] = f32::NEG_INFINITY;
        levels[33] = f32::INFINITY;
        let ramp = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
        let lookup = build_field_lookup_tables(&ramp, &levels).unwrap();

        assert_eq!(&lookup.stops[..ramp.len()], &ramp);
        assert_eq!(lookup.stops.len(), ramp.len() + levels.len());
        assert_eq!(
            lookup.metadata,
            vec![
                ContourLookupMetadataGpu {
                    finite_count: 30,
                    negative_infinity_count: 1,
                },
                ContourLookupMetadataGpu {
                    finite_count: 1,
                    negative_infinity_count: 1,
                },
            ]
        );

        let search = &lookup.stops[ramp.len()..];
        let first_finite = &search[..lookup.metadata[0].finite_count as usize];
        assert!(first_finite.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
        assert!(search[30..32].iter().all(|record| *record == [0.0; 4]));
        assert_eq!(search[32][0], levels[34]);
        assert_eq!(search[32][1], 34.0);
        assert!(search[33..35].iter().all(|record| *record == [0.0; 4]));

        let duplicate_indices: Vec<u32> = first_finite
            .iter()
            .filter(|record| record[0] == 50.0)
            .map(|record| record[1] as u32)
            .collect();
        assert_eq!(duplicate_indices, vec![2, 3]);
        for record in first_finite {
            let original = record[1] as usize;
            assert_eq!(record[1], original as f32);
            assert_eq!(record[0], levels[original]);
        }
    }

    #[test]
    fn contour_lookup_pads_empty_metadata_and_preserves_31_32_1023_indices() {
        let empty = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        assert_eq!(empty.stops, vec![[0.0; 4]]);
        assert_eq!(empty.metadata, vec![ContourLookupMetadataGpu::default()]);

        let allowed = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY];
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &allowed).unwrap();
        assert_eq!(lookup.stops.len(), 1 + CONTOUR_LEVEL_LOOKUP_CAPACITY);
        assert_eq!(lookup.metadata.len(), CONTOUR_LEVEL_BLOCK_COUNT);
        let search = &lookup.stops[1..];
        for index in [31usize, 32, 1023] {
            let block = index / CONTOUR_LEVEL_BLOCK_SIZE;
            let start = block * CONTOUR_LEVEL_BLOCK_SIZE;
            let end = start + lookup.metadata[block].finite_count as usize;
            let record = search[start..end]
                .iter()
                .find(|record| record[1] == index as f32)
                .unwrap_or_else(|| panic!("numeric original index {index} was not preserved"));
            assert_eq!(record[1].to_bits(), (index as f32).to_bits());
        }

        let rejected = vec![0.0; CONTOUR_LEVEL_LOOKUP_CAPACITY + 1];
        assert!(matches!(
            build_field_lookup_tables(&[[0.0; 4]], &rejected),
            Err(crate::FiggyError::StateAllocationFailed {
                resource: "contour level lookup",
                ..
            })
        ));
    }

    #[test]
    fn contour_lookup_dimensions_match_the_model_and_shader() {
        assert_eq!(CONTOUR_LEVEL_BLOCK_SIZE, 32);
        assert_eq!(CONTOUR_LEVEL_BLOCK_COUNT, 32);
        assert_eq!(
            CONTOUR_LEVEL_LOOKUP_CAPACITY,
            crate::data_config::MAX_CONTOUR_LEVELS
        );

        let shader = include_str!("field_columnar.wgsl");
        assert!(shader.contains("const CONTOUR_LEVEL_BLOCK_SIZE: u32 = 32u;"));
        assert!(shader.contains("let original = firstTrailingBit(candidates);"));
        assert!(shader.contains("let original = u32(record.y) - start;"));
        assert!(shader.contains("contour_lookup_metadata[block].finite_count"));
        assert!(shader.contains("metadata.negative_infinity_count"));
        assert!(shader.contains("let search_lo = s.z_lo - margin;"));
        assert!(shader.contains("let search_hi = s.z_hi + margin;"));
        assert!(shader.contains("contour_lower_bound(start, count, search_lo)"));
        assert!(shader.contains("contour_upper_bound(start, count, search_hi)"));
        assert!(
            !shader.contains("for (var i = 0u; i < field.level_count"),
            "contour fragments must not return to a full level scan"
        );
    }

    #[test]
    fn field_table_exact_charge_includes_zero_level_lookup_metadata_padding() {
        let Some((device, _queue)) = shared_device() else {
            return;
        };
        let ledger = std::sync::Arc::new(crate::gpu_memory::GpuLedger::new());
        let layout = create_field_data_bind_group_layout(&device);
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field table charge test pool"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let lookup = build_field_lookup_tables(&[[0.0; 4]], &[]).unwrap();
        let params = <FieldParamsGpu as bytemuck::Zeroable>::zeroed();
        let (_bind_group, charge) = create_field_data_bind_group(
            &device,
            &ledger,
            &layout,
            &pool,
            FieldTables {
                grid: &[GridColumnGpu { base: 0, len: 0 }],
                levels: &[0.0],
                stops: &lookup.stops,
                level_colors: &[[0.0; 4]],
                lookup_metadata: &lookup.metadata,
                params: &params,
            },
        );

        let expected = std::mem::size_of::<GridColumnGpu>()
            + std::mem::size_of::<f32>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<[f32; 4]>()
            + std::mem::size_of::<ContourLookupMetadataGpu>()
            + std::mem::size_of::<FieldParamsGpu>();
        assert_eq!(expected, 116);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            expected as u64
        );
        drop(charge);
        assert_eq!(
            ledger
                .snapshot()
                .live_bytes_of(crate::gpu_memory::GpuResourceKind::FieldTable),
            0
        );
    }

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
    fn psf_sprite_fades_to_zero_at_square_edges() {
        let psf = bake_psf_rgba(128);
        let px = |x: usize, y: usize, c: usize| psf[(y * 128 + x) * 4 + c];

        for i in 0..128 {
            assert_eq!(px(i, 0, 0), 0, "top edge core leaked at x={i}");
            assert_eq!(px(i, 0, 1), 0, "top edge halo leaked at x={i}");
            assert_eq!(px(i, 127, 0), 0, "bottom edge core leaked at x={i}");
            assert_eq!(px(i, 127, 1), 0, "bottom edge halo leaked at x={i}");
            assert_eq!(px(0, i, 0), 0, "left edge core leaked at y={i}");
            assert_eq!(px(0, i, 1), 0, "left edge halo leaked at y={i}");
            assert_eq!(px(127, i, 0), 0, "right edge core leaked at y={i}");
            assert_eq!(px(127, i, 1), 0, "right edge halo leaked at y={i}");
        }
    }

    #[test]
    fn log_transform_guards_manual_range_without_raising_tiny_positive_min() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
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

    #[test]
    fn scatter_transform_inverted_axes_swap_final_ranges() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: 160,
            height: 120,
        });
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 8.0;
            axis.major_tick_length = 0.0;
        }
        config.bottom_x.scale = crate::config::AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        config.left_y.min = -5.0;
        config.left_y.max = 5.0;

        let normal = scatter_transform_from_config(&config);
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let inverted = scatter_transform_from_config(&config);

        assert_eq!(inverted.scale_log, normal.scale_log);
        assert!((inverted.data_min[0] - normal.data_max[0]).abs() < 1.0e-6);
        assert!((inverted.data_max[0] - normal.data_min[0]).abs() < 1.0e-6);
        assert!((inverted.data_min[1] - normal.data_max[1]).abs() < 1.0e-6);
        assert!((inverted.data_max[1] - normal.data_min[1]).abs() < 1.0e-6);
        for v in inverted.data_min.into_iter().chain(inverted.data_max) {
            assert!(v.is_finite(), "{inverted:?}");
        }
    }

    #[test]
    fn gpu_transform_uses_auto_fit_ssot_bounds_bit_for_bit() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 17,
            y: 29,
            width: 900,
            height: 600,
        });
        let x_extent = crate::chart::FitExtent {
            min: 1_700_000_000_000.125,
            max: 1_700_000_000_009.875,
            min_positive: Some(1_700_000_000_000.125),
        };
        let y_extent = crate::chart::FitExtent {
            min: -3.25,
            max: 8.75,
            min_positive: Some(0.125),
        };
        crate::chart::apply_auto_fit_all(&mut config, &x_extent, &y_extent, 0.05);

        let transform = scatter_transform_from_config(&config);
        let x_min = crate::data::split_f64_to_f32_pair(config.bottom_x.min);
        let x_max = crate::data::split_f64_to_f32_pair(config.bottom_x.max);
        let y_min = crate::data::split_f64_to_f32_pair(config.left_y.min);
        let y_max = crate::data::split_f64_to_f32_pair(config.left_y.max);
        assert_eq!(
            [transform.data_min[0], transform.data_min_lo[0]],
            [x_min.0, x_min.1]
        );
        assert_eq!(
            [transform.data_max[0], transform.data_max_lo[0]],
            [x_max.0, x_max.1]
        );
        assert_eq!(
            [transform.data_min[1], transform.data_min_lo[1]],
            [y_min.0, y_min.1]
        );
        assert_eq!(
            [transform.data_max[1], transform.data_max_lo[1]],
            [y_max.0, y_max.1]
        );

        // Layout changes only the panel affine. They can no longer manufacture
        // a second, margin-extended axis range behind Config's back.
        let mut different_layout = config.clone();
        different_layout.bottom_x.out_margin += 40.0;
        different_layout.left_y.out_margin += 30.0;
        let moved = scatter_transform_from_config(&different_layout);
        assert_eq!(moved.data_min, transform.data_min);
        assert_eq!(moved.data_max, transform.data_max);
        assert_eq!(moved.data_min_lo, transform.data_min_lo);
        assert_eq!(moved.data_max_lo, transform.data_max_lo);
        assert_ne!(moved.data_to_panel_offset, transform.data_to_panel_offset);
        assert_ne!(moved.data_to_panel_scale, transform.data_to_panel_scale);
    }

    /// Smoke-test the texture-upload API path (no readback). Validation
    /// would surface here if the call shape were wrong.
    #[test]
    fn rgba_texture_upload_roundtrips_api() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping texture upload test");
            return;
        };

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
        )
        .expect("upload texture");

        // Wait for the queued write_texture to complete; validation errors
        // surface during this poll.
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });

        assert_eq!(tex.width(), 2);
        assert_eq!(tex.height(), 2);
        assert_eq!(tex.format(), wgpu::TextureFormat::Rgba8Unorm);
    }

    /// Compile the WGSL and build the fullscreen textured pipeline. Shader
    /// or layout mismatches would panic during creation.
    #[test]
    fn fullscreen_textured_pipeline_compiles_and_creates() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter — skipping pipeline test");
            return;
        };

        let bgl = create_texture_bind_group_layout(&device);
        let pipeline = create_fullscreen_textured_pipeline(
            &device,
            &bgl,
            // Same non-sRGB format the surface picks at runtime.
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = pipeline;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    #[test]
    fn streamed_point_style_map_reuses_rows_and_charges_only_meta() {
        let (device, _queue) = shared_device().expect("streamed point style map GPU adapter");
        let layout = create_scatter_style_map_bind_group_layout(&device);
        let map = create_scatter_style_map(
            &device,
            &layout,
            &[ScatterStyleSlotGpu {
                color_premul: [1.0, 0.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            &[ScatterStyleOverrideGpu {
                point_index: 17,
                _pad: [0; 3],
                color_premul: [0.0, 1.0, 0.0, 1.0],
                meta: [3.0, 0.0, 1.0, 0.0],
            }],
            ScatterStyleMapMeta {
                style_count: 1,
                override_count: 1,
                has_index: 1,
                _pad: 0,
            },
        );
        assert_eq!(map.meta._pad, 0);
        let tally = crate::gpu_memory::ChargeTally::new();
        let streamed = map.stream_bind_group(&device, &layout, 17, &tally);
        assert_eq!(tally.bytes(), 16);
        drop(map);
        let _ = streamed;
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Compile the scatter WGSL entries added outside the common block for
    /// per-point style mapping and picked-point decoration. This catches WGSL
    /// syntax/layout drift without running a full render snapshot.
    #[test]
    fn scatter_mapped_and_pick_ring_pipelines_compile() {
        let Some((device, _queue)) = shared_device() else {
            println!("no adapter ??skipping scatter entry pipeline test");
            return;
        };
        let transform_bgl = create_scatter_transform_bind_group_layout(&device);
        let style_bgl = create_style_bind_group_layout(&device);
        let style_map_bgl = create_scatter_style_map_bind_group_layout(&device);
        let shaders = ShaderModules::new(&device);

        let mapped = create_scatter_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let picked = create_scatter_columnar_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring",
            "fs_pick_ring",
            "figgy picked point ring pipeline test",
        );
        let picked_mapped = create_scatter_columnar_mapped_pipeline_with_entries(
            &device,
            &shaders.scatter,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
            "vs_pick_ring_mapped",
            "fs_pick_ring",
            "figgy picked point mapped ring pipeline test",
        );
        let errorbar_mapped = create_errorbar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
            1,
        );
        let bar_mapped = create_bar_columnar_mapped_pipeline(
            &device,
            &transform_bgl,
            &style_bgl,
            &style_map_bgl,
            wgpu::TextureFormat::Bgra8Unorm,
        );

        let _ = (mapped, picked, picked_mapped, errorbar_mapped, bar_mapped);
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
    }

    /// Wire up sampler + bind-group layout + bind group end-to-end; a
    /// slot-type mismatch would panic in `create_bind_group`.
    #[test]
    fn texture_sampler_bind_group_wires_up() {
        let Some((device, queue)) = shared_device() else {
            println!("no adapter — skipping bind group test");
            return;
        };

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
        )
        .expect("upload texture");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = create_linear_sampler(&device);
        let layout = create_texture_bind_group_layout(&device);
        let _bind_group = create_texture_bind_group(&device, &layout, &view, &sampler);

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        });
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
                println!(
                    "  max_texture_dim_2d     : {}",
                    limits.max_texture_dimension_2d
                );
                println!("  max_buffer_size        : {}", limits.max_buffer_size);
                println!("  max_bind_groups        : {}", limits.max_bind_groups);
            }
            Err(e) => panic!("request_device failed on available adapter: {e}"),
        }
    }
}
