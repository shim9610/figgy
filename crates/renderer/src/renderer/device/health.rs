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
///
/// The record also keeps who requested the device ([`DeviceOrigin`]), and
/// its strong count tells the browser drop whether it is the device's last
/// holder (`release_after_submitted_work`, #322).
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
    /// Who requested the device, set once when the bundle is built and
    /// never changed (#322). Only the browser drop reads it: a native drop
    /// never destroys a device.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    origin: DeviceOrigin,
}

/// Who requested a bundle's device: the fact that decides whether figgy may
/// destroy it when the last holder of the bundle is dropped (#322).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DeviceOrigin {
    /// figgy requested the device itself (a windowed or headless renderer,
    /// a browser chart's own device), so no host code holds it.
    RequestedByFiggy,
    /// The host built the bundle from its own device and queue
    /// ([`RendererDevice::new`], `RendererDevice::blocking`), or the record
    /// was not built from a bundle. figgy never destroys such a device.
    #[default]
    SuppliedByHost,
}

impl DeviceHealth {
    /// The record of `device`, requested by `origin`: registers the
    /// device-lost callback and the uncaptured-error handler.
    ///
    /// Both callbacks hold the record weakly, so in the browser the record's
    /// strong count is the number of `DeviceHealth` values that hold it:
    /// every bundle clone, renderer, engine and ticket of the device
    /// (`Self::is_last_holder_of_requested_device`). A callback that runs
    /// after every holder is gone has no one left to tell.
    pub(crate) fn watch(device: &wgpu::Device, origin: DeviceOrigin) -> Self {
        let health = Self {
            state: Arc::new(DeviceHealthState {
                origin,
                ..DeviceHealthState::default()
            }),
            #[cfg(not(target_arch = "wasm32"))]
            poller: None,
        };
        let lost = Arc::downgrade(&health.state);
        device.set_device_lost_callback(move |reason, message| {
            let Some(lost) = lost.upgrade() else {
                // Every holder of the record is gone: no one reads the loss.
                return;
            };
            let text = format!("{reason:?}: {message}");
            if lost.lost.set(text).is_err() {
                // A second callback for the same device: the first reason
                // already stands and is what every later call returns.
            }
        });
        let errors = Arc::downgrade(&health.state);
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            let Some(errors) = errors.upgrade() else {
                // Every holder of the record is gone: no call reports it.
                return;
            };
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

    /// Whether this value is the only holder of the record of a device
    /// figgy requested itself, so destroying the device can break no one
    /// (#322).
    ///
    /// The last-owner test is the shared-storage pattern of "Identity and
    /// staleness checks" (`ARCHITECTURE.md`): the strong count of the one
    /// record every holder of the device's bundle shares. Every bundle
    /// clone, renderer, engine, preparation job and pick or export ticket
    /// that can use the device holds a `DeviceHealth` clone, and the device's
    /// own callbacks hold the record weakly ([`Self::watch`]), so a count of
    /// one means no one else can still reach the device through figgy. A
    /// host-supplied device is never reported, whatever the count.
    ///
    /// Browser only, and the unit tests: a native bundle's poll thread
    /// also holds the record, and native drops never destroy the device.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn is_last_holder_of_requested_device(&self) -> bool {
        self.state.origin == DeviceOrigin::RequestedByFiggy && Arc::strong_count(&self.state) == 1
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

/// Keep the browser `GPUDevice` and `GPUQueue` behind `device` and `queue`
/// reachable until the work submitted on the queue completes, then release
/// them; destroy the device first when figgy requested it and no one else
/// holds it.
///
/// wgpu's WebGPU backend releases nothing on drop: a device lives as long as
/// JavaScript can reach it. Chromium 152 to 158 can deadlock the page's main
/// thread when a `GPUDevice` is garbage collected: Dawn delivers its
/// per-submit callbacks only when the device is destroyed, even after
/// `onSubmittedWorkDone` resolved, and a collected device delivers them
/// inside the GPU reply handler while that handler holds its lock, where the
/// callback's own command flush waits on the same lock (#322). Holding the
/// handles until `onSubmittedWorkDone` resolves is not enough on these
/// versions; destroying the device explicitly, while still holding it, runs
/// those callbacks outside the collection, and the collected device has
/// nothing left to deliver.
///
/// The rule, run in a task after the dropping renderer's own fields are
/// gone (they hold clones of `health` until then):
/// 1. When the record says figgy requested the device and this task holds
///    the record's last clone
///    ([`DeviceHealth::is_last_holder_of_requested_device`]), call
///    `device.destroy()`.
/// 2. Wait for `onSubmittedWorkDone`.
/// 3. When step 1 did not destroy and the test now passes (another holder
///    released its clone meanwhile), destroy and wait once more.
/// 4. Release the handles.
///
/// A host-supplied device ([`RendererDevice::new`]) is never destroyed, nor
/// one another chart, renderer, bundle clone or pending ticket still holds:
/// each of those holds the record. A lost device has no work left, so
/// nothing is held then.
#[cfg(target_arch = "wasm32")]
pub(crate) fn release_after_submitted_work(
    health: &DeviceHealth,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) {
    if health.is_lost() {
        return;
    }
    let held = (health.clone(), device.clone(), queue.clone());
    // `spawn_local` runs the task from the microtask queue, after the drop
    // that called this returns: the renderer's fields have released their
    // clones of the record by then.
    wasm_bindgen_futures::spawn_local(async move {
        let (health, device, queue) = held;
        let destroyed = destroy_if_last_holder(&health, &device);
        submitted_work_done(&queue).await;
        if !destroyed && destroy_if_last_holder(&health, &device) {
            submitted_work_done(&queue).await;
        }
        drop((device, queue, health));
    });
}

/// Step 1 of [`release_after_submitted_work`]: destroy `device` when
/// `health` is the last holder of a device figgy requested.
#[cfg(target_arch = "wasm32")]
fn destroy_if_last_holder(health: &DeviceHealth, device: &wgpu::Device) -> bool {
    let last = health.is_last_holder_of_requested_device() && !health.is_lost();
    if last {
        device.destroy();
    }
    last
}

/// Resolves once the work submitted on `queue` before this call completes,
/// or when the callback is dropped without running (the device is gone).
#[cfg(target_arch = "wasm32")]
fn submitted_work_done(queue: &wgpu::Queue) -> impl std::future::Future<Output = bool> + 'static {
    // wgpu requires a Send completion callback even for WebGPU, but
    // browser handles must stay on their owning JS thread: the callback
    // only sends.
    let (sender, receiver) = futures_channel::oneshot::channel();
    queue.on_submitted_work_done(move || {
        if sender.send(()).is_err() {
            // The receiver was dropped: the waiting task was cancelled.
        }
    });
    async move { receiver.await.is_ok() }
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
        let done = submitted_work_done(queue);
        wasm_bindgen_futures::spawn_local(async move {
            if done.await {
                ledger.complete_retirement(batch);
            }
        });
    }
}

/// The ownership fact and the last-holder test the browser drop reads
/// (#322). Native bundles here are built without a poll thread, whose drive
/// also holds the record.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn the_callbacks_do_not_count_as_holders_of_a_requested_device() {
        let (device, _queue) = crate::test_gpu::device();
        let health = DeviceHealth::watch(&device, DeviceOrigin::RequestedByFiggy);
        assert!(health.is_last_holder_of_requested_device());
        let other = health.clone();
        assert!(!health.is_last_holder_of_requested_device());
        assert!(!other.is_last_holder_of_requested_device());
        drop(other);
        assert!(health.is_last_holder_of_requested_device());
    }

    #[test]
    fn a_host_supplied_device_never_reports_a_last_holder() {
        let (device, queue) = crate::test_gpu::device();
        let gpu = RendererDevice::without_poller(device, queue, DeviceOrigin::SuppliedByHost);
        assert!(!gpu.health().is_last_holder_of_requested_device());
        assert!(!DeviceHealth::default().is_last_holder_of_requested_device());
    }

    #[test]
    fn a_renderer_and_its_shared_bundle_hold_a_requested_device() {
        let (device, queue) = crate::test_gpu::device();
        let gpu = RendererDevice::without_poller(device, queue, DeviceOrigin::RequestedByFiggy);
        let renderer = Renderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm, 1 << 20)
            .expect("required GPU test: renderer initialization failed");
        drop(gpu);
        let shared = renderer.renderer_device();
        assert!(!shared.health().is_last_holder_of_requested_device());
        // The renderer's own fields hold the record until its drop glue
        // ends; then the shared bundle is the last holder.
        drop(renderer);
        assert!(shared.health().is_last_holder_of_requested_device());
    }

    #[test]
    fn a_device_lost_after_a_holder_drops_still_reaches_the_rest() {
        let (device, _queue) = crate::test_gpu::device();
        let health = DeviceHealth::watch(&device, DeviceOrigin::RequestedByFiggy);
        let kept = health.clone();
        drop(health);
        device.destroy();
        let _ = device.poll(wgpu::PollType::Poll);
        assert!(kept.is_lost());
    }
}
