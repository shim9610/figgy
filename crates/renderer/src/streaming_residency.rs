//! Failure-atomic resident candidates populated only by bounded source tickets.
use super::*;
use crate::data_render::column_pool::StreamedRangeCandidate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingResidencyOperation(StreamJob);

pub(super) struct StreamResidency {
    job: StreamJob,
    revision: RenderRevision,
    chart_stamps: Vec<(ChartId, RenderRevision)>,
    sources: Vec<crate::StreamColumn>,
    candidate: StreamedRangeCandidate,
    report: ResidentAdmission,
    column: usize,
    offset: u64,
    submitted: u64,
    total: u64,
    max_chunk: u64,
    max_chunk_limit: u64,
    pending: Option<StreamTicket>,
}

impl StreamResidency {
    pub(super) fn has_source(&self, job: StreamJob, column: &str) -> bool {
        self.job == job && self.sources.iter().any(|source| source.id == column)
    }

    fn current(&self, renderer: &Renderer) -> bool {
        self.candidate.is_current(&renderer.pool)
            && self.chart_stamps.iter().all(|(id, revision)| {
                renderer
                    .chart_states
                    .get(id)
                    .is_some_and(|state| state.revisions.desired == *revision)
            })
            && self.sources.iter().all(|source| {
                renderer
                    .streaming_sources
                    .get(&source.id)
                    .is_some_and(|current| {
                        current.revision == source.revision
                            && current.len == source.len
                            && current.encoding == source.encoding
                    })
            })
            && renderer.chart_states.iter().all(|(id, state)| {
                !series_list_references_any_column(
                    &state.series,
                    self.sources.iter().map(|source| source.id.as_str()),
                ) || self
                    .chart_stamps
                    .iter()
                    .any(|(known, revision)| known == id && *revision == state.revisions.desired)
            })
    }
}

pub(super) fn prune(renderer: &mut Renderer) {
    let Some(runtime) = renderer.stream_runtime.as_ref() else {
        return;
    };
    let mut index = runtime.residencies.len();
    while index > 0 {
        index -= 1;
        let state = &renderer.stream_runtime.as_ref().unwrap().residencies[index];
        if state.current(renderer)
            && renderer
                .stream_runtime
                .as_ref()
                .unwrap()
                .scheduler
                .contains_job(state.job.chart.sequence, state.job.id)
        {
            continue;
        }
        let runtime = renderer.stream_runtime.as_mut().unwrap();
        let state = runtime.residencies.swap_remove(index);
        runtime.scheduler.cancel(state.job.id);
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
    }
}

pub(super) fn cancel_chart(runtime: &mut StreamRuntime, chart: ChartId) {
    let mut index = runtime.residencies.len();
    while index > 0 {
        index -= 1;
        if runtime.residencies[index].job.chart == chart {
            let state = runtime.residencies.swap_remove(index);
            runtime.scheduler.cancel(state.job.id);
        }
    }
}

fn stale() -> FiggyError {
    FiggyError::StaleStateToken {
        reason: "resident candidate is stale or not ready".into(),
    }
}

impl Renderer {
    /// Borrow the exact referenced-column closure owned by this candidate.
    pub fn stream_residency_columns(
        &self,
        operation: StreamingResidencyOperation,
    ) -> Result<&[crate::StreamColumn]> {
        let index = self.residency_index(operation)?;
        Ok(&self.stream_runtime.as_ref().unwrap().residencies[index].sources)
    }

    fn residency_index(&self, operation: StreamingResidencyOperation) -> Result<usize> {
        if operation.0.chart.renderer_identity != self.renderer_identity {
            return Err(stale());
        }
        self.stream_runtime
            .as_ref()
            .and_then(|runtime| {
                runtime
                    .residencies
                    .iter()
                    .position(|state| state.job == operation.0)
            })
            .ok_or_else(stale)
    }

    /// Start an unpublished candidate. An unset policy or running display
    /// returns no report/token; a denied admission returns only its report.
    pub fn begin_stream_residency(
        &mut self,
        chart: ChartId,
        render_config: &Config,
        max_chunk: u64,
    ) -> Result<(
        Option<StreamingResidencyOperation>,
        Option<ResidentAdmission>,
    )> {
        self.chart_config(chart)?;
        let Some(cap) = self.auto_resident_working_set_limit else {
            return Ok((None, None));
        };
        crate::chart::validate_renderer_config(render_config)?;
        self.service_stream_requests();
        if self.active_stream_job(chart).is_some() && !self.active_auto_stream_is_terminal(chart) {
            return Ok((None, None));
        }
        if max_chunk == 0 {
            return Err(stale());
        }
        if let Some(job) = self.active_stream_job(chart) {
            self.publish_auto_stream_completion(job)
                .map_err(StreamRequestError::into_figgy)?;
        }
        let runtime = self.stream_runtime.as_ref().ok_or_else(stale)?;
        if runtime
            .residencies
            .iter()
            .any(|state| state.job.chart == chart)
        {
            return Err(stale());
        }
        let limits = runtime.scheduler.limits();
        let mut roots = std::collections::HashSet::new();
        let mut allocation_error = None;
        for series in &self.chart_states[&chart].series {
            visit_series_columns(series, &mut |id| {
                if allocation_error.is_none() {
                    if let Err(error) =
                        try_insert_column_id(&mut roots, id, "resident candidate closure")
                    {
                        allocation_error = Some(error);
                    }
                }
            });
        }
        if let Some(error) = allocation_error {
            return Err(error);
        }
        let closure = self.transition_column_closure(roots.iter().map(String::as_str))?;
        if closure.is_empty() {
            return Ok((None, None));
        }
        if self.chart_states.iter().any(|(id, state)| {
            series_list_references_any_column(&state.series, closure.iter().map(String::as_str))
                && self.active_stream_job(*id).is_some()
                && !self.active_auto_stream_is_terminal(*id)
        }) {
            return Ok((None, None));
        }
        let mut sources = Vec::new();
        sources.try_reserve_exact(closure.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident candidate source metadata",
                reason: error.to_string(),
            }
        })?;
        for id in &closure {
            let Some(source) = self.streaming_sources.get(id) else {
                return Ok((None, None));
            };
            if source.len == 0 {
                return Ok((None, None));
            }
            sources.push(source.column.clone());
        }
        sources.sort_by(|a, b| a.id.cmp(&b.id));
        let mut lengths = HashMap::new();
        lengths
            .try_reserve(sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident candidate lengths",
                reason: error.to_string(),
            })?;
        lengths.extend(
            sources
                .iter()
                .map(|source| (source.id.as_str(), source.len)),
        );
        let derived =
            self.automatic_resident_derived_buffers_for_lengths(chart, render_config, &lengths)?;
        let mut lengths = Vec::new();
        lengths.try_reserve_exact(sources.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident candidate lengths",
                reason: error.to_string(),
            }
        })?;
        lengths.extend(sources.iter().map(|source| source.len));
        let total = lengths
            .iter()
            .try_fold(0u64, |sum, len| sum.checked_add(*len))
            .ok_or_else(stale)?;
        let mut report =
            self.resident_admission_for_validated_streamed_lengths(&lengths, 0, &derived, 0, cap);
        let max_encoding = sources
            .iter()
            .map(|source| source.encoding.bytes_per_value())
            .max()
            .unwrap();
        let max_chunk_limit = (limits.max_chunk_bytes / max_encoding)
            .min(limits.max_in_flight_bytes / 16)
            .min(self.device.limits().max_buffer_size / 8)
            .min(u64::from(self.device.limits().max_storage_buffer_binding_size) / 8)
            .min(*lengths.iter().max().unwrap());
        if max_chunk_limit == 0 {
            return Err(stale());
        }
        if matches!(
            report.status,
            ResidentAdmissionStatus::Admissible
                | ResidentAdmissionStatus::MemoryBudgetExceeded
                | ResidentAdmissionStatus::MemoryBudgetUnset
        ) {
            report.pool_transition_bytes = report.pool_capacity_after;
            report.upload_staging_bytes = max_chunk_limit
                .checked_mul(16)
                .and_then(|bytes| bytes.checked_mul(limits.max_slots as u64))
                .ok_or_else(stale)?
                .min(limits.max_in_flight_bytes);
            report.transition_peak_bytes = report
                .current_gpu_bytes
                .checked_add(report.pool_transition_bytes)
                .and_then(|bytes| bytes.checked_add(report.upload_staging_bytes))
                .and_then(|bytes| bytes.checked_add(report.derived_resident_bytes))
                .ok_or_else(stale)?;
            report.status = match report.memory_budget_bytes {
                None => ResidentAdmissionStatus::MemoryBudgetUnset,
                Some(budget) if report.transition_peak_bytes > budget => {
                    ResidentAdmissionStatus::MemoryBudgetExceeded
                }
                Some(_) => ResidentAdmissionStatus::Admissible,
            };
        }
        if !report.is_admissible() {
            return Ok((None, Some(report)));
        }
        let mut chart_stamps = Vec::new();
        chart_stamps
            .try_reserve_exact(self.chart_states.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident candidate chart stamps",
                reason: error.to_string(),
            })?;
        for (id, state) in &self.chart_states {
            if *id == chart
                || series_list_references_any_column(
                    &state.series,
                    closure.iter().map(String::as_str),
                )
            {
                chart_stamps.push((*id, state.revisions.desired));
            }
        }
        let revisions = self.chart_states[&chart].revisions;
        let runtime = self.stream_runtime.as_mut().unwrap();
        runtime
            .residencies
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident candidate jobs",
                reason: error.to_string(),
            })?;
        let id = if runtime.scheduler.job_for_chart(chart.sequence).is_some() {
            runtime.scheduler.start_auxiliary(
                chart.sequence,
                SourceStamp(revisions.data.sequence),
                ViewEpoch(revisions.view.sequence),
            )
        } else {
            runtime.scheduler.start_job(
                chart.sequence,
                SourceStamp(revisions.data.sequence),
                ViewEpoch(revisions.view.sequence),
            )
        }
        .map_err(|error| StreamRequestError::from(error).into_figgy())?;
        let job = StreamJob { chart, id };
        let candidate = match self.pool.begin_streamed_range_candidate(
            &sources,
            report.pool_capacity_after,
            pool_alloc_ctx(
                &self.device,
                &self.queue,
                self.memory_budget,
                &self.gpu_ledger,
            ),
            &self.gpu_ledger,
        ) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.stream_runtime.as_mut().unwrap().scheduler.cancel(id);
                return Err(error.into());
            }
        };
        self.stream_runtime
            .as_mut()
            .unwrap()
            .residencies
            .push(StreamResidency {
                job,
                revision: revisions.desired,
                chart_stamps,
                sources,
                candidate,
                report: report.clone(),
                column: 0,
                offset: 0,
                submitted: 0,
                total,
                max_chunk: max_chunk.min(max_chunk_limit),
                max_chunk_limit,
                pending: None,
            });
        Ok((Some(StreamingResidencyOperation(job)), Some(report)))
    }

    pub fn set_stream_residency_chunk_budget(
        &mut self,
        operation: StreamingResidencyOperation,
        max_chunk: u64,
    ) -> Result<()> {
        self.service_stream_requests();
        let index = self.residency_index(operation)?;
        if max_chunk == 0 {
            return Err(stale());
        }
        let state = &mut self.stream_runtime.as_mut().unwrap().residencies[index];
        state.max_chunk = max_chunk.min(state.max_chunk_limit);
        Ok(())
    }

    pub fn request_stream_residency_ranges(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.service_stream_requests();
        let index = self.residency_index(operation)?;
        let state = &self.stream_runtime.as_ref().unwrap().residencies[index];
        let (revision, submitted_primitives, total_primitives) =
            (state.revision, state.submitted, state.total);
        if state.column == state.sources.len() {
            return Ok(
                if self
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .scheduler
                    .has_slots_for_job(state.job.id)
                {
                    crate::AutoStreamingRangeRequest::AllSubmitted {
                        revision,
                        total_primitives,
                    }
                } else {
                    crate::AutoStreamingRangeRequest::Complete {
                        revision,
                        pending_latest: None,
                    }
                },
            );
        }
        let ticket = if let Some(ticket) = state.pending {
            ticket
        } else {
            let source = &state.sources[state.column];
            let name = source.id.clone();
            let range = StreamSourceRange {
                column: &name,
                offset: state.offset,
                len: (source.len - state.offset).min(state.max_chunk),
            };
            match self
                .request_stream_columns(operation.0, &[range])
                .map_err(StreamRequestError::into_figgy)?
            {
                StreamRequestStatus::Backpressure => {
                    return Ok(crate::AutoStreamingRangeRequest::Backpressure {
                        revision,
                        submitted_primitives,
                        total_primitives,
                    });
                }
                StreamRequestStatus::Ready(ticket) => {
                    self.stream_runtime.as_mut().unwrap().residencies[index].pending = Some(ticket);
                    ticket
                }
            }
        };
        let ranges = self
            .stream_request_columns(ticket)
            .map_err(StreamRequestError::into_figgy)?
            .iter()
            .map(|column| crate::AutoStreamRange {
                id: column.column.clone(),
                revision: column.range.revision,
                source_len: column.range.source_len,
                offset: column.range.offset,
                len: column.range.len,
                encoding: column.range.encoding,
            })
            .collect();
        Ok(crate::AutoStreamingRangeRequest::Ready {
            revision,
            submitted_primitives,
            total_primitives,
            ranges,
        })
    }

    pub fn submit_stream_residency_ranges(
        &mut self,
        operation: StreamingResidencyOperation,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.service_stream_requests();
        let index = self.residency_index(operation)?;
        let state = &self.stream_runtime.as_ref().unwrap().residencies[index];
        let ticket = state.pending.ok_or_else(stale)?;
        let column = &state.sources[state.column];
        let target = state.candidate.pool().buffer().clone();
        let offset = state.candidate.pool().slot(&column.id).unwrap().offset + state.offset * 8;
        let requested = self
            .stream_request_columns(ticket)
            .map_err(StreamRequestError::into_figgy)?;
        if requested.len() != 1 || sources.len() != 1 {
            return Err(stale());
        }
        let range = requested[0].range;
        let input = resolve_stream_range_source(&requested[0], sources)?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("bounded resident candidate range"),
            });
        let chunk =
            match self.accept_stream_sources_with_headroom(ticket, &[input], &mut encoder, 0) {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop(encoder);
                    self.end_gpu_frame();
                    return Err(error.into_figgy());
                }
            };
        let source = chunk
            .column_handle(range)
            .map_err(|error| StreamRequestError::from(error).into_figgy())?;
        encoder.copy_buffer_to_buffer(&chunk.work, source.offset, &target, offset, range.len * 8);
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop((chunk, target));
        if let Err(error) = result {
            self.discard_stream_recording(ticket)
                .map_err(StreamRequestError::into_figgy)?;
            if let Some(state) = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .residencies
                .iter_mut()
                .find(|state| state.job == operation.0)
            {
                state.pending = None;
            }
            self.end_gpu_frame();
            return Err(error.into_figgy());
        }
        let state = &mut self.stream_runtime.as_mut().unwrap().residencies[index];
        state.offset += range.len;
        state.submitted += range.len;
        state.pending = None;
        if state.offset == state.sources[state.column].len {
            state.column += 1;
            state.offset = 0;
        }
        let progress = crate::StreamingProgress::Submitted {
            submitted_primitives: state.submitted,
            total_primitives: state.total,
        };
        self.end_gpu_frame();
        Ok(progress)
    }

    pub fn finish_stream_residency(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<ResidentAdmission> {
        self.service_stream_requests();
        let index = self.residency_index(operation)?;
        let state = &self.stream_runtime.as_ref().unwrap().residencies[index];
        if state.column != state.sources.len()
            || state.chart_stamps.iter().any(|(chart, _)| {
                self.active_stream_job(*chart).is_some()
                    && !self.active_auto_stream_is_terminal(*chart)
            })
            || state.pending.is_some()
            || self
                .stream_runtime
                .as_ref()
                .unwrap()
                .scheduler
                .has_slots_for_job(state.job.id)
            || self
                .auto_resident_working_set_limit
                .is_none_or(|cap| state.report.resident_working_set_bytes > cap)
        {
            return Err(stale());
        }
        let usage = self.gpu_memory_usage().total_bytes();
        if self.memory_budget.is_none_or(|budget| {
            usage
                .checked_add(state.report.derived_resident_bytes)
                .is_none_or(|peak| peak > budget)
        }) {
            return Err(stale());
        }
        let mut names = Vec::new();
        names
            .try_reserve_exact(state.sources.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "resident candidate commit columns",
                reason: error.to_string(),
            })?;
        for source in &state.sources {
            let mut name = String::new();
            name.try_reserve_exact(source.id.len()).map_err(|error| {
                FiggyError::StateAllocationFailed {
                    resource: "resident candidate commit column id",
                    reason: error.to_string(),
                }
            })?;
            name.push_str(&source.id);
            names.push(name);
        }
        let invalidation = self.prepare_columns_invalidation(names.iter().map(String::as_str))?;
        let runtime = self.stream_runtime.as_mut().unwrap();
        for id in &names {
            let bounds = match self.streaming_sources[id].statistics {
                crate::StreamStatistics::Known(bounds) => bounds,
                _ => return Err(stale()),
            };
            runtime.residencies[index]
                .candidate
                .set_statistics(id, bounds);
        }
        let mut changed = Vec::new();
        changed.try_reserve_exact(names.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "resident candidate picker columns",
                reason: error.to_string(),
            }
        })?;
        changed.extend(names.iter().map(String::as_str));
        let prepared_picker = prepare_picker_for_pool_mutation(
            &mut self.picker,
            runtime.residencies[index].candidate.pool(),
            &self.chart_states,
            true,
            &changed,
        )?;
        let state = runtime.residencies.swap_remove(index);
        state.candidate.commit(&mut self.pool);
        for id in &names {
            self.streaming_sources.remove(id);
        }
        invalidation.publish(&mut self.chart_states, &mut self.visual_revision);
        if let Some(picker) = prepared_picker {
            picker.commit();
        }
        self.pending_defrag = false;
        runtime.scheduler.cancel(state.job.id);
        for (chart, _) in state.chart_stamps {
            self.retire_stream_after_resident_commit(chart);
        }
        self.end_gpu_frame();
        Ok(state.report)
    }

    pub fn cancel_stream_residency(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<()> {
        self.service_stream_requests();
        let index = self.residency_index(operation)?;
        let runtime = self.stream_runtime.as_mut().unwrap();
        let state = runtime.residencies.swap_remove(index);
        runtime.scheduler.cancel(state.job.id);
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
        drop(state);
        self.end_gpu_frame();
        Ok(())
    }
}

impl WindowedRenderer<'_> {
    pub fn stream_residency_columns(
        &self,
        operation: StreamingResidencyOperation,
    ) -> Result<&[crate::StreamColumn]> {
        self.inner.stream_residency_columns(operation)
    }
    pub fn begin_stream_residency(
        &mut self,
        chart: ChartId,
        render_config: &Config,
        max_chunk: u64,
    ) -> Result<(
        Option<StreamingResidencyOperation>,
        Option<ResidentAdmission>,
    )> {
        self.inner
            .begin_stream_residency(chart, render_config, max_chunk)
    }
    pub fn request_stream_residency_ranges(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.inner.request_stream_residency_ranges(operation)
    }
    pub fn submit_stream_residency_ranges(
        &mut self,
        operation: StreamingResidencyOperation,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.inner
            .submit_stream_residency_ranges(operation, sources)
    }
    pub fn set_stream_residency_chunk_budget(
        &mut self,
        operation: StreamingResidencyOperation,
        max_chunk: u64,
    ) -> Result<()> {
        self.inner
            .set_stream_residency_chunk_budget(operation, max_chunk)
    }
    pub fn finish_stream_residency(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<ResidentAdmission> {
        self.inner.finish_stream_residency(operation)
    }
    pub fn cancel_stream_residency(
        &mut self,
        operation: StreamingResidencyOperation,
    ) -> Result<()> {
        self.inner.cancel_stream_residency(operation)
    }
}

#[cfg(test)]
#[path = "streaming_residency_tests.rs"]
mod tests;
