//! GPU resource checks, error scopes, device health and the poll thread.

use super::*;

pub(in crate::renderer) fn validate_texture_extent(
    caps: RendererDeviceCaps,
    resource: &'static str,
    width: u32,
    height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(FiggyError::InvalidChartArea { width, height });
    }
    let max_dim = width.max(height);
    if max_dim > caps.max_texture_dimension_2d {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: max_dim as u64,
            limit: caps.max_texture_dimension_2d as u64,
        });
    }
    Ok(())
}

pub(in crate::renderer) fn validate_buffer_size(
    caps: RendererDeviceCaps,
    resource: &'static str,
    size: u64,
) -> Result<()> {
    if size > caps.max_buffer_size {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: size,
            limit: caps.max_buffer_size,
        });
    }
    Ok(())
}

pub(crate) fn create_texture_checked(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Texture> {
    // gpu-alloc: caller
    gpu_checked(device, resource, || device.create_texture(desc))
}

pub(crate) fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer> {
    // gpu-alloc: caller
    gpu_checked(device, resource, || device.create_buffer(desc))
}

// ---------------------------------------------------------------------------
// GPU errors as values (#152)
// ---------------------------------------------------------------------------

/// Run `work` inside wgpu error scopes for out-of-memory, validation and
/// internal errors, and return what they captured as an error.
///
/// Native wgpu resolves a popped scope at once (`wgpu_core` returns a ready
/// future), so a synchronous call learns about its own failure before it
/// returns. A browser scope resolves asynchronously, so there the error
/// reaches the device's uncaptured-error handler instead, which
/// [`DeviceHealth`] records and the next [`DeviceHealth::check`] returns.
pub(crate) fn gpu_checked<T>(
    device: &wgpu::Device,
    resource: &'static str,
    work: impl FnOnce() -> T,
) -> Result<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let value = work();
        // Popped in reverse push order, every one, before any is inspected.
        let internal = resolve_ready(internal.pop());
        let validation = resolve_ready(validation.pop());
        let out_of_memory = resolve_ready(out_of_memory.pop());
        for captured in [out_of_memory, validation, internal] {
            match captured {
                ScopeResult::Clear => {}
                ScopeResult::Captured(error) => return Err(gpu_error(resource, error)),
                ScopeResult::Unresolved => {
                    return Err(FiggyError::internal(
                        "a native wgpu error scope did not resolve when popped",
                    ));
                }
            }
        }
        Ok(value)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _scoped_by_the_uncaptured_handler = (device, resource);
        Ok(work())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) enum ScopeResult {
    Clear,
    Captured(wgpu::Error),
    Unresolved,
}

/// Poll a popped native error scope once. `wgpu_core` hands back a ready
/// future; anything else is reported, never waited on.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn resolve_ready(
    popped: impl std::future::Future<Output = Option<wgpu::Error>>,
) -> ScopeResult {
    let mut popped = std::pin::pin!(popped);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match popped.as_mut().poll(&mut context) {
        std::task::Poll::Ready(None) => ScopeResult::Clear,
        std::task::Poll::Ready(Some(error)) => ScopeResult::Captured(error),
        std::task::Poll::Pending => ScopeResult::Unresolved,
    }
}

/// One wgpu error as a figgy error: out of memory is an allocation failure,
/// validation and internal errors are [`FiggyError::GpuValidation`].
/// Native only: browser errors reach [`DeviceHealth`] as text.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn gpu_error(resource: &'static str, error: wgpu::Error) -> FiggyError {
    match error {
        wgpu::Error::OutOfMemory { .. } => FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: error.to_string(),
        },
        other => FiggyError::GpuValidation {
            resource,
            reason: other.to_string(),
        },
    }
}

/// The health of one wgpu device: whether it was lost, and how many errors
/// reached its uncaptured-error handler; on native also the bundle's poll
/// thread, when it has one.
///
/// [`RendererDevice::new`] registers the device-lost callback and the
/// uncaptured-error handler once; every renderer and engine built from that
/// bundle shares the record. wgpu keeps one callback and one handler per
/// device, so a host that registers its own after building the bundle
/// replaces figgy's, and figgy then learns of a loss only from the failures
/// it causes.
///
/// Every clone owns the poll thread (`gpu_poller.rs`): it lives as long as
/// some renderer, engine or ticket of the bundle holds the record, or a wait
/// is pending.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeviceHealth {
    state: Arc<DeviceHealthState>,
    /// `None` for a `RendererDevice::blocking` bundle and a record not
    /// built from a bundle.
    #[cfg(not(target_arch = "wasm32"))]
    poller: Option<crate::gpu_poller::Poller>,
}

#[derive(Debug, Default)]
pub(super) struct DeviceHealthState {
    lost: std::sync::OnceLock<String>,
    errors: std::sync::atomic::AtomicU64,
    reported: std::sync::atomic::AtomicU64,
    first_error: std::sync::OnceLock<String>,
}

impl DeviceHealth {
    pub(crate) fn watch(device: &wgpu::Device) -> Self {
        let health = Self::default();
        let lost = Arc::clone(&health.state);
        device.set_device_lost_callback(move |reason, message| {
            let text = format!("{reason:?}: {message}");
            if lost.lost.set(text).is_err() {
                // A second callback for the same device: the first reason
                // already stands and is what every later call returns.
            }
        });
        let errors = Arc::clone(&health.state);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            errors
                .errors
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if errors.first_error.set(error.to_string()).is_err() {
                // Not the first: the count above reports it, and the first
                // error's text stays the one shown.
            }
        }));
        health
    }

    /// `Err` once the device is lost, or when GPU errors reached the
    /// uncaptured-error handler since the last check.
    pub(crate) fn check(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        if let Some(reason) = self.state.lost.get() {
            return Err(FiggyError::DeviceLost {
                reason: reason.clone(),
            });
        }
        let errors = self.state.errors.load(Ordering::Acquire);
        let reported = self.state.reported.fetch_max(errors, Ordering::AcqRel);
        if errors > reported {
            let first = self
                .state
                .first_error
                .get()
                .map(String::as_str)
                .unwrap_or("no message recorded");
            return Err(FiggyError::GpuValidation {
                resource: "figgy GPU work",
                reason: format!(
                    "{} GPU error(s) reached the uncaptured-error handler; the first: {first}",
                    errors - reported
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn is_lost(&self) -> bool {
        self.state.lost.get().is_some()
    }

    /// Start the poll thread over `device`, the device this record watches.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn start_poller(&mut self, device: &wgpu::Device) -> Result<()> {
        let drive = DevicePoll {
            device: device.clone(),
            state: Arc::clone(&self.state),
        };
        let poller = crate::gpu_poller::Poller::start(drive).map_err(|error| {
            FiggyError::PollThreadStartFailed {
                reason: error.0.to_string(),
            }
        })?;
        self.poller = Some(poller);
        Ok(())
    }

    /// A record with a poll thread over `device` and no callbacks: for the
    /// tests that build engines without a bundle, so their asynchronous
    /// waits complete.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) fn polled_for_test(device: &wgpu::Device) -> Self {
        let mut health = Self::default();
        health.start_poller(device).expect("figgy poll thread");
        health
    }

    /// The bundle's poll thread; `None` without one.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn poller(&self) -> Option<&crate::gpu_poller::Poller> {
        self.poller.as_ref()
    }
}

/// What the poll thread polls: the device, and the health state it reads
/// without the poller, so the thread does not own itself.
///
/// The thread reads only whether the device is lost. It never takes the
/// error count that [`DeviceHealth::check`] reports: those errors belong to
/// the host call whose work caused them, not to the background poll.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct DevicePoll {
    device: wgpu::Device,
    state: Arc<DeviceHealthState>,
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::gpu_poller::Drive for DevicePoll {
    fn wait(&self) -> bool {
        if self.state.lost.get().is_some() {
            return false;
        }
        // As in `gpu_poll`: wgpu 30 panics in a poll on a lost device.
        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
        }));
        matches!(polled, Ok(Ok(_)))
    }
}

/// `device.poll` that never runs on a lost device and returns every failure.
///
/// wgpu 30 turns a poll error on a lost device into a panic
/// (`handle_error_fatal`), and the device-lost callback that tells figgy
/// about the loss runs inside that same poll. On native the poll therefore
/// runs under `catch_unwind`: this is the one place where wgpu offers no
/// other signal, and the caught panic becomes [`FiggyError::DeviceLost`].
pub(crate) fn gpu_poll(
    health: &DeviceHealth,
    device: &wgpu::Device,
    poll: wgpu::PollType,
) -> Result<wgpu::PollStatus> {
    health.check()?;
    #[cfg(not(target_arch = "wasm32"))]
    let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.poll(poll)))
        .map_err(|_| FiggyError::DeviceLost {
            reason: "wgpu Device::poll failed fatally (device lost during the poll)".into(),
        });
    #[cfg(target_arch = "wasm32")]
    let polled: Result<_> = Ok(device.poll(poll));
    let status = polled?.map_err(|error| FiggyError::GpuValidation {
        resource: "device poll",
        reason: error.to_string(),
    })?;
    health.check()?;
    Ok(status)
}

/// Allocation context for one column-pool call: the device, the queue and
/// the bundle's ledger, whose budget and running total the pool checks at the
/// allocation itself (#160).
///
/// A free function over the fields rather than a `&self` method, so the
/// caller's `&mut self.pool` borrow stays disjoint from the device, queue and
/// ledger borrows.
pub(in crate::renderer) fn pool_alloc_ctx<'a>(
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    ledger: &'a Arc<GpuLedger>,
) -> data_render::GpuAllocCtx<'a> {
    data_render::GpuAllocCtx::new(device, queue, ledger)
}

/// Hand every retirement the ledger holds to the queue's completion: the
/// resources are destroyed and their bytes credited once the work submitted
/// before this call completes. One path for every renderer on a device
/// bundle (#160).
pub(crate) fn retire_on_completion(ledger: &Arc<GpuLedger>, queue: &wgpu::Queue) {
    let batch = ledger.take_retirement();
    if batch.is_empty() {
        return;
    }
    let ledger = Arc::clone(ledger);
    #[cfg(not(target_arch = "wasm32"))]
    queue.on_submitted_work_done(move || ledger.complete_retirement(batch));
    #[cfg(target_arch = "wasm32")]
    {
        // wgpu requires a Send completion callback even for WebGPU, but
        // browser buffer handles must stay on their owning JS thread.
        let (sender, receiver) = futures_channel::oneshot::channel();
        queue.on_submitted_work_done(move || {
            if sender.send(()).is_err() {
                // The receiver was dropped: the waiting task was cancelled.
            }
        });
        wasm_bindgen_futures::spawn_local(async move {
            if receiver.await.is_ok() {
                ledger.complete_retirement(batch);
            }
        });
    }
}
