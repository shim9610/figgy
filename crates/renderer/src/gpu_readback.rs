//! GPU completions: wait for a wgpu callback, by awaiting it, by blocking on
//! it, or by checking it without waiting; and the readback built on one.
//!
//! wgpu reports a buffer map or `on_submitted_work_done` through a callback
//! that runs only inside a device poll (or a submit). [`completion`] pairs
//! such a callback with a [`Completion`], which a caller waits on in one of
//! three ways, each with the same result (#160):
//!
//! - [`Completion::wait`] awaits it. On native the bundle's poll thread
//!   (`gpu_poller.rs`) polls the device, so the future returns `Pending`
//!   until the work completes; a bundle without the thread
//!   (`RendererDevice::blocking`) fails
//!   with [`FiggyError::AsyncWaitUnavailable`]. In the browser the device
//!   polls itself.
//! - `Completion::wait_blocking` (native) polls the device on the
//!   caller's thread until the work completes.
//! - [`Completion::try_take`] returns the value if the callback has run,
//!   and `None` otherwise, without blocking.
//!
//! Every ticket and export that reads a `MAP_READ` buffer back goes through
//! [`GpuReadback`], so the wait, the error cases and the unmap are written
//! once. The ticket keeps what is its own: the buffer's charges, the
//! identities that decode the result, and its public error type, which maps
//! [`ReadbackError`] to the variants it has always reported.
//!
//! Streaming readbacks (`streaming_auxiliary.rs`) keep their own code: the
//! streaming files are frozen (rule 8).

use futures_channel::oneshot;

use crate::FiggyError;
use crate::renderer::DeviceHealth;

/// Why a wait or a readback failed, before the caller maps it to its own
/// error type.
#[derive(Debug)]
pub(crate) enum ReadbackError {
    /// The device poll failed or found the device lost or in error, or the
    /// bundle has no poll thread for an asynchronous wait
    /// ([`FiggyError::AsyncWaitUnavailable`]).
    Poll(FiggyError),
    /// The callback was dropped before it ran.
    Canceled(oneshot::Canceled),
    /// The map request failed.
    Map(wgpu::BufferAsyncError),
    /// The mapped range could not be read.
    Range(wgpu::MapRangeError),
    /// The health check after the unmap found a device error
    /// ([`GpuReadback::read_checked`] only).
    Health(FiggyError),
}

/// The outcome of a ticket's `try_resolve`: the result once the GPU work
/// has completed, or the ticket back while it has not, to try again later
/// (for example on the next frame). Checking never blocks.
#[derive(Debug)]
pub enum TryResolve<T, Ticket> {
    /// The work has completed; the ticket is consumed.
    Ready(T),
    /// The work is still running.
    Pending(Ticket),
}

/// The result of one wgpu callback made by [`completion`].
pub(crate) struct Completion<T> {
    receiver: oneshot::Receiver<T>,
    /// The value, once a wait has received it.
    value: Option<T>,
    /// This wait's share of the poll thread's count; `None` without a poll
    /// thread. Holding it keeps the thread alive until the wait settles.
    #[cfg(not(target_arch = "wasm32"))]
    wait: Option<crate::gpu_poller::WaitMark>,
}

/// A wgpu callback that hands its value to the returned [`Completion`], for
/// the device `health` watches.
pub(crate) fn completion<T: Send + 'static>(
    health: &DeviceHealth,
) -> (impl FnOnce(T) + Send + 'static, Completion<T>) {
    let (sender, receiver) = oneshot::channel();
    #[cfg(not(target_arch = "wasm32"))]
    let (fired, wait) = match health.poller().map(crate::gpu_poller::Poller::marks) {
        Some((fired, wait)) => (Some(fired), Some(wait)),
        None => (None, None),
    };
    #[cfg(target_arch = "wasm32")]
    let _ = health;
    let callback = move |value: T| {
        if sender.send(value).is_err() {
            // The receiver was dropped: the waiting call was cancelled.
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(fired) = fired {
            fired.fire();
        }
    };
    let completion = Completion {
        receiver,
        value: None,
        #[cfg(not(target_arch = "wasm32"))]
        wait,
    };
    (callback, completion)
}

impl<T> Completion<T> {
    /// Await the callback. On native the bundle's poll thread drives the
    /// device; without one this fails at once. Cancelling the await keeps
    /// the wait: a later wait of any kind picks it up.
    ///
    /// The device health is checked before and after, as a blocking wait's
    /// poll checks it ([`crate::renderer::gpu_poll`]), so the three waits
    /// report a lost device or a GPU error alike.
    pub(crate) async fn wait(&mut self, health: &DeviceHealth) -> Result<(), ReadbackError> {
        if self.value.is_some() {
            return Ok(());
        }
        health.check().map_err(ReadbackError::Poll)?;
        #[cfg(not(target_arch = "wasm32"))]
        match &self.wait {
            Some(wait) => wait.register(),
            None => return Err(ReadbackError::Poll(FiggyError::AsyncWaitUnavailable)),
        }
        let value = (&mut self.receiver)
            .await
            .map_err(ReadbackError::Canceled)?;
        self.received(value);
        health.check().map_err(ReadbackError::Poll)
    }

    /// Poll the device on this thread until `submission` (`None`: all
    /// submitted work) completes, then receive the callback's value.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wait_blocking(
        &mut self,
        health: &DeviceHealth,
        device: &wgpu::Device,
        submission: Option<wgpu::SubmissionIndex>,
    ) -> Result<(), ReadbackError> {
        if self.value.is_some() {
            return Ok(());
        }
        crate::renderer::gpu_poll(
            health,
            device,
            wgpu::PollType::Wait {
                submission_index: submission,
                timeout: None,
            },
        )
        .map_err(ReadbackError::Poll)?;
        let value = receive_after_poll(&mut self.receiver).map_err(ReadbackError::Canceled)?;
        self.received(value);
        Ok(())
    }

    /// Whether the callback has run, without blocking. On native the first
    /// call hands the wait to the poll thread, so a later call finds it done;
    /// without a poll thread each call polls the device once, without
    /// waiting. In the browser the device polls itself.
    pub(crate) fn poll_ready(
        &mut self,
        health: &DeviceHealth,
        device: &wgpu::Device,
    ) -> Result<bool, ReadbackError> {
        if self.value.is_some() {
            return Ok(true);
        }
        // As the other waits: the health before and once the value is in.
        health.check().map_err(ReadbackError::Poll)?;
        #[cfg(not(target_arch = "wasm32"))]
        match &self.wait {
            Some(wait) => wait.register(),
            None => {
                crate::renderer::gpu_poll(health, device, wgpu::PollType::Poll)
                    .map_err(ReadbackError::Poll)?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = device;
        match self.receiver.try_recv().map_err(ReadbackError::Canceled)? {
            Some(value) => {
                self.received(value);
                health.check().map_err(ReadbackError::Poll)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Whether the callback has run, without polling the device or handing
    /// the wait to the poll thread: for callers that drive the device
    /// themselves (the streaming continuations, rule 8).
    pub(crate) fn try_receive(&mut self) -> Result<bool, ReadbackError> {
        if self.value.is_some() {
            return Ok(true);
        }
        match self.receiver.try_recv().map_err(ReadbackError::Canceled)? {
            Some(value) => {
                self.received(value);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// The value a wait received. An error if no wait has finished: a
    /// figgy bug, reported instead of a panic.
    pub(crate) fn take(&mut self) -> Result<T, ReadbackError> {
        self.value
            .take()
            .ok_or(ReadbackError::Poll(FiggyError::Internal {
                invariant: "a GPU completion is taken only after a wait received it",
            }))
    }

    /// Keep the value, and drop this wait's share of the poll thread: the
    /// callback has run, so the wait is settled.
    fn received(&mut self, value: T) {
        self.value = Some(value);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.wait = None;
        }
    }
}

/// The output of a wgpu future that native wgpu returns already complete:
/// an error scope's pop, an adapter or a device request. wgpu 30's native
/// backend does that work when the future is made and returns `ready(..)`
/// (`backend/wgpu_core.rs`: `request_adapter`, `request_device`,
/// `pop_error_scope`), so one poll takes it. `None` if the future is not
/// ready: the caller reports that as an error and never waits.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ready_now<F: std::future::Future>(future: F) -> Option<F::Output> {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => Some(output),
        Poll::Pending => None,
    }
}

/// Take a callback's value once a blocking poll has seen its work complete.
///
/// The poll that completed the work does not always run the callback
/// itself: when the poll thread or another caller polls the same device at
/// once, the callback runs on that thread, just after. So this thread parks
/// until the callback's send wakes it. It never waits for GPU work, only for
/// a callback already being run, and spurious wake-ups only repeat the
/// check.
#[cfg(not(target_arch = "wasm32"))]
fn receive_after_poll<T>(receiver: &mut oneshot::Receiver<T>) -> Result<T, oneshot::Canceled> {
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    /// Wakes the blocked thread when the callback sends.
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    if let Some(value) = receiver.try_recv()? {
        return Ok(value);
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match std::pin::Pin::new(&mut *receiver).poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// A `MAP_READ` buffer whose map has been requested, waiting to be read.
pub(crate) struct GpuReadback {
    device: wgpu::Device,
    health: DeviceHealth,
    buffer: wgpu::Buffer,
    /// Mapped bytes from offset 0; `None` maps the whole buffer.
    len: Option<wgpu::BufferAddress>,
    map: Completion<Result<(), wgpu::BufferAsyncError>>,
    /// The submission a blocking read waits for; `None` waits for all work.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    submission: Option<wgpu::SubmissionIndex>,
}

impl GpuReadback {
    /// Request the map of `buffer[..len]` (`None`: the whole buffer) for
    /// reading, after the work already submitted to the queue. A blocking
    /// read waits for every submission.
    pub(crate) fn map(
        device: &wgpu::Device,
        health: &DeviceHealth,
        buffer: &wgpu::Buffer,
        len: Option<wgpu::BufferAddress>,
    ) -> Self {
        let (callback, map) = completion(health);
        match len {
            Some(len) => buffer.slice(..len).map_async(wgpu::MapMode::Read, callback),
            None => buffer.slice(..).map_async(wgpu::MapMode::Read, callback),
        }
        Self::from_completion(device, health, buffer, len, map, None)
    }

    /// A readback whose map the caller requested itself with a callback from
    /// [`completion`], for example with `CommandEncoder::map_buffer_on_submit`.
    /// `submission`, when given, is the one submission a blocking read waits
    /// for.
    pub(crate) fn from_completion(
        device: &wgpu::Device,
        health: &DeviceHealth,
        buffer: &wgpu::Buffer,
        len: Option<wgpu::BufferAddress>,
        map: Completion<Result<(), wgpu::BufferAsyncError>>,
        submission: Option<wgpu::SubmissionIndex>,
    ) -> Self {
        Self {
            device: device.clone(),
            health: health.clone(),
            buffer: buffer.clone(),
            len,
            map,
            submission,
        }
    }

    /// The size of the whole `MAP_READ` buffer, for tests that check what a
    /// ticket reads back.
    #[cfg(test)]
    pub(crate) fn buffer_size(&self) -> wgpu::BufferAddress {
        self.buffer.size()
    }

    /// Await the map. See [`Completion::wait`].
    pub(crate) async fn wait(&mut self) -> Result<(), ReadbackError> {
        self.map.wait(&self.health).await
    }

    /// Poll the device on this thread until the map completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wait_blocking(&mut self) -> Result<(), ReadbackError> {
        self.map
            .wait_blocking(&self.health, &self.device, self.submission.clone())
    }

    /// Whether the map has completed, without blocking. See
    /// [`Completion::poll_ready`].
    pub(crate) fn poll_ready(&mut self) -> Result<bool, ReadbackError> {
        self.map.poll_ready(&self.health, &self.device)
    }

    /// Whether the map has completed, without polling. See
    /// [`Completion::try_receive`].
    pub(crate) fn try_receive(&mut self) -> Result<bool, ReadbackError> {
        self.map.try_receive()
    }

    /// After a wait: pass the mapped bytes to `decode`, and unmap. `decode`
    /// runs while the buffer is mapped and its result is returned after the
    /// unmap.
    pub(crate) fn finish<T>(
        &mut self,
        decode: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, ReadbackError> {
        self.map.take()?.map_err(ReadbackError::Map)?;
        decode_mapped(&self.buffer, self.len, decode)
    }

    /// [`Self::finish`], then the device health check: the unmap is the
    /// caller's last GPU call, so its error belongs to this read and not to a
    /// later call (#158).
    pub(crate) fn finish_checked<T>(
        &mut self,
        decode: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, ReadbackError> {
        let health = self.health.clone();
        let value = self.finish(decode)?;
        health.check().map_err(ReadbackError::Health)?;
        Ok(value)
    }
}

/// Read `buffer[..len]` (`None`: the whole buffer) once its map has
/// completed, pass the bytes to `decode`, and unmap.
fn decode_mapped<T>(
    buffer: &wgpu::Buffer,
    len: Option<wgpu::BufferAddress>,
    decode: impl FnOnce(&[u8]) -> T,
) -> Result<T, ReadbackError> {
    let mapped = match len {
        Some(len) => buffer.get_mapped_range(..len),
        None => buffer.get_mapped_range(..),
    }
    .map_err(ReadbackError::Range)?;
    let value = decode(&mapped);
    drop(mapped);
    buffer.unmap();
    Ok(value)
}
