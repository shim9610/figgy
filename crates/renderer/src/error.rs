//! Single error enum for all fallible figgy operations.
//!
//! Only variants meaningful to library users are exposed; internal invariant
//! violations panic. `From` conversions propagate sub-errors (e.g.
//! [`AllocError`] from the column pool).

use crate::data_render::AllocError;

/// Public error type returned across the figgy API.
#[derive(Debug)]
pub enum FiggyError {
    /// Column pool allocation / management error.
    Pool(AllocError),

    /// Renderer-owned exact GPU picking failed.
    GpuPick(crate::gpu_pick::GpuPickError),

    /// `Config.chart_area` has zero size (cannot raster).
    InvalidChartArea { width: u32, height: u32 },

    /// A renderer-owned chart config violates a render invariant.
    InvalidConfig {
        field: &'static str,
        reason: &'static str,
    },

    /// CPU raster target allocation failed — typically a zero/oversized area.
    RasterWrapFailed { reason: String },

    /// No compatible wgpu adapter found.
    AdapterUnavailable,

    /// wgpu device creation failed (unsupported limits/features).
    DeviceCreationFailed { reason: String },

    /// wgpu surface creation failed (window handle incompatibility).
    SurfaceCreationFailed { reason: String },

    /// wgpu surface configuration failed (unsupported/empty capabilities).
    SurfaceConfigurationFailed { reason: String },

    /// Acquiring the next surface texture failed.
    SurfaceAcquireFailed { reason: String },

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
    UnknownColumn { id: String },

    /// Registered host source has no resident GPU allocation yet.
    ColumnNotResident {
        id: String,
    },

    InvalidStreamSource {
        id: String,
        reason: &'static str,
    },

    /// The column id belongs to renderer maintenance and is not host-mutable.
    ReservedColumnId { id: String },

    /// Renderer-owned chart id is not live in this renderer.
    UnknownChart { id: crate::renderer::ChartId },

    /// A checked identity/revision issuer has no successor.
    CounterExhausted { counter: &'static str },

    /// An invocation token no longer identifies current renderer state.
    StaleStateToken { reason: String },

    /// A renderer-state candidate could not reserve CPU memory before commit.
    StateAllocationFailed {
        resource: &'static str,
        reason: String,
    },

    /// A series declaration is internally inconsistent for the requested
    /// render path.
    InvalidSeriesConfig { series_id: String, reason: String },

    /// Handle generation no longer matches after an invalidating pool mutation.
    StaleHandle { generation: u32, current: u32 },

    /// A `PreparedFrame` no longer matches its captured renderer resources —
    /// for example it belongs to another renderer, the target pipeline or pool
    /// layout changed, a captured column allocation was replaced, or a
    /// captured `ChartView` was rewritten after `Renderer::prepare`.
    /// Recovery: build the current items and call `prepare` again.
    StalePreparedFrame { reason: String },
}

impl std::fmt::Display for FiggyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pool(e) => write!(f, "column pool: {e}"),
            Self::GpuPick(e) => write!(f, "GPU pick: {e}"),
            Self::InvalidChartArea { width, height } => {
                write!(f, "invalid chart area: {width}x{height}")
            }
            Self::InvalidConfig { field, reason } => {
                write!(f, "invalid chart config field {field}: {reason}")
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
            Self::InvalidSeriesConfig { series_id, reason } => {
                write!(f, "invalid series config for {series_id}: {reason}")
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
        }
    }
}

impl std::error::Error for FiggyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pool(e) => Some(e),
            Self::GpuPick(e) => Some(e),
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

/// Convenience alias.
pub type Result<T> = std::result::Result<T, FiggyError>;
