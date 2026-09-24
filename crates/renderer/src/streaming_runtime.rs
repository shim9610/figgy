//! Renderer-owned bounded replay and display execution. Frozen execution
//! snapshots derive from the chart registry; source payloads stay with the host.
//! Draw progress advances only when the corresponding GPU work is submitted.

use super::streaming_surface::{
    StreamSurface, StreamSurfaceError, StreamSurfaceSpec, StreamTransfer,
};
use super::*;
use crate::streaming::{
    ColumnInput, ColumnRange, JobId, RequestStatus, RequestTicket, SourceStamp, StreamError,
    StreamLimits, StreamScheduler, SubmissionReceipt, ViewEpoch,
};
use crate::streaming_upload::{
    ChunkStatisticsPlan, ChunkUploadBudget, ChunkUploadError, RecordedChunk,
    record_chunk_collecting_statistics, record_source_chunk_collecting_statistics,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "streaming_auxiliary.rs"]
mod auxiliary;
#[path = "streaming_arc_runtime.rs"]
mod arc_runtime;
#[path = "streaming_field_runtime.rs"]
mod field_runtime;
#[path = "streaming_residency.rs"]
mod residency;
pub use residency::StreamingResidencyOperation;
#[path = "streaming_field_fit.rs"]
mod field_fit;
#[path = "streaming_field_pick.rs"]
mod field_pick;
#[path = "streaming_selection.rs"]
mod selection;
pub use selection::{StreamingSelectionRequest, StreamingSelectionTicket};

pub(super) fn is_stream_heatmap(rt: &DataRenderType) -> bool {
    match rt {
        DataRenderType::Heatmap { .. } => true,
        DataRenderType::Scatter { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } => false,
    }
}

#[derive(Debug)]
pub(crate) enum StreamRequestError {
    State(FiggyError),
    Scheduler(StreamError),
    Upload(ChunkUploadError),
    Surface(StreamSurfaceError),
    Source {
        id: String,
        error: crate::ColumnRangeWriteError,
    },
}
impl From<StreamSurfaceError> for StreamRequestError {
    fn from(error: StreamSurfaceError) -> Self {
        Self::Surface(error)
    }
}
impl From<FiggyError> for StreamRequestError {
    fn from(e: FiggyError) -> Self {
        Self::State(e)
    }
}
impl From<StreamError> for StreamRequestError {
    fn from(e: StreamError) -> Self {
        Self::Scheduler(e)
    }
}
impl StreamRequestError {
    pub(crate) fn into_figgy(self) -> FiggyError {
        match self {
            Self::State(error) => error,
            Self::Scheduler(StreamError::Overflow) => FiggyError::CounterExhausted {
                counter: "stream display serial",
            },
            Self::Scheduler(StreamError::InvalidLimits) => FiggyError::InvalidConfig {
                field: "streaming limits",
                reason: "limits must be non-zero and chunk bytes must fit the in-flight budget",
            },
            Self::Scheduler(StreamError::TooManyJobs) => FiggyError::StateAllocationFailed {
                resource: "streaming chart slots",
                reason: "configured active chart limit reached".into(),
            },
            Self::Scheduler(StreamError::TooLarge) => FiggyError::StateAllocationFailed {
                resource: "streaming request",
                reason: "request exceeds the configured streaming or device limit".into(),
            },
            Self::Scheduler(StreamError::AllocationFailed) => FiggyError::StateAllocationFailed {
                resource: "streaming metadata",
                reason: "host allocation failed".into(),
            },
            Self::Scheduler(error) => FiggyError::StaleStateToken {
                reason: format!("stream job is not current or ready: {error:?}"),
            },
            Self::Upload(error) => FiggyError::GpuResourceAllocationFailed {
                resource: "stream chunk upload",
                reason: format!("{error:?}"),
            },
            Self::Surface(error) => FiggyError::GpuResourceAllocationFailed {
                resource: "stream display surface",
                reason: format!("{error:?}"),
            },
            Self::Source { id, error } => FiggyError::InvalidStreamSource {
                id,
                reason: error.reason(),
            },
        }
    }
}
pub(crate) type StreamResult<T> = std::result::Result<T, StreamRequestError>;

impl From<crate::StreamingLimits> for StreamLimits {
    fn from(value: crate::StreamingLimits) -> Self {
        Self {
            max_jobs: value.max_active_charts,
            max_slots: value.max_in_flight_chunks,
            max_columns_per_request: value.max_columns_per_chunk,
            max_chunk_bytes: value.max_chunk_input_bytes,
            max_in_flight_bytes: value.max_in_flight_gpu_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamJob {
    chart: ChartId,
    id: JobId,
}

/// Opaque renderer-bound identity for an independent exact replay operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingOperation(StreamJob);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamTicket {
    job: StreamJob,
    ticket: RequestTicket,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamRequestStatus {
    Ready(StreamTicket),
    Backpressure,
}
pub(crate) struct StreamSourceRange<'a> {
    pub column: &'a str,
    pub offset: u64,
    pub len: u64,
}
#[derive(Debug)]
pub(crate) struct StreamRequestedColumn {
    pub column: String,
    pub range: ColumnRange,
}
pub(crate) struct StreamInput<'a> {
    pub column: &'a str,
    /// Little-endian scalar f32 or interleaved hi/lo f32 pairs, per request.
    pub bytes: &'a [u8],
}
#[derive(Clone, Copy)]
struct StreamSourceInput<'a> {
    id: &'a str,
    revision: u64,
    source_len: u64,
    source_offset: Option<u64>,
    source: crate::StreamColumnSource<'a>,
}
#[derive(Clone, Copy)]
enum StreamSupply<'a> {
    Encoded(&'a [StreamInput<'a>]),
    Sources(&'a [StreamSourceInput<'a>]),
}
struct NamedRequest {
    ticket: StreamTicket,
    columns: Vec<StreamRequestedColumn>,
    selection: bool,
}
struct StreamCompletion {
    receipt: SubmissionReceipt,
    done: Arc<AtomicBool>,
}

/// Frozen renderer-owned input for one automatic stream. Payloads deliberately
/// stay outside this snapshot: the host binds the exact captured revisions for
/// each bounded step, while all renderer state and GPU view resources used by
/// the execution live here until its final display is published.
pub(super) struct AutoStreamExecutionSnapshot {
    pub(super) desired: RenderRevision,
    data_revision: RenderRevision,
    series_revision: RenderRevision,
    view_revision: RenderRevision,
    target: StreamTargetKey,
    config: Config,
    /// Document pixel dimensions retained independently of the display transform.
    document_config: Config,
    display_scale: f32,
    pub(super) series: Vec<SeriesConfig>,
    styles: RegisteredChartStyles,
    pub(super) sources: HashMap<ColumnId, crate::StreamColumn>,
    pub(super) view: ChartView,
    /// Present only while a new streamed revision is being rendered beside
    /// the still-authoritative resident closure.
    handoff: Option<Arc<ResidentStreamHandoff>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamTargetKey {
    size: (u32, u32),
    format: wgpu::TextureFormat,
    sample_count: u32,
    clear_color: [u32; 4],
    max_primitives_per_chunk: u64,
}

impl StreamTargetKey {
    fn new(
        size: (u32, u32),
        format: wgpu::TextureFormat,
        sample_count: u32,
        clear_color: crate::Color,
        max_primitives_per_chunk: u64,
    ) -> Self {
        Self {
            size,
            format,
            sample_count,
            clear_color: [
                clear_color.r.to_bits(),
                clear_color.g.to_bits(),
                clear_color.b.to_bits(),
                clear_color.a.to_bits(),
            ],
            max_primitives_per_chunk,
        }
    }
}

enum StreamExecutionMode {
    Explicit,
    Auto {
        /// Temporarily `None` only while the renderer rewrites the job-owned
        /// auto-fit view. No host callback can observe that internal section.
        snapshot: Option<Arc<AutoStreamExecutionSnapshot>>,
        /// Per-revision statistics collected by the same staging writes that
        /// feed this execution. They survive cursor rewinds and never read the
        /// source a second time merely to recover bounds.
        statistics: HashMap<ColumnId, StreamStatisticsCache>,
        auto_fit_padding: Option<f64>,
        final_fit_published: bool,
        published_revision: Option<RenderRevision>,
        /// The only queued desired-state identity. Repeated draw requests
        /// overwrite this value and retain no additional state snapshots.
        pending_latest: Option<RenderRevision>,
        cancel_requested: bool,
        all_submitted: bool,
        final_display_published: bool,
    },
}
struct PlannedStatisticsSource {
    request_index: usize,
    cache: StreamStatisticsCache,
}
struct PreparedStreamStatistics {
    plans: Vec<ChunkStatisticsPlan>,
    sources: Vec<PlannedStatisticsSource>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamDrawRequestStatus {
    Ready(StreamTicket),
    Backpressure,
    /// Every primitive was submitted, not necessarily completed on the GPU.
    AllSubmitted,
}

struct StreamDrawCursor {
    job: StreamJob,
    series: usize,
    phase_index: usize,
    offset: u64,
    max_primitives: u64,
    max_primitives_limit: u64,
    pending: Option<StreamTicket>,
    view_revision: Arc<std::sync::atomic::AtomicU64>,
    expected_view_revision: u64,
    display_view_revision: Arc<std::sync::atomic::AtomicU64>,
    displayed_view_revision: u64,
    // The internal caller owns this target's allocation charge and initializes
    // its prefix before beginning. No display snapshot is produced here.
    target: wgpu::Texture,
    surface: Option<StreamSurface>,
    display_bind_group: Option<wgpu::BindGroup>,
    surface_clear: Option<wgpu::Color>,
    display_dirty: bool,
    display_serial: u64,
    hist_envelope: Option<Arc<data_render::bar_envelope::Persistent>>,
    hist_series: Option<usize>,
    hist_overlay: bool,
    mode: StreamExecutionMode,
    auxiliary: Option<Arc<auxiliary::StreamAuxiliaryTarget>>,
    auxiliary_pick: Option<auxiliary::AuxiliaryPick>,
    arc: Option<arc_runtime::StreamArcState>,
    field: Option<field_runtime::StreamFieldState>,
    field_fit: Option<field_fit::StreamFieldFit>,
    field_fits: HashMap<usize, Option<crate::gpu_errorbar::GpuSeriesExtent>>,
    selection: selection::StreamSelectionState,
}

pub(super) struct StreamDisplayPacket {
    pub job: StreamJob,
    pub bind_group: wgpu::BindGroup,
    pub _charge: crate::gpu_memory::SharedCharge,
    pub serial: u64,
    pub size: (u32, u32),
}
pub(super) struct StreamRuntime {
    scheduler: StreamScheduler,
    requests: Vec<NamedRequest>,
    completions: Vec<StreamCompletion>,
    draws: Vec<StreamDrawCursor>,
    residencies: Vec<residency::StreamResidency>,
    retired_status: Vec<(ChartId, crate::StreamingStatus)>,
    target_generation: u64,
    transfer: Option<StreamTransfer>,
    #[cfg(test)]
    reject_next_completion_reserve: bool,
}

impl StreamRuntime {
    fn has_data_slots(&self, job: JobId) -> bool {
        self.requests.iter().any(|request| request.ticket.job.id == job && !request.selection
            && self.scheduler.contains_ticket(request.ticket.ticket))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StreamDrawPhase {
    Field,
    Histogram,
    Errorbar,
    Line,
    Scatter,
}

impl StreamDrawPhase {
    pub(super) fn halo(self) -> u64 {
        match self {
            Self::Field => 0,
            Self::Histogram => 1,
            Self::Errorbar => 0,
            Self::Line => 1,
            Self::Scatter => 0,
        }
    }
}

impl StreamDrawCursor {
    fn auto_snapshot(&self) -> Option<&Arc<AutoStreamExecutionSnapshot>> {
        if let Some(auxiliary) = &self.auxiliary {
            return Some(&auxiliary.snapshot);
        }
        match &self.mode {
            StreamExecutionMode::Explicit => None,
            StreamExecutionMode::Auto { snapshot, .. } => snapshot.as_ref(),
        }
    }

    fn auto_terminal(&self, runtime: &StreamRuntime) -> bool {
        matches!(
            &self.mode,
            StreamExecutionMode::Auto {
                all_submitted: true,
                final_display_published: true,
                ..
            }
        ) && !runtime.has_data_slots(self.job.id)
    }
}

pub(super) fn histogram_edge_column(series: &SeriesConfig) -> Option<&str> {
    let DataRenderType::Histogram { bar } = &series.render_type else {
        return None;
    };
    Some(match bar.orientation {
        crate::data_config::BarOrientation::Vertical => &series.x_column,
        crate::data_config::BarOrientation::Horizontal => &series.y_column,
    })
}

fn stream_phase_total(
    series: &SeriesConfig,
    phase: StreamDrawPhase,
    mut source_len: impl FnMut(&str) -> Option<u64>,
) -> StreamResult<u64> {
    if phase == StreamDrawPhase::Field {
        field_runtime::field_total(series, source_len)
    } else if phase == StreamDrawPhase::Histogram {
        let edge_id = histogram_edge_column(series).ok_or(StreamError::InvalidRange)?;
        let value_id = if edge_id == series.x_column {
            series.y_column.as_str()
        } else {
            series.x_column.as_str()
        };
        let edges = source_len(edge_id).ok_or(StreamError::InvalidRange)?;
        let values = source_len(value_id).ok_or(StreamError::InvalidRange)?;
        Ok(edges.saturating_sub(1).min(values))
    } else {
        let columns = stream_phase_columns(series, phase);
        let mut total = u64::MAX;
        for id in &columns.ids[..columns.data_count] {
            total = total.min(source_len(id).ok_or(StreamError::InvalidRange)?);
        }
        Ok(total.saturating_sub(phase.halo()))
    }
}

fn stream_phase_request_len(
    series: &SeriesConfig,
    phase: StreamDrawPhase,
    id: &str,
    primitives: u64,
) -> StreamResult<u64> {
    if phase == StreamDrawPhase::Histogram {
        if histogram_edge_column(series) == Some(id) {
            primitives
                .checked_add(1)
                .ok_or(StreamError::Overflow.into())
        } else {
            Ok(primitives)
        }
    } else {
        primitives
            .checked_add(phase.halo())
            .ok_or(StreamError::Overflow.into())
    }
}

pub(super) struct StreamPhaseColumns<'a> {
    pub ids: [&'a str; 7],
    pub count: usize,
    pub data_count: usize,
    pub style_index: Option<&'a str>,
}

fn push_unique_phase_column<'a>(id: &'a str, ids: &mut [&'a str; 7], count: &mut usize) {
    if !ids[..*count].contains(&id) {
        ids[*count] = id;
        *count += 1;
    }
}

pub(super) fn stream_phase_columns(
    series: &SeriesConfig,
    phase: StreamDrawPhase,
) -> StreamPhaseColumns<'_> {
    let x = series.x_column.as_str();
    let y = series.y_column.as_str();
    let mut ids = [x; 7];
    ids[0] = x;
    let mut count = 1;
    push_unique_phase_column(y, &mut ids, &mut count);
    if phase == StreamDrawPhase::Errorbar {
        for error in [
            extract_err_y(&series.render_type),
            extract_err_x(&series.render_type),
        ]
        .into_iter()
        .flatten()
        {
            match error {
                ErrorRef::Symmetric { column } => {
                    push_unique_phase_column(column, &mut ids, &mut count)
                }
                ErrorRef::Asymmetric { lower, upper } => {
                    push_unique_phase_column(lower, &mut ids, &mut count);
                    push_unique_phase_column(upper, &mut ids, &mut count);
                }
            }
        }
    }
    let data_count = count;
    let style_index = match phase {
        StreamDrawPhase::Scatter => extract_scatter(&series.render_type)
            .and_then(|style| style.point_style_index_column.as_deref()),
        StreamDrawPhase::Errorbar => extract_errorbar_style(&series.render_type)
            .and_then(|style| style.error_bar_style_index_column.as_deref()),
        StreamDrawPhase::Line | StreamDrawPhase::Histogram | StreamDrawPhase::Field => None,
    };
    if let Some(id) = style_index {
        push_unique_phase_column(id, &mut ids, &mut count);
    }
    StreamPhaseColumns {
        ids,
        count,
        data_count,
        style_index,
    }
}

fn mapped_stream_base_bytes(series: &SeriesConfig, device: &wgpu::Device) -> StreamResult<u64> {
    let mut total = 0u64;
    let max_binding = u64::from(device.limits().max_storage_buffer_binding_size)
        .min(device.limits().max_buffer_size);
    let maps = [
        extract_scatter(&series.render_type).map(|scatter| {
            (
                scatter.point_style_index_column.is_some(),
                scatter.point_style_table.as_ref().map_or(0, Vec::len),
                scatter.point_style_overrides.as_ref().map_or(0, |rows| {
                    rows.iter()
                        .filter(|row| u32::try_from(row.index).is_ok())
                        .count()
                }),
            )
        }),
        extract_errorbar_style(&series.render_type).map(|style| {
            (
                style.error_bar_style_index_column.is_some(),
                style.error_bar_style_table.as_ref().map_or(0, Vec::len),
                style.error_bar_style_overrides.as_ref().map_or(0, |rows| {
                    rows.iter()
                        .filter(|row| u32::try_from(row.index).is_ok())
                        .count()
                }),
            )
        }),
    ];
    for (has_index, slots, overrides) in maps.into_iter().flatten() {
        if !has_index && slots == 0 && overrides == 0 {
            continue;
        }
        let slots = u64::try_from(slots).map_err(|_| StreamError::TooLarge)?;
        let overrides = u64::try_from(overrides).map_err(|_| StreamError::TooLarge)?;
        if slots > u64::from(u32::MAX) || overrides > u64::from(u32::MAX) {
            return Err(StreamError::TooLarge.into());
        }
        let style_bytes = slots.max(1).checked_mul(32).ok_or(StreamError::Overflow)?;
        let override_bytes = overrides
            .max(1)
            .checked_mul(48)
            .ok_or(StreamError::Overflow)?;
        if style_bytes > max_binding || override_bytes > max_binding {
            return Err(StreamError::TooLarge.into());
        }
        total = total
            .checked_add(style_bytes)
            .and_then(|bytes| bytes.checked_add(override_bytes))
            .and_then(|bytes| bytes.checked_add(16))
            .ok_or(StreamError::Overflow)?;
    }
    Ok(total)
}

fn charge_mapped_stream_bases(
    styles: &mut RegisteredChartStyles,
    ledger: &Arc<crate::gpu_memory::GpuLedger>,
) {
    for style in &mut styles.styles {
        if let Some(map) = &mut style.scatter_map {
            map.charge_stream_base(ledger);
        }
        if let Some(map) = &mut style.errorbar_map {
            map.charge_stream_base(ledger);
        }
    }
}

fn stream_draw_phases(draw_style: &DrawStyle, series: &SeriesConfig) -> StreamResult<&'static [StreamDrawPhase]> {
    let primitives = effective_series_primitives(draw_style, &series.render_type);
    if matches!(draw_style, DrawStyle::Constellation(_)) {
        return Ok(if primitives.line { &[StreamDrawPhase::Line, StreamDrawPhase::Scatter] } else { &[] });
    }
    match &series.render_type {
        DataRenderType::Histogram { .. } => Ok(if primitives.bar { &[StreamDrawPhase::Histogram] } else { &[] }),
        DataRenderType::Line { .. } => Ok(&[StreamDrawPhase::Line]),
        DataRenderType::Scatter { .. } => Ok(&[StreamDrawPhase::Scatter]),
        DataRenderType::ScatterLine { .. } => {
            Ok(&[StreamDrawPhase::Line, StreamDrawPhase::Scatter])
        }
        DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. } => {
            Ok(&[StreamDrawPhase::Errorbar, StreamDrawPhase::Scatter])
        }
        DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. } => Ok(&[
            StreamDrawPhase::Errorbar,
            StreamDrawPhase::Line,
            StreamDrawPhase::Scatter,
        ]),
        DataRenderType::Heatmap { .. } => Ok(if primitives.field { &[StreamDrawPhase::Field] } else { &[] }),
        DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } if primitives.field || primitives.contour => Err(FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: "stream draw cursor does not yet support field or contour passes".into(),
        }
        .into()),
        DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } => Ok(&[]),
    }
}

fn stream_fit_extent(bounds: crate::StreamBounds) -> crate::FitExtent {
    crate::FitExtent {
        min: bounds.min,
        max: bounds.max,
        min_positive: bounds.min_positive,
    }
}

fn stream_cached_extent(
    statistics: &HashMap<ColumnId, StreamStatisticsCache>,
    id: &str,
) -> Option<crate::FitExtent> {
    statistics
        .get(id)
        .and_then(|cache| cache.bounds)
        .map(stream_fit_extent)
}

fn stream_error_extent(
    statistics: &HashMap<ColumnId, StreamStatisticsCache>,
    values: &str,
    error: &ErrorRef,
) -> Option<crate::FitExtent> {
    let value = stream_cached_extent(statistics, values)?;
    let (lower, upper) = match error {
        ErrorRef::Symmetric { column } => (
            stream_cached_extent(statistics, column)?,
            stream_cached_extent(statistics, column)?,
        ),
        ErrorRef::Asymmetric { lower, upper } => (
            stream_cached_extent(statistics, lower)?,
            stream_cached_extent(statistics, upper)?,
        ),
    };
    let min = value.min - lower.max;
    let max = value.max + upper.max;
    if !min.is_finite() || !max.is_finite() || min > max {
        return None;
    }
    let mut min_positive = value.min_positive;
    for candidate in [min, max] {
        if candidate > 0.0
            && candidate.is_finite()
            && min_positive.is_none_or(|current| candidate < current)
        {
            min_positive = Some(candidate);
        }
    }
    Some(crate::FitExtent {
        min,
        max,
        min_positive,
    })
}

fn auto_stream_fit_extents(
    snapshot: &AutoStreamExecutionSnapshot,
    statistics: &HashMap<ColumnId, StreamStatisticsCache>,
    field_fits: &HashMap<usize, Option<crate::gpu_errorbar::GpuSeriesExtent>>,
) -> (crate::FitExtent, crate::FitExtent) {
    let mut x = crate::FitExtent::EMPTY;
    let mut y = crate::FitExtent::EMPTY;
    for (index, series) in snapshot.series.iter().enumerate() {
        if is_stream_heatmap(&series.render_type) {
            if let Some(Some(extent)) = field_fits.get(&index) {
                x.union(&crate::FitExtent { min: extent.x.min, max: extent.x.max, min_positive: extent.x.min_positive });
                y.union(&crate::FitExtent { min: extent.y.min, max: extent.y.max, min_positive: extent.y.min_positive });
            }
            continue;
        }
        if let Some(extent) = stream_cached_extent(statistics, &series.x_column) {
            x.union(&extent);
        }
        if let Some(extent) = stream_cached_extent(statistics, &series.y_column) {
            y.union(&extent);
        }
        if let Some(error) = extract_err_x(&series.render_type)
            && let Some(extent) = stream_error_extent(statistics, &series.x_column, error)
        {
            x.union(&extent);
        }
        if let Some(error) = extract_err_y(&series.render_type)
            && let Some(extent) = stream_error_extent(statistics, &series.y_column, error)
        {
            y.union(&extent);
        }
        if let DataRenderType::Histogram { bar } = &series.render_type {
            let baseline = crate::FitExtent {
                min: bar.baseline,
                max: bar.baseline,
                min_positive: (bar.baseline > 0.0).then_some(bar.baseline),
            };
            match bar.orientation {
                crate::data_config::BarOrientation::Vertical => y.union(&baseline),
                crate::data_config::BarOrientation::Horizontal => x.union(&baseline),
            }
        }
    }
    (x, y)
}

fn resolve_stream_source<'a>(
    requested: &StreamRequestedColumn,
    bindings: &[crate::StreamSourceBinding<'a>],
) -> Result<StreamSourceInput<'a>> {
    let mut found = None;
    for binding in bindings {
        if binding.id == requested.column {
            if found.is_some() {
                return Err(FiggyError::InvalidStreamSource {
                    id: requested.column.clone(),
                    reason: "duplicate source binding",
                });
            }
            found = Some(*binding);
        }
    }
    let binding = found.ok_or_else(|| FiggyError::UnknownColumn {
        id: requested.column.clone(),
    })?;
    if binding.revision != requested.range.revision {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "source binding revision does not match registration",
        });
    }
    if binding.source.encoding() != requested.range.encoding {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "source binding encoding does not match registration",
        });
    }
    if u64::try_from(binding.source.len()).ok() != Some(requested.range.source_len) {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "source binding length does not match registration",
        });
    }
    Ok(StreamSourceInput {
        id: binding.id,
        revision: binding.revision,
        source_len: requested.range.source_len,
        source_offset: None,
        source: binding.source,
    })
}

fn resolve_stream_range_source<'a>(
    requested: &StreamRequestedColumn,
    bindings: &[crate::StreamRangeSourceBinding<'a>],
) -> Result<StreamSourceInput<'a>> {
    let mut found = None;
    let mut same_id = None;
    for binding in bindings {
        if binding.id == requested.column {
            same_id.get_or_insert(*binding);
            if binding.offset != requested.range.offset
                || u64::try_from(binding.source.len()).ok() != Some(requested.range.len)
            {
                continue;
            }
            if found.is_some() {
                return Err(FiggyError::InvalidStreamSource {
                    id: requested.column.clone(),
                    reason: "duplicate range source binding",
                });
            }
            found = Some(*binding);
        }
    }
    let binding = found.or(same_id).ok_or_else(|| FiggyError::UnknownColumn {
        id: requested.column.clone(),
    })?;
    if binding.revision != requested.range.revision {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "range source revision does not match registration",
        });
    }
    if binding.source_len != requested.range.source_len {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "range source logical length does not match registration",
        });
    }
    if binding.offset != requested.range.offset {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "range source offset does not match the pending request",
        });
    }
    if binding.source.encoding() != requested.range.encoding {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "range source encoding does not match registration",
        });
    }
    if u64::try_from(binding.source.len()).ok() != Some(requested.range.len) {
        return Err(FiggyError::InvalidStreamSource {
            id: requested.column.clone(),
            reason: "range source payload length does not match the pending request",
        });
    }
    Ok(StreamSourceInput {
        id: binding.id,
        revision: binding.revision,
        source_len: binding.source_len,
        source_offset: Some(binding.offset),
        source: binding.source,
    })
}

impl Renderer {
    pub(super) fn try_cached_stream_auto_fit(&mut self, chart: ChartId, padding: f64) -> Result<bool> {
        let config = {
            let Some(runtime) = self.stream_runtime.as_ref() else { return Ok(false); };
            let Some(draw) = runtime.draws.iter().find(|draw| draw.job.chart == chart && draw.auxiliary.is_none()) else { return Ok(false); };
            let StreamExecutionMode::Auto { snapshot: Some(snapshot), statistics, .. } = &draw.mode else { return Ok(false); };
            let state = self.chart_states.get(&chart).ok_or(FiggyError::UnknownChart { id: chart })?;
            if !draw.auto_terminal(runtime)
                || snapshot.data_revision != state.revisions.data
                || snapshot.series_revision != state.revisions.series
                || snapshot.view_revision != state.revisions.view
                || !field_fit::all_fit_inputs_ready(snapshot, statistics, &draw.field_fits)
            { return Ok(false); }
            let (x, y) = auto_stream_fit_extents(snapshot, statistics, &draw.field_fits);
            let mut config = state.config.clone();
            crate::chart::apply_auto_fit_all(&mut config, &x, &y, padding);
            config
        };
        if !stream_config_equal(self.chart_config(chart)?, &config) {
            self.set_chart_config(chart, config)?;
        }
        self.chart_states.get_mut(&chart).unwrap().stream_auto_fit_padding = None;
        Ok(true)
    }

    pub(super) fn auto_stream_snapshot(
        &self,
        job: StreamJob,
    ) -> Option<Arc<AutoStreamExecutionSnapshot>> {
        self.stream_runtime
            .as_ref()?
            .draws
            .iter()
            .find(|draw| draw.job == job)?
            .auto_snapshot()
            .cloned()
    }

    fn pending_auto_fit_config(&self, job: StreamJob) -> StreamResult<Option<Config>> {
        let draw = self
            .stream_runtime
            .as_ref()
            .and_then(|runtime| runtime.draws.iter().find(|draw| draw.job == job))
            .ok_or(StreamError::Stale)?;
        let StreamExecutionMode::Auto {
            snapshot,
            statistics,
            auto_fit_padding,
            ..
        } = &draw.mode
        else {
            return Ok(None);
        };
        let Some(padding) = *auto_fit_padding else {
            return Ok(None);
        };
        let snapshot = snapshot.as_ref().ok_or(StreamError::WrongState)?;
        let (x, y) = auto_stream_fit_extents(snapshot, statistics, &draw.field_fits);
        let mut config = snapshot.config.clone();
        crate::chart::apply_auto_fit_all(&mut config, &x, &y, padding);
        Ok((!stream_config_equal(&snapshot.config, &config)).then_some(config))
    }

    fn restart_auto_stream_with_config(
        &mut self,
        job: StreamJob,
        config: Config,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.discard_pending_selection(job)?;
        let snapshot = {
            let draw = self
                .stream_runtime
                .as_mut()
                .and_then(|runtime| runtime.draws.iter_mut().find(|draw| draw.job == job))
                .ok_or(StreamError::Stale)?;
            let StreamExecutionMode::Auto { snapshot, .. } = &mut draw.mode else {
                return Err(StreamError::WrongState.into());
            };
            snapshot.take().ok_or(StreamError::WrongState)?
        };
        let mut snapshot = match Arc::try_unwrap(snapshot) {
            Ok(snapshot) => snapshot,
            Err(snapshot) => {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap();
                let StreamExecutionMode::Auto { snapshot: slot, .. } = &mut draw.mode else {
                    unreachable!()
                };
                *slot = Some(snapshot);
                return Err(FiggyError::StaleStateToken {
                    reason: "automatic stream snapshot was retained across an axis restart"
                        .into(),
                }
                .into());
            }
        };

        let chart = Chart::new(config.clone());
        let panel = snapshot.view.panel_rect;
        let updated = self
            .refresh_axis(&mut snapshot.view, &chart, panel)
            .and_then(|()| self.update_transform(&snapshot.view, &chart));
        if let Err(error) = updated {
            let draw = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap();
            let StreamExecutionMode::Auto { snapshot: slot, .. } = &mut draw.mode else {
                unreachable!()
            };
            *slot = Some(Arc::new(snapshot));
            return Err(error.into());
        }
        snapshot.config = config;
        snapshot.document_config.top_x.min = snapshot.config.top_x.min;
        snapshot.document_config.top_x.max = snapshot.config.top_x.max;
        snapshot.document_config.bottom_x.min = snapshot.config.bottom_x.min;
        snapshot.document_config.bottom_x.max = snapshot.config.bottom_x.max;
        snapshot.document_config.left_y.min = snapshot.config.left_y.min;
        snapshot.document_config.left_y.max = snapshot.config.left_y.max;
        snapshot.document_config.right_y.min = snapshot.config.right_y.min;
        snapshot.document_config.right_y.max = snapshot.config.right_y.max;

        let (target, clear) = {
            let draw = self
                .stream_runtime
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .unwrap();
            (
                draw.target.clone(),
                draw.surface_clear.ok_or(StreamError::WrongState)?,
            )
        };
        let target_view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy automatic stream axis restart"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some(panel) = data_render::clamp_rect_to_target(snapshot.view.panel_rect, (
                target.width(),
                target.height(),
            )) {
                pass.set_viewport(
                    panel.x as f32,
                    panel.y as f32,
                    panel.width as f32,
                    panel.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                pass.set_pipeline(&self.pipelines.axis);
                pass.set_bind_group(0, &snapshot.view.grid_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        let expected_view_revision = snapshot.view.stream_revision.load(Ordering::Acquire);
        let snapshot = Arc::new(snapshot);
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        draw.series = 0;
        draw.phase_index = 0;
        draw.offset = 0;
        draw.pending = None;
        draw.expected_view_revision = expected_view_revision;
        draw.display_dirty = true;
        draw.hist_envelope = None;
        draw.hist_series = None;
        draw.hist_overlay = false;
        draw.arc = None;
        draw.field = None;
        draw.field_fit = None;
        draw.selection = selection::StreamSelectionState::default();
        let StreamExecutionMode::Auto {
            snapshot: slot,
            all_submitted,
            final_display_published,
            ..
        } = &mut draw.mode
        else {
            unreachable!()
        };
        *slot = Some(snapshot);
        *all_submitted = false;
        *final_display_published = false;
        Ok(self.queue.submit([encoder.finish()]))
    }

    fn publish_auto_stream_completion(&mut self, job: StreamJob) -> StreamResult<RenderRevision> {
        let (snapshot_desired, snapshot_config, auto_fit_padding, statistic_copies, handoff) = {
            let draw = self
                .stream_runtime
                .as_ref()
                .and_then(|runtime| runtime.draws.iter().find(|draw| draw.job == job))
                .ok_or(StreamError::Stale)?;
            match &draw.mode {
                StreamExecutionMode::Auto {
                    snapshot,
                    statistics,
                    auto_fit_padding,
                    final_fit_published,
                    published_revision,
                    ..
                } => {
                    if *final_fit_published {
                        return Ok(published_revision
                            .unwrap_or(snapshot.as_ref().ok_or(StreamError::WrongState)?.desired));
                    }
                    let snapshot = snapshot.as_ref().ok_or(StreamError::WrongState)?;
                    let mut copies = Vec::new();
                    copies
                        .try_reserve(statistics.len())
                        .map_err(|_| StreamError::AllocationFailed)?;
                    for (id, cache) in statistics {
                        let source = snapshot.sources.get(id).ok_or(StreamError::Stale)?;
                        if !cache.covers(0..source.len) {
                            continue;
                        }
                        if snapshot.handoff.is_some() {
                            copies.push((
                                id.clone(),
                                cache
                                    .try_coverage_copy()
                                    .map_err(|_| StreamError::AllocationFailed)?,
                            ));
                            continue;
                        }
                        let Some(live) = self.streaming_sources.get(id) else {
                            continue;
                        };
                        if live.revision != source.revision
                            || matches!(live.statistics, crate::StreamStatistics::Known(_))
                        {
                            continue;
                        }
                        copies.push((
                            id.clone(),
                            cache
                                .try_coverage_copy()
                                .map_err(|_| StreamError::AllocationFailed)?,
                        ));
                    }
                    (
                        snapshot.desired,
                        snapshot.config.clone(),
                        *auto_fit_padding,
                        copies,
                        snapshot.handoff.clone(),
                    )
                }
                StreamExecutionMode::Explicit => return Err(StreamError::WrongState.into()),
            }
        };
        if let Some(handoff) = handoff {
            let revision = match self.commit_completed_resident_stream_handoff(
                job.chart,
                &handoff,
                &statistic_copies,
                &snapshot_config,
                auto_fit_padding,
            ) {
                Ok(revision) => revision,
                Err(error) => {
                    let _ = self.cancel_chart_stream(job.chart);
                    return Err(error.into());
                }
            };
            let draw = self
                .stream_runtime
                .as_mut()
                .and_then(|runtime| runtime.draws.iter_mut().find(|draw| draw.job == job))
                .ok_or(StreamError::Stale)?;
            let StreamExecutionMode::Auto {
                snapshot,
                auto_fit_padding,
                final_fit_published,
                published_revision,
                ..
            } = &mut draw.mode
            else {
                return Err(StreamError::WrongState.into());
            };
            *final_fit_published = true;
            *published_revision = Some(revision);
            *auto_fit_padding = None;
            let state = &self.chart_states[&job.chart];
            let snapshot = Arc::get_mut(snapshot.as_mut().expect("handoff retains snapshot"))
                .expect("completion exclusively owns the handoff snapshot");
            snapshot.desired = revision;
            snapshot.data_revision = state.revisions.data;
            snapshot.series_revision = state.revisions.series;
            snapshot.view_revision = state.revisions.view;
            snapshot.handoff = None;
            return Ok(revision);
        }
        let statistic_count = u64::try_from(statistic_copies.len())
            .map_err(|_| StreamError::Overflow)?;
        let final_fit_epoch = self
            .next_stream_source_fit_epoch
            .checked_add(statistic_count)
            .ok_or(FiggyError::CounterExhausted {
                counter: "stream source fit epoch",
            })?;

        let publish_config = auto_fit_padding.is_some()
            && self
                .chart_states
                .get(&job.chart)
                .is_some_and(|state| state.revisions.desired == snapshot_desired);
        let config_revisions = if publish_config {
            let state = self
                .chart_states
                .get(&job.chart)
                .ok_or(FiggyError::UnknownChart { id: job.chart })?;
            Some((
                state
                    .revisions
                    .desired
                    .successor("chart desired revision")?,
                state
                    .revisions
                    .config
                    .successor("chart config revision")?,
                state.revisions.view.successor("chart view revision")?,
                state.revisions.raster.successor("chart raster revision")?,
                self.visual_revision.successor("renderer visual revision")?,
            ))
        } else {
            None
        };

        let snapshot = {
            let draw = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap();
            let StreamExecutionMode::Auto { snapshot, .. } = &mut draw.mode else {
                unreachable!()
            };
            snapshot.take().ok_or(StreamError::WrongState)?
        };
        let mut snapshot = match Arc::try_unwrap(snapshot) {
            Ok(snapshot) => snapshot,
            Err(snapshot) => {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap();
                let StreamExecutionMode::Auto { snapshot: slot, .. } = &mut draw.mode else {
                    unreachable!()
                };
                *slot = Some(snapshot);
                return Err(FiggyError::StaleStateToken {
                    reason: "automatic stream snapshot was retained across completion".into(),
                }
                .into());
            }
        };

        let mut fit_epoch = self.next_stream_source_fit_epoch;
        for (id, cache) in statistic_copies {
            fit_epoch += 1;
            let source = self
                .streaming_sources
                .get_mut(&id)
                .expect("validated live source remains registered");
            source.statistics_cache = cache;
            source.column.statistics =
                crate::StreamStatistics::Known(source.statistics_cache.bounds);
            source.fit_epoch = fit_epoch;
        }
        self.next_stream_source_fit_epoch = final_fit_epoch;

        if let Some((desired, config, view, raster, visual)) = config_revisions {
            let state = self
                .chart_states
                .get_mut(&job.chart)
                .ok_or(FiggyError::UnknownChart { id: job.chart })?;
            state.config.top_x.min = snapshot_config.top_x.min;
            state.config.top_x.max = snapshot_config.top_x.max;
            state.config.bottom_x.min = snapshot_config.bottom_x.min;
            state.config.bottom_x.max = snapshot_config.bottom_x.max;
            state.config.left_y.min = snapshot_config.left_y.min;
            state.config.left_y.max = snapshot_config.left_y.max;
            state.config.right_y.min = snapshot_config.right_y.min;
            state.config.right_y.max = snapshot_config.right_y.max;
            state.stream_auto_fit_padding = None;
            state.revisions.desired = desired;
            state.revisions.config = config;
            state.revisions.view = view;
            state.revisions.raster = raster;
            self.visual_revision = visual;
            snapshot.desired = desired;
            snapshot.view_revision = view;
        }
        let revision = snapshot.desired;
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        let StreamExecutionMode::Auto {
            snapshot: slot,
            auto_fit_padding,
            final_fit_published,
            published_revision,
            ..
        } = &mut draw.mode
        else {
            unreachable!()
        };
        *slot = Some(Arc::new(snapshot));
        *final_fit_published = true;
        *published_revision = Some(revision);
        *auto_fit_padding = None;
        Ok(revision)
    }

    pub(super) fn active_stream_job(&self, chart: ChartId) -> Option<StreamJob> {
        self.stream_runtime.as_ref().and_then(|runtime| {
            runtime
                .draws
                .iter()
                .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none() && draw.surface.is_some())
                .map(|draw| draw.job)
        })
    }

    fn stream_job_is_auto(&self, job: StreamJob) -> bool {
        self.auto_stream_snapshot(job).is_some()
    }
}

impl Renderer {
    /// Install bounded streaming resources once. This does not retain a source
    /// reference or start a chart.
    pub fn configure_streaming(&mut self, limits: crate::StreamingLimits) -> Result<()> {
        self.configure_streaming_runtime(limits.into())
            .map_err(StreamRequestError::into_figgy)
    }

    /// Start or replace one chart's renderer-owned stream cursor and bounded
    /// accumulation surface. Partial output is shown by normal
    /// `prepare_registered` calls.
    pub fn begin_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<()> {
        let clear = wgpu::Color {
            r: f64::from(options.clear_color.r),
            g: f64::from(options.clear_color.g),
            b: f64::from(options.clear_color.b),
            a: f64::from(options.clear_color.a),
        };
        self.begin_chart_stream_surface(
            chart,
            view,
            options.size,
            clear,
            options.max_primitives_per_chunk,
        )
        .map(|_| ())
        .map_err(StreamRequestError::into_figgy)
    }

    /// Request automatic execution of the latest renderer-owned chart state.
    ///
    /// Decoration-only requests reuse the active stream. A data, view, target,
    /// or chunk-budget change replaces it only after the new execution has
    /// passed every fallible preflight, then captures the current SSOT once.
    pub fn request_auto_streaming_chart(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        let render_config = self.chart_config(chart)?.clone();
        self.request_auto_streaming_chart_with_config(chart, view, render_config, options)
    }

    /// Automatic streaming with a host-derived display config. The renderer's
    /// chart remains the document/export authority; only the owned execution
    /// snapshot uses this scaled configuration.
    pub fn request_auto_streaming_chart_with_config(
        &mut self,
        chart: ChartId,
        _view: &ChartView,
        render_config: Config,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.request_auto_streaming_chart_with_display_scale(chart, _view, render_config, 1.0, options)
    }

    pub fn request_auto_streaming_chart_with_display_scale(
        &mut self, chart: ChartId, _view: &ChartView, render_config: Config,
        display_scale: f32, options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        if !display_scale.is_finite() || display_scale <= 0.0 {
            return Err(FiggyError::InvalidConfig { field: "stream display scale", reason: "must be finite and positive" });
        }
        crate::chart::validate_renderer_config(&render_config)?;
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        let revisions = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?
            .revisions;
        let desired = revisions.desired;
        let target = StreamTargetKey::new(
            options.size,
            self.surface_format,
            self.target_sample_count,
            options.clear_color,
            options.max_primitives_per_chunk,
        );

        if let Some(job) = self.active_stream_job(chart) {
            let (automatic, terminal, active_revision, pending_latest, inputs_match) = {
                let runtime = self.stream_runtime.as_ref().unwrap();
                let draw = runtime
                    .draws
                    .iter()
                    .find(|draw| draw.job == job)
                    .ok_or_else(|| FiggyError::StaleStateToken {
                        reason: "active stream job has no draw cursor".into(),
                    })?;
                match &draw.mode {
                    StreamExecutionMode::Explicit => (false, false, desired, None, false),
                    StreamExecutionMode::Auto {
                        snapshot,
                        pending_latest,
                        auto_fit_padding,
                        ..
                    } => {
                        let snapshot = snapshot
                            .as_ref()
                            .expect("automatic execution retains its snapshot");
                        (
                            true,
                            draw.auto_terminal(runtime),
                            snapshot.desired,
                            *pending_latest,
                            snapshot.data_revision == revisions.data
                                && snapshot.series_revision == revisions.series
                                && snapshot.view_revision == revisions.view
                                && stream_config_equal(&snapshot.config, &render_config)
                                && snapshot.display_scale.to_bits() == display_scale.to_bits()
                                && *auto_fit_padding == self.chart_states[&chart].stream_auto_fit_padding
                                && snapshot.target == target,
                        )
                    }
                }
            };
            if !automatic {
                return Err(FiggyError::StaleStateToken {
                    reason: "an explicit streaming execution is active for this chart".into(),
                });
            }
            if !terminal && inputs_match {
                return Ok(crate::AutoStreamingRequest::Active {
                    revision: active_revision,
                    pending_latest: None,
                });
            }
            if terminal {
                self.publish_auto_stream_completion(job)
                    .map_err(StreamRequestError::into_figgy)?;
            }
            let current = self
                .chart_states
                .get(&chart)
                .ok_or(FiggyError::UnknownChart { id: chart })?
                .revisions;
            if terminal
                && inputs_match
                && pending_latest.is_none()
                && current.data == revisions.data
                && current.view == revisions.view
            {
                return Ok(crate::AutoStreamingRequest::Complete {
                    revision: current.desired,
                    pending_latest: None,
                });
            }
            // Keep the old execution authoritative while the latest candidate
            // is prepared. `start_job` replaces a same-chart job only after
            // every fallible draw/surface preflight below has succeeded.
        }

        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let desired = state.revisions.desired;
        let data_revision = state.revisions.data;
        let series_revision = state.revisions.series;
        let view_revision = state.revisions.view;
        let config = render_config;
        let series = state.series.clone();
        let auto_fit_padding = state.stream_auto_fit_padding;
        let mut sources = HashMap::new();
        sources
            .try_reserve(self.streaming_sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "automatic stream source snapshot",
                reason: error.to_string(),
            })?;
        let mut source_error = None;
        for declaration in &series {
            visit_series_columns(declaration, &mut |id| {
                if source_error.is_some() || sources.contains_key(id) {
                    return;
                }
                match self.streaming_sources.get(id) {
                    Some(source) => {
                        sources.insert(id.to_owned(), source.column.clone());
                    }
                    None => source_error = Some(id.to_owned()),
                }
            });
        }
        if let Some(id) = source_error {
            return Err(FiggyError::ColumnNotResident { id });
        }
        let mut source_versions = Vec::new();
        source_versions
            .try_reserve_exact(sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "automatic stream source versions",
                reason: error.to_string(),
            })?;
        source_versions.extend(
            sources
                .values()
                .map(|source| crate::AutoStreamSourceVersion {
                    id: source.id.clone(),
                    revision: source.revision,
                }),
        );
        source_versions.sort_unstable_by(|left, right| left.id.cmp(&right.id));

        let mut statistics = HashMap::new();
        statistics
            .try_reserve(sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "automatic stream statistics snapshot",
                reason: error.to_string(),
            })?;
        for id in sources.keys() {
            let cache = self
                .streaming_sources
                .get(id)
                .expect("automatic source snapshot came from the live registry")
                .statistics_cache
                .try_coverage_copy()
                .map_err(|error| FiggyError::StateAllocationFailed {
                    resource: "automatic stream statistics coverage",
                    reason: error.to_string(),
                })?;
            statistics.insert(id.clone(), cache);
        }

        let snapshot_chart = Chart::new(config.clone());
        let owned_view = self.create_chart_view(&snapshot_chart, config.chart_area.0)?;
        let job = self
            .begin_chart_stream_surface_with_sources(
                chart,
                &owned_view,
                options.size,
                wgpu::Color {
                    r: f64::from(options.clear_color.r),
                    g: f64::from(options.clear_color.g),
                    b: f64::from(options.clear_color.b),
                    a: f64::from(options.clear_color.a),
                },
                options.max_primitives_per_chunk,
                None,
                Some((&config, display_scale)),
            )
            .map_err(StreamRequestError::into_figgy)?;
        let styles = self
            .chart_states
            .get_mut(&chart)
            .and_then(|state| state.prepared_styles.take())
            .expect("successful stream-surface preparation publishes chart styles");
        let snapshot = Arc::new(AutoStreamExecutionSnapshot {
            desired,
            document_config: self.chart_states[&chart].config.clone(),
            display_scale,
            data_revision,
            series_revision,
            view_revision,
            target,
            config,
            series,
            styles,
            sources,
            view: owned_view,
            handoff: None,
        });
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .expect("new automatic stream cursor exists");
        draw.mode = StreamExecutionMode::Auto {
            snapshot: Some(snapshot),
            statistics,
            auto_fit_padding,
            final_fit_published: false,
            published_revision: None,
            pending_latest: None,
            cancel_requested: false,
            all_submitted: false,
            final_display_published: false,
        };
        Ok(crate::AutoStreamingRequest::Started {
            revision: desired,
            sources: source_versions,
        })
    }

    /// Render a complete replacement revision beside the current resident
    /// closure. Resident authority is retained until every requested range,
    /// GPU receipt, and final display refresh succeeds; only then is the
    /// closure atomically published as streamed.
    pub fn request_resident_stream_handoff_with_config(
        &mut self,
        chart: ChartId,
        _view: &ChartView,
        render_config: Config,
        columns: Vec<crate::StreamColumn>,
        options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        self.request_resident_stream_handoff_with_display_scale(chart, _view, render_config, 1.0, columns, options)
    }

    pub fn request_resident_stream_handoff_with_display_scale(
        &mut self, chart: ChartId, _view: &ChartView, render_config: Config, display_scale: f32,
        columns: Vec<crate::StreamColumn>, options: crate::StreamingChartOptions,
    ) -> Result<crate::AutoStreamingRequest> {
        if !display_scale.is_finite() || display_scale <= 0.0 {
            return Err(FiggyError::InvalidConfig { field: "stream display scale", reason: "must be finite and positive" });
        }
        crate::chart::validate_renderer_config(&render_config)?;
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        if let Some(job) = self.active_stream_job(chart) {
            let automatic = self
                .stream_runtime
                .as_ref()
                .and_then(|runtime| runtime.draws.iter().find(|draw| draw.job == job))
                .is_some_and(|draw| matches!(draw.mode, StreamExecutionMode::Auto { .. }));
            if !automatic {
                return Err(FiggyError::StaleStateToken {
                    reason: "an explicit streaming execution is active for the resident handoff chart"
                        .into(),
                });
            }
        }

        let handoff = Arc::new(self.prepare_resident_stream_handoff(chart, columns)?);
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let desired = state.revisions.desired;
        let data_revision = state.revisions.data;
        let series_revision = state.revisions.series;
        let view_revision = state.revisions.view;
        let series = state.series.clone();
        let auto_fit_padding = state.stream_auto_fit_padding;
        let target = StreamTargetKey::new(
            options.size,
            self.surface_format,
            self.target_sample_count,
            options.clear_color,
            options.max_primitives_per_chunk,
        );

        // Validate the candidate closure above even when the execution can be
        // reused. Invalid or stale metadata must never take a coalescing shortcut.
        if let Some(runtime) = self.stream_runtime.as_ref()
            && let Some(draw) = runtime.draws.iter().find(|draw| draw.job.chart == chart && draw.auxiliary.is_none())
            && let StreamExecutionMode::Auto {
                snapshot: Some(snapshot), auto_fit_padding: active_padding, ..
            } = &draw.mode
            && snapshot.handoff.as_ref().is_some_and(|current| {
                current.columns.len() == handoff.columns.len()
                    && handoff.columns.iter().all(|column| current.columns.contains(column))
                    && current.allocation_epochs.len() == handoff.allocation_epochs.len()
                    && handoff.allocation_epochs.iter().all(|epoch| current.allocation_epochs.contains(epoch))
                    && current.expected_stream_revisions.len() == handoff.expected_stream_revisions.len()
                    && handoff.expected_stream_revisions.iter().all(|revision| current.expected_stream_revisions.contains(revision))
                    && current.expected_chart_revisions.len() == handoff.expected_chart_revisions.len()
                    && handoff.expected_chart_revisions.iter().all(|revision| current.expected_chart_revisions.contains(revision))
            })
            && snapshot.data_revision == data_revision
            && snapshot.series_revision == series_revision
            && snapshot.view_revision == view_revision
            && stream_config_equal(&snapshot.config, &render_config)
            && snapshot.display_scale.to_bits() == display_scale.to_bits()
            && snapshot.target == target
            && *active_padding == auto_fit_padding
        {
            return Ok(crate::AutoStreamingRequest::Active {
                revision: snapshot.desired,
                pending_latest: None,
            });
        }

        let mut sources = HashMap::new();
        sources
            .try_reserve(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff source snapshot",
                reason: error.to_string(),
            })?;
        let mut source_versions = Vec::new();
        source_versions
            .try_reserve_exact(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff source versions",
                reason: error.to_string(),
            })?;
        let mut statistics = HashMap::new();
        statistics
            .try_reserve(handoff.columns.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident stream handoff statistics",
                reason: error.to_string(),
            })?;
        for source in &handoff.columns {
            source_versions.push(crate::AutoStreamSourceVersion {
                id: source.id.clone(),
                revision: source.revision,
            });
            statistics.insert(source.id.clone(), StreamStatisticsCache::default());
            sources.insert(source.id.clone(), source.clone());
        }
        source_versions.sort_unstable_by(|left, right| left.id.cmp(&right.id));

        let snapshot_chart = Chart::new(render_config.clone());
        let owned_view = self.create_chart_view(&snapshot_chart, render_config.chart_area.0)?;
        let job = self
            .begin_chart_stream_surface_with_sources(
                chart,
                &owned_view,
                options.size,
                wgpu::Color {
                    r: f64::from(options.clear_color.r),
                    g: f64::from(options.clear_color.g),
                    b: f64::from(options.clear_color.b),
                    a: f64::from(options.clear_color.a),
                },
                options.max_primitives_per_chunk,
                Some(&sources),
                Some((&render_config, display_scale)),
            )
            .map_err(StreamRequestError::into_figgy)?;
        let styles = self
            .chart_states
            .get_mut(&chart)
            .and_then(|state| state.prepared_styles.take())
            .expect("successful stream-surface preparation publishes chart styles");
        let snapshot = Arc::new(AutoStreamExecutionSnapshot {
            desired,
            document_config: self.chart_states[&chart].config.clone(),
            data_revision,
            series_revision,
            view_revision,
            target,
            config: render_config,
            display_scale,
            series,
            styles,
            sources,
            view: owned_view,
            handoff: Some(handoff),
        });
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .expect("new resident handoff cursor exists");
        draw.mode = StreamExecutionMode::Auto {
            snapshot: Some(snapshot),
            statistics,
            auto_fit_padding,
            final_fit_published: false,
            published_revision: None,
            pending_latest: None,
            cancel_requested: false,
            all_submitted: false,
            final_display_published: false,
        };
        Ok(crate::AutoStreamingRequest::Started {
            revision: desired,
            sources: source_versions,
        })
    }

    /// Advance one renderer-selected chunk of the active automatic execution.
    /// Source references remain borrowed for this call only.
    pub fn auto_stream_chart_step(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        self.service_stream_requests();
        let job = self
            .active_stream_job(chart)
            .ok_or_else(|| FiggyError::StaleStateToken {
                reason: "automatic streaming chart has no active execution".into(),
            })?;
        self.pump_stream_selection_sources(job, None, sources).map_err(StreamRequestError::into_figgy)?;
        let (revision, pending_latest, terminal) = {
            let runtime = self.stream_runtime.as_ref().unwrap();
            let draw = runtime
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .ok_or_else(|| FiggyError::StaleStateToken {
                    reason: "automatic streaming cursor is no longer current".into(),
                })?;
            let StreamExecutionMode::Auto {
                snapshot,
                pending_latest,
                ..
            } = &draw.mode
            else {
                return Err(FiggyError::StaleStateToken {
                    reason: "active stream is explicitly host-driven".into(),
                });
            };
            (
                snapshot
                    .as_ref()
                    .expect("automatic execution retains its snapshot")
                    .desired,
                *pending_latest,
                draw.auto_terminal(runtime),
            )
        };
        if terminal && self.stream_runtime.as_ref().unwrap().draws.iter().find(|draw| draw.job == job).unwrap().selection.terminal() {
            let revision = self
                .publish_auto_stream_completion(job)
                .map_err(StreamRequestError::into_figgy)?;
            return Ok(crate::AutoStreamingProgress::Complete {
                revision,
                pending_latest,
            });
        }
        let progress = self.stream_chart_step_mode(chart, None, sources, true)?;
        Ok(match progress {
            crate::StreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
            } => crate::AutoStreamingProgress::Submitted {
                revision,
                submitted_primitives,
                total_primitives,
            },
            crate::StreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
            } => crate::AutoStreamingProgress::Backpressure {
                revision,
                submitted_primitives,
                total_primitives,
            },
            crate::StreamingProgress::AllSubmitted { total_primitives } => {
                crate::AutoStreamingProgress::AllSubmitted {
                    revision,
                    total_primitives,
                }
            }
        })
    }

    /// Reserve or inspect the next exact renderer-selected ranges without
    /// advancing the automatic CPU cursor. This is the async range-provider
    /// boundary: the host can fetch only `Ready::ranges`, then submit them with
    /// `auto_stream_chart_submit_ranges`.
    pub fn auto_stream_chart_request_ranges(
        &mut self,
        chart: ChartId,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.service_stream_requests();
        let job = self
            .active_stream_job(chart)
            .ok_or_else(|| FiggyError::StaleStateToken {
                reason: "automatic streaming chart has no active execution".into(),
            })?;
        let (revision, pending_latest, terminal) = {
            let runtime = self.stream_runtime.as_ref().unwrap();
            let draw = runtime
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .ok_or_else(|| FiggyError::StaleStateToken {
                    reason: "automatic streaming cursor is no longer current".into(),
                })?;
            let StreamExecutionMode::Auto {
                snapshot,
                pending_latest,
                ..
            } = &draw.mode
            else {
                return Err(FiggyError::StaleStateToken {
                    reason: "active stream is explicitly host-driven".into(),
                });
            };
            (
                snapshot
                    .as_ref()
                    .expect("automatic execution retains its snapshot")
                    .desired,
                *pending_latest,
                draw.auto_terminal(runtime),
            )
        };
        if terminal {
            let revision = self
                .publish_auto_stream_completion(job)
                .map_err(StreamRequestError::into_figgy)?;
            return Ok(crate::AutoStreamingRangeRequest::Complete {
                revision,
                pending_latest,
            });
        }
        match self
            .request_chart_stream_draw(job)
            .map_err(StreamRequestError::into_figgy)?
        {
            StreamDrawRequestStatus::Backpressure => {
                let (submitted_primitives, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                Ok(crate::AutoStreamingRangeRequest::Backpressure {
                    revision,
                    submitted_primitives,
                    total_primitives,
                })
            }
            StreamDrawRequestStatus::AllSubmitted => {
                let (_, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                Ok(crate::AutoStreamingRangeRequest::AllSubmitted {
                    revision,
                    total_primitives,
                })
            }
            StreamDrawRequestStatus::Ready(ticket) => {
                let requested = self
                    .stream_request_columns(ticket)
                    .map_err(StreamRequestError::into_figgy)?;
                let mut ranges = Vec::new();
                ranges.try_reserve_exact(requested.len()).map_err(|error| {
                    FiggyError::StateAllocationFailed {
                        resource: "automatic stream range request",
                        reason: error.to_string(),
                    }
                })?;
                ranges.extend(requested.iter().map(|column| crate::AutoStreamRange {
                    id: column.column.clone(),
                    revision: column.range.revision,
                    source_len: column.range.source_len,
                    offset: column.range.offset,
                    len: column.range.len,
                    encoding: column.range.encoding,
                }));
                let (submitted_primitives, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                Ok(crate::AutoStreamingRangeRequest::Ready {
                    revision,
                    submitted_primitives,
                    total_primitives,
                    ranges,
                })
            }
        }
    }

    /// Commit the pending range-provider request. The supplied `ColumnSource`
    /// objects remain borrowed for this call only; cursor publication still
    /// occurs after successful queue submission.
    pub fn auto_stream_chart_submit_ranges(
        &mut self,
        chart: ChartId,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::AutoStreamingProgress> {
        self.service_stream_requests();
        let job = self
            .active_stream_job(chart)
            .ok_or_else(|| FiggyError::StaleStateToken {
                reason: "automatic streaming chart has no active execution".into(),
            })?;
        let (revision, ticket) = {
            let runtime = self.stream_runtime.as_ref().unwrap();
            let draw = runtime
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .ok_or_else(|| FiggyError::StaleStateToken {
                    reason: "automatic streaming cursor is no longer current".into(),
                })?;
            let StreamExecutionMode::Auto { snapshot, .. } = &draw.mode else {
                return Err(FiggyError::StaleStateToken {
                    reason: "active stream is explicitly host-driven".into(),
                });
            };
            (
                snapshot
                    .as_ref()
                    .expect("automatic execution retains its snapshot")
                    .desired,
                draw.pending.ok_or_else(|| FiggyError::StaleStateToken {
                    reason: "automatic stream has no pending range request".into(),
                })?,
            )
        };
        let (ordered, count) = {
            let requested = self
                .stream_request_columns(ticket)
                .map_err(StreamRequestError::into_figgy)?;
            let first = requested
                .first()
                .ok_or_else(|| FiggyError::StaleStateToken {
                    reason: "stream request has no columns".into(),
                })?;
            if requested.len() > 7 {
                return Err(FiggyError::GpuResourceLimit {
                    resource: "stream columns per chunk",
                    requested: requested.len() as u64,
                    limit: 7,
                });
            }
            let first = resolve_stream_range_source(first, sources)?;
            let mut ordered = [first; 7];
            for (index, column) in requested.iter().enumerate().skip(1) {
                ordered[index] = resolve_stream_range_source(column, sources)?;
            }
            (ordered, requested.len())
        };
        self.submit_chart_stream_surface_sources(ticket, &ordered[..count], None)
            .map_err(StreamRequestError::into_figgy)?;
        let (submitted_primitives, total_primitives) = self
            .stream_progress_counts(job)
            .map_err(StreamRequestError::into_figgy)?;
        Ok(crate::AutoStreamingProgress::Submitted {
            revision,
            submitted_primitives,
            total_primitives,
        })
    }

    /// Queue interruption only for the automatic streaming path. Resident
    /// rendering and the legacy explicit streaming controls are untouched.
    pub fn interrupt_render(&mut self, chart: ChartId) -> Result<crate::RenderInterruptStatus> {
        self.chart_config(chart)?;
        self.service_stream_requests();
        let Some(job) = self.active_stream_job(chart) else {
            return Ok(crate::RenderInterruptStatus::Resident);
        };
        self.retain_stream_status(chart)?;
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .expect("active stream job has a cursor");
        let StreamExecutionMode::Auto {
            cancel_requested, ..
        } = &mut draw.mode
        else {
            return Ok(crate::RenderInterruptStatus::Resident);
        };
        *cancel_requested = true;
        Ok(crate::RenderInterruptStatus::StreamCancelQueued)
    }

    pub(super) fn active_auto_stream_is_terminal(&self, chart: ChartId) -> bool {
        let Some(job) = self.active_stream_job(chart) else {
            return false;
        };
        let Some(runtime) = self.stream_runtime.as_ref() else {
            return false;
        };
        runtime
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .is_some_and(|draw| {
                matches!(draw.mode, StreamExecutionMode::Auto { .. })
                    && draw.auto_terminal(runtime)
            })
    }

    /// Infallible half of `cancel_chart_stream` used only after a resident
    /// promotion has committed. The terminal display and scheduler ownership
    /// are no longer authoritative, while submitted GPU owners remain charged
    /// through their normal completion callbacks.
    pub(super) fn retire_stream_after_resident_commit(&mut self, chart: ChartId) {
        let Some(runtime) = self.stream_runtime.as_mut() else {
            return;
        };
        runtime.scheduler.cancel_chart(chart.sequence);
        runtime.draws.retain(|draw| draw.job.chart != chart || draw.auxiliary.is_some());
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
    }

    /// Advance one exact renderer-selected chunk. Sources write the requested
    /// ranges directly into mapped staging through `ColumnPairWriter`; neither
    /// encoded payloads nor source references survive this call.
    pub fn stream_chart_step(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.stream_chart_step_mode(chart, Some(view), sources, false)
    }

    fn stream_chart_step_mode(
        &mut self,
        chart: ChartId,
        view: Option<&ChartView>,
        sources: &[crate::StreamSourceBinding<'_>],
        automatic: bool,
    ) -> Result<crate::StreamingProgress> {
        self.chart_config(chart)?;
        self.service_stream_requests();
        let job = self
            .stream_runtime
            .as_ref()
            .and_then(|runtime| {
                runtime
                    .draws
                    .iter()
                    .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none() && draw.surface.is_some())
                    .map(|draw| draw.job)
            })
            .ok_or_else(|| FiggyError::StaleStateToken {
                reason: "streaming chart has not been started or is no longer current".into(),
            })?;
        if self.stream_job_is_auto(job) != automatic {
            return Err(FiggyError::StaleStateToken {
                reason: "streaming chart belongs to a different execution mode".into(),
            });
        }
        if !automatic { self.pump_stream_selection_sources(job, view, sources).map_err(StreamRequestError::into_figgy)?; }
        match self
            .request_chart_stream_draw(job)
            .map_err(StreamRequestError::into_figgy)?
        {
            StreamDrawRequestStatus::Backpressure => {
                let (submitted_primitives, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                Ok(crate::StreamingProgress::Backpressure {
                    submitted_primitives,
                    total_primitives,
                })
            }
            StreamDrawRequestStatus::AllSubmitted => {
                let (_, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                if !self.stream_runtime.as_ref().unwrap().draws.iter().find(|draw| draw.job == job).unwrap().selection.terminal() {
                    return Ok(crate::StreamingProgress::Submitted { submitted_primitives: total_primitives, total_primitives });
                }
                Ok(crate::StreamingProgress::AllSubmitted { total_primitives })
            }
            StreamDrawRequestStatus::Ready(ticket) => {
                let (ordered, count) = {
                    let requested = self
                        .stream_request_columns(ticket)
                        .map_err(StreamRequestError::into_figgy)?;
                    let first = requested
                        .first()
                        .ok_or_else(|| FiggyError::StaleStateToken {
                            reason: "stream request has no columns".into(),
                        })?;
                    if requested.len() > 7 {
                        return Err(FiggyError::GpuResourceLimit {
                            resource: "stream columns per chunk",
                            requested: requested.len() as u64,
                            limit: 7,
                        });
                    }
                    let first = resolve_stream_source(first, sources)?;
                    let mut ordered = [first; 7];
                    for (index, column) in requested.iter().enumerate().skip(1) {
                        ordered[index] = resolve_stream_source(column, sources)?;
                    }
                    (ordered, requested.len())
                };
                self.submit_chart_stream_surface_sources(ticket, &ordered[..count], view)
                    .map_err(StreamRequestError::into_figgy)?;
                let (submitted_primitives, total_primitives) = self
                    .stream_progress_counts(job)
                    .map_err(StreamRequestError::into_figgy)?;
                Ok(crate::StreamingProgress::Submitted {
                    submitted_primitives,
                    total_primitives,
                })
            }
        }
    }

    /// Cancel new work immediately. Already submitted GPU owners retire only
    /// after queue completion and remain included in the renderer budget.
    pub fn cancel_streaming_chart(&mut self, chart: ChartId) -> Result<()> {
        self.cancel_chart_stream(chart)
            .map_err(StreamRequestError::into_figgy)
    }

    /// Inspect the latest execution without polling, reserving ranges,
    /// submitting work, publishing fit results, or requesting a redraw.
    pub fn stream_status(&self, chart: ChartId) -> Result<crate::StreamingStatus> {
        use crate::StreamingState;
        let state = self.chart_states.get(&chart).ok_or(FiggyError::UnknownChart { id: chart })?;
        let mut status = crate::StreamingStatus {
            status: StreamingState::Idle,
            job_id: None,
            revision: state.revisions.desired.sequence,
            desired_revision: state.revisions.desired.sequence,
            published_revision: None,
            submitted_primitives: 0,
            total_primitives: 0,
            in_flight_chunks: 0,
            reserved_gpu_bytes: 0,
            auto_fit_pending: state.stream_auto_fit_padding.is_some(),
        };
        let Some(runtime) = self.stream_runtime.as_ref() else { return Ok(status); };
        if let Some(draw) = runtime.draws.iter().find(|draw| draw.job.chart == chart && draw.auxiliary.is_none()) {
            status.job_id = Some(draw.job.id.sequence());
            let (submitted, total) = self.stream_progress_counts(draw.job)
                .map_err(StreamRequestError::into_figgy)?;
            status.submitted_primitives = submitted;
            status.total_primitives = total;
            status.status = StreamingState::Active;
            if let StreamExecutionMode::Auto {
                snapshot, published_revision, auto_fit_padding, final_fit_published,
                cancel_requested, all_submitted, ..
            } = &draw.mode {
                status.revision = published_revision.or_else(|| snapshot.as_ref().map(|s| s.desired))
                    .map_or(status.revision, |revision| revision.sequence);
                status.published_revision = published_revision.map(|revision| revision.sequence);
                status.auto_fit_pending |= auto_fit_padding.is_some() && !final_fit_published;
                status.status = if *cancel_requested { StreamingState::Cancelling }
                    else if draw.auto_terminal(runtime) && *final_fit_published { StreamingState::Complete }
                    else if *all_submitted { StreamingState::AllSubmitted }
                    else { StreamingState::Active };
            }
        } else if let Some((_, retained)) = runtime.retired_status.iter().find(|(id, _)| *id == chart) {
            status = *retained;
            status.desired_revision = state.revisions.desired.sequence;
            status.auto_fit_pending = state.stream_auto_fit_padding.is_some();
            status.status = StreamingState::Cancelled;
        }
        if let Some(job) = status.job_id {
            (status.in_flight_chunks, status.reserved_gpu_bytes) = runtime.scheduler.job_usage(job);
            if status.status == StreamingState::Cancelled && status.in_flight_chunks != 0 {
                status.status = StreamingState::Cancelling;
            }
        }
        Ok(status)
    }

    fn retain_stream_status(&mut self, chart: ChartId) -> Result<()> {
        let status = self.stream_status(chart)?;
        if status.job_id.is_none() { return Ok(()); }
        let runtime = self.stream_runtime.as_mut().expect("status belongs to runtime");
        if let Some((_, retained)) = runtime.retired_status.iter_mut().find(|(id, _)| *id == chart) {
            *retained = status;
        } else {
            runtime.retired_status.try_reserve(1).map_err(|error| FiggyError::StateAllocationFailed {
                resource: "retained stream status", reason: error.to_string(),
            })?;
            runtime.retired_status.push((chart, status));
        }
        Ok(())
    }

    /// Adjust only subsequent range sizes. An already-issued range remains
    /// unchanged; the initial preflight cap and execution identity are retained.
    pub fn set_stream_chunk_budget(&mut self, chart: ChartId, primitives: u64) -> Result<()> {
        self.chart_config(chart)?;
        let draw = self.stream_runtime.as_mut()
            .and_then(|runtime| runtime.draws.iter_mut().find(|draw| draw.job.chart == chart && draw.auxiliary.is_none()))
            .ok_or_else(|| FiggyError::StaleStateToken { reason: "streaming chart has no active execution".into() })?;
        if primitives == 0 || primitives > draw.max_primitives_limit {
            return Err(FiggyError::InvalidConfig {
                field: "stream chunk budget", reason: "must be non-zero and no greater than the execution's initial cap",
            });
        }
        draw.max_primitives = primitives;
        Ok(())
    }

    /// Browser drain boundary. The host must have submitted or discarded its
    /// earlier recordings and released prepared tokens, as for end_gpu_frame.
    /// No new work is submitted; this fence excludes future queue submissions.
    #[cfg(target_arch = "wasm32")]
    pub async fn cancel_streaming_chart_and_wait(&mut self, chart: ChartId) -> Result<()> {
        self.cancel_streaming_chart(chart)?;
        self.end_gpu_frame();
        self.wait_submitted_work().await;
        self.service_stream_requests();
        if self.stream_status(chart)?.in_flight_chunks != 0 {
            return Err(FiggyError::StaleStateToken { reason: "cancelled stream still owns an uncompleted recording".into() });
        }
        Ok(())
    }

    /// Metadata-only incoming column admission; this is not a reservation or
    /// proof for a chart's additional derived resources.
    pub fn inspect_column_admission(&self, lengths: &[u64]) -> crate::ResidentAdmission {
        self.resident_admission(crate::ResidentAdmissionRequest {
            column_value_counts: lengths,
            already_resident_working_set_bytes: 0,
            derived_buffer_sizes: &[],
            transition_headroom_bytes: 0,
            working_set_limit_bytes: self.auto_resident_working_set_limit.unwrap_or(u64::MAX),
        })
    }

    /// Validate the configured chart against the currently connected exact
    /// streaming passes. This does not allocate resources or start a job.
    pub fn inspect_streaming_capability(&self, chart: ChartId) -> Result<()> {
        let state = self.chart_states.get(&chart).ok_or(FiggyError::UnknownChart { id: chart })?;
        for series in &state.series {
            PrepareContext::validate_stream_series(&state.config, series)?;
            stream_draw_phases(&state.config.draw_style, series).map_err(StreamRequestError::into_figgy)?;
        }
        Ok(())
    }

    pub fn streaming_usage(&self) -> crate::StreamingUsage {
        let (active_charts, in_flight_chunks, reserved_gpu_bytes) = self.stream_request_usage();
        crate::StreamingUsage {
            active_charts,
            in_flight_chunks,
            reserved_gpu_bytes,
        }
    }

    /// Whether this chart currently owns a streaming accumulation surface.
    ///
    /// Hosts use this only to select the registered mixed prepare path; the
    /// renderer remains the owner of the cursor and all stream state.
    pub fn is_streaming_chart(&self, chart: ChartId) -> bool {
        self.chart_has_stream_surface(chart)
    }

    fn stream_progress_counts(&self, job: StreamJob) -> StreamResult<(u64, u64)> {
        self.validate_stream_job(job)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        let auto_snapshot = draw.auto_snapshot().cloned();
        let (series, sources) = auto_snapshot.as_ref().map_or_else(
            || (self.chart_states[&job.chart].series.as_slice(), None),
            |snapshot| (snapshot.series.as_slice(), Some(&snapshot.sources)),
        );
        let mut completed = 0u64;
        let mut total = 0u64;
        let draw_style = auto_snapshot.as_ref().map_or(&self.chart_states[&job.chart].config.draw_style,
            |snapshot| &snapshot.config.draw_style);
        for (series_index, series) in series.iter().enumerate() {
            for (phase_index, phase) in stream_draw_phases(draw_style, series)?.iter().enumerate() {
                let phase_total = stream_phase_total(series, *phase, |id| {
                    sources.map_or_else(
                        || self.streaming_sources.get(id).map(|source| source.len),
                        |sources| sources.get(id).map(|source| source.len),
                    )
                })?;
                total = total
                    .checked_add(phase_total)
                    .ok_or(StreamError::Overflow)?;
                if series_index < draw.series
                    || (series_index == draw.series && phase_index < draw.phase_index)
                {
                    completed = completed
                        .checked_add(phase_total)
                        .ok_or(StreamError::Overflow)?;
                } else if series_index == draw.series && phase_index == draw.phase_index {
                    completed = completed
                        .checked_add(draw.offset.min(phase_total))
                        .ok_or(StreamError::Overflow)?;
                }
            }
        }
        Ok((completed, total))
    }

    pub(super) fn chart_has_stream_surface(&self, chart: ChartId) -> bool {
        self.stream_runtime.as_ref().is_some_and(|runtime| {
            runtime
                .draws
                .iter()
                .any(|draw| draw.job.chart == chart && draw.auxiliary.is_none() && draw.surface.is_some())
        })
    }

    pub(super) fn preflight_chart_stream_display(
        &self,
        chart: ChartId,
        view: &ChartView,
    ) -> StreamResult<()> {
        let draw = self
            .stream_runtime
            .as_ref()
            .and_then(|runtime| {
                runtime
                    .draws
                    .iter()
                    .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none() && draw.surface.is_some())
            })
            .ok_or(StreamError::WrongState)?;
        self.validate_stream_job(draw.job)?;
        let effective_view = draw.auto_snapshot().map_or(view, |snapshot| &snapshot.view);
        if !Arc::ptr_eq(&effective_view.stream_revision, &draw.view_revision)
            || effective_view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
        {
            return Err(StreamError::Stale.into());
        }
        let display_needs_refresh = draw.display_dirty
            || draw.display_view_revision.load(Ordering::Acquire) != draw.displayed_view_revision;
        if display_needs_refresh && draw.display_serial == u64::MAX {
            return Err(StreamError::Overflow.into());
        }
        Ok(())
    }

    /// Allocate a bounded prefix/display pair and seed the prefix once. The
    /// caller still owns the frame boundary for any unrelated recorded work.
    pub(crate) fn begin_chart_stream_surface(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        size: (u32, u32),
        clear: wgpu::Color,
        max_primitives: u64,
    ) -> StreamResult<StreamJob> {
        self.begin_chart_stream_surface_with_sources(chart, view, size, clear, max_primitives, None, None)
    }

    fn begin_chart_stream_surface_with_sources(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        size: (u32, u32),
        clear: wgpu::Color,
        max_primitives: u64,
        pending_sources: Option<&HashMap<ColumnId, crate::StreamColumn>>,
        display: Option<(&Config, f32)>,
    ) -> StreamResult<StreamJob> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.chart_config(chart)?;
        let runtime = self
            .stream_runtime
            .as_ref()
            .ok_or(StreamError::InvalidLimits)?;
        if !runtime
            .transfer
            .as_ref()
            .is_some_and(|transfer| transfer.matches(self.surface_format, self.target_sample_count))
        {
            let transfer =
                StreamTransfer::new(&self.device, self.surface_format, self.target_sample_count)?;
            self.stream_runtime.as_mut().unwrap().transfer = Some(transfer);
        }
        let surface = StreamSurface::new(
            &self.device,
            &self.gpu_ledger,
            self.stream_runtime
                .as_ref()
                .unwrap()
                .transfer
                .as_ref()
                .unwrap(),
            StreamSurfaceSpec {
                width: size.0,
                height: size.1,
                format: self.surface_format,
                sample_count: self.target_sample_count,
            },
            self.memory_budget.unwrap_or(u64::MAX),
            self.pool
                .gpu_bytes()
                .checked_add(self.pool.retired_bytes())
                .ok_or(StreamError::Overflow)?,
        )?;
        let display_view = surface.resolved().create_view(&Default::default());
        let display_bind_group = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            data_render::create_texture_bind_group(
                &self.device,
                &self.texture_bgl,
                &display_view,
                &self.sampler,
            )
        }))
        .map_err(|_| StreamSurfaceError::AllocationFailed)?;
        let job = self.begin_chart_stream_draw_with_sources(
            chart,
            view,
            surface.prefix(),
            max_primitives,
            pending_sources,
            display,
        )?;
        let target_view = surface.prefix().create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy stream prefix initialization"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some(panel) = data_render::clamp_rect_to_target(view.panel_rect, size) {
                pass.set_viewport(
                    panel.x as f32,
                    panel.y as f32,
                    panel.width as f32,
                    panel.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                pass.set_pipeline(&self.pipelines.axis);
                pass.set_bind_group(0, &view.grid_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        draw.surface = Some(surface);
        draw.display_bind_group = Some(display_bind_group);
        draw.surface_clear = Some(clear);
        draw.display_dirty = true;
        Ok(job)
    }

    /// The owned prefix cannot be substituted by host input at supply time.
    pub(crate) fn submit_chart_stream_surface(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamInput<'_>],
        view: &ChartView,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.service_stream_requests();
        self.validate_stream_job(ticket.job)?;
        let target = wgpu::Texture::clone(
            self.stream_runtime
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .find(|draw| draw.job == ticket.job)
                .and_then(|draw| draw.surface.as_ref())
                .ok_or(StreamError::WrongState)?
                .prefix(),
        );
        self.submit_chart_stream_draw(ticket, inputs, view, &target)
    }

    fn submit_chart_stream_surface_sources(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamSourceInput<'_>],
        view: Option<&ChartView>,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.service_stream_requests();
        self.validate_stream_job(ticket.job)?;
        let target = wgpu::Texture::clone(
            self.stream_runtime
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .find(|draw| draw.job == ticket.job)
                .map(|draw| &draw.target)
                .ok_or(StreamError::WrongState)?,
        );
        self.submit_chart_stream_draw_supply(ticket, StreamSupply::Sources(inputs), view, &target)
    }

    /// Refresh only after a prefix change. Decoration never feeds back into P.
    pub(crate) fn refresh_chart_stream_display(
        &mut self,
        job: StreamJob,
        view: &ChartView,
    ) -> StreamResult<bool> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(job)?;
        let auto_snapshot = self.auto_stream_snapshot(job);
        let stream_view = auto_snapshot
            .as_ref()
            .map_or(view, |snapshot| &snapshot.view);
        let display_view = view;
        let runtime = self.stream_runtime.as_ref().unwrap();
        let draw = runtime
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        if !Arc::ptr_eq(&stream_view.stream_revision, &draw.view_revision)
            || stream_view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
            || (auto_snapshot.is_none()
                && !Arc::ptr_eq(
                    &display_view.content_revision,
                    &draw.display_view_revision,
                ))
        {
            return Err(StreamError::Stale.into());
        }
        let surface = draw.surface.as_ref().ok_or(StreamError::WrongState)?;
        let display_source_changed = !Arc::ptr_eq(
            &display_view.content_revision,
            &draw.display_view_revision,
        );
        let current_display_view_revision = display_view
            .content_revision
            .load(Ordering::Acquire);
        let auto_receipts_complete = matches!(
            &draw.mode,
            StreamExecutionMode::Auto {
                all_submitted: true,
                ..
            }
        ) && !runtime.has_data_slots(job.id);
        if !draw.display_dirty
            && !display_source_changed
            && current_display_view_revision == draw.displayed_view_revision
        {
            if auto_receipts_complete {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap();
                let StreamExecutionMode::Auto {
                    final_display_published,
                    ..
                } = &mut draw.mode
                else {
                    unreachable!("receipt completion was checked for an automatic stream")
                };
                *final_display_published = true;
            }
            return Ok(false);
        }
        let next_serial = draw
            .display_serial
            .checked_add(1)
            .ok_or(StreamError::Overflow)?;
        let size = (surface.prefix().width(), surface.prefix().height());
        let panel = data_render::clamp_rect_to_target(display_view.panel_rect, size);
        let data = data_render::clamp_rect_to_target(
            auto_snapshot
                .as_ref()
                .map_or(&self.chart_states[&job.chart].config, |snapshot| {
                    &snapshot.config
                })
                .data_area()
                .map_or(stream_view.panel_rect, |area| area.0),
            size,
        );
        let histogram = if draw.hist_overlay {
            Some((
                draw.hist_envelope.as_ref().ok_or(StreamError::WrongState)?,
                draw.hist_series.ok_or(StreamError::WrongState)?,
            ))
        } else {
            None
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        surface.record_display(
            runtime.transfer.as_ref().ok_or(StreamError::WrongState)?,
            &mut encoder,
            |pass| {
                if let (Some(panel), Some(data), Some((envelope, series))) =
                    (panel, data, histogram)
                {
                    pass.set_viewport(
                        panel.x as f32,
                        panel.y as f32,
                        panel.width as f32,
                        panel.height as f32,
                        0.0,
                        1.0,
                    );
                    pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                    envelope.draw(
                        pass,
                        &stream_view.transform_bg,
                        auto_snapshot.as_ref().map_or_else(
                            || {
                                &self.chart_states[&job.chart]
                                    .prepared_styles
                                    .as_ref()
                                    .unwrap()
                                    .styles[series]
                                    .bar_bg
                            },
                            |snapshot| &snapshot.styles.styles[series].bar_bg,
                        ),
                    );
                }
                if let (Some(panel), Some(data)) = (panel, data) {
                    pass.set_viewport(panel.x as f32, panel.y as f32, panel.width as f32, panel.height as f32, 0.0, 1.0);
                    pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                    for selected in &draw.selection.ready {
                        data_render::issue_series_picked(pass, &selected.packet.layers());
                    }
                }
                if let Some(panel) = panel {
                    pass.set_viewport(
                        panel.x as f32,
                        panel.y as f32,
                        panel.width as f32,
                        panel.height as f32,
                        0.0,
                        1.0,
                    );
                    pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                    pass.set_pipeline(&self.pipelines.axis);
                    pass.set_bind_group(0, &display_view.decoration_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
            },
        )?;
        self.queue.submit([encoder.finish()]);
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        draw.display_dirty = false;
        draw.display_serial = next_serial;
        draw.display_view_revision = Arc::clone(&display_view.content_revision);
        draw.displayed_view_revision = current_display_view_revision;
        if auto_receipts_complete
            && let StreamExecutionMode::Auto {
                final_display_published,
                ..
            } = &mut draw.mode
        {
            *final_display_published = true;
        }
        Ok(true)
    }

    pub(super) fn prepare_chart_stream_display(
        &mut self,
        chart: ChartId,
        view: &ChartView,
    ) -> StreamResult<Option<StreamDisplayPacket>> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        let Some(job) = self.stream_runtime.as_ref().and_then(|runtime| {
            runtime
                .draws
                .iter()
                .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none() && draw.surface.is_some())
                .map(|draw| draw.job)
        }) else {
            return Ok(None);
        };
        self.refresh_chart_stream_display(job, view)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .and_then(|runtime| runtime.draws.iter().find(|draw| draw.job == job))
            .ok_or(StreamError::Stale)?;
        let surface = draw.surface.as_ref().ok_or(StreamError::WrongState)?;
        Ok(Some(StreamDisplayPacket {
            job,
            bind_group: draw
                .display_bind_group
                .as_ref()
                .ok_or(StreamError::WrongState)?
                .clone(),
            _charge: surface.resolved().shared_charge(),
            serial: draw.display_serial,
            size: (surface.prefix().width(), surface.prefix().height()),
        }))
    }

    pub(super) fn stream_display_packet_is_current(&self, packet: &StreamDisplayPacket) -> bool {
        self.stream_runtime.as_ref().is_some_and(|runtime| {
            runtime.scheduler.job_for_chart(packet.job.chart.sequence) == Some(packet.job.id)
                && runtime.draws.iter().any(|draw| {
                    draw.job == packet.job
                        && draw.surface.is_some()
                        && !draw.display_dirty
                        && draw.display_view_revision.load(Ordering::Acquire)
                            == draw.displayed_view_revision
                        && draw.display_serial == packet.serial
                })
        })
    }

    /// Borrowed current output, not an immutable PreparedFrame snapshot. The
    /// frame compositor must finish its use before the next refresh or mutation.
    pub(crate) fn chart_stream_display(&mut self, job: StreamJob) -> StreamResult<&TrackedTexture> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(job)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        if draw.display_dirty {
            return Err(StreamError::WrongState.into());
        }
        Ok(draw
            .surface
            .as_ref()
            .ok_or(StreamError::WrongState)?
            .resolved())
    }

    #[cfg(test)]
    pub(super) fn chart_stream_prefix_for_test(
        &self,
        job: StreamJob,
    ) -> StreamResult<wgpu::Texture> {
        self.validate_stream_job(job)?;
        self.stream_runtime
            .as_ref()
            .and_then(|runtime| runtime.draws.iter().find(|draw| draw.job == job))
            .and_then(|draw| draw.surface.as_ref())
            .map(|surface| wgpu::Texture::clone(surface.prefix()))
            .ok_or(StreamError::WrongState.into())
    }

    /// Prepare one exact data-only execution against an already initialized
    /// target. The caller must not rewrite the view or target while this job is
    /// active. Config and series are always read from the chart registry.
    pub(crate) fn begin_chart_stream_draw(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        target: &wgpu::Texture,
        max_primitives_per_chunk: u64,
    ) -> StreamResult<StreamJob> {
        self.begin_chart_stream_draw_with_sources(
            chart,
            view,
            target,
            max_primitives_per_chunk,
            None,
            None,
        )
    }

    fn begin_chart_stream_draw_with_sources(
        &mut self,
        chart: ChartId,
        view: &ChartView,
        target: &wgpu::Texture,
        max_primitives_per_chunk: u64,
        pending_sources: Option<&HashMap<ColumnId, crate::StreamColumn>>,
        display: Option<(&Config, f32)>,
    ) -> StreamResult<StreamJob> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        let runtime = self
            .stream_runtime
            .as_ref()
            .ok_or(StreamError::InvalidLimits)?;
        let limits = runtime.scheduler.limits();
        if runtime.scheduler.job_for_chart(chart.sequence).is_none()
            && runtime.scheduler.active_jobs() >= limits.max_jobs
        {
            return Err(StreamError::TooManyJobs.into());
        }
        if max_primitives_per_chunk == 0
            || target.format() != self.surface_format
            || target.sample_count() != self.target_sample_count
            || !target
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || target.dimension() != wgpu::TextureDimension::D2
            || target.depth_or_array_layers() != 1
            || target.width() == 0
            || target.height() == 0
        {
            return Err(StreamError::InvalidRange.into());
        }
        if view.content_revision.load(Ordering::Acquire) == u64::MAX
            || view.stream_revision.load(Ordering::Acquire) == u64::MAX
        {
            return Err(StreamError::Overflow.into());
        }
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let (config, display_scale) = display.unwrap_or((&state.config, 1.0));
        for series in &state.series {
            PrepareContext::validate_stream_series(config, series)?;
            for phase in stream_draw_phases(&config.draw_style, series)? {
                let source_metadata = |id: &str| {
                    pending_sources
                        .and_then(|sources| sources.get(id))
                        .or_else(|| self.streaming_sources.get(id).map(|source| &source.column))
                };
                if *phase == StreamDrawPhase::Field {
                    field_runtime::preflight(&self.device, limits, config, series, max_primitives_per_chunk, source_metadata)?;
                    continue;
                }
                let columns = stream_phase_columns(series, *phase);
                let total =
                    stream_phase_total(series, *phase, |id| source_metadata(id).map(|s| s.len))?;
                if let Some(id) = columns.style_index {
                    let available = source_metadata(id).ok_or(StreamError::InvalidRange)?.len;
                    if available < total {
                        return Err(FiggyError::InvalidSeriesConfig {
                            series_id: series.series_id.clone(),
                            reason: format!("style index column {id:?} has {available} values, but this pass uses {total}"),
                        }.into());
                    }
                }
                if total == 0 {
                    continue;
                }
                if *phase == StreamDrawPhase::Line && arc_runtime::needs_arc(&config.draw_style, series)
                    && total >= u64::from(u32::MAX) {
                    return Err(StreamError::TooLarge.into());
                }
                let primitives = total.min(max_primitives_per_chunk);
                if *phase == StreamDrawPhase::Histogram {
                    let bar = extract_bar(&series.render_type).ok_or(StreamError::WrongState)?;
                    let pixels = match bar.orientation {
                        crate::data_config::BarOrientation::Vertical => view.panel_rect.width,
                        crate::data_config::BarOrientation::Horizontal => view.panel_rect.height,
                    }
                    .max(1);
                    let pixel_bytes = u64::from(pixels)
                        .checked_mul(16)
                        .ok_or(StreamError::Overflow)?;
                    let scratch_bytes = u64::from(pixels)
                        .checked_mul(4)
                        .ok_or(StreamError::Overflow)?;
                    let max_groups = self.device.limits().max_compute_workgroups_per_dimension;
                    let reduce_groups = primitives.div_ceil(64);
                    if total > u64::from(u32::MAX) + 1
                        || pixel_bytes > self.device.limits().max_buffer_size
                        || pixel_bytes
                            > u64::from(self.device.limits().max_storage_buffer_binding_size)
                        || scratch_bytes
                            > u64::from(self.device.limits().max_storage_buffer_binding_size)
                        || pixels.div_ceil(64) > max_groups
                        || reduce_groups.min(65535) > u64::from(max_groups)
                        || reduce_groups.div_ceil(65535) > u64::from(max_groups)
                    {
                        return Err(StreamError::TooLarge.into());
                    }
                }
                let mut input = 0u64;
                let mut work_bytes = 0u64;
                for id in &columns.ids[..columns.count] {
                    let source = source_metadata(id).ok_or(StreamError::InvalidRange)?;
                    let rows = if *phase == StreamDrawPhase::Line && arc_runtime::needs_arc(&config.draw_style, series) {
                        (total + 1).min(max_primitives_per_chunk).min(257)
                    } else {
                        stream_phase_request_len(series, *phase, id, primitives)?
                    };
                    if *phase == StreamDrawPhase::Histogram && rows > u64::from(u32::MAX) {
                        return Err(StreamError::TooLarge.into());
                    }
                    input = input
                        .checked_add(
                            rows.checked_mul(source.encoding.bytes_per_value())
                                .ok_or(StreamError::Overflow)?,
                        )
                        .ok_or(StreamError::Overflow)?;
                    work_bytes = work_bytes
                        .checked_add(rows.checked_mul(8).ok_or(StreamError::Overflow)?)
                        .ok_or(StreamError::Overflow)?;
                }
                let charge = work_bytes.checked_mul(2).ok_or(StreamError::Overflow)?;
                if columns.count > limits.max_columns_per_request
                    || input > limits.max_chunk_bytes
                    || charge > limits.max_in_flight_bytes
                    || work_bytes > self.device.limits().max_buffer_size
                    || work_bytes > u64::from(self.device.limits().max_storage_buffer_binding_size)
                    || usize::try_from(work_bytes).is_err()
                {
                    return Err(StreamError::TooLarge.into());
                }
            }
        }
        let styles_current = state
            .prepared_styles
            .as_ref()
            .is_some_and(|styles| styles.series_revision == state.revisions.series
                && styles.styles.iter().all(|style| style.display_scale.to_bits() == display_scale.to_bits()));
        let uncharged_base_bytes = if styles_current {
            let mut bytes = 0u64;
            for style in &state.prepared_styles.as_ref().unwrap().styles {
                for map in [style.scatter_map.as_ref(), style.errorbar_map.as_ref()]
                    .into_iter()
                    .flatten()
                {
                    if !map.stream_base_is_charged() {
                        bytes = bytes
                            .checked_add(map.stream_base_bytes())
                            .ok_or(StreamError::Overflow)?;
                    }
                }
            }
            bytes
        } else {
            let mut bytes = 0u64;
            for series in &state.series {
                bytes = bytes
                    .checked_add(mapped_stream_base_bytes(series, &self.device)?)
                    .ok_or(StreamError::Overflow)?;
            }
            bytes
        };
        if self
            .gpu_memory_usage()
            .total_bytes()
            .checked_add(uncharged_base_bytes)
            .ok_or(StreamError::Overflow)?
            > self.memory_budget.unwrap_or(u64::MAX)
        {
            return Err(StreamError::TooLarge.into());
        }
        let scheduler = &self.stream_runtime.as_ref().unwrap().scheduler;
        if scheduler.job_for_chart(chart.sequence).is_none()
            && scheduler.active_jobs() >= scheduler.limits().max_jobs
        {
            return Err(StreamError::TooManyJobs.into());
        }
        self.stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .try_reserve(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        let style_update = if styles_current {
            None
        } else {
            let mut styles = Vec::new();
            styles
                .try_reserve_exact(state.series.len())
                .map_err(|_| StreamError::AllocationFailed)?;
            styles.extend(
                state
                    .series
                    .iter()
                    .map(|series| self.create_style_for_series_scaled(series, display_scale)),
            );
            let styles = RegisteredChartStyles {
                series_revision: state.revisions.series,
                styles,
            };
            Some(styles)
        };
        let styles = &style_update
            .as_ref()
            .or(state.prepared_styles.as_ref())
            .unwrap()
            .styles;
        for (series_config, style) in state.series.iter().zip(styles) {
            let series = [Series { config: series_config, style }];
            self.pipelines.ensure_styles_for_items(
                &self.device, &self.queue, &self.transform_bgl, &self.style_bgl,
                &self.star_data_bgl, self.surface_format,
                &[ChartDrawItem { view, chart_config: config, series: &series }],
            );
            self.pipelines.ensure_precise_variants_for_items(
                &self.device,
                &self.transform_bgl,
                &self.style_bgl,
                &self.per_point_style_map_bgl,
                &self.data_selection_bgl,
                &self.field_bgl,
                self.contour_label_pipelines.as_ref(),
                self.surface_format,
                &[ChartDrawItem {
                    view,
                    chart_config: config,
                    series: &series,
                }],
            );
        }
        let transform = data_render::scatter_transform_from_config(config);
        let job = self.begin_chart_stream(chart)?;
        if let Some(mut styles) = style_update {
            charge_mapped_stream_bases(&mut styles, &self.gpu_ledger);
            self.chart_states.get_mut(&chart).unwrap().prepared_styles = Some(styles);
        } else {
            charge_mapped_stream_bases(
                self.chart_states
                    .get_mut(&chart)
                    .unwrap()
                    .prepared_styles
                    .as_mut()
                    .unwrap(),
                &self.gpu_ledger,
            );
        }
        let expected_view_revision = view.advance_stream_revision()?;
        let displayed_view_revision = view.advance_content_revision()?;
        data_render::update_scatter_transform(&self.queue, &view.transform_buffer, &transform);
        self.stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .push(StreamDrawCursor {
                job,
                series: 0,
                phase_index: 0,
                offset: 0,
                max_primitives: max_primitives_per_chunk,
                max_primitives_limit: max_primitives_per_chunk,
                pending: None,
                view_revision: Arc::clone(&view.stream_revision),
                expected_view_revision,
                display_view_revision: Arc::clone(&view.content_revision),
                displayed_view_revision,
                target: target.clone(),
                surface: None,
                display_bind_group: None,
                surface_clear: None,
                display_dirty: false,
                display_serial: 0,
                hist_envelope: None,
                hist_series: None,
                hist_overlay: false,
                mode: StreamExecutionMode::Explicit,
                auxiliary: None,
                auxiliary_pick: None,
                arc: None,
                field: None,
                field_fit: None,
                field_fits: HashMap::new(),
                selection: selection::StreamSelectionState::default(),
            });
        Ok(job)
    }

    /// Only this cursor chooses series and ranges. Repeated requests return the
    /// same outstanding ticket; arbitrary low-level uploads cannot advance it.
    pub(crate) fn request_chart_stream_draw(
        &mut self,
        job: StreamJob,
    ) -> StreamResult<StreamDrawRequestStatus> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(job)?;
        let auto_snapshot = self.auto_stream_snapshot(job);
        let runtime = self.stream_runtime.as_mut().unwrap();
        let draw = runtime
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        if let Some(ticket) = draw.pending {
            return Ok(StreamDrawRequestStatus::Ready(ticket));
        }
        let series_list = auto_snapshot.as_ref().map_or_else(
            || self.chart_states[&job.chart].series.as_slice(),
            |snapshot| snapshot.series.as_slice(),
        );
        let draw_style = auto_snapshot.as_ref().map_or(&self.chart_states[&job.chart].config.draw_style,
            |snapshot| &snapshot.config.draw_style);
        while let Some(series) = series_list.get(draw.series) {
            let phases = stream_draw_phases(draw_style, series)?;
            let Some(phase) = phases.get(draw.phase_index) else {
                draw.series += 1;
                draw.phase_index = 0;
                draw.offset = 0;
                continue;
            };
            let columns = stream_phase_columns(series, *phase);
            let total = stream_phase_total(series, *phase, |id| {
                auto_snapshot.as_ref().map_or_else(
                    || self.streaming_sources.get(id).map(|source| source.len),
                    |snapshot| snapshot.sources.get(id).map(|source| source.len),
                )
            })?;
            if draw.offset == total {
                draw.phase_index += 1;
                draw.offset = 0;
                draw.arc = None;
                draw.field = None;
                continue;
            }
            if *phase == StreamDrawPhase::Field {
                if let Some(pick) = &draw.auxiliary_pick {
                    if !pick.field_enabled() {
                        draw.offset = total;
                        continue;
                    }
                    return self.request_stream_field_pick(job);
                }
                if draw.auxiliary.is_none() && !draw.field_fits.contains_key(&draw.series) {
                    drop(auto_snapshot);
                    return self.request_stream_field_fit(job);
                }
                return self.request_stream_field(job);
            }
            if *phase == StreamDrawPhase::Line && draw.auxiliary_pick.is_none()
                && arc_runtime::needs_arc(draw_style, series) {
                let names = [series.x_column.clone(), series.y_column.clone()];
                let config = auto_snapshot.as_ref().map_or(&self.chart_states[&job.chart].config, |snapshot| &snapshot.config).clone();
                return self.request_stream_arc(job, &config, &names, total + 1);
            }
            let primitives = (total - draw.offset).min(draw.max_primitives);
            let offset = draw.offset;
            // Bounded request names bridge the immutable registry borrow and
            // mutable admission call; no declaration or payload is copied.
            let copy_name = |name: &str| -> StreamResult<String> {
                let mut copy = String::new();
                copy.try_reserve_exact(name.len())
                    .map_err(|_| StreamError::AllocationFailed)?;
                copy.push_str(name);
                Ok(copy)
            };
            let mut names: [String; 7] = std::array::from_fn(|_| String::new());
            for (index, id) in columns.ids[..columns.count].iter().enumerate() {
                names[index] = copy_name(id)?;
            }
            let mut ranges: [StreamSourceRange<'_>; 7] =
                std::array::from_fn(|index| StreamSourceRange {
                    column: &names[index],
                    offset,
                    len: primitives,
                });
            for (index, id) in columns.ids[..columns.count].iter().enumerate() {
                ranges[index].len = stream_phase_request_len(series, *phase, id, primitives)?;
            }
            return match self.request_stream_columns(job, &ranges[..columns.count])? {
                StreamRequestStatus::Backpressure => Ok(StreamDrawRequestStatus::Backpressure),
                StreamRequestStatus::Ready(ticket) => {
                    self.stream_runtime
                        .as_mut()
                        .unwrap()
                        .draws
                        .iter_mut()
                        .find(|draw| draw.job == job)
                        .unwrap()
                        .pending = Some(ticket);
                    Ok(StreamDrawRequestStatus::Ready(ticket))
                }
            };
        }
        if let StreamExecutionMode::Auto { all_submitted, .. } = &mut draw.mode {
            *all_submitted = true;
        }
        Ok(StreamDrawRequestStatus::AllSubmitted)
    }

    /// Own the entire upload/draw submission. Invalid target/view/ticket checks
    /// precede upload; cursor progress is published only after queue submission.
    pub(crate) fn submit_chart_stream_draw(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamInput<'_>],
        view: &ChartView,
        target: &wgpu::Texture,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.submit_chart_stream_draw_supply(
            ticket,
            StreamSupply::Encoded(inputs),
            Some(view),
            target,
        )
    }

    fn submit_chart_stream_draw_supply(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        view: Option<&ChartView>,
        target: &wgpu::Texture,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(ticket.job)?;
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.field.as_ref().is_some_and(|field| field.pick.is_some())) {
            return self.submit_stream_field_pick(ticket, supply, view, target);
        }
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.field_fit.is_some()) {
            return self.submit_stream_field_fit(ticket, supply, view, target);
        }
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.field.is_some()) {
            return self.submit_stream_field_supply(ticket, supply, view, target);
        }
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.arc.is_some()) {
            return self.submit_stream_arc_supply(ticket, supply, view, target);
        }
        let auto_snapshot = self.auto_stream_snapshot(ticket.job);
        let auxiliary = self.stream_runtime.as_ref().unwrap().draws.iter()
            .find(|draw| draw.job == ticket.job).and_then(|draw| draw.auxiliary.clone());
        let view = match auto_snapshot.as_ref() {
            Some(snapshot) => &snapshot.view,
            None => view.ok_or(StreamError::WrongState)?,
        };
        let (series_index, phase_index, draw_offset, prior_histogram, prior_hist_series) = {
            let draw = self
                .stream_runtime
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .find(|draw| draw.job == ticket.job)
                .ok_or(StreamError::WrongState)?;
            if draw.pending != Some(ticket)
                || !Arc::ptr_eq(&view.stream_revision, &draw.view_revision)
                || view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
                || target != &draw.target
            {
                return Err(StreamError::Stale.into());
            }
            (
                draw.series,
                draw.phase_index,
                draw.offset,
                draw.hist_envelope.clone(),
                draw.hist_series,
            )
        };
        let execution_series = auto_snapshot.as_ref().map_or_else(
            || self.chart_states[&ticket.job.chart].series[series_index].clone(),
            |snapshot| snapshot.series[series_index].clone(),
        );
        let execution_config = auto_snapshot.as_ref().map_or(&self.chart_states[&ticket.job.chart].config,
            |snapshot| &snapshot.config);
        let phase = *stream_draw_phases(&execution_config.draw_style, &execution_series)?
            .get(phase_index)
            .ok_or(StreamError::WrongState)?;
        let styled_points = !matches!(execution_config.draw_style, DrawStyle::Precise)
            && matches!(phase, StreamDrawPhase::Scatter | StreamDrawPhase::Errorbar);
        let (columns, column_count) = {
            let requested = self.stream_request_columns(ticket)?;
            let first = requested.first().ok_or(StreamError::WrongState)?.range;
            if requested.len() > 7 {
                return Err(StreamError::TooLarge.into());
            }
            let mut columns = [first; 7];
            for (index, column) in requested.iter().enumerate() {
                columns[index] = column.range;
            }
            (columns, requested.len())
        };
        let primitives = if phase == StreamDrawPhase::Histogram {
            if histogram_edge_column(&execution_series) == Some(execution_series.x_column.as_str())
            {
                columns[0].len.saturating_sub(1)
            } else {
                columns[0].len
            }
        } else {
            columns[0].len.saturating_sub(phase.halo())
        };
        self.validate_stream_supply_kind(ticket, supply)?;
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.auxiliary_pick.is_some()) {
            return self.submit_stream_pick_supply(ticket, supply, series_index, phase, &columns[..column_count], draw_offset, primitives);
        }
        let histogram = if phase == StreamDrawPhase::Histogram {
            let execution_styles = auto_snapshot.as_ref().map_or_else(
                || {
                    &self.chart_states[&ticket.job.chart]
                        .prepared_styles
                        .as_ref()
                        .unwrap()
                        .styles
                },
                |snapshot| &snapshot.styles.styles,
            );
            let bar = extract_bar(&execution_series.render_type).ok_or(StreamError::WrongState)?;
            let pixels = match bar.orientation {
                crate::data_config::BarOrientation::Vertical => view.panel_rect.width,
                crate::data_config::BarOrientation::Horizontal => view.panel_rect.height,
            };
            let existing = prior_histogram.as_ref();
            if existing.is_some() && prior_hist_series != Some(series_index) {
                return Err(StreamError::WrongState.into());
            }
            let pool_bytes = self
                .pool
                .gpu_bytes()
                .checked_add(self.pool.retired_bytes())
                .ok_or(StreamError::Overflow)?;
            let envelope = match existing {
                Some(envelope) => Arc::clone(envelope),
                None => Arc::new(data_render::bar_envelope::Persistent::new(
                    auxiliary.as_ref().and_then(|aux| aux.pipelines.as_ref()).unwrap_or(&self.pipelines)
                        .bar_envelope
                        .as_ref()
                        .ok_or(StreamError::WrongState)?,
                    &self.device,
                    &self.gpu_ledger,
                    pixels,
                    self.memory_budget.unwrap_or(u64::MAX),
                    pool_bytes,
                    execution_styles[series_index]
                        .bar_map
                        .as_ref()
                        .map(|map| &map.bind_group),
                )?),
            };
            let total = stream_phase_total(&execution_series, phase, |id| {
                auto_snapshot.as_ref().map_or_else(
                    || self.streaming_sources.get(id).map(|source| source.len),
                    |snapshot| snapshot.sources.get(id).map(|source| source.len),
                )
            })?;
            let final_chunk = draw_offset
                .checked_add(primitives)
                .ok_or(StreamError::Overflow)?
                == total;
            let global_start = u32::try_from(draw_offset).map_err(|_| StreamError::TooLarge)?;
            Some((
                envelope,
                existing.is_none(),
                final_chunk,
                global_start,
                pool_bytes,
            ))
        } else {
            None
        };
        let transform_headroom = if styled_points { data_render::STREAM_POINT_TRANSFORM_BYTES } else { 0 };
        let mapped_style_headroom = if styled_points { 0 } else {
            let style = auto_snapshot.as_ref().map_or_else(
                || {
                    &self.chart_states[&ticket.job.chart]
                        .prepared_styles
                        .as_ref()
                        .unwrap()
                        .styles[series_index]
                },
                |snapshot| &snapshot.styles.styles[series_index],
            );
            match phase {
                StreamDrawPhase::Field => unreachable!("field uses its bounded tile executor"),
                StreamDrawPhase::Scatter if style.scatter_map.is_some() => 16,
                StreamDrawPhase::Errorbar if style.errorbar_map.is_some() => 16,
                _ => 0,
            }
        };
        if (mapped_style_headroom != 0 || styled_points)
            && draw_offset
                .checked_add(primitives)
                .ok_or(StreamError::Overflow)?
                > u64::from(u32::MAX)
        {
            return Err(StreamError::TooLarge.into());
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy stream chunk draw"),
            });
        if let Some((envelope, true, _, _, _)) = &histogram {
            envelope.record_clear(&mut encoder);
        }
        let chunk = match self.accept_stream_supply_with_headroom(
            ticket,
            supply,
            &mut encoder,
            mapped_style_headroom + transform_headroom,
        ) {
            Ok(chunk) => chunk,
            Err(error) => {
                drop((encoder, histogram, prior_histogram));
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let pending_fit = match self.pending_auto_fit_config(ticket.job) {
            Ok(config) => config,
            Err(error) => {
                drop((encoder, chunk, histogram, prior_histogram));
                let discarded = self.discard_stream_recording(ticket);
                self.end_gpu_frame();
                discarded?;
                return Err(error);
            }
        };
        if let Some(config) = pending_fit {
            drop((encoder, chunk, histogram, prior_histogram));
            drop(auto_snapshot);
            self.discard_stream_recording(ticket)?;
            let restarted = self.restart_auto_stream_with_config(ticket.job, config);
            self.end_gpu_frame();
            return restarted;
        }
        let built = {
            let (states, preparation) = self.preparation_parts();
            let (config, series_config, style) = auto_snapshot.as_ref().map_or_else(
                || {
                    let state = &states[&ticket.job.chart];
                    (
                        &state.config,
                        &state.series[series_index],
                        &state.prepared_styles.as_ref().unwrap().styles[series_index],
                    )
                },
                |snapshot| {
                    (
                        &snapshot.config,
                        &snapshot.series[series_index],
                        &snapshot.styles.styles[series_index],
                    )
                },
            );
            let series = Series {
                config: series_config,
                style,
            };
            preparation.build_stream_series_with_pipelines(
                view,
                config,
                &series,
                &chunk,
                &columns[..column_count],
                phase,
                auxiliary.as_ref().and_then(|aux| aux.pipelines.as_ref()).unwrap_or(preparation.pipelines),
            )
        };
        let mut packet = match built {
            Ok(packet) => packet,
            Err(error) => {
                drop((encoder, chunk, histogram, prior_histogram));
                let discarded = self.discard_stream_recording(ticket);
                self.end_gpu_frame();
                discarded?;
                return Err(error.into());
            }
        };
        if styled_points {
            let config = auto_snapshot.as_ref().map_or(&self.chart_states[&ticket.job.chart].config,
                |snapshot| &snapshot.config);
            let (bind_group, charge) = data_render::create_stream_point_transform_bind_group(
                &self.gpu_ledger, &self.device, &self.transform_bgl,
                &data_render::scatter_transform_from_config(config), draw_offset as u32,
            );
            if let Some(layer) = &mut packet.scatter { layer.transform_bg = bind_group.clone(); }
            if let Some(layer) = &mut packet.errorbar { layer.transform_bg = bind_group; }
            packet._stream_transform_charge = Some(charge);
        }
        if mapped_style_headroom != 0 {
            let mapped = (|| -> StreamResult<()> {
                let global_base = u32::try_from(draw_offset).map_err(|_| StreamError::TooLarge)?;
                let style = auto_snapshot.as_ref().map_or_else(
                    || {
                        &self.chart_states[&ticket.job.chart]
                            .prepared_styles
                            .as_ref()
                            .expect("validated explicit stream styles")
                            .styles[series_index]
                    },
                    |snapshot| &snapshot.styles.styles[series_index],
                );
                let (map, layer_bg) = match phase {
                    StreamDrawPhase::Scatter => (
                        style.scatter_map.as_ref().ok_or(StreamError::WrongState)?,
                        &mut packet
                            .scatter
                            .as_mut()
                            .ok_or(StreamError::WrongState)?
                            .style_map_bg,
                    ),
                    StreamDrawPhase::Errorbar => (
                        style.errorbar_map.as_ref().ok_or(StreamError::WrongState)?,
                        &mut packet
                            .errorbar
                            .as_mut()
                            .ok_or(StreamError::WrongState)?
                            .style_map_bg,
                    ),
                    StreamDrawPhase::Histogram | StreamDrawPhase::Line | StreamDrawPhase::Field => {
                        return Err(StreamError::WrongState.into());
                    }
                };
                let tally = crate::gpu_memory::ChargeTally::new();
                *layer_bg = Some(map.stream_bind_group(
                    &self.device,
                    &self.per_point_style_map_bgl,
                    global_base,
                    &tally,
                ));
                packet._stream_style_charge = Some(crate::gpu_memory::shared_charge(
                    tally,
                    &self.gpu_ledger,
                    crate::gpu_memory::GpuResourceKind::Uniform,
                ));
                Ok(())
            })();
            if let Err(error) = mapped {
                drop((encoder, packet, chunk, histogram, prior_histogram));
                let discarded = self.discard_stream_recording(ticket);
                self.end_gpu_frame();
                discarded?;
                return Err(error);
            }
        }
        let histogram_chunk = if let Some((envelope, _, _, global_start, pool_bytes)) = &histogram {
            let recorded = (|| -> std::result::Result<_, StreamError> {
                let layers = packet.layers();
                let bar = layers.bar.as_ref().ok_or(StreamError::WrongState)?;
                auxiliary.as_ref().and_then(|aux| aux.pipelines.as_ref()).unwrap_or(&self.pipelines)
                    .bar_envelope
                    .as_ref()
                    .ok_or(StreamError::WrongState)?
                    .record_stream_chunk(
                        &self.device,
                        &mut encoder,
                        &self.gpu_ledger,
                        self.memory_budget.unwrap_or(u64::MAX),
                        *pool_bytes,
                        envelope,
                        &chunk.work,
                        bar.edges,
                        bar.values,
                        *global_start,
                        bar.transform_bg,
                        bar.style_bg,
                    )
            })();
            match recorded {
                Ok(chunk) => Some(chunk),
                Err(error) => {
                    drop((encoder, packet, chunk, histogram, prior_histogram));
                    let discarded = self.discard_stream_recording(ticket);
                    self.end_gpu_frame();
                    discarded?;
                    return Err(error.into());
                }
            }
        } else {
            None
        };
        let execution_config = auto_snapshot.as_ref().map_or_else(
            || &self.chart_states[&ticket.job.chart].config,
            |snapshot| &snapshot.config,
        );
        let size = (target.width(), target.height());
        let panel = data_render::clamp_rect_to_target(view.panel_rect, size);
        let data = data_render::clamp_rect_to_target(
            execution_config
                .data_area()
                .map_or(view.panel_rect, |area| area.0),
            size,
        );
        if let (Some(panel), Some(data)) = (panel, data) {
            let target_view = target.create_view(&wgpu::TextureViewDescriptor {
                mip_level_count: Some(1),
                ..Default::default()
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy stream data prefix"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                panel.x as f32,
                panel.y as f32,
                panel.width as f32,
                panel.height as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(data.x, data.y, data.width, data.height);
            let mut layers = packet.layers();
            match phase {
                StreamDrawPhase::Field => unreachable!("field uses its bounded tile executor"),
                StreamDrawPhase::Histogram => {
                    layers.errorbar = None;
                    layers.line = None;
                    layers.scatter = None;
                }
                StreamDrawPhase::Errorbar => {
                    layers.line = None;
                    layers.scatter = None;
                }
                StreamDrawPhase::Line => {
                    layers.errorbar = None;
                    layers.scatter = None;
                }
                StreamDrawPhase::Scatter => {
                    layers.errorbar = None;
                    layers.line = None;
                }
            }
            let mapped_bar = if phase == StreamDrawPhase::Histogram
                && layers
                    .bar
                    .as_ref()
                    .is_some_and(|bar| bar.style_map_bg.is_some())
            {
                layers.bar.take()
            } else {
                None
            };
            data_render::issue_series_data(&mut pass, &layers);
            if let (Some(bar), Some(hist_chunk)) = (mapped_bar.as_ref(), histogram_chunk.as_ref()) {
                hist_chunk.draw_full_mapped_bars(
                    &mut pass,
                    &chunk.work,
                    bar.edges,
                    bar.values,
                    bar.transform_bg,
                    bar.style_bg,
                );
            }
            if let Some((envelope, _, true, _, _)) = &histogram {
                let bar_bg = auto_snapshot.as_ref().map_or_else(
                    || {
                        &self.chart_states[&ticket.job.chart]
                            .prepared_styles
                            .as_ref()
                            .expect("validated explicit stream styles")
                            .styles[series_index]
                            .bar_bg
                    },
                    |snapshot| &snapshot.styles.styles[series_index].bar_bg,
                );
                envelope.draw(&mut pass, &view.transform_bg, bar_bg);
            }
        }
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop((packet, histogram_chunk, chunk));
        match result {
            Ok(submission) => {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == ticket.job)
                    .unwrap();
                draw.offset += primitives;
                draw.pending = None;
                draw.display_dirty = true;
                if let Some((envelope, _, final_chunk, _, _)) = histogram {
                    if final_chunk {
                        draw.hist_envelope = None;
                        draw.hist_series = None;
                        draw.hist_overlay = false;
                    } else {
                        draw.hist_envelope = Some(envelope);
                        draw.hist_series = Some(series_index);
                        draw.hist_overlay = true;
                    }
                }
                drop(prior_histogram);
                // This is a Renderer-owned submission. External host recordings
                // must already be submitted, as for draw/export owned paths.
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                drop((histogram, prior_histogram));
                let discarded = self.discard_stream_recording(ticket);
                self.end_gpu_frame();
                discarded?;
                Err(error)
            }
        }
    }

    /// One-time limits installation: never reset ticket/job issuers and revive
    /// an old host reply. Does not enable a primitive or alter resident APIs.
    pub(crate) fn configure_streaming_runtime(&mut self, limits: StreamLimits) -> StreamResult<()> {
        if self.stream_runtime.is_some() {
            return Err(StreamError::WrongState.into());
        }
        let scheduler = StreamScheduler::new(self.renderer_identity, limits)?;
        self.stream_runtime = Some(StreamRuntime {
            scheduler,
            requests: Vec::new(),
            completions: Vec::new(),
            draws: Vec::new(),
            residencies: Vec::new(),
            retired_status: Vec::new(),
            target_generation: self.target_pipeline_generation,
            transfer: None,
            #[cfg(test)]
            reject_next_completion_reserve: false,
        });
        Ok(())
    }

    /// Reclaim stale Requested slots and exactly completed submissions, never
    /// merely cancelled Recorded/Submitted resources. Native hosts must also
    /// service_gpu_completions nonblockingly to drive queue callbacks while idle.
    /// Source/config mutations remain failure-atomic: only published revisions
    /// affect this lazy invalidation. Hosts report view-only changes by beginning
    /// a replacement job (or cancelling) before supplying more input.
    pub(crate) fn service_stream_requests(&mut self) {
        let Some(runtime) = &mut self.stream_runtime else {
            return;
        };
        let mut index = 0;
        while index < runtime.completions.len() {
            let completion = &runtime.completions[index];
            if completion.done.load(Ordering::Acquire)
                && runtime.scheduler.complete(completion.receipt).is_ok()
            {
                runtime.completions.swap_remove(index);
            } else {
                // An unrecognized receipt is never evidence to release a slot.
                index += 1;
            }
        }
        let states = &self.chart_states;
        runtime.retired_status.retain(|(chart, _)| states.contains_key(chart));
        let identity = self.renderer_identity;
        let target_valid = runtime.target_generation == self.target_pipeline_generation;
        let StreamRuntime {
            scheduler, draws, ..
        } = runtime;
        for draw in draws.iter() {
            if matches!(
                &draw.mode,
                StreamExecutionMode::Auto {
                    cancel_requested: true,
                    ..
                }
            ) && scheduler.job_for_chart(draw.job.chart.sequence) == Some(draw.job.id)
            {
                scheduler.cancel_chart(draw.job.chart.sequence);
            }
        }
        scheduler.retain_jobs(|job, key, source, view| {
            let chart = ChartId {
                renderer_identity: identity,
                sequence: key,
            };
            if draws.iter().any(|draw| draw.job.id == job && draw.auxiliary.is_some()) {
                states.contains_key(&chart)
            } else if draws
                .iter()
                .any(|draw| draw.job.id == job && draw.auto_snapshot().is_some())
            {
                target_valid && states.contains_key(&chart)
            } else {
                target_valid
                    && states.get(&chart).is_some_and(|state| {
                        state.revisions.data.sequence == source.0
                            && state.revisions.view.sequence == view.0
                    })
            }
        });
        runtime.target_generation = self.target_pipeline_generation;
        for draw in &runtime.draws {
            if draw.auto_snapshot().is_none()
                && draw.view_revision.load(Ordering::Acquire) != draw.expected_view_revision
                && runtime.scheduler.job_for_chart(draw.job.chart.sequence) == Some(draw.job.id)
            {
                runtime.scheduler.cancel_chart(draw.job.chart.sequence);
            }
        }
        runtime.draws.retain(|draw| {
            runtime.scheduler.contains_job(draw.job.chart.sequence, draw.job.id)
        });
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
        residency::prune(self);
    }

    pub(crate) fn stream_request_usage(&self) -> (usize, usize, u64) {
        self.stream_runtime.as_ref().map_or((0, 0, 0), |runtime| {
            (
                runtime.scheduler.active_jobs(),
                runtime.scheduler.occupied_slots(),
                runtime.scheduler.charged_bytes(),
            )
        })
    }

    pub(crate) fn begin_chart_stream(&mut self, chart: ChartId) -> StreamResult<StreamJob> {
        self.sync_external_invalidations()?;
        let state = self
            .chart_states
            .get(&chart)
            .ok_or(FiggyError::UnknownChart { id: chart })?;
        let source = SourceStamp(state.revisions.data.sequence);
        let view = ViewEpoch(state.revisions.view.sequence);
        self.service_stream_requests();
        let runtime = self
            .stream_runtime
            .as_mut()
            .ok_or(StreamError::InvalidLimits)?;
        let id = runtime.scheduler.start_job(chart.sequence, source, view)?;
        runtime.draws.retain(|draw| draw.job.chart != chart || draw.auxiliary.is_some());
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
        Ok(StreamJob { chart, id })
    }

    pub(crate) fn cancel_chart_stream(&mut self, chart: ChartId) -> StreamResult<()> {
        // Foreign ChartIds must never cancel a local chart with the same sequence.
        self.chart_config(chart)?;
        self.retain_stream_status(chart)?;
        self.service_stream_requests();
        let runtime = self
            .stream_runtime
            .as_mut()
            .ok_or(StreamError::InvalidLimits)?;
        residency::cancel_chart(runtime, chart);
        runtime.scheduler.cancel_chart(chart.sequence);
        runtime.draws.retain(|draw| draw.job.chart != chart || draw.auxiliary.is_some());
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
        Ok(())
    }

    fn validate_stream_job(&self, job: StreamJob) -> StreamResult<()> {
        if job.chart.renderer_identity != self.renderer_identity
            || !self.chart_states.contains_key(&job.chart)
            || !self.stream_runtime.as_ref().is_some_and(|runtime| runtime.scheduler.contains_job(job.chart.sequence, job.id))
        {
            return Err(StreamError::Stale.into());
        }
        Ok(())
    }

    pub(crate) fn request_stream_columns(
        &mut self,
        job: StreamJob,
        ranges: &[StreamSourceRange<'_>],
    ) -> StreamResult<StreamRequestStatus> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(job)?;
        let auto_snapshot = self.auto_stream_snapshot(job);
        let runtime = self
            .stream_runtime
            .as_ref()
            .ok_or(StreamError::InvalidLimits)?;
        let limits = runtime.scheduler.limits();
        if ranges.is_empty() || ranges.len() > limits.max_columns_per_request {
            return Err(StreamError::InvalidRange.into());
        }
        let series = auto_snapshot.as_ref().map_or_else(
            || self.chart_states[&job.chart].series.as_slice(),
            |snapshot| snapshot.series.as_slice(),
        );
        // Validate ALL dependencies, ranges and budgets before metadata allocation.
        let mut input_bytes = 0u64;
        let mut charged_bytes = 0u64;
        for range in ranges {
            if !series
                .iter()
                .any(|s| series_references_column(s, range.column))
                && !runtime.residencies.iter().any(|candidate| candidate.has_source(job, range.column))
            {
                return Err(StreamError::InvalidRange.into());
            }
            let (revision, source_len, encoding) = auto_snapshot
                .as_ref()
                .map_or_else(
                    || {
                        self.streaming_sources
                            .get(range.column)
                            .map(|source| (source.revision, source.len, source.encoding))
                    },
                    |snapshot| {
                        snapshot
                            .sources
                            .get(range.column)
                            .map(|source| (source.revision, source.len, source.encoding))
                    },
                )
                .ok_or(StreamError::InvalidRange)?;
            let resolved = ColumnRange {
                column: 0,
                revision,
                source_len,
                offset: range.offset,
                len: range.len,
                encoding,
            };
            input_bytes = input_bytes
                .checked_add(resolved.byte_len()?)
                .ok_or(StreamError::Overflow)?;
            charged_bytes = charged_bytes
                .checked_add(range.len.checked_mul(16).ok_or(StreamError::Overflow)?)
                .ok_or(StreamError::Overflow)?;
        }
        if input_bytes > limits.max_chunk_bytes
            || charged_bytes > limits.max_in_flight_bytes
            || charged_bytes / 2 > self.device.limits().max_buffer_size
            || charged_bytes / 2 > u64::from(self.device.limits().max_storage_buffer_binding_size)
            || usize::try_from(charged_bytes / 2).is_err()
        {
            return Err(StreamError::TooLarge.into());
        }
        if runtime.scheduler.occupied_slots() >= limits.max_slots
            || runtime
                .scheduler
                .charged_bytes()
                .checked_add(charged_bytes)
                .ok_or(StreamError::Overflow)?
                > limits.max_in_flight_bytes
        {
            return Ok(StreamRequestStatus::Backpressure);
        }
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(ranges.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        let mut numeric = Vec::new();
        numeric
            .try_reserve_exact(ranges.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        for (index, range) in ranges.iter().enumerate() {
            let (revision, source_len, encoding) = auto_snapshot.as_ref().map_or_else(
                || {
                    let source = &self.streaming_sources[range.column];
                    (source.revision, source.len, source.encoding)
                },
                |snapshot| {
                    let source = &snapshot.sources[range.column];
                    (source.revision, source.len, source.encoding)
                },
            );
            let mut column = String::new();
            column
                .try_reserve_exact(range.column.len())
                .map_err(|_| StreamError::AllocationFailed)?;
            column.push_str(range.column);
            // Identical names share local identity, including disjoint ranges.
            let ordinal = ranges[..index]
                .iter()
                .position(|r| r.column == range.column)
                .unwrap_or(index);
            let resolved = ColumnRange {
                column: u64::try_from(ordinal).map_err(|_| StreamError::Overflow)?,
                revision,
                source_len,
                offset: range.offset,
                len: range.len,
                encoding,
            };
            numeric.push(resolved);
            columns.push(StreamRequestedColumn {
                column,
                range: resolved,
            });
        }
        let runtime = self.stream_runtime.as_mut().unwrap();
        runtime
            .requests
            .try_reserve(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        match runtime.scheduler.request(job.id, &numeric)? {
            RequestStatus::Backpressure => Ok(StreamRequestStatus::Backpressure),
            RequestStatus::Ready(ticket) => {
                let ticket = StreamTicket { job, ticket };
                runtime.requests.push(NamedRequest { ticket, columns, selection: false });
                Ok(StreamRequestStatus::Ready(ticket))
            }
        }
    }

    pub(crate) fn stream_request_columns(
        &mut self,
        ticket: StreamTicket,
    ) -> StreamResult<&[StreamRequestedColumn]> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(ticket.job)?;
        let runtime = self.stream_runtime.as_ref().unwrap();
        if !runtime.scheduler.is_requested(ticket.ticket) {
            return Err(StreamError::Stale.into());
        }
        let request = runtime
            .requests
            .iter()
            .find(|request| request.ticket == ticket)
            .ok_or(StreamError::Stale)?;
        Ok(&request.columns)
    }

    /// Preflight borrowed payload and source authority without creating GPU
    /// resources. Histogram's persistent reduction must call this first too.
    fn validate_stream_supply(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamInput<'_>],
    ) -> StreamResult<()> {
        let columns = self.stream_request_columns(ticket)?;
        if inputs.len() != columns.len() {
            return Err(StreamError::InvalidPayload.into());
        }
        for (input, column) in inputs.iter().zip(columns) {
            if input.column != column.column
                || u64::try_from(input.bytes.len()).map_err(|_| StreamError::Overflow)?
                    != column.range.byte_len()?
            {
                return Err(StreamError::InvalidPayload.into());
            }
        }
        self.validate_live_stream_request(ticket)
    }

    fn validate_stream_sources(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamSourceInput<'_>],
    ) -> StreamResult<()> {
        let columns = self.stream_request_columns(ticket)?;
        if inputs.len() != columns.len() {
            return Err(StreamError::InvalidPayload.into());
        }
        for (input, column) in inputs.iter().zip(columns) {
            if input.id != column.column
                || input.revision != column.range.revision
                || input.source.encoding() != column.range.encoding
                || input.source_len != column.range.source_len
            {
                return Err(StreamError::InvalidPayload.into());
            }
            let supplied_len =
                u64::try_from(input.source.len()).map_err(|_| StreamError::Overflow)?;
            match input.source_offset {
                None if supplied_len == column.range.source_len => {}
                Some(offset)
                    if offset == column.range.offset && supplied_len == column.range.len => {}
                _ => return Err(StreamError::InvalidPayload.into()),
            }
        }
        self.validate_live_stream_request(ticket)
    }

    fn validate_stream_supply_kind(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
    ) -> StreamResult<()> {
        match supply {
            StreamSupply::Encoded(inputs) => self.validate_stream_supply(ticket, inputs),
            StreamSupply::Sources(inputs) => self.validate_stream_sources(ticket, inputs),
        }
    }

    fn validate_live_stream_request(&self, ticket: StreamTicket) -> StreamResult<()> {
        // The chart stamp protects remove/re-register ABA even if a host reuses
        // its source revision. Check live metadata too, before numeric conversion.
        let runtime = self.stream_runtime.as_ref().unwrap();
        let request = runtime
            .requests
            .iter()
            .find(|r| r.ticket == ticket)
            .unwrap();
        let auto_snapshot = runtime
            .draws
            .iter()
            .find(|draw| draw.job == ticket.job)
            .and_then(StreamDrawCursor::auto_snapshot);
        for column in &request.columns {
            let source = auto_snapshot
                .map_or_else(
                    || {
                        self.streaming_sources
                            .get(&column.column)
                            .map(|source| &source.column)
                    },
                    |snapshot| snapshot.sources.get(&column.column),
                )
                .ok_or(StreamError::Stale)?;
            if source.revision != column.range.revision
                || source.len != column.range.source_len
                || source.encoding != column.range.encoding
            {
                return Err(StreamError::Stale.into());
            }
        }
        Ok(())
    }

    fn prepare_stream_statistics(
        &self,
        ticket: StreamTicket,
    ) -> StreamResult<PreparedStreamStatistics> {
        let request = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .requests
            .iter()
            .find(|request| request.ticket == ticket)
            .ok_or(StreamError::Stale)?;
        let mut plans = Vec::new();
        plans
            .try_reserve_exact(request.columns.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        plans.resize_with(request.columns.len(), ChunkStatisticsPlan::default);
        if request.selection || self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.auxiliary.is_some()) {
            return Ok(PreparedStreamStatistics { plans, sources: Vec::new() });
        }
        let automatic = self.stream_job_is_auto(ticket.job);
        let mut order = Vec::new();
        order
            .try_reserve_exact(request.columns.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        order.extend(0..request.columns.len());
        order.sort_unstable_by(|a, b| {
            let a_index = *a;
            let b_index = *b;
            let a = &request.columns[a_index];
            let b = &request.columns[b_index];
            a.column
                .cmp(&b.column)
                .then(a.range.offset.cmp(&b.range.offset))
                .then(a_index.cmp(&b_index))
        });
        let mut sources: Vec<PlannedStatisticsSource> = Vec::new();
        sources
            .try_reserve_exact(request.columns.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        for request_index in order {
            let column = &request.columns[request_index];
            let (source_statistics, source_cache) = if automatic {
                let draw = self
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .draws
                    .iter()
                    .find(|draw| draw.job == ticket.job)
                    .ok_or(StreamError::Stale)?;
                let StreamExecutionMode::Auto {
                    snapshot,
                    statistics,
                    ..
                } = &draw.mode
                else {
                    return Err(StreamError::WrongState.into());
                };
                let source = snapshot
                    .as_ref()
                    .ok_or(StreamError::WrongState)?
                    .sources
                    .get(&column.column)
                    .ok_or(StreamError::Stale)?;
                let cache = statistics
                    .get(&column.column)
                    .ok_or(StreamError::Stale)?
                    .try_coverage_copy()
                    .map_err(|_| StreamError::AllocationFailed)?;
                (source.statistics, cache)
            } else {
                let source = self
                    .streaming_sources
                    .get(&column.column)
                    .ok_or(StreamError::Stale)?;
                (
                    source.statistics,
                    source
                        .statistics_cache
                        .try_coverage_copy()
                        .map_err(|_| StreamError::AllocationFailed)?,
                )
            };
            let end = column
                .range
                .offset
                .checked_add(column.range.len)
                .ok_or(StreamError::Overflow)?;
            let range = column.range.offset..end;
            if matches!(source_statistics, crate::StreamStatistics::Known(_)) {
                continue;
            }
            let planned_index = sources
                .iter()
                .position(|planned| request.columns[planned.request_index].column == column.column);
            let planned_index = match planned_index {
                Some(index) => index,
                None => {
                    sources.push(PlannedStatisticsSource {
                        request_index,
                        cache: source_cache,
                    });
                    sources.len() - 1
                }
            };
            let planned = &mut sources[planned_index].cache;
            let gaps = planned
                .uncovered(range.clone())
                .map_err(|_| StreamError::AllocationFailed)?;
            if !gaps.is_empty() {
                if planned.coverage_len_after_insert(&range) > MAX_STREAM_STATISTICS_RANGES {
                    return Err(StreamError::TooLarge.into());
                }
                planned
                    .covered
                    .try_reserve(1)
                    .map_err(|_| StreamError::AllocationFailed)?;
                planned.insert_measured(range, None);
            }
            plans[request_index] = ChunkStatisticsPlan { ranges: gaps };
        }
        let completing_columns = sources
            .iter()
            .filter(|planned| {
                let id = &request.columns[planned.request_index].column;
                let source_len = if automatic {
                    self.auto_stream_snapshot(ticket.job)
                        .and_then(|snapshot| snapshot.sources.get(id).map(|source| source.len))
                        .unwrap_or(0)
                } else {
                    self.streaming_sources[id].len
                };
                planned.cache.covers(0..source_len)
            })
            .count() as u64;
        if !automatic {
            self.next_stream_source_fit_epoch
                .checked_add(completing_columns)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "stream source fit epoch",
                })?;
        }
        Ok(PreparedStreamStatistics { plans, sources })
    }

    fn stream_upload_budget(&self, headroom_bytes: u64) -> StreamResult<ChunkUploadBudget> {
        let limits = self
            .stream_runtime
            .as_ref()
            .ok_or(StreamError::InvalidLimits)?
            .scheduler
            .limits();
        Ok(ChunkUploadBudget {
            max_columns: limits.max_columns_per_request,
            max_input_bytes: limits.max_chunk_bytes,
            max_work_buffer_bytes: self.device.limits().max_buffer_size,
            max_upload_bytes: limits.max_in_flight_bytes,
            renderer_budget_bytes: self
                .memory_budget
                .unwrap_or(u64::MAX)
                .checked_sub(headroom_bytes)
                .ok_or(StreamError::TooLarge)?,
            pool_bytes: self
                .pool
                .gpu_bytes()
                .checked_add(self.pool.retired_bytes())
                .ok_or(StreamError::Overflow)?,
        })
    }

    fn commit_stream_statistics(
        &mut self,
        ticket: StreamTicket,
        chunk: &RecordedChunk,
        prepared: PreparedStreamStatistics,
    ) {
        if self.stream_runtime.as_ref().unwrap().requests.iter().any(|request| request.ticket == ticket && request.selection) {
            return;
        }
        if self.stream_runtime.as_ref().unwrap().draws.iter().any(|draw| draw.job == ticket.job && draw.auxiliary.is_some()) {
            return;
        }
        if self.stream_job_is_auto(ticket.job) {
            let runtime = self
                .stream_runtime
                .as_mut()
                .expect("automatic stream has a runtime");
            let request = runtime
                .requests
                .iter()
                .find(|request| request.ticket == ticket)
                .expect("accepted ticket retains request metadata");
            let draw = runtime
                .draws
                .iter_mut()
                .find(|draw| draw.job == ticket.job)
                .expect("automatic stream retains its draw cursor");
            let StreamExecutionMode::Auto { statistics, .. } = &mut draw.mode else {
                unreachable!("automatic job keeps automatic execution state")
            };
            for ((column, uploaded), plan) in request
                .columns
                .iter()
                .zip(&chunk.columns)
                .zip(&prepared.plans)
            {
                if plan.ranges.is_empty() {
                    continue;
                }
                statistics
                    .get_mut(&column.column)
                    .expect("automatic statistics were created for every source")
                    .merge_bounds(
                        uploaded
                            .statistics
                            .expect("requested upload statistics were collected"),
                    );
            }
            for planned in prepared.sources {
                let column = &request.columns[planned.request_index];
                statistics
                    .get_mut(&column.column)
                    .expect("automatic statistics were created for every source")
                    .covered = planned.cache.covered;
            }
            return;
        }
        let request = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .requests
            .iter()
            .find(|request| request.ticket == ticket)
            .expect("accepted ticket retains request metadata");
        for ((column, uploaded), plan) in request
            .columns
            .iter()
            .zip(&chunk.columns)
            .zip(&prepared.plans)
        {
            if plan.ranges.is_empty() {
                continue;
            }
            let source = self
                .streaming_sources
                .get_mut(&column.column)
                .expect("validated source remains registered during upload");
            source.statistics_cache.merge_bounds(
                uploaded
                    .statistics
                    .expect("requested upload statistics were collected"),
            );
        }
        for planned in prepared.sources {
            let column = &request.columns[planned.request_index];
            let source = self
                .streaming_sources
                .get_mut(&column.column)
                .expect("validated source remains registered during upload");
            source.statistics_cache.covered = planned.cache.covered;
            if source.statistics_cache.covers(0..source.len)
                && !matches!(source.statistics, crate::StreamStatistics::Known(_))
            {
                self.next_stream_source_fit_epoch += 1;
                source.fit_epoch = self.next_stream_source_fit_epoch;
                source.column.statistics =
                    crate::StreamStatistics::Known(source.statistics_cache.bounds);
            } else if !source.statistics_cache.covered.is_empty()
                && matches!(source.statistics, crate::StreamStatistics::Unknown)
            {
                source.column.statistics = crate::StreamStatistics::Pending;
            }
        }
    }

    /// Validates live authority and every borrowed payload before allocating or
    /// calling the staging writer. Success is Recorded, NOT drawn or submitted.
    pub(crate) fn accept_stream_columns(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamInput<'_>],
        encoder: &mut wgpu::CommandEncoder,
    ) -> StreamResult<RecordedChunk> {
        self.accept_stream_columns_with_headroom(ticket, inputs, encoder, 0)
    }

    /// Keep room for a bounded GPU owner created only after a successful
    /// upload. A failed staging upload therefore cannot allocate that owner.
    fn accept_stream_columns_with_headroom(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamInput<'_>],
        encoder: &mut wgpu::CommandEncoder,
        headroom_bytes: u64,
    ) -> StreamResult<RecordedChunk> {
        self.validate_stream_supply(ticket, inputs)?;
        let prepared = self.prepare_stream_statistics(ticket)?;
        let request = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .requests
            .iter()
            .find(|request| request.ticket == ticket)
            .ok_or(StreamError::Stale)?;
        let mut numeric = Vec::new();
        numeric
            .try_reserve_exact(inputs.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        for (input, column) in inputs.iter().zip(&request.columns) {
            numeric.push(ColumnInput {
                range: column.range,
                bytes: input.bytes,
            });
        }
        let budget = self.stream_upload_budget(headroom_bytes)?;
        let runtime = self.stream_runtime.as_mut().unwrap();
        let mut upload_error = None;
        let result = runtime
            .scheduler
            .accept(ticket.ticket, &numeric, |columns| {
                record_chunk_collecting_statistics(
                    &self.device,
                    encoder,
                    &self.gpu_ledger,
                    budget,
                    columns,
                    &prepared.plans,
                )
                .map_err(|error| {
                    upload_error = Some(error);
                    StreamError::WriterFailed
                })
            });
        let chunk = result.map_err(|error| {
            upload_error.map_or(
                StreamRequestError::Scheduler(error),
                StreamRequestError::Upload,
            )
        })?;
        self.commit_stream_statistics(ticket, &chunk, prepared);
        Ok(chunk)
    }

    fn accept_stream_supply_with_headroom(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        encoder: &mut wgpu::CommandEncoder,
        headroom_bytes: u64,
    ) -> StreamResult<RecordedChunk> {
        match supply {
            StreamSupply::Encoded(inputs) => {
                self.accept_stream_columns_with_headroom(ticket, inputs, encoder, headroom_bytes)
            }
            StreamSupply::Sources(inputs) => {
                self.accept_stream_sources_with_headroom(ticket, inputs, encoder, headroom_bytes)
            }
        }
    }

    fn accept_stream_sources_with_headroom(
        &mut self,
        ticket: StreamTicket,
        inputs: &[StreamSourceInput<'_>],
        encoder: &mut wgpu::CommandEncoder,
        headroom_bytes: u64,
    ) -> StreamResult<RecordedChunk> {
        self.validate_stream_sources(ticket, inputs)?;
        let prepared = self.prepare_stream_statistics(ticket)?;
        let budget = self.stream_upload_budget(headroom_bytes)?;
        let runtime = self.stream_runtime.as_mut().unwrap();
        let mut upload_error = None;
        let result = runtime.scheduler.accept_expected(ticket.ticket, |ranges| {
            record_source_chunk_collecting_statistics(
                &self.device,
                encoder,
                &self.gpu_ledger,
                budget,
                ranges,
                |index| crate::streaming_upload::StreamSourceSlice {
                    source: inputs[index].source,
                    source_len: inputs[index].source_len,
                    source_offset: inputs[index].source_offset,
                },
                &prepared.plans,
            )
            .map_err(|error| {
                upload_error = Some(error);
                StreamError::WriterFailed
            })
        });
        let chunk = result.map_err(|error| match upload_error {
            Some(ChunkUploadError::SourceWrite { index, error }) => StreamRequestError::Source {
                id: inputs[index].id.to_owned(),
                error,
            },
            Some(error) => StreamRequestError::Upload(error),
            None => StreamRequestError::Scheduler(error),
        })?;
        self.commit_stream_statistics(ticket, &chunk, prepared);
        Ok(chunk)
    }

    /// Caller must first discard ALL commands/owners of this recording. This
    /// releases scheduler admission only; GPU charges still use the ledger's
    /// host submission/completion boundary.
    pub(crate) fn discard_stream_recording(&mut self, ticket: StreamTicket) -> StreamResult<()> {
        let runtime = self.stream_runtime.as_mut().ok_or(StreamError::Stale)?;
        runtime.scheduler.discard_recorded(ticket.ticket)?;
        runtime.requests.retain(|r| r.ticket != ticket);
        for draw in &mut runtime.draws {
            if draw.pending == Some(ticket) {
                draw.pending = None;
            }
        }
        Ok(())
    }

    /// Submit this ticket's upload and draw commands on the Renderer queue.
    /// The internal executor must supply the matching recording, not an unrelated
    /// command buffer, and retain any reusable work owners for their actual use.
    /// On rejection the command buffer is dropped but admission remains Recorded;
    /// discard_stream_recording requires dropping ALL remaining command/owners.
    /// This low-level submit does not report the retirement boundary: its
    /// caller still owns recorded resources. The chart draw executor drops its
    /// owners and then reports the Renderer-owned boundary; external host
    /// command buffers must already be submitted before that mutable path.
    pub(crate) fn queue_stream_recording(
        &mut self,
        ticket: StreamTicket,
        commands: wgpu::CommandBuffer,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        self.validate_stream_job(ticket.job)?;
        let runtime = self.stream_runtime.as_mut().unwrap();
        if runtime.completions.len() >= runtime.scheduler.limits().max_slots {
            return Err(StreamError::WrongState.into());
        }
        #[cfg(test)]
        if std::mem::take(&mut runtime.reject_next_completion_reserve) {
            return Err(StreamError::AllocationFailed.into());
        }
        // All completion metadata exists before the admission transition or
        // queue submission. The callback only marks its own bounded receipt.
        runtime
            .completions
            .try_reserve_exact(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        let done = Arc::new(AtomicBool::new(false));
        let callback_done = Arc::clone(&done);
        let receipt = runtime.scheduler.submit(ticket.ticket)?;
        runtime.completions.push(StreamCompletion { receipt, done });
        let submission = self.queue.submit([commands]);
        self.queue.on_submitted_work_done(move || {
            callback_done.store(true, Ordering::Release);
        });
        Ok(submission)
    }

    #[cfg(test)]
    pub(crate) fn reject_next_stream_completion_reserve_for_test(&mut self) {
        self.stream_runtime
            .as_mut()
            .expect("test configured the stream runtime")
            .reject_next_completion_reserve = true;
    }

    #[cfg(test)]
    pub(crate) fn submit_stream_recording(
        &mut self,
        ticket: StreamTicket,
    ) -> StreamResult<SubmissionReceipt> {
        Ok(self
            .stream_runtime
            .as_mut()
            .ok_or(StreamError::Stale)?
            .scheduler
            .submit(ticket.ticket)?)
    }

    /// Only the executor's exact queue completion proof may call this, never
    /// merely queue.submit, cancellation, or a newer frame boundary.
    #[cfg(test)]
    pub(crate) fn complete_stream_submission(
        &mut self,
        receipt: SubmissionReceipt,
    ) -> StreamResult<()> {
        let runtime = self.stream_runtime.as_mut().ok_or(StreamError::Stale)?;
        runtime.scheduler.complete(receipt)?;
        runtime
            .requests
            .retain(|r| runtime.scheduler.contains_ticket(r.ticket.ticket));
        Ok(())
    }
}
