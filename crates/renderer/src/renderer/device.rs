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
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut bundle = Self::without_poller(device, queue);
        #[cfg(not(target_arch = "wasm32"))]
        bundle.health.start_poller(&bundle.device)?;
        Ok(bundle)
    }

    /// Bundle a device and queue for blocking calls only, without a poll
    /// thread: every `_blocking` call polls the device on the caller's
    /// thread. An asynchronous GPU wait on this bundle fails with
    /// [`FiggyError::AsyncWaitUnavailable`] instead of never completing.
    /// Registers the same callbacks as [`Self::new`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn blocking(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::without_poller(device, queue)
    }

    fn without_poller(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let health = DeviceHealth::watch(&device);
        Self {
            device,
            queue,
            health,
            // host-alloc: W1-a
            ledger: Arc::new(GpuLedger::new()),
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
