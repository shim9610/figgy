//! One GPU-to-CPU readback: wait for the map, read the mapped bytes, unmap.
//!
//! Every ticket and export that reads a `MAP_READ` buffer back goes through
//! [`GpuReadback`], so the wait, the error cases and the unmap are written
//! once (#160). The ticket keeps what is its own: the buffer's charges, the
//! identities that decode the result, and its public error type, which maps
//! [`ReadbackError`] to the variants it has always reported.
//!
//! Streaming readbacks (`streaming_auxiliary.rs`) keep their own code: the
//! streaming files are frozen (rule 8).

use futures_channel::oneshot;

use crate::FiggyError;
use crate::renderer::DeviceHealth;

/// Why a readback failed, before the caller maps it to its own error type.
#[derive(Debug)]
pub(crate) enum ReadbackError {
    /// The device poll failed, or found the device lost or in error. Native
    /// only: the browser's device polls itself.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Poll(FiggyError),
    /// The map callback was dropped before it ran.
    Canceled(oneshot::Canceled),
    /// The map request failed.
    Map(wgpu::BufferAsyncError),
    /// The mapped range could not be read.
    Range(wgpu::MapRangeError),
    /// The health check after the unmap found a device error
    /// ([`GpuReadback::read_checked`] only).
    Health(FiggyError),
}

/// A `MAP_READ` buffer whose map has been requested, waiting to be read.
pub(crate) struct GpuReadback {
    /// Polled for the map on native; the browser's device polls itself.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    device: wgpu::Device,
    health: DeviceHealth,
    buffer: wgpu::Buffer,
    /// Mapped bytes from offset 0; `None` maps the whole buffer.
    len: Option<wgpu::BufferAddress>,
    receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
    /// The submission the poll waits for; `None` waits for all work.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    submission: Option<wgpu::SubmissionIndex>,
}

impl GpuReadback {
    /// Request the map of `buffer[..len]` (`None`: the whole buffer) for
    /// reading, after the work already submitted to the queue. The read waits
    /// for every submission.
    pub(crate) fn map(
        device: &wgpu::Device,
        health: &DeviceHealth,
        buffer: &wgpu::Buffer,
        len: Option<wgpu::BufferAddress>,
    ) -> Self {
        let (sender, receiver) = oneshot::channel();
        let callback = move |result| {
            if sender.send(result).is_err() {
                // The receiver was dropped: the waiting call was cancelled.
            }
        };
        match len {
            Some(len) => buffer.slice(..len).map_async(wgpu::MapMode::Read, callback),
            None => buffer.slice(..).map_async(wgpu::MapMode::Read, callback),
        }
        Self::from_receiver(device, health, buffer, len, receiver, None)
    }

    /// A readback whose map the caller requested itself, for example with
    /// `CommandEncoder::map_buffer_on_submit`. `submission`, when given, is
    /// the one submission the read waits for.
    pub(crate) fn from_receiver(
        device: &wgpu::Device,
        health: &DeviceHealth,
        buffer: &wgpu::Buffer,
        len: Option<wgpu::BufferAddress>,
        receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
        submission: Option<wgpu::SubmissionIndex>,
    ) -> Self {
        Self {
            device: device.clone(),
            health: health.clone(),
            buffer: buffer.clone(),
            len,
            receiver,
            submission,
        }
    }

    /// The size of the whole `MAP_READ` buffer, for tests that check what a
    /// ticket reads back.
    #[cfg(test)]
    pub(crate) fn buffer_size(&self) -> wgpu::BufferAddress {
        self.buffer.size()
    }

    /// Wait for the map, pass the mapped bytes to `decode`, and unmap.
    ///
    /// On native the device is polled to completion first, so the wait
    /// resolves at once; in the browser the wait yields to the event loop.
    /// `decode` runs while the buffer is mapped and its result is returned
    /// after the unmap.
    pub(crate) async fn read<T>(self, decode: impl FnOnce(&[u8]) -> T) -> Result<T, ReadbackError> {
        self.read_inner(decode).await.map(|(value, _health)| value)
    }

    /// [`Self::read`], then the device health check: the unmap is the
    /// caller's last GPU call, so its error belongs to this read and not to a
    /// later call (#158).
    pub(crate) async fn read_checked<T>(
        self,
        decode: impl FnOnce(&[u8]) -> T,
    ) -> Result<T, ReadbackError> {
        let (value, health) = self.read_inner(decode).await?;
        health.check().map_err(ReadbackError::Health)?;
        Ok(value)
    }

    async fn read_inner<T>(
        self,
        decode: impl FnOnce(&[u8]) -> T,
    ) -> Result<(T, DeviceHealth), ReadbackError> {
        let Self {
            device,
            health,
            buffer,
            len,
            receiver,
            submission,
        } = self;
        #[cfg(not(target_arch = "wasm32"))]
        crate::renderer::gpu_poll(
            &health,
            &device,
            wgpu::PollType::Wait {
                submission_index: submission,
                timeout: None,
            },
        )
        .map_err(ReadbackError::Poll)?;
        #[cfg(target_arch = "wasm32")]
        let _ = (device, submission);
        receiver
            .await
            .map_err(ReadbackError::Canceled)?
            .map_err(ReadbackError::Map)?;
        let value = decode_mapped(&buffer, len, decode)?;
        Ok((value, health))
    }

    /// Nonblocking [`Self::read`] for callers that drive the device
    /// themselves: `Ok(None)` while the map is still pending, without polling.
    /// Once the map has completed, `decode` runs and the buffer is unmapped.
    pub(crate) fn try_read<T>(
        &mut self,
        decode: impl FnOnce(&[u8]) -> T,
    ) -> Result<Option<T>, ReadbackError> {
        let Some(result) = self.receiver.try_recv().map_err(ReadbackError::Canceled)? else {
            return Ok(None);
        };
        result.map_err(ReadbackError::Map)?;
        decode_mapped(&self.buffer, self.len, decode).map(Some)
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
