//! Payload-free host source declarations. Resident metadata remains in ColumnPool.

use crate::{FiggyError, Result};

/// Borrowed host source bound to one registered stream revision. The renderer
/// keeps neither the trait object nor its payload after a step returns.
#[derive(Clone, Copy)]
pub struct StreamSourceBinding<'a> {
    pub id: &'a str,
    pub revision: u64,
    pub source: crate::data::StreamColumnSource<'a>,
}

/// Borrowed payload for exactly one renderer-requested range. `source`
/// contains only the requested values beginning at `offset`; `source_len` is
/// the full logical column length captured by the registered revision.
#[derive(Clone, Copy)]
pub struct StreamRangeSourceBinding<'a> {
    pub id: &'a str,
    pub revision: u64,
    pub source_len: u64,
    pub offset: u64,
    pub source: crate::data::StreamColumnSource<'a>,
}

/// Explicit bounded resources for all streamed charts in one renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingLimits {
    pub max_active_charts: usize,
    pub max_in_flight_chunks: usize,
    pub max_columns_per_chunk: usize,
    pub max_chunk_input_bytes: u64,
    pub max_in_flight_gpu_bytes: u64,
}

/// Renderer-owned accumulation surface and cursor options for one chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamingChartOptions {
    pub size: (u32, u32),
    pub clear_color: crate::Color,
    pub max_primitives_per_chunk: u64,
}

/// Result of one non-blocking host-driven streaming step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingProgress {
    Submitted {
        submitted_primitives: u64,
        total_primitives: u64,
    },
    Backpressure {
        submitted_primitives: u64,
        total_primitives: u64,
    },
    AllSubmitted {
        total_primitives: u64,
    },
}

/// Current scheduler reservations. Submitted GPU resources remain counted
/// until their queue completion callback is observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingUsage {
    pub active_charts: usize,
    pub in_flight_chunks: usize,
    pub reserved_gpu_bytes: u64,
}

/// Read-only execution metadata. Revisions and job IDs are renderer-local
/// diagnostic identities, not host-issued tokens. Counts measure submitted
/// primitives (including distinct primitive passes), not completed GPU work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingStatus {
    pub status: StreamingState,
    pub job_id: Option<u64>,
    pub revision: u64,
    pub desired_revision: u64,
    pub published_revision: Option<u64>,
    pub submitted_primitives: u64,
    pub total_primitives: u64,
    pub in_flight_chunks: usize,
    pub reserved_gpu_bytes: u64,
    pub auto_fit_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingState {
    Idle,
    Active,
    AllSubmitted,
    Complete,
    Cancelling,
    Cancelled,
}

/// Exact source identity captured by one renderer-owned automatic execution.
///
/// The payload remains host-owned and is supplied only for the bounded range
/// selected by [`crate::Renderer::auto_stream_chart_step`].  Keeping this
/// descriptor public lets a host pin the matching immutable payload revision
/// until the renderer reports that the execution is terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoStreamSourceVersion {
    pub id: String,
    pub revision: u64,
}

/// Result of asking the renderer to draw the latest registered chart state.
///
/// Decoration-only requests reuse the active accumulation. A data, series,
/// view, target, or chunk-budget change starts one replacement from the latest
/// renderer SSOT after its fallible preflight succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoStreamingRequest {
    Started {
        revision: crate::RenderRevision,
        sources: Vec<AutoStreamSourceVersion>,
    },
    Active {
        revision: crate::RenderRevision,
        pending_latest: Option<crate::RenderRevision>,
    },
    Complete {
        revision: crate::RenderRevision,
        pending_latest: Option<crate::RenderRevision>,
    },
}

/// Progress of one renderer-selected chunk in automatic streaming mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoStreamingProgress {
    Submitted {
        revision: crate::RenderRevision,
        submitted_primitives: u64,
        total_primitives: u64,
    },
    Backpressure {
        revision: crate::RenderRevision,
        submitted_primitives: u64,
        total_primitives: u64,
    },
    /// Every chunk has been submitted. The execution remains active until all
    /// chart-specific receipts complete and its final display refresh is
    /// published.
    AllSubmitted {
        revision: crate::RenderRevision,
        total_primitives: u64,
    },
    Complete {
        revision: crate::RenderRevision,
        pending_latest: Option<crate::RenderRevision>,
    },
}

/// One exact source range selected by the renderer-owned automatic cursor.
/// Hosts may fetch this range asynchronously, but must submit the same source
/// revision before asking the cursor to advance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoStreamRange {
    pub id: String,
    pub revision: u64,
    pub source_len: u64,
    pub offset: u64,
    pub len: u64,
    pub encoding: StreamEncoding,
}

/// Split-phase form of [`AutoStreamingProgress`] for range providers.
///
/// `Ready` reserves one bounded request without advancing the CPU cursor.
/// Repeated calls return the same ranges until
/// [`crate::Renderer::auto_stream_chart_submit_ranges`] commits that request or
/// the execution is cancelled/replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoStreamingRangeRequest {
    Ready {
        revision: crate::RenderRevision,
        submitted_primitives: u64,
        total_primitives: u64,
        ranges: Vec<AutoStreamRange>,
    },
    Backpressure {
        revision: crate::RenderRevision,
        submitted_primitives: u64,
        total_primitives: u64,
    },
    AllSubmitted {
        revision: crate::RenderRevision,
        total_primitives: u64,
    },
    Complete {
        revision: crate::RenderRevision,
        pending_latest: Option<crate::RenderRevision>,
    },
}

/// Outcome of the single public interruption request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderInterruptStatus {
    /// Cancellation was recorded for the active automatic stream and is
    /// applied at the next renderer service boundary.
    StreamCancelQueued,
    /// No automatic stream is active. Resident rendering is left untouched.
    Resident,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEncoding {
    ScalarF32,
    HiLoF32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Every replay must return identical encoded bytes for each
/// `(source id, revision, encoding, index)`. Publish a new revision before
/// changing any value; the Renderer caches first-seen extrema by index.
pub enum StreamReplay {
    Sequential,
    RandomAccess,
    /// Rejected: exact redraw, zoom and queries require replay.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamBounds {
    /// Bounds and smallest positive value describe the same complete logical
    /// source revision as encoded for the GPU. Scalar values are widened from
    /// their uploaded f32; hi/lo values whose GPU `hi + lo` reconstruction is
    /// finite use `hi as f64 + lo as f64`. Partial chunk extrema must not be
    /// published as complete statistics.
    pub min: f64,
    pub max: f64,
    pub min_positive: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamStatistics {
    /// Complete revision-wide statistics; None means no finite values.
    Known(Option<StreamBounds>),
    Pending,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamColumn {
    pub id: String,
    pub len: u64,
    pub encoding: StreamEncoding,
    pub replay: StreamReplay,
    pub revision: u64,
    pub statistics: StreamStatistics,
}

impl StreamColumn {
    pub(crate) fn validate(&self) -> Result<()> {
        let invalid = |reason| FiggyError::InvalidStreamSource {
            id: self.id.clone(),
            reason,
        };
        if self.id.is_empty() {
            return Err(invalid("empty source id"));
        }
        if self.len > u32::MAX as u64 || usize::try_from(self.len).is_err() {
            return Err(invalid(
                "logical length exceeds the global index representation",
            ));
        }
        if self.replay == StreamReplay::Unavailable {
            return Err(invalid("source must support same-revision replay"));
        }
        if self.statistics != StreamStatistics::Unknown {
            return Err(invalid(
                "stream statistics are renderer-owned; register them as Unknown",
            ));
        }
        validate_statistics(self.len, self.statistics).map_err(invalid)
    }
}

pub(crate) fn validate_statistics(
    len: u64,
    statistics: StreamStatistics,
) -> std::result::Result<(), &'static str> {
    if let StreamStatistics::Known(Some(bounds)) = statistics {
        if len == 0
            || !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || bounds.min > bounds.max
            || bounds
                .min_positive
                .is_some_and(|v| !v.is_finite() || v <= 0.0 || v < bounds.min || v > bounds.max)
            || (bounds.max > 0.0) != bounds.min_positive.is_some()
            || (bounds.min > 0.0 && bounds.min_positive != Some(bounds.min))
        {
            return Err("invalid complete source statistics");
        }
    }
    Ok(())
}

/// Borrowed single-authority metadata; no resident stats are mirrored.
pub enum LogicalColumn<'a> {
    Resident(&'a crate::data_render::column_pool::ColumnSlot),
    Streamed(&'a StreamColumn),
}

impl LogicalColumn<'_> {
    pub fn len(&self) -> u64 {
        match self {
            Self::Resident(slot) => slot.len_values as u64,
            Self::Streamed(source) => source.len,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn statistics(&self) -> StreamStatistics {
        match self {
            Self::Streamed(source) => source.statistics,
            Self::Resident(slot) => StreamStatistics::Known(
                (slot.min.is_finite() && slot.max.is_finite() && slot.min <= slot.max).then_some(
                    StreamBounds {
                        min: slot.min,
                        max: slot.max,
                        min_positive: slot.min_positive,
                    },
                ),
            ),
        }
    }
}
