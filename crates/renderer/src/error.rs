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

    /// `Config.chart_area` has zero size (cannot raster).
    InvalidChartArea { width: u32, height: u32 },

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
    SurfaceAcquireFailed { error: wgpu::SurfaceError },

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

    /// Referenced column id is not in the pool.
    UnknownColumn { id: String },

    /// A series declaration is internally inconsistent for the requested
    /// render path.
    InvalidSeriesConfig { series_id: String, reason: String },

    /// Handle's generation no longer matches the pool (stale after defrag / clear).
    StaleHandle { generation: u32, current: u32 },
}

impl std::fmt::Display for FiggyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pool(e) => write!(f, "column pool: {e}"),
            Self::InvalidChartArea { width, height } => {
                write!(f, "invalid chart area: {width}x{height}")
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
            Self::SurfaceAcquireFailed { error } => {
                write!(f, "wgpu surface acquire failed: {error:?}")
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
        }
    }
}

impl std::error::Error for FiggyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pool(e) => Some(e),
            _ => None,
        }
    }
}

impl From<AllocError> for FiggyError {
    fn from(e: AllocError) -> Self {
        Self::Pool(e)
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, FiggyError>;
