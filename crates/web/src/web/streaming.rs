//! Streaming JS API: exact non-resident streaming, automatic streaming
//! requests and ranges, stream operations, status, usage and residency.

use super::*;

pub(super) fn js_stream_sources(values: &js_sys::Array) -> Result<Vec<JsStreamColumn>, JsValue> {
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(values.length() as usize)
        .map_err(js_err)?;
    for index in 0..values.length() {
        sources.push(
            JsStreamColumn::from_js(&values.get(index))
                .map_err(|reason| js_err(format!("stream source {index}: {reason}")))?,
        );
    }
    Ok(sources)
}

pub(super) fn validate_stream_bindings(
    ids: &[String],
    revisions: &[f64],
    sources: &[JsStreamColumn],
) -> Result<Vec<u64>, JsValue> {
    if ids.len() != revisions.len() || ids.len() != sources.len() {
        return Err(js_err(
            "stream ids, revisions, and typed-array sources must have equal lengths",
        ));
    }
    let mut parsed = Vec::new();
    parsed.try_reserve_exact(revisions.len()).map_err(js_err)?;
    for (index, revision) in revisions.iter().copied().enumerate() {
        parsed.push(js_safe_u64(revision, &format!("stream revision {index}"))?);
    }
    Ok(parsed)
}

pub(super) fn parse_stream_encoding(value: &str) -> Result<StreamEncoding, JsValue> {
    match value {
        "f32" => Ok(StreamEncoding::ScalarF32),
        "f64" => Ok(StreamEncoding::HiLoF32),
        _ => Err(js_err("stream encoding must be 'f32' or 'f64'")),
    }
}

pub(super) fn stream_range_bindings<'a>(
    ids: &'a [String],
    revisions: &[f64],
    source_lengths: &[f64],
    offsets: &[f64],
    sources: &'a [JsStreamColumn],
) -> Result<Vec<StreamRangeSourceBinding<'a>>, JsValue> {
    if [
        revisions.len(),
        source_lengths.len(),
        offsets.len(),
        sources.len(),
    ]
    .into_iter()
    .any(|len| len != ids.len())
    {
        return Err(js_err(
            "range ids, revisions, source lengths, offsets, and typed arrays must have equal lengths",
        ));
    }
    let mut bindings = Vec::new();
    bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
    for index in 0..ids.len() {
        bindings.push(StreamRangeSourceBinding {
            id: &ids[index],
            revision: js_safe_u64(revisions[index], "stream revision")?,
            source_len: js_safe_u64(source_lengths[index], "stream source length")?,
            offset: js_safe_u64(offsets[index], "stream offset")?,
            source: sources[index].source(),
        });
    }
    Ok(bindings)
}

pub(super) fn stream_encoding_name(value: StreamEncoding) -> &'static str {
    match value {
        StreamEncoding::ScalarF32 => "f32",
        StreamEncoding::HiLoF32 => "f64",
    }
}

pub(super) fn stream_columns_from_metadata(
    ids: &[String],
    revisions: &[f64],
    lengths: &[f64],
    encodings: &[String],
) -> Result<Vec<StreamColumn>, JsValue> {
    if ids.len() != revisions.len() || ids.len() != lengths.len() || ids.len() != encodings.len() {
        return Err(js_err(
            "stream ids, revisions, lengths, and encodings must have equal lengths",
        ));
    }
    let mut columns = Vec::new();
    columns.try_reserve_exact(ids.len()).map_err(js_err)?;
    for index in 0..ids.len() {
        columns.push(StreamColumn {
            id: ids[index].clone(),
            len: js_safe_u64(lengths[index], &format!("stream length {index}"))?,
            encoding: parse_stream_encoding(&encodings[index])?,
            replay: StreamReplay::RandomAccess,
            revision: js_safe_u64(revisions[index], &format!("stream revision {index}"))?,
            statistics: StreamStatistics::Unknown,
        });
    }
    Ok(columns)
}

/// Result of one non-blocking exact streaming step.
#[wasm_bindgen]
pub struct StreamingStepResult {
    pub(super) status: &'static str,
    pub(super) revision: String,
    pub(super) submitted_primitives: f64,
    pub(super) total_primitives: f64,
    pub(super) pending_latest: bool,
}

#[wasm_bindgen]
impl StreamingStepResult {
    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> String {
        self.revision.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.to_owned()
    }

    #[wasm_bindgen(getter)]
    pub fn submitted_primitives(&self) -> f64 {
        self.submitted_primitives
    }

    #[wasm_bindgen(getter)]
    pub fn total_primitives(&self) -> f64 {
        self.total_primitives
    }

    #[wasm_bindgen(getter)]
    pub fn pending_latest(&self) -> bool {
        self.pending_latest
    }
}

/// Result of asking the renderer to start or coalesce one automatic draw.
#[wasm_bindgen]
pub struct AutoStreamingRequestResult {
    pub(super) status: &'static str,
    pub(super) revision: String,
    pub(super) job_id: Option<String>,
    pub(super) submitted_primitives: f64,
    pub(super) total_primitives: f64,
    pub(super) source_ids: Vec<String>,
    pub(super) source_revisions: Vec<f64>,
    pub(super) pending_latest: bool,
}

#[wasm_bindgen]
impl AutoStreamingRequestResult {
    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> String {
        self.revision.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn job_id(&self) -> Option<String> {
        self.job_id.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn submitted_primitives(&self) -> f64 {
        self.submitted_primitives
    }
    #[wasm_bindgen(getter)]
    pub fn total_primitives(&self) -> f64 {
        self.total_primitives
    }
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.to_owned()
    }

    #[wasm_bindgen(getter)]
    pub fn source_ids(&self) -> Vec<String> {
        self.source_ids.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn source_revisions(&self) -> Vec<f64> {
        self.source_revisions.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn pending_latest(&self) -> bool {
        self.pending_latest
    }
}

/// Exact range selected by the renderer-owned automatic cursor.
#[wasm_bindgen]
pub struct AutoStreamingRangeRequestResult {
    pub(super) selection_ticket: Option<renderer::StreamingSelectionTicket>,
    pub(super) status: &'static str,
    pub(super) revision: String,
    pub(super) source_ids: Vec<String>,
    pub(super) source_revisions: Vec<f64>,
    pub(super) source_lengths: Vec<f64>,
    pub(super) offsets: Vec<f64>,
    pub(super) lengths: Vec<f64>,
    pub(super) encodings: Vec<String>,
    pub(super) submitted_primitives: f64,
    pub(super) total_primitives: f64,
    pub(super) pending_latest: bool,
}

impl AutoStreamingRangeRequestResult {
    pub(super) fn from_core(
        request: renderer::AutoStreamingRangeRequest,
        revision: String,
        complete_counts: (f64, f64),
    ) -> Result<Self, JsValue> {
        let mut response = Self {
            selection_ticket: None,
            status: "backpressure",
            revision,
            source_ids: Vec::new(),
            source_revisions: Vec::new(),
            source_lengths: Vec::new(),
            offsets: Vec::new(),
            lengths: Vec::new(),
            encodings: Vec::new(),
            submitted_primitives: 0.0,
            total_primitives: 0.0,
            pending_latest: false,
        };
        match request {
            renderer::AutoStreamingRangeRequest::Ready {
                submitted_primitives,
                total_primitives,
                ranges,
                ..
            } => {
                response.status = "ready";
                response.submitted_primitives = submitted_primitives as f64;
                response.total_primitives = total_primitives as f64;
                response
                    .source_ids
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                response
                    .source_revisions
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                response
                    .source_lengths
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                response
                    .offsets
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                response
                    .lengths
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                response
                    .encodings
                    .try_reserve_exact(ranges.len())
                    .map_err(js_err)?;
                for range in ranges {
                    response.source_ids.push(range.id);
                    response.source_revisions.push(range.revision as f64);
                    response.source_lengths.push(range.source_len as f64);
                    response.offsets.push(range.offset as f64);
                    response.lengths.push(range.len as f64);
                    response
                        .encodings
                        .push(stream_encoding_name(range.encoding).to_owned());
                }
            }
            renderer::AutoStreamingRangeRequest::Backpressure {
                submitted_primitives,
                total_primitives,
                ..
            } => {
                response.submitted_primitives = submitted_primitives as f64;
                response.total_primitives = total_primitives as f64;
            }
            renderer::AutoStreamingRangeRequest::AllSubmitted {
                total_primitives, ..
            } => {
                response.status = "all_submitted";
                response.submitted_primitives = total_primitives as f64;
                response.total_primitives = total_primitives as f64;
            }
            renderer::AutoStreamingRangeRequest::Complete { pending_latest, .. } => {
                response.status = "complete";
                response.submitted_primitives = complete_counts.0;
                response.total_primitives = complete_counts.1;
                response.pending_latest = pending_latest.is_some();
            }
        }
        Ok(response)
    }
}

/// Opaque renderer-issued replay operation. Contains no column payload.
#[wasm_bindgen]
pub struct StreamingOperationHandle {
    pub(super) operation: renderer::StreamingOperation,
    pub(super) revision: String,
    pub(super) counts: Cell<(f64, f64)>,
}

#[wasm_bindgen]
impl AutoStreamingRangeRequestResult {
    pub fn same_selection_request(&self, other: &AutoStreamingRangeRequestResult) -> bool {
        self.selection_ticket.is_some() && self.selection_ticket == other.selection_ticket
    }

    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> String {
        self.revision.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn status(&self) -> String {
        self.status.to_owned()
    }

    #[wasm_bindgen(getter)]
    pub fn source_ids(&self) -> Vec<String> {
        self.source_ids.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn source_revisions(&self) -> Vec<f64> {
        self.source_revisions.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn source_lengths(&self) -> Vec<f64> {
        self.source_lengths.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn offsets(&self) -> Vec<f64> {
        self.offsets.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn lengths(&self) -> Vec<f64> {
        self.lengths.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn encodings(&self) -> Vec<String> {
        self.encodings.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn submitted_primitives(&self) -> f64 {
        self.submitted_primitives
    }

    #[wasm_bindgen(getter)]
    pub fn total_primitives(&self) -> f64 {
        self.total_primitives
    }

    #[wasm_bindgen(getter)]
    pub fn pending_latest(&self) -> bool {
        self.pending_latest
    }
}

pub(super) fn resident_admission_reason(status: renderer::ResidentAdmissionStatus) -> &'static str {
    match status {
        renderer::ResidentAdmissionStatus::Admissible => "admissible",
        renderer::ResidentAdmissionStatus::MemoryBudgetUnset => "memory_budget_unset",
        renderer::ResidentAdmissionStatus::ColumnLengthLimitExceeded { .. } => {
            "column_length_limit"
        }
        renderer::ResidentAdmissionStatus::DerivedBufferLimitExceeded { .. } => {
            "derived_buffer_limit"
        }
        renderer::ResidentAdmissionStatus::ArithmeticOverflow => "arithmetic_overflow",
        renderer::ResidentAdmissionStatus::WorkingSetLimitExceeded => "working_set_limit",
        renderer::ResidentAdmissionStatus::DeviceBufferLimitExceeded => "device_buffer_limit",
        renderer::ResidentAdmissionStatus::PoolCapacityUnavailable => "pool_capacity_unavailable",
        renderer::ResidentAdmissionStatus::MemoryBudgetExceeded => "memory_budget",
    }
}

/// Current bounded renderer-owned streaming reservations.
#[wasm_bindgen]
pub struct StreamingUsageResult {
    pub(super) active_charts: u32,
    pub(super) in_flight_chunks: u32,
    pub(super) reserved_gpu_bytes: f64,
}

#[wasm_bindgen]
impl StreamingUsageResult {
    #[wasm_bindgen(getter)]
    pub fn active_charts(&self) -> u32 {
        self.active_charts
    }

    #[wasm_bindgen(getter)]
    pub fn in_flight_chunks(&self) -> u32 {
        self.in_flight_chunks
    }

    #[wasm_bindgen(getter)]
    pub fn reserved_gpu_bytes(&self) -> f64 {
        self.reserved_gpu_bytes
    }
}

pub(super) struct PendingStreamHandoffMetadata {
    pub(super) columns: Vec<(String, usize, u64)>,
    pub(super) next_column_revision: u64,
}

#[wasm_bindgen]
impl FiggyChart {
    // ---- exact non-resident streaming ----

    /// Install the renderer's bounded streaming resources once.
    /// Byte limits are JavaScript safe integers; no source payload is retained.
    pub fn configure_streaming(
        &mut self,
        max_active_charts: u32,
        max_in_flight_chunks: u32,
        max_columns_per_chunk: u32,
        max_chunk_input_bytes: f64,
        max_in_flight_gpu_bytes: f64,
    ) -> Result<(), JsValue> {
        self.renderer
            .configure_streaming(StreamingLimits {
                max_active_charts: max_active_charts as usize,
                max_in_flight_chunks: max_in_flight_chunks as usize,
                max_columns_per_chunk: max_columns_per_chunk as usize,
                max_chunk_input_bytes: js_safe_u64(max_chunk_input_bytes, "max_chunk_input_bytes")?,
                max_in_flight_gpu_bytes: js_safe_u64(
                    max_in_flight_gpu_bytes,
                    "max_in_flight_gpu_bytes",
                )?,
            })
            .map_err(js_err)
    }

    /// Configure the total GPU budget and chart-local packed-view limit.
    /// Automatic streaming keeps original sources nonresident, then caches
    /// only source rows whose geometry reaches the current view when the
    /// completed packed view fits both limits.
    pub fn configure_auto_residency(
        &mut self,
        memory_budget_bytes: f64,
        working_set_limit_bytes: f64,
    ) -> Result<(), JsValue> {
        let memory_budget_bytes = js_safe_u64(memory_budget_bytes, "memory_budget_bytes")?;
        let working_set_limit_bytes =
            js_safe_u64(working_set_limit_bytes, "working_set_limit_bytes")?;
        let current = self.renderer.gpu_memory_usage().total_bytes();
        if memory_budget_bytes < current {
            return Err(js_err(format!(
                "memory_budget_bytes {memory_budget_bytes} is below current renderer usage {current}"
            )));
        }
        let exceeded = self.renderer.set_memory_budget(Some(memory_budget_bytes));
        debug_assert!(exceeded.is_none());
        self.renderer
            .set_auto_resident_working_set_limit(Some(working_set_limit_bytes));
        Ok(())
    }

    /// Ask the renderer to draw the latest chart state. Decoration-only
    /// changes reuse the active accumulation; data, series, view, target,
    /// and chunk-budget changes replace it after successful preflight.
    pub fn request_auto_streaming_chart(
        &mut self,
        max_primitives_per_chunk: f64,
    ) -> Result<AutoStreamingRequestResult, JsValue> {
        let max_primitives_per_chunk =
            js_safe_u64(max_primitives_per_chunk, "max_primitives_per_chunk")?;
        let (display_config, _, display_scale) = self.display_config();
        let result = self
            .renderer
            .request_auto_streaming_chart_with_display_scale(
                self.chart_id,
                &self.view,
                display_config,
                display_scale,
                StreamingChartOptions {
                    size: self.surface_size,
                    clear_color: self.clear_color,
                    max_primitives_per_chunk,
                },
            )
            .map_err(js_err)?;
        if matches!(&result, renderer::AutoStreamingRequest::Started { .. }) {
            self.request_host_redraw();
        }
        let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        Ok(match result {
            renderer::AutoStreamingRequest::Started { sources, .. } => {
                let mut source_ids = Vec::new();
                let mut source_revisions = Vec::new();
                source_ids
                    .try_reserve_exact(sources.len())
                    .map_err(js_err)?;
                source_revisions
                    .try_reserve_exact(sources.len())
                    .map_err(js_err)?;
                for source in sources {
                    source_ids.push(source.id);
                    source_revisions.push(source.revision as f64);
                }
                AutoStreamingRequestResult {
                    status: "started",
                    revision: status.revision.to_string(),
                    job_id: status.job_id.map(|id| id.to_string()),
                    submitted_primitives: status.submitted_primitives as f64,
                    total_primitives: status.total_primitives as f64,
                    source_ids,
                    source_revisions,
                    pending_latest: false,
                }
            }
            renderer::AutoStreamingRequest::Active { pending_latest, .. } => {
                AutoStreamingRequestResult {
                    status: "active",
                    revision: status.revision.to_string(),
                    job_id: status.job_id.map(|id| id.to_string()),
                    submitted_primitives: status.submitted_primitives as f64,
                    total_primitives: status.total_primitives as f64,
                    source_ids: Vec::new(),
                    source_revisions: Vec::new(),
                    pending_latest: pending_latest.is_some(),
                }
            }
            renderer::AutoStreamingRequest::Complete { pending_latest, .. } => {
                AutoStreamingRequestResult {
                    status: "complete",
                    revision: status.revision.to_string(),
                    job_id: status.job_id.map(|id| id.to_string()),
                    submitted_primitives: status.submitted_primitives as f64,
                    total_primitives: status.total_primitives as f64,
                    source_ids: Vec::new(),
                    source_revisions: Vec::new(),
                    pending_latest: pending_latest.is_some(),
                }
            }
        })
    }

    /// Advance at most one renderer-selected automatic chunk. The typed
    /// arrays are borrowed only for this call.
    pub fn auto_stream_chart_step(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<StreamingStepResult, JsValue> {
        let sources = js_stream_sources(&sources)?;
        let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
        let mut bindings = Vec::new();
        bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
        for ((id, revision), source) in ids.iter().zip(revisions).zip(&sources) {
            bindings.push(StreamSourceBinding {
                id,
                revision,
                source: source.source(),
            });
        }
        let progress = self
            .renderer
            .auto_stream_chart_step(self.chart_id, &bindings)
            .map_err(js_err)?;
        let result = match progress {
            renderer::AutoStreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
                ..
            } => StreamingStepResult {
                status: "submitted",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: submitted_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
                ..
            } => StreamingStepResult {
                status: "backpressure",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: submitted_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::AllSubmitted {
                total_primitives, ..
            } => StreamingStepResult {
                status: "all_submitted",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: total_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::Complete { pending_latest, .. } => {
                let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
                StreamingStepResult {
                    status: "complete",
                    revision: self
                        .renderer
                        .stream_status(self.chart_id)
                        .map_err(js_err)?
                        .revision
                        .to_string(),
                    submitted_primitives: counts.submitted_primitives as f64,
                    total_primitives: counts.total_primitives as f64,
                    pending_latest: pending_latest.is_some(),
                }
            }
        };
        self.request_host_redraw();
        Ok(result)
    }

    /// Register replayable stream metadata without requiring a complete
    /// typed array at the wasm boundary. Payload ranges are supplied only
    /// after the renderer requests them.
    pub fn register_streaming_column_sources(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        lengths: Vec<f64>,
        encodings: Vec<String>,
    ) -> Result<(), JsValue> {
        let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
        let mut next_columns = self.columns.clone();
        let mut next_revisions = self.column_revisions.clone();
        next_columns.try_reserve(ids.len()).map_err(js_err)?;
        next_revisions.try_reserve(ids.len()).map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        for source in &columns {
            next_columns.insert(
                source.id.clone(),
                usize::try_from(source.len).map_err(js_err)?,
            );
            next_revisions.insert(source.id.clone(), source.revision);
            next_column_revision = next_column_revision.max(
                source
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        self.renderer
            .register_streamed_columns(columns)
            .map_err(js_err)?;
        self.columns = next_columns;
        self.column_revisions = next_revisions;
        self.next_column_revision = next_column_revision;
        Ok(())
    }

    /// Replace existing stream metadata. The new revision becomes live
    /// only after the renderer accepts the whole batch.
    pub fn replace_streaming_column_sources(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        lengths: Vec<f64>,
        encodings: Vec<String>,
    ) -> Result<(), JsValue> {
        let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
        let mut next_columns = self.columns.clone();
        let mut next_revisions = self.column_revisions.clone();
        next_columns.try_reserve(ids.len()).map_err(js_err)?;
        next_revisions.try_reserve(ids.len()).map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        for source in &columns {
            next_columns.insert(
                source.id.clone(),
                usize::try_from(source.len).map_err(js_err)?,
            );
            next_revisions.insert(source.id.clone(), source.revision);
            next_column_revision = next_column_revision.max(
                source
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        self.renderer
            .replace_streamed_columns(columns)
            .map_err(js_err)?;
        self.columns = next_columns;
        self.column_revisions = next_revisions;
        self.next_column_revision = next_column_revision;
        self.retire_series_extent_cache();
        Ok(())
    }

    /// Start a two-phase resident or mixed closure replacement. Existing
    /// authority remains live until the range cursor completes.
    pub fn request_resident_stream_handoff(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        lengths: Vec<f64>,
        encodings: Vec<String>,
        max_primitives_per_chunk: f64,
    ) -> Result<AutoStreamingRequestResult, JsValue> {
        let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
        let mut pending_columns = Vec::new();
        pending_columns
            .try_reserve_exact(columns.len())
            .map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        for source in &columns {
            if !self.columns.contains_key(&source.id) {
                return Err(js_err(format!(
                    "resident stream handoff column {:?} is not registered",
                    source.id
                )));
            }
            pending_columns.push((
                source.id.clone(),
                usize::try_from(source.len).map_err(js_err)?,
                source.revision,
            ));
            next_column_revision = next_column_revision.max(
                source
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        let max_primitives_per_chunk =
            js_safe_u64(max_primitives_per_chunk, "max_primitives_per_chunk")?;
        let mut source_ids = Vec::new();
        let mut source_revisions = Vec::new();
        source_ids
            .try_reserve_exact(columns.len())
            .map_err(js_err)?;
        source_revisions
            .try_reserve_exact(columns.len())
            .map_err(js_err)?;
        source_ids.extend(columns.iter().map(|source| source.id.clone()));
        source_ids.sort_unstable();
        for id in &source_ids {
            source_revisions.push(
                columns
                    .iter()
                    .find(|source| &source.id == id)
                    .expect("sorted handoff id came from the prepared metadata")
                    .revision as f64,
            );
        }
        let (display_config, _, display_scale) = self.display_config();
        let result = self
            .renderer
            .request_resident_stream_handoff_with_display_scale(
                self.chart_id,
                &self.view,
                display_config,
                display_scale,
                columns,
                StreamingChartOptions {
                    size: self.surface_size,
                    clear_color: self.clear_color,
                    max_primitives_per_chunk,
                },
            )
            .map_err(js_err)?;
        let (name, pending_latest) = match result {
            renderer::AutoStreamingRequest::Started { .. } => {
                self.pending_stream_handoff = Some(PendingStreamHandoffMetadata {
                    columns: pending_columns,
                    next_column_revision,
                });
                self.request_host_redraw();
                ("started", false)
            }
            renderer::AutoStreamingRequest::Active { pending_latest, .. } => {
                source_ids.clear();
                source_revisions.clear();
                ("active", pending_latest.is_some())
            }
            renderer::AutoStreamingRequest::Complete { pending_latest, .. } => {
                source_ids.clear();
                source_revisions.clear();
                ("complete", pending_latest.is_some())
            }
        };
        let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        Ok(AutoStreamingRequestResult {
            status: name,
            revision: status.revision.to_string(),
            job_id: status.job_id.map(|id| id.to_string()),
            submitted_primitives: status.submitted_primitives as f64,
            total_primitives: status.total_primitives as f64,
            source_ids,
            source_revisions,
            pending_latest,
        })
    }

    /// Reserve or inspect the next exact range request. Repeated calls
    /// return the same ranges until `auto_stream_chart_submit_ranges`.
    pub fn auto_stream_chart_request_ranges(
        &mut self,
    ) -> Result<AutoStreamingRangeRequestResult, JsValue> {
        let result = match self
            .renderer
            .auto_stream_chart_request_ranges(self.chart_id)
        {
            Ok(result) => result,
            Err(error) => {
                self.pending_stream_handoff = None;
                return Err(js_err(error));
            }
        };
        let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        let response = AutoStreamingRangeRequestResult::from_core(
            result,
            counts.revision.to_string(),
            (
                counts.submitted_primitives as f64,
                counts.total_primitives as f64,
            ),
        )?;
        if response.status == "complete" {
            if let Some(pending) = self.pending_stream_handoff.take() {
                for (id, len, revision) in pending.columns {
                    self.columns.insert(id.clone(), len);
                    self.column_revisions.insert(id, revision);
                }
                self.next_column_revision = pending.next_column_revision;
                self.retire_series_extent_cache();
            }
        }
        self.request_host_redraw();
        Ok(response)
    }

    /// Independent selected-row request; it never advances the data cursor.
    pub fn stream_selection_request_ranges(
        &mut self,
    ) -> Result<AutoStreamingRangeRequestResult, JsValue> {
        let request = self
            .renderer
            .request_stream_selection_ranges(self.chart_id)
            .map_err(js_err)?;
        let revision_name = self
            .renderer
            .stream_status(self.chart_id)
            .map_err(js_err)?
            .desired_revision
            .to_string();
        let (ticket, _revision, request) = match request {
            renderer::StreamingSelectionRequest::Ready {
                ticket,
                revision,
                ranges,
            } => (
                Some(ticket),
                revision,
                renderer::AutoStreamingRangeRequest::Ready {
                    revision,
                    ranges,
                    submitted_primitives: 0,
                    total_primitives: 0,
                },
            ),
            renderer::StreamingSelectionRequest::Backpressure { revision } => (
                None,
                revision,
                renderer::AutoStreamingRangeRequest::Backpressure {
                    revision,
                    submitted_primitives: 0,
                    total_primitives: 0,
                },
            ),
            renderer::StreamingSelectionRequest::Complete { revision } => (
                None,
                revision,
                renderer::AutoStreamingRangeRequest::Complete {
                    revision,
                    pending_latest: None,
                },
            ),
            renderer::StreamingSelectionRequest::Failed { revision } => {
                let mut response = AutoStreamingRangeRequestResult::from_core(
                    renderer::AutoStreamingRangeRequest::Complete {
                        revision,
                        pending_latest: None,
                    },
                    revision_name,
                    (0.0, 0.0),
                )?;
                response.status = "failed";
                return Ok(response);
            }
        };
        let mut result =
            AutoStreamingRangeRequestResult::from_core(request, revision_name, (0.0, 0.0))?;
        result.selection_ticket = ticket;
        Ok(result)
    }

    pub fn stream_selection_submit_ranges(
        &mut self,
        request: &AutoStreamingRangeRequestResult,
        ids: Vec<String>,
        revisions: Vec<f64>,
        source_lengths: Vec<f64>,
        offsets: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<(), JsValue> {
        let ticket = request
            .selection_ticket
            .ok_or_else(|| js_err("not a selected-row request"))?;
        let sources = js_stream_sources(&sources)?;
        let bindings =
            stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
        self.renderer
            .submit_stream_selection_ranges(ticket, &bindings)
            .map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    pub fn discard_stream_selection_request(
        &mut self,
        request: &AutoStreamingRangeRequestResult,
    ) -> Result<(), JsValue> {
        let ticket = request
            .selection_ticket
            .ok_or_else(|| js_err("not a selected-row request"))?;
        self.renderer
            .abandon_stream_selection(ticket)
            .map_err(js_err)
    }

    pub fn suspend_stream_selection(&mut self) -> Result<(), JsValue> {
        self.renderer
            .suspend_stream_selection(self.chart_id)
            .map_err(js_err)
    }

    /// Submit exactly the typed-array chunks named by the pending range
    /// request. The arrays are borrowed for this call only.
    pub fn auto_stream_chart_submit_ranges(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        source_lengths: Vec<f64>,
        offsets: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<StreamingStepResult, JsValue> {
        let sources = js_stream_sources(&sources)?;
        let bindings =
            stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
        let progress = self
            .renderer
            .auto_stream_chart_submit_ranges(self.chart_id, &bindings)
            .map_err(js_err)?;
        let result = match progress {
            renderer::AutoStreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
                ..
            } => StreamingStepResult {
                status: "submitted",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: submitted_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
                ..
            } => StreamingStepResult {
                status: "backpressure",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: submitted_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::AllSubmitted {
                total_primitives, ..
            } => StreamingStepResult {
                status: "all_submitted",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: total_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::AutoStreamingProgress::Complete { pending_latest, .. } => {
                let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
                StreamingStepResult {
                    status: "complete",
                    revision: self
                        .renderer
                        .stream_status(self.chart_id)
                        .map_err(js_err)?
                        .revision
                        .to_string(),
                    submitted_primitives: counts.submitted_primitives as f64,
                    total_primitives: counts.total_primitives as f64,
                    pending_latest: pending_latest.is_some(),
                }
            }
        };
        self.request_host_redraw();
        Ok(result)
    }

    /// Queue cancellation only when an automatic stream is active.
    pub fn interrupt_render(&mut self) -> Result<String, JsValue> {
        let status = self
            .renderer
            .interrupt_render(self.chart_id)
            .map_err(js_err)?;
        self.pending_stream_handoff = None;
        Ok(match status {
            renderer::RenderInterruptStatus::StreamCancelQueued => {
                // Even an otherwise clean frame must retire the cancelled
                // surface before deciding between streaming and resident
                // display coordinates.
                self.request_host_redraw();
                "stream_cancel_queued".to_owned()
            }
            renderer::RenderInterruptStatus::Resident => "resident".to_owned(),
        })
    }

    /// Register N replayable typed-array columns without uploading or
    /// retaining their contents. Each source must be a `Float32Array` or
    /// `Float64Array`; the latter uses the renderer's hi/lo encoding.
    pub fn register_streaming_columns(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<(), JsValue> {
        let sources = js_stream_sources(&sources)?;
        let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
        let mut next_columns = self.columns.clone();
        let mut next_revisions = self.column_revisions.clone();
        next_columns.try_reserve(ids.len()).map_err(js_err)?;
        next_revisions.try_reserve(ids.len()).map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        let mut columns = Vec::new();
        columns.try_reserve_exact(ids.len()).map_err(js_err)?;
        for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
            columns.push(StreamColumn {
                id: id.clone(),
                len: source.len() as u64,
                encoding: source.encoding(),
                replay: StreamReplay::RandomAccess,
                revision: *revision,
                statistics: StreamStatistics::Unknown,
            });
            next_columns.insert(id.clone(), source.len());
            next_revisions.insert(id.clone(), *revision);
            next_column_revision = next_column_revision.max(
                revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        self.renderer
            .register_streamed_columns(columns)
            .map_err(js_err)?;
        self.columns = next_columns;
        self.column_revisions = next_revisions;
        self.next_column_revision = next_column_revision;
        Ok(())
    }

    /// Atomically publish newer revisions for existing streamed columns.
    /// The renderer still retains no JavaScript typed-array reference.
    pub fn replace_streaming_columns(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<(), JsValue> {
        let sources = js_stream_sources(&sources)?;
        let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
        let mut next_columns = self.columns.clone();
        let mut next_revisions = self.column_revisions.clone();
        next_columns.try_reserve(ids.len()).map_err(js_err)?;
        next_revisions.try_reserve(ids.len()).map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        let mut columns = Vec::new();
        columns.try_reserve_exact(ids.len()).map_err(js_err)?;
        for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
            columns.push(StreamColumn {
                id: id.clone(),
                len: source.len() as u64,
                encoding: source.encoding(),
                replay: StreamReplay::RandomAccess,
                revision: *revision,
                statistics: StreamStatistics::Unknown,
            });
            next_columns.insert(id.clone(), source.len());
            next_revisions.insert(id.clone(), *revision);
            next_column_revision = next_column_revision.max(
                revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        self.renderer
            .replace_streamed_columns(columns)
            .map_err(js_err)?;
        self.columns = next_columns;
        self.column_revisions = next_revisions;
        self.next_column_revision = next_column_revision;
        self.retire_series_extent_cache();
        Ok(())
    }

    /// Normalize a complete referenced-column closure from resident or
    /// mixed authority to exact streamed metadata. The typed arrays are
    /// borrowed only for the atomic transition and are not retained by
    /// Rust; the high-level JS executor keeps the replayable references.
    pub fn demote_auto_resident_columns(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<(), JsValue> {
        let sources = js_stream_sources(&sources)?;
        let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
        let mut next_columns = self.columns.clone();
        let mut next_revisions = self.column_revisions.clone();
        next_columns.try_reserve(ids.len()).map_err(js_err)?;
        next_revisions.try_reserve(ids.len()).map_err(js_err)?;
        let mut next_column_revision = self.next_column_revision;
        let mut columns = Vec::new();
        let mut bindings = Vec::new();
        columns.try_reserve_exact(ids.len()).map_err(js_err)?;
        bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
        for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
            columns.push(StreamColumn {
                id: id.clone(),
                len: source.len() as u64,
                encoding: source.encoding(),
                replay: StreamReplay::RandomAccess,
                revision: *revision,
                statistics: StreamStatistics::Unknown,
            });
            bindings.push(StreamSourceBinding {
                id,
                revision: *revision,
                source: source.source(),
            });
            next_columns.insert(id.clone(), source.len());
            next_revisions.insert(id.clone(), *revision);
            next_column_revision = next_column_revision.max(
                revision
                    .checked_add(1)
                    .ok_or_else(|| js_err("column revision counter exhausted"))?,
            );
        }
        let mut changed_ids = Vec::new();
        changed_ids.try_reserve_exact(ids.len()).map_err(js_err)?;
        changed_ids.extend(ids.iter().map(String::as_str));
        self.renderer
            .demote_resident_columns_to_streamed(&changed_ids, columns, &bindings)
            .map_err(js_err)?;
        self.columns = next_columns;
        self.column_revisions = next_revisions;
        self.next_column_revision = next_column_revision;
        self.retire_series_extent_cache();
        self.request_host_redraw();
        Ok(())
    }

    /// Start or restart this chart's exact streaming surface and renderer-
    /// owned CPU cursor. No LOD or downsampling is performed.
    pub fn begin_streaming(&mut self, max_primitives_per_chunk: f64) -> Result<(), JsValue> {
        let max_primitives_per_chunk =
            js_safe_u64(max_primitives_per_chunk, "max_primitives_per_chunk")?;
        let (display_config, _, _) = self.display_config();
        let display_chart = Chart::new(display_config);
        self.renderer
            .update_transform(&self.view, &display_chart)
            .map_err(js_err)?;
        self.renderer
            .begin_streaming_chart(
                self.chart_id,
                &self.view,
                StreamingChartOptions {
                    size: self.surface_size,
                    clear_color: self.clear_color,
                    max_primitives_per_chunk,
                },
            )
            .map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Advance at most one renderer-selected chunk. `sources` contains the
    /// original replayable typed arrays; only the requested range is read
    /// directly into mapped staging, and no owned CPU data mirror is made.
    pub fn stream_step(
        &mut self,
        ids: Vec<String>,
        revisions: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<StreamingStepResult, JsValue> {
        let sources = js_stream_sources(&sources)?;
        let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
        let mut bindings = Vec::new();
        bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
        for ((id, revision), source) in ids.iter().zip(revisions).zip(&sources) {
            bindings.push(StreamSourceBinding {
                id,
                revision,
                source: source.source(),
            });
        }
        let progress = self
            .renderer
            .stream_chart_step(self.chart_id, &self.view, &bindings)
            .map_err(js_err)?;
        let result = match progress {
            renderer::StreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
            } => {
                self.request_host_redraw();
                StreamingStepResult {
                    status: "submitted",
                    revision: self
                        .renderer
                        .stream_status(self.chart_id)
                        .map_err(js_err)?
                        .revision
                        .to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                }
            }
            renderer::StreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
            } => StreamingStepResult {
                status: "backpressure",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: submitted_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
            renderer::StreamingProgress::AllSubmitted { total_primitives } => StreamingStepResult {
                status: "all_submitted",
                revision: self
                    .renderer
                    .stream_status(self.chart_id)
                    .map_err(js_err)?
                    .revision
                    .to_string(),
                submitted_primitives: total_primitives as f64,
                total_primitives: total_primitives as f64,
                pending_latest: false,
            },
        };
        Ok(result)
    }

    pub fn cancel_streaming(&mut self) -> Result<(), JsValue> {
        self.renderer
            .cancel_streaming_chart(self.chart_id)
            .map_err(js_err)?;
        Ok(())
    }

    /// Cancel this chart and resolve only after its submitted work and
    /// retired renderer-owned surfaces have crossed the completion fence.
    pub async fn cancel_streaming_and_wait(&mut self) -> Result<(), JsValue> {
        self.pending_stream_handoff = None;
        self.renderer
            .cancel_streaming_chart_and_wait(self.chart_id)
            .await
            .map_err(js_err)
    }

    pub fn set_stream_chunk_budget(&mut self, primitives: f64) -> Result<(), JsValue> {
        let primitives = js_safe_u64(primitives, "stream chunk budget")?;
        self.renderer
            .set_stream_chunk_budget(self.chart_id, primitives)
            .map_err(js_err)
    }

    /// Wake after work already submitted to this queue completes. The
    /// returned promise owns only its completion receiver, not a chart or
    /// renderer borrow. Resolving does not service receipts or draw a frame.
    pub fn streaming_gpu_ready(&self) -> js_sys::Promise {
        let (sender, receiver) = futures_channel::oneshot::channel();
        self.renderer.queue().on_submitted_work_done(move || {
            let _ = sender.send(());
        });
        wasm_bindgen_futures::future_to_promise(async move {
            receiver
                .await
                .map_err(|_| js_err("stream GPU completion callback was dropped"))?;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Pure metadata inspection; does not poll, reserve a range, or draw.
    pub fn stream_status(&self) -> Result<String, JsValue> {
        let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        let residency = self
            .renderer
            .view_residency_status(self.chart_id)
            .map_err(js_err)?;
        let name = match status.status {
            renderer::StreamingState::Idle => "idle",
            renderer::StreamingState::Active => "active",
            renderer::StreamingState::AllSubmitted => "all_submitted",
            renderer::StreamingState::Complete => "complete",
            renderer::StreamingState::Cancelling => "cancelling",
            renderer::StreamingState::Cancelled => "cancelled",
        };
        Ok(serde_json::json!({
            "status": name,
            "revision": status.revision.to_string(),
            "desired_revision": status.desired_revision.to_string(),
            "published_revision": status.published_revision.map(|value| value.to_string()),
            "job_id": status.job_id.map(|value| value.to_string()),
            "submitted_primitives": status.submitted_primitives,
            "total_primitives": status.total_primitives,
            "in_flight_chunks": status.in_flight_chunks,
            "reserved_gpu_bytes": status.reserved_gpu_bytes,
            "auto_fit_pending": status.auto_fit_pending,
            "view_residency": {
                "state": residency.state,
                "needed_bytes": residency.needed_bytes,
                "refusal_reason": residency.refusal_reason,
                "picking_available": residency.picking_available,
            },
        })
        .to_string())
    }

    /// Metadata-only admission for incoming columns; no payload is read,
    /// registered, uploaded, or reserved. Chart-derived buffers are excluded.
    pub fn inspect_column_admission(&self, lengths: Vec<f64>) -> Result<String, JsValue> {
        let lengths = lengths
            .into_iter()
            .map(|value| js_safe_u64(value, "column length"))
            .collect::<Result<Vec<_>, _>>()?;
        let report = self.renderer.inspect_column_admission(&lengths);
        Ok(serde_json::json!({
            "admissible": report.is_admissible(),
            "reason": resident_admission_reason(report.status),
            "resident_column_bytes": report.resident_column_bytes,
            "resident_working_set_bytes": report.resident_working_set_bytes,
            "transition_peak_bytes": report.transition_peak_bytes,
            "current_gpu_bytes": report.current_gpu_bytes,
            "memory_budget_bytes": report.memory_budget_bytes,
            "working_set_limit_bytes": report.working_set_limit_bytes,
            "device_buffer_limit_bytes": report.device_buffer_limit_bytes,
            "includes_chart_derived_resources": false,
            "reservation": false,
        })
        .to_string())
    }

    pub fn streaming_capabilities(&self) -> String {
        let supported = self.renderer.inspect_streaming_capability(self.chart_id);
        let render_supported = supported.is_ok();
        serde_json::json!({
            "render_supported": render_supported,
            "reason": supported.err().map(|error| error.to_string()),
            "range_sources": true,
            "adaptive_chunk_budget": true,
            "async_cancel": true,
            "stream_pick": false,
            "stream_export": render_supported,
            "operations_require_completed_revision": true,
            "contour": false,
        })
        .to_string()
    }

    /// Synchronous fit request routing, allowing the facade to wait for
    /// renderer progress without holding a wasm mutable borrow.
    pub fn request_stream_auto_fit(&mut self, padding: f64) -> Result<bool, JsValue> {
        let streamed = self.chart_series().iter().any(|series| {
            referenced_columns(series).into_iter().any(|id| {
                matches!(
                    self.renderer.logical_column(id),
                    Some(renderer::LogicalColumn::Streamed(_))
                )
            })
        });
        if streamed {
            self.renderer
                .request_stream_auto_fit(self.chart_id, padding)
                .map_err(js_err)?;
        }
        Ok(streamed)
    }

    pub fn streaming_usage(&self) -> StreamingUsageResult {
        let usage = self.renderer.streaming_usage();
        StreamingUsageResult {
            active_charts: usage.active_charts as u32,
            in_flight_chunks: usage.in_flight_chunks as u32,
            reserved_gpu_bytes: usage.reserved_gpu_bytes as f64,
        }
    }

    pub fn stream_operation_request_ranges(
        &mut self,
        handle: &StreamingOperationHandle,
    ) -> Result<AutoStreamingRangeRequestResult, JsValue> {
        let request = self
            .renderer
            .request_stream_operation_ranges(handle.operation)
            .map_err(js_err)?;
        let response = AutoStreamingRangeRequestResult::from_core(
            request,
            handle.revision.clone(),
            handle.counts.get(),
        )?;
        handle
            .counts
            .set((response.submitted_primitives, response.total_primitives));
        Ok(response)
    }

    pub fn set_stream_operation_chunk_budget(
        &mut self,
        handle: &StreamingOperationHandle,
        max_primitives: f64,
    ) -> Result<(), JsValue> {
        let budget = js_safe_u64(max_primitives, "stream operation chunk budget")?;
        self.renderer
            .set_stream_operation_chunk_budget(handle.operation, budget)
            .map_err(js_err)
    }

    pub fn stream_operation_submit_ranges(
        &mut self,
        handle: &StreamingOperationHandle,
        ids: Vec<String>,
        revisions: Vec<f64>,
        source_lengths: Vec<f64>,
        offsets: Vec<f64>,
        sources: js_sys::Array,
    ) -> Result<StreamingStepResult, JsValue> {
        let sources = js_stream_sources(&sources)?;
        let bindings =
            stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
        let progress = self
            .renderer
            .submit_stream_operation_ranges(handle.operation, &bindings)
            .map_err(js_err)?;
        let (status, submitted, total) = match progress {
            renderer::StreamingProgress::Submitted {
                submitted_primitives,
                total_primitives,
            } => ("submitted", submitted_primitives, total_primitives),
            renderer::StreamingProgress::Backpressure {
                submitted_primitives,
                total_primitives,
            } => ("backpressure", submitted_primitives, total_primitives),
            renderer::StreamingProgress::AllSubmitted { total_primitives } => {
                ("all_submitted", total_primitives, total_primitives)
            }
        };
        handle.counts.set((submitted as f64, total as f64));
        Ok(StreamingStepResult {
            status,
            revision: handle.revision.clone(),
            submitted_primitives: submitted as f64,
            total_primitives: total as f64,
            pending_latest: false,
        })
    }

    pub async fn cancel_stream_operation_and_wait(
        &mut self,
        handle: &StreamingOperationHandle,
    ) -> Result<(), JsValue> {
        self.renderer
            .cancel_stream_operation_and_wait(handle.operation)
            .await
            .map_err(js_err)
    }
}
