//! The GPU device behind a renderer: adapter and device creation, device
//! capabilities, the render target and its MSAA resolve, the per-target
//! pipelines, pipeline prewarming, GPU completion servicing and the browser
//! error scopes used on WebGPU.

use super::*;

#[cfg(target_arch = "wasm32")]
mod browser;
mod create;
mod health;
mod init;
mod pipeline_kinds;
mod precise_variants;
mod prewarm;
mod renderer_prewarm;
mod target;
mod target_pipelines;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::*;
pub(super) use create::*;
pub(crate) use health::*;
pub(super) use pipeline_kinds::*;
#[cfg(target_arch = "wasm32")]
pub(super) use precise_variants::series_pipeline_labels;
pub(super) use prewarm::*;
pub(super) use target::*;
pub(super) use target_pipelines::*;

/// A wgpu device/queue pair used by figgy.
///
/// wgpu does not expose a public parent-device id on `Queue`, so figgy cannot
/// prove at runtime that arbitrary handles came from the same logical device.
/// Passing them as one value keeps the pair together throughout the renderer
/// API and avoids accepting loose device/queue arguments at every call site.
#[derive(Clone)]
pub struct RendererDevice {
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    /// Device-lost and uncaptured-error record, registered once here and
    /// shared by every renderer built from this bundle (#152).
    pub(super) health: DeviceHealth,
    /// The GPU byte ledger and memory budget, created once here and shared
    /// by every renderer built from this bundle (#160).
    pub(super) ledger: Arc<GpuLedger>,
    /// The shader set a renderer built from this bundle starts on
    /// ([`Self::with_shader_set`], #299).
    pub(super) shader_set: crate::shader_assembly::ShaderSet,
    /// The pipeline cache every renderer built from this bundle creates its
    /// pipelines through (#478): empty when the bundle is made on a device
    /// with `Features::PIPELINE_CACHE`, a host's saved one after
    /// [`Self::with_pipeline_cache`], none without the feature or after
    /// [`Self::without_pipeline_cache`]. A cloneable handle, shared, not a
    /// copy.
    pub(super) pipeline_cache: Option<wgpu::PipelineCache>,
}

impl RendererDevice {
    /// Bundle a wgpu device and queue that came from the same `request_device`
    /// result or host render state, for blocking and asynchronous calls.
    ///
    /// Registers the device's device-lost callback and uncaptured-error
    /// handler, so figgy reports a lost device and every GPU error as a
    /// [`FiggyError`] instead of wgpu's default panic. wgpu keeps one of each
    /// per device: build one bundle per device and clone it.
    ///
    /// On native this also starts the bundle's poll thread, which completes
    /// asynchronous GPU waits (readbacks, exports, `wait_submitted_work`):
    /// wgpu completes a wait only inside a device poll, and nothing else
    /// polls a native device. The thread parks while no wait is pending and
    /// exits once every clone of the bundle and every pending wait is gone.
    /// Fails with [`FiggyError::PollThreadStartFailed`] when the OS cannot
    /// start it. A host that only makes blocking calls builds the bundle with
    /// `Self::blocking` instead, which starts no thread. In the browser the
    /// device polls itself and no thread is started.
    ///
    /// The device stays the host's: figgy never destroys it, also not when
    /// the last renderer built from it is dropped.
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        Self::polled(device, queue, DeviceOrigin::SuppliedByHost)
    }

    /// Request a device and queue from `adapter` with figgy's device
    /// descriptor ([`crate::gpu::request_device_async`]) and bundle them as
    /// [`Self::new`] does, with the poll thread on native.
    ///
    /// figgy owns this device. In the browser, when the last holder of the
    /// bundle is dropped (the last renderer, clone or pending ticket built
    /// from it), figgy destroys the device after the work submitted on it,
    /// so the page never relies on garbage collection to end it (#322). A
    /// host that keeps the raw `wgpu::Device` or `wgpu::Queue` from
    /// [`Self::device`] past that point keeps a clone of this bundle as well.
    /// Native drops never destroy a device. Fails with
    /// [`FiggyError::DeviceCreationFailed`] when the request fails.
    pub async fn request_async(adapter: &wgpu::Adapter) -> Result<Self> {
        let (device, queue) =
            data_render::request_device_async(adapter)
                .await
                .map_err(|error| FiggyError::DeviceCreationFailed {
                    reason: format!("{error}"),
                })?;
        Self::requested(device, queue)
    }

    /// Bundle a device and queue figgy requested itself: [`Self::new`],
    /// with the record saying figgy owns the device (#322).
    pub(crate) fn requested(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        Self::polled(device, queue, DeviceOrigin::RequestedByFiggy)
    }

    fn polled(device: wgpu::Device, queue: wgpu::Queue, origin: DeviceOrigin) -> Result<Self> {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut bundle = Self::without_poller(device, queue, origin);
        #[cfg(not(target_arch = "wasm32"))]
        bundle.health.start_poller(&bundle.device)?;
        Ok(bundle)
    }

    /// Bundle a device and queue for blocking calls only, without a poll
    /// thread: every `_blocking` call polls the device on the caller's
    /// thread. An asynchronous GPU wait on this bundle fails with
    /// [`FiggyError::AsyncWaitUnavailable`] instead of never completing.
    /// Registers the same callbacks as [`Self::new`], and the device stays
    /// the host's.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn blocking(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::without_poller(device, queue, DeviceOrigin::SuppliedByHost)
    }

    /// Have every renderer built from this bundle start on `set` (#299).
    ///
    /// A renderer starts on [`ShaderSet::Basic`](crate::shader_assembly::ShaderSet::Basic)
    /// and moves to the full set when it is first handed a chart that needs
    /// it, or when the host asks ([`Renderer::require_shader_set`]). A host
    /// that knows its charts use an axis break, a probability axis or polar
    /// coordinates passes `ShaderSet::Full` here, so the renderer never
    /// creates or compiles a module of the basic set.
    pub fn with_shader_set(mut self, set: crate::shader_assembly::ShaderSet) -> Self {
        self.shader_set = set;
        self
    }

    /// Create every pipeline of the renderers built from this bundle through
    /// a pipeline cache loaded from `data` (#478): the bytes an earlier run's
    /// [`Self::pipeline_cache_data`] returned on the same adapter and driver
    /// ([`Self::pipeline_cache_key`] names them), or `None` for an empty
    /// cache. Data wgpu cannot use (another adapter, another driver or wgpu
    /// version, truncated or not a cache) is discarded and the cache starts
    /// empty; the pipelines are the same either way, only their creation is
    /// faster with a matching cache.
    ///
    /// Without `Features::PIPELINE_CACHE` on the device this returns the
    /// bundle unchanged, with no cache. figgy's own device requests
    /// (`request_device_async`, [`Self::request_async`] and the window
    /// constructors) ask for the feature whenever the adapter has it; a host
    /// device needs it in its own descriptor. wgpu implements it on Vulkan
    /// only, so DX12, Metal and the browser keep their drivers' and
    /// browser's own caches. Call it before building a renderer from the
    /// bundle: a renderer keeps the cache it was built with. Streaming
    /// pipelines do not use the cache.
    pub fn with_pipeline_cache(mut self, data: Option<&[u8]>) -> Self {
        self.pipeline_cache = create_pipeline_cache(&self.device, data);
        self
    }

    /// Create the pipelines with no pipeline cache, as before #478. A bundle
    /// on a device with `Features::PIPELINE_CACHE` starts with an empty one.
    pub fn without_pipeline_cache(mut self) -> Self {
        self.pipeline_cache = None;
        self
    }

    /// The pipeline cache's contents, for a host to save and hand back to
    /// [`Self::with_pipeline_cache`] in a later run, under
    /// [`Self::pipeline_cache_key`]. `None` without a cache or when the
    /// backend reports nothing. The cache grows as pipelines are created, so
    /// save it once the charts have drawn (after a first frame, on exit).
    /// Write it to a temporary file and rename it over the old one, so a
    /// crash never leaves half a cache.
    pub fn pipeline_cache_data(&self) -> Option<Vec<u8>> {
        self.pipeline_cache.as_ref()?.get_data()
    }

    /// The name to save an adapter's pipeline cache under, so a host keeps
    /// one per adapter and driver (`wgpu::util::pipeline_cache_key`). `None`
    /// where the backend has no pipeline cache.
    pub fn pipeline_cache_key(adapter_info: &wgpu::AdapterInfo) -> Option<String> {
        wgpu::util::pipeline_cache_key(adapter_info)
    }

    /// The bundle's pipeline cache, for the renderers built from it.
    pub(crate) fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.pipeline_cache.as_ref()
    }

    /// Whether the bundle creates its renderers' pipelines through a
    /// pipeline cache.
    #[cfg(test)]
    pub(crate) fn has_pipeline_cache(&self) -> bool {
        self.pipeline_cache.is_some()
    }

    fn without_poller(device: wgpu::Device, queue: wgpu::Queue, origin: DeviceOrigin) -> Self {
        let health = DeviceHealth::watch(&device, origin);
        let pipeline_cache = create_pipeline_cache(&device, None);
        Self {
            device,
            queue,
            health,
            // host-alloc: W1-a
            ledger: Arc::new(GpuLedger::new()),
            shader_set: crate::shader_assembly::ShaderSet::Basic,
            pipeline_cache,
        }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
    /// The device's health record, shared by every user of this bundle.
    /// Read by the native polls; the browser's device polls itself.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn health(&self) -> &DeviceHealth {
        &self.health
    }

    /// The bundle's GPU byte ledger and memory budget, shared by every
    /// renderer built from this bundle. A standalone [`crate::ColumnPool`]
    /// charges it through [`crate::GpuAllocCtx::new`].
    pub fn ledger(&self) -> &Arc<GpuLedger> {
        &self.ledger
    }
}

/// A pipeline cache on `device` from `data`, or none without
/// `Features::PIPELINE_CACHE` (#478).
fn create_pipeline_cache(
    device: &wgpu::Device,
    data: Option<&[u8]>,
) -> Option<wgpu::PipelineCache> {
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return None;
    }
    // SAFETY: wgpu requires `data` to be what `PipelineCache::get_data`
    // returned for a compatible adapter. The only bytes figgy passes are a
    // host's from `RendererDevice::pipeline_cache_data`, documented as such,
    // and `fallback: true` has wgpu check the data's header against this
    // adapter and its driver and start an empty cache on a mismatch.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("figgy pipeline cache"),
            data,
            fallback: true,
        })
    };
    Some(cache)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RendererDeviceCaps {
    pub(super) features: wgpu::Features,
    pub(super) max_texture_dimension_2d: u32,
    pub(super) max_buffer_size: u64,
    /// The arc-length scan and star pass bind the WHOLE pool as a single
    /// storage-buffer binding, so the pool can never exceed this. A request
    /// above it is clamped at construction (see [`effective_pool_capacity`])
    /// rather than deferred into a wgpu validation panic on the first
    /// dashed/sketch/milkyway line.
    pub(super) max_storage_buffer_binding_size: u64,
    /// Per-dimension dispatch ceiling for the arc-length compute scan.
    pub(super) max_compute_workgroups_per_dimension: u32,
    /// The arc scan needs 256-wide workgroups; downlevel (GL-class)
    /// adapters report smaller/zero compute limits.
    pub(super) max_compute_invocations_per_workgroup: u32,
}

impl RendererDeviceCaps {
    pub(super) fn from_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            features: device.features(),
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
            max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        }
    }
}

/// The largest pool the renderer can actually bind, given the device.
///
/// The arc-scan compute and the Milkyway star pass bind the entire pool
/// buffer as one storage binding, so the pool is capped by BOTH
/// `max_storage_buffer_binding_size` and `max_buffer_size`. `try_new` clamps
/// the caller's requested capacity to this instead of letting an oversized
/// pool become a runtime validation panic — a caller that needs a bigger
/// pool must request higher device limits when creating the device (wgpu's
/// defaults are only 128 MiB / 256 MiB). Callers can read the granted size
/// back via `renderer.pool().capacity()`.
///
/// This clamp is a convenience for the caller, not the enforcement point. The
/// pool applies the same combined ceiling at every buffer it creates, reading
/// the device's limits there rather than trusting a number captured once —
/// see `column_pool::buffer_ceiling`.
pub(super) fn effective_pool_capacity(requested: u64, caps: RendererDeviceCaps) -> u64 {
    requested
        .min(caps.max_storage_buffer_binding_size)
        .min(caps.max_buffer_size)
}

/// The next renderer identity: an issued number from one process-wide counter,
/// incremented with a check and never wrapped, so no two renderers share one.
/// `ChartId` and `RenderRevision` carry it. See "Identity and staleness
/// checks" in `ARCHITECTURE.md`.
pub(super) fn issue_renderer_identity() -> Result<u64> {
    static NEXT_RENDERER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT_RENDERER_ID
        .try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| current.checked_add(1),
        )
        .map(|previous| previous + 1)
        .map_err(|_| FiggyError::CounterExhausted {
            counter: "renderer identity",
        })
}

pub(super) fn validate_target_format(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
) -> Result<()> {
    if !caps.features.contains(format.required_features()) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format requires device features that were not enabled".into(),
        });
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format is not guaranteed to support RENDER_ATTACHMENT".into(),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "figgy pipelines require alpha blending into the target".into(),
        });
    }
    Ok(())
}

pub(super) fn validate_target_sample_count(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<()> {
    validate_target_format(caps, format)?;
    if sample_count == 1 {
        return Ok(());
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features.flags.sample_count_supported(sample_count) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: format!("format does not support {sample_count}x MSAA"),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format does not support MSAA resolve targets".into(),
        });
    }
    Ok(())
}

pub(super) fn preferred_msaa_sample_count(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
) -> u32 {
    let features = format.guaranteed_format_features(caps.features);
    let can_resolve = features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    if can_resolve && features.flags.sample_count_supported(4) {
        4
    } else if can_resolve && features.flags.sample_count_supported(2) {
        2
    } else {
        1
    }
}
