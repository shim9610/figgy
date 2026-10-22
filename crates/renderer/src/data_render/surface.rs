//! GPU bootstrap: instance, adapter and device requests, surface
//! configuration, frame acquisition and the clear pass.

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
/// storm while keeping test parallelism. Initialization failure panics so a
/// missing adapter or rejected device cannot turn a GPU test into a false pass.
/// The Option return is retained for existing test fixture call sites.
#[cfg(test)]
pub(crate) fn shared_device() -> Option<(std::sync::Arc<wgpu::Device>, std::sync::Arc<wgpu::Queue>)>
{
    use std::sync::{Arc, OnceLock};
    static SHARED: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let (device, queue) = crate::test_gpu::device();
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
pub(super) fn target_format_is_supported(
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

pub(super) fn is_rgba8_surface_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

pub(super) fn choose_surface_format(
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

pub(super) fn choose_present_mode(caps: &wgpu::SurfaceCapabilities) -> Option<wgpu::PresentMode> {
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

pub(super) fn choose_alpha_mode(
    caps: &wgpu::SurfaceCapabilities,
) -> Option<wgpu::CompositeAlphaMode> {
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

pub(super) fn acquire_surface_frame(
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
