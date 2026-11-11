//! Single error enum for all fallible figgy operations.
//!
//! Every failure reaches the caller as a value: product code has no panic
//! path (#152). An internal invariant that does not hold is a figgy bug and
//! returns [`FiggyError::Internal`]. `From` conversions propagate sub-errors
//! (e.g. [`AllocError`] from the column pool).

use crate::data_render::AllocError;

/// Public error type returned across the figgy API.
#[derive(Debug)]
pub enum FiggyError {
    /// Column pool allocation / management error.
    Pool(AllocError),

    /// Renderer-owned exact GPU picking failed.
    GpuPick(crate::gpu_pick::GpuPickError),

    /// `Config.chart_area` has zero size (cannot raster).
    InvalidChartArea {
        width: u32,
        height: u32,
    },

    /// A renderer-owned chart config violates a render invariant.
    InvalidConfig {
        field: &'static str,
        /// Which rule failed; its `Display` is the sentence (#215).
        kind: crate::RuleKind,
    },

    /// CPU raster target allocation failed — typically a zero/oversized area.
    RasterWrapFailed {
        reason: String,
    },

    /// No compatible wgpu adapter found.
    AdapterUnavailable,

    /// wgpu device creation failed (unsupported limits/features).
    DeviceCreationFailed {
        reason: String,
    },

    /// wgpu surface creation failed (window handle incompatibility).
    SurfaceCreationFailed {
        reason: String,
    },

    /// wgpu surface configuration failed (unsupported/empty capabilities).
    SurfaceConfigurationFailed {
        reason: String,
    },

    /// Acquiring the next surface texture failed.
    SurfaceAcquireFailed {
        reason: String,
    },

    /// The render target format cannot be used by figgy's blended pipelines.
    UnsupportedSurfaceFormat {
        format: wgpu::TextureFormat,
        reason: String,
    },

    /// A requested GPU resource exceeds the current device's limits.
    GpuResourceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },

    /// A GPU resource allocation failed after passing static device limits.
    GpuResourceAllocationFailed {
        resource: &'static str,
        reason: String,
    },

    /// Referenced column id is not registered.
    UnknownColumn {
        id: String,
    },

    /// Registered host source has no resident GPU allocation yet.
    ColumnNotResident {
        id: String,
    },

    InvalidStreamSource {
        id: String,
        reason: &'static str,
    },

    /// The column id belongs to renderer maintenance and is not host-mutable.
    ReservedColumnId {
        id: String,
    },

    /// Renderer-owned chart id is not live in this renderer.
    UnknownChart {
        id: crate::renderer::ChartId,
    },

    /// A checked identity/revision issuer has no successor.
    CounterExhausted {
        counter: &'static str,
    },

    /// An invocation token no longer identifies current renderer state.
    StaleStateToken {
        reason: String,
    },

    /// A renderer-state candidate could not reserve CPU memory before commit.
    StateAllocationFailed {
        resource: &'static str,
        reason: String,
    },

    /// A series declaration is internally inconsistent for the requested
    /// render path.
    InvalidSeriesConfig {
        series_id: String,
        /// Which rule failed; its `Display` is the sentence (#215).
        kind: crate::RuleKind,
    },

    /// Handle generation no longer matches after an invalidating pool mutation.
    StaleHandle {
        generation: u32,
        current: u32,
    },

    /// A `PreparedFrame` no longer matches its captured renderer resources —
    /// for example it belongs to another renderer, the target pipeline or pool
    /// layout changed, a captured column allocation was replaced, or a
    /// captured `ChartView` was rewritten after `Renderer::prepare`.
    /// Recovery: build the current items and call `prepare` again.
    StalePreparedFrame {
        reason: String,
    },

    /// An internal invariant did not hold. This is a figgy bug; it is
    /// reported instead of panicking so the host keeps running (#152).
    Internal {
        invariant: &'static str,
    },

    /// The GPU reported a validation or internal error for figgy's work.
    GpuValidation {
        resource: &'static str,
        reason: String,
    },

    /// Font bytes handed to [`register_font_bytes`](crate::text_render::register_font_bytes)
    /// hold no face the text stack can use.
    InvalidFont {
        reason: &'static str,
    },

    /// The wgpu device was lost. Every later GPU call on this renderer
    /// returns this error; the host recreates the device and the renderer.
    DeviceLost {
        reason: String,
    },

    /// The OS could not start the native poll thread of a
    /// [`RendererDevice`](crate::RendererDevice), which completes
    /// asynchronous GPU waits. `RendererDevice::blocking`
    /// builds a bundle without one.
    PollThreadStartFailed {
        reason: String,
    },

    /// A monotonic chart's resize or layout fit (#271), or an export at
    /// another scale, would move the x-to-pixel mapping of a monotonic
    /// series without its group's source: `change` says how (the plot width
    /// in device pixels, for one). The renderer keeps no copy of the
    /// samples, so the call that moves it takes the source
    /// (`MonotonicChart::resize`, `MonotonicChart::fit_layout`,
    /// `ExportSource::Monotonic`). The refused call changes nothing. A frame
    /// or a pick never fails with it.
    MonotonicSourceRequired {
        series_id: String,
        change: crate::monotonic::MappingChange,
    },

    /// A monotonic call (#271) on group `group` was refused by the
    /// envelope core: an invalid grid or mapping, an x axis that is not
    /// linear and unbroken, an append before a full call or with another
    /// channel count, or a slide over samples that were not kept.
    Monotonic {
        group: String,
        error: crate::monotonic::MonotonicError,
    },

    /// No series of the chart names monotonic group `group` (#271).
    UnknownMonotonicGroup {
        group: String,
    },

    /// An asynchronous GPU wait on a bundle built with
    /// `RendererDevice::blocking`: it has
    /// no poll thread to complete the wait. Use the `_blocking` call, or
    /// build the bundle with [`RendererDevice::new`](crate::RendererDevice::new).
    AsyncWaitUnavailable,
}

impl std::fmt::Display for FiggyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pool(e) => write!(f, "column pool: {e}"),
            Self::GpuPick(e) => write!(f, "GPU pick: {e}"),
            Self::InvalidChartArea { width, height } => {
                write!(f, "invalid chart area: {width}x{height}")
            }
            Self::InvalidConfig { field, kind } => {
                write!(f, "invalid chart config field {field}: {kind}")
            }
            Self::RasterWrapFailed { reason } => write!(f, "raster wrap failed: {reason}"),
            Self::AdapterUnavailable => write!(f, "no compatible wgpu adapter"),
            Self::DeviceCreationFailed { reason } => {
                write!(f, "wgpu device creation failed: {reason}")
            }
            Self::SurfaceCreationFailed { reason } => {
                write!(f, "wgpu surface creation failed: {reason}")
            }
            Self::SurfaceConfigurationFailed { reason } => {
                write!(f, "wgpu surface configuration failed: {reason}")
            }
            Self::SurfaceAcquireFailed { reason } => {
                write!(f, "wgpu surface acquire failed: {reason}")
            }
            Self::UnsupportedSurfaceFormat { format, reason } => {
                write!(f, "unsupported surface format {format:?}: {reason}")
            }
            Self::GpuResourceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "{resource} exceeds GPU limit: requested {requested}, limit {limit}"
            ),
            Self::GpuResourceAllocationFailed { resource, reason } => {
                write!(f, "{resource} GPU allocation failed: {reason}")
            }
            Self::UnknownColumn { id } => write!(f, "unknown column id: {id}"),
            Self::ColumnNotResident { id } => {
                write!(f, "column is registered but not GPU resident: {id}")
            }
            Self::InvalidStreamSource { id, reason } => {
                write!(f, "invalid streamed source {id}: {reason}")
            }
            Self::ReservedColumnId { id } => {
                write!(f, "column id is reserved for renderer maintenance: {id}")
            }
            Self::UnknownChart { id } => write!(f, "unknown chart id: {id:?}"),
            Self::CounterExhausted { counter } => {
                write!(f, "renderer counter exhausted: {counter}")
            }
            Self::StaleStateToken { reason } => write!(f, "stale renderer state token: {reason}"),
            Self::StateAllocationFailed { resource, reason } => {
                write!(f, "{resource} allocation failed: {reason}")
            }
            Self::InvalidSeriesConfig { series_id, kind } => {
                write!(f, "invalid series config for {series_id}: {kind}")
            }
            Self::StaleHandle {
                generation,
                current,
            } => write!(
                f,
                "stale column handle (handle generation {generation}, pool generation {current})"
            ),
            Self::StalePreparedFrame { reason } => {
                write!(f, "stale prepared frame: {reason}")
            }
            Self::InvalidFont { reason } => write!(f, "invalid font: {reason}"),
            Self::Internal { invariant } => {
                write!(f, "internal invariant failed (figgy bug): {invariant}")
            }
            Self::GpuValidation { resource, reason } => {
                write!(f, "{resource}: GPU error: {reason}")
            }
            Self::DeviceLost { reason } => write!(f, "wgpu device lost: {reason}"),
            Self::PollThreadStartFailed { reason } => {
                write!(f, "could not start the GPU poll thread: {reason}")
            }
            Self::MonotonicSourceRequired { series_id, change } => write!(
                f,
                "monotonic series {series_id}: the call would move its x mapping ({change:?}); \
                 the call takes the group's source"
            ),
            Self::Monotonic { group, error } => write!(f, "monotonic group {group}: {error}"),
            Self::UnknownMonotonicGroup { group } => {
                write!(f, "no series of the chart names monotonic group {group}")
            }
            Self::AsyncWaitUnavailable => write!(
                f,
                "asynchronous GPU wait on a blocking-only device bundle (no poll thread)"
            ),
        }
    }
}

impl std::error::Error for FiggyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pool(e) => Some(e),
            Self::GpuPick(e) => Some(e),
            Self::Monotonic { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl From<AllocError> for FiggyError {
    fn from(e: AllocError) -> Self {
        Self::Pool(e)
    }
}

impl From<crate::gpu_pick::GpuPickError> for FiggyError {
    fn from(error: crate::gpu_pick::GpuPickError) -> Self {
        Self::GpuPick(error)
    }
}

impl FiggyError {
    /// The error for an internal invariant that did not hold.
    pub(crate) const fn internal(invariant: &'static str) -> Self {
        Self::Internal { invariant }
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, FiggyError>;
