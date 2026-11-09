//! A renderer without a window, in one call (#265).
//!
//! It makes the GPU instance, adapter and device itself, honouring
//! `WGPU_BACKEND` (and the other `wgpu` environment selections) as the
//! tests do, and renders into `Rgba8Unorm`, the format the exports read
//! back. The instance and adapter are not kept: the device holds what it
//! needs for as long as the renderer lives.

use crate::{FiggyError, Renderer, RendererDevice, Result};

/// What [`Renderer::headless`] asks the GPU for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadlessOptions {
    /// The initial GPU column pool in bytes; it grows on demand, as
    /// [`Renderer::try_new`] documents.
    pub column_pool_bytes: u64,
    /// The adapter to prefer: `None` takes the first the system offers.
    pub power_preference: wgpu::PowerPreference,
}

/// A 16 MiB column pool on whatever adapter the system offers first.
impl Default for HeadlessOptions {
    fn default() -> Self {
        Self {
            column_pool_bytes: 16 * 1024 * 1024,
            power_preference: wgpu::PowerPreference::None,
        }
    }
}

/// The texture format a headless renderer draws into: the exports' own.
const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

async fn headless_device(options: HeadlessOptions) -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: options.power_preference,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|_| FiggyError::AdapterUnavailable)?;
    crate::gpu::request_device_async(&adapter)
        .await
        .map_err(|error| FiggyError::DeviceCreationFailed {
            reason: format!("{error}"),
        })
}

impl Renderer {
    /// A renderer with its own GPU device and no window, for exports and
    /// tests: the instance, adapter and device requests, the device's poll
    /// thread (native) and [`Renderer::try_new`] in one call. In the
    /// browser the pipelines compile through WebGPU's asynchronous compiler.
    pub async fn headless(options: HeadlessOptions) -> Result<Self> {
        let (device, queue) = headless_device(options).await?;
        let gpu = RendererDevice::new(device, queue)?;
        let mut observer = |_| {};
        Self::try_new_with_sample_count_observed_async(
            gpu,
            HEADLESS_FORMAT,
            options.column_pool_bytes,
            1,
            false,
            &mut observer,
        )
        .await
    }

    /// [`Self::headless`] with [`HeadlessOptions::default`], without an
    /// executor (native): wgpu completes the adapter and device requests
    /// when they are made. Blocking and asynchronous calls both work on it.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless_blocking() -> Result<Self> {
        Self::headless_blocking_with(HeadlessOptions::default())
    }

    /// [`Self::headless_blocking`] with the host's `options`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless_blocking_with(options: HeadlessOptions) -> Result<Self> {
        let (device, queue) = crate::gpu_readback::ready_now(headless_device(options)).ok_or(
            FiggyError::internal("a native wgpu adapter and device request completes at once"),
        )??;
        Self::try_new(
            RendererDevice::new(device, queue)?,
            HEADLESS_FORMAT,
            options.column_pool_bytes,
        )
    }
}
