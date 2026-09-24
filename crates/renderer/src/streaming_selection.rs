//! Replaceable selection suffixes. Source rows are borrowed only for upload;
//! the visible suffix remains authoritative while a candidate is incomplete.
use super::super::selection_prepare::{SelectionColumns, SelectionRefFilter, SelectionRows};
use super::*;

#[path = "streaming_field_selection.rs"]
mod field;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingSelectionTicket {
    ticket: StreamTicket,
    revision: RenderRevision,
}
impl StreamingSelectionTicket {
    pub(super) fn raw(self) -> StreamTicket {
        self.ticket
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingSelectionRequest {
    Ready {
        ticket: StreamingSelectionTicket,
        revision: RenderRevision,
        ranges: Vec<crate::AutoStreamRange>,
    },
    Backpressure {
        revision: RenderRevision,
    },
    Complete {
        revision: RenderRevision,
    },
    Failed {
        revision: RenderRevision,
    },
}

#[derive(Clone, Copy, Default)]
struct SelectionCursor {
    series: usize,
    pass: u8,
    ordinal: usize,
}

#[derive(Clone, Copy)]
struct SelectionEntry {
    series: usize,
    index: usize,
    filter: SelectionRefFilter,
    bar: bool,
    field: Option<field::Cell>,
}

#[derive(Default)]
pub(super) struct StreamSelectionState {
    revision: Option<RenderRevision>,
    view_revision: u64,
    pub(super) ready: Vec<StreamSelectionPacket>,
    candidate: Vec<StreamSelectionPacket>,
    cursor: SelectionCursor,
    pending: Option<StreamTicket>,
    entry: Option<SelectionEntry>,
    next_cursor: SelectionCursor,
    admitted: bool,
    complete: bool,
    failed: bool,
    last_issued: Option<StreamTicket>,
}

pub(super) struct StreamSelectionPacket {
    pub(super) packet: PreparedSeries,
    _map_charge: Option<crate::gpu_memory::SharedCharge>,
}

impl StreamSelectionState {
    pub(super) fn pending(&self) -> Option<StreamTicket> {
        self.pending
    }
    pub(super) fn terminal(&self) -> bool {
        self.complete || self.failed
    }
}

fn next_entry(
    config: &Config,
    series: &[SeriesConfig],
    mut cursor: SelectionCursor,
    mut len: impl FnMut(&str) -> Option<u64>,
) -> Option<(SelectionEntry, SelectionCursor)> {
    while let Some(declaration) = series.get(cursor.series) {
        let primitives = effective_series_primitives(&config.draw_style, &declaration.render_type);
        let mut selected = None;
        match cursor.pass {
            0 => {
                if let Some(picks) = config
                    .picked_data
                    .as_ref()
                    .filter(|p| p.visible && (primitives.bar || primitives.field))
                {
                    while let Some(pick) = picks.refs.get(cursor.ordinal) {
                        let ordinal = cursor.ordinal;
                        cursor.ordinal += 1;
                        if primitives.field
                            && picked_data_ref_matches_series(declaration, pick)
                            && let PickedDataRef::MatrixCell {
                                x_index, y_index, ..
                            } = pick
                            && let Some(cell) =
                                field::Cell::resolve(declaration, [*x_index, *y_index], &mut len)
                        {
                            return Some((
                                SelectionEntry {
                                    series: cursor.series,
                                    index: 0,
                                    filter: SelectionRefFilter::Typed(ordinal),
                                    bar: false,
                                    field: Some(cell),
                                },
                                cursor,
                            ));
                        }
                        if picked_data_ref_matches_series(declaration, pick)
                            && let PickedDataRef::HistogramBin { bin_index, .. } = pick
                        {
                            selected = Some((*bin_index, SelectionRefFilter::Typed(ordinal), true));
                            break;
                        }
                    }
                }
            }
            1 => {
                if let Some(picks) = config
                    .picked_points
                    .as_ref()
                    .filter(|p| p.visible && (primitives.line || primitives.scatter))
                {
                    while let Some(pick) = picks.refs.get(cursor.ordinal) {
                        let ordinal = cursor.ordinal;
                        cursor.ordinal += 1;
                        if picked_ref_matches_series(declaration, pick) {
                            selected = Some((
                                pick.point_index,
                                SelectionRefFilter::Legacy(ordinal),
                                false,
                            ));
                            break;
                        }
                    }
                }
            }
            2 => {
                if let Some(picks) = config
                    .picked_data
                    .as_ref()
                    .filter(|p| p.visible && (primitives.line || primitives.scatter))
                {
                    while let Some(pick) = picks.refs.get(cursor.ordinal) {
                        let ordinal = cursor.ordinal;
                        cursor.ordinal += 1;
                        if picked_data_ref_matches_series(declaration, pick)
                            && let PickedDataRef::Point { point_index, .. } = pick
                        {
                            selected =
                                Some((*point_index, SelectionRefFilter::Typed(ordinal), false));
                            break;
                        }
                    }
                }
            }
            _ => {
                cursor.series += 1;
                cursor.pass = 0;
                cursor.ordinal = 0;
                continue;
            }
        }
        if let Some((index, filter, bar)) = selected {
            let x = len(&declaration.x_column).unwrap_or(0);
            let y = len(&declaration.y_column).unwrap_or(0);
            let count = if bar {
                if histogram_edge_column(declaration) == Some(declaration.x_column.as_str()) {
                    x.saturating_sub(1).min(y)
                } else {
                    y.saturating_sub(1).min(x)
                }
            } else {
                x.min(y)
            };
            if (index as u64) < count {
                return Some((
                    SelectionEntry {
                        series: cursor.series,
                        index,
                        filter,
                        bar,
                        field: None,
                    },
                    cursor,
                ));
            }
        } else {
            cursor.pass += 1;
            cursor.ordinal = 0;
        }
    }
    None
}

struct SelectionColumnPlan<'a> {
    ids: [&'a str; 3],
    rows: [u64; 3],
    offsets: [u64; 3],
    count: usize,
}
fn columns_for<'a>(
    config: &Config,
    series: &'a SeriesConfig,
    entry: SelectionEntry,
) -> SelectionColumnPlan<'a> {
    let mut plan = SelectionColumnPlan {
        ids: [""; 3],
        rows: [0; 3],
        offsets: [entry.index as u64; 3],
        count: 0,
    };
    if let Some(cell) = entry.field {
        plan.ids = [&series.x_column, &series.y_column, ""];
        plan.rows = [cell.axes[0].len, cell.axes[1].len, 0];
        plan.offsets = [cell.axes[0].offset, cell.axes[1].offset, 0];
        plan.count = if series.x_column == series.y_column
            && plan.offsets[0] == plan.offsets[1]
            && plan.rows[0] == plan.rows[1]
        {
            1
        } else {
            2
        };
        return plan;
    }
    let mut add = |id: &'a str, rows| {
        if let Some(index) = plan.ids[..plan.count].iter().position(|old| *old == id) {
            plan.rows[index] = plan.rows[index].max(rows);
        } else {
            plan.ids[plan.count] = id;
            plan.rows[plan.count] = rows;
            plan.count += 1;
        }
    };
    for id in [&series.x_column, &series.y_column] {
        add(
            id,
            if entry.bar && histogram_edge_column(series) == Some(id.as_str()) {
                2
            } else {
                1
            },
        );
    }
    if !entry.bar
        && matches!(config.draw_style, DrawStyle::Precise)
        && let Some(id) = extract_scatter(&series.render_type)
            .and_then(|scatter| scatter.point_style_index_column.as_deref())
    {
        add(id, 1);
    }
    plan
}

impl Renderer {
    fn selection_config(&self, job: StreamJob) -> StreamResult<(Config, RenderRevision)> {
        self.validate_stream_job(job)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        let state = &self.chart_states[&job.chart];
        if let Some(snapshot) = draw.auto_snapshot() {
            if draw.auxiliary.is_some() {
                return Ok((snapshot.config.clone(), snapshot.desired));
            }
            let mut config = snapshot.config.clone();
            if state.revisions.data == snapshot.data_revision
                && state.revisions.series == snapshot.series_revision
                && state.revisions.view == snapshot.view_revision
            {
                let current = state.config.scaled(snapshot.display_scale);
                config.picked_points = current.picked_points;
                config.picked_data = current.picked_data;
            }
            Ok((config, state.revisions.selection))
        } else {
            Ok((state.config.clone(), state.revisions.selection))
        }
    }

    pub(super) fn discard_pending_selection(&mut self, job: StreamJob) -> StreamResult<()> {
        let runtime = self
            .stream_runtime
            .as_mut()
            .ok_or(StreamError::WrongState)?;
        let pending = runtime
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .and_then(|draw| draw.selection.pending.take());
        if let Some(ticket) = pending {
            if runtime.scheduler.is_requested(ticket.ticket) {
                runtime.scheduler.discard_requested(ticket.ticket)?;
            }
            runtime.requests.retain(|request| request.ticket != ticket);
        }
        Ok(())
    }

    fn prepare_selection_candidate(
        &mut self,
        job: StreamJob,
        config: &Config,
        revision: RenderRevision,
    ) -> StreamResult<()> {
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let view_revision = draw.expected_view_revision;
        let changed = draw.selection.revision != Some(revision)
            || draw.selection.view_revision != view_revision;
        if changed {
            self.discard_pending_selection(job)?;
            let state = &mut self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .selection;
            state.candidate.clear();
            state.cursor = SelectionCursor::default();
            state.entry = None;
            state.revision = Some(revision);
            state.view_revision = view_revision;
            state.complete = false;
            state.admitted = false;
            state.failed = false;
            state.last_issued = None;
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        if draw.selection.admitted || draw.selection.failed {
            return Ok(());
        }
        let snapshot = draw.auto_snapshot();
        let series = snapshot.map_or(self.chart_states[&job.chart].series.as_slice(), |s| {
            s.series.as_slice()
        });
        let sources = |id: &str| {
            snapshot.map_or_else(
                || self.streaming_sources.get(id).map(|source| &source.column),
                |s| s.sources.get(id),
            )
        };
        let styles = snapshot.map_or_else(
            || {
                &self.chart_states[&job.chart]
                    .prepared_styles
                    .as_ref()
                    .unwrap()
                    .styles
            },
            |s| &s.styles.styles,
        );
        let limits = self.stream_runtime.as_ref().unwrap().scheduler.limits();
        let mut cursor = SelectionCursor::default();
        let mut bytes = 0u64;
        let mut staging = 0u64;
        let mut count = 0usize;
        while let Some((entry, next)) =
            next_entry(config, series, cursor, |id| sources(id).map(|s| s.len))
        {
            cursor = next;
            let columns = columns_for(config, &series[entry.series], entry);
            let mut work = 0u64;
            let mut input = 0u64;
            for index in 0..columns.count {
                let source = sources(columns.ids[index]).ok_or(StreamError::InvalidRange)?;
                if columns.offsets[index]
                    .checked_add(columns.rows[index])
                    .ok_or(StreamError::Overflow)?
                    > source.len
                {
                    return Err(StreamError::InvalidRange.into());
                }
                work += columns.rows[index] * 8;
                input += columns.rows[index] * source.encoding.bytes_per_value();
            }
            if columns.count > limits.max_columns_per_request
                || input > limits.max_chunk_bytes
                || work * 2 > limits.max_in_flight_bytes
                || work > self.device.limits().max_buffer_size
            {
                return Err(StreamError::TooLarge.into());
            }
            let mapped = entry.field.is_none()
                && !entry.bar
                && matches!(config.draw_style, DrawStyle::Precise)
                && styles[entry.series].scatter_map.is_some();
            let overhead = if entry.field.is_some() {
                field::OVERHEAD
            } else {
                std::mem::size_of::<PrimitiveStyle>() as u64
                    + if entry.bar {
                        std::mem::size_of::<data_render::DataSelectionGpu>() as u64
                    } else {
                        0
                    }
                    + if mapped { 16 } else { 0 }
            };
            bytes = bytes
                .checked_add(work + overhead)
                .ok_or(StreamError::Overflow)?;
            staging = staging.max(work);
            count = count.checked_add(1).ok_or(StreamError::Overflow)?;
        }
        if self
            .gpu_memory_usage()
            .total_bytes()
            .checked_add(bytes)
            .and_then(|value| value.checked_add(staging))
            .ok_or(StreamError::Overflow)?
            > self.memory_budget.unwrap_or(u64::MAX)
        {
            return Err(StreamError::TooLarge.into());
        }
        let state = &mut self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap()
            .selection;
        state
            .candidate
            .try_reserve_exact(count)
            .map_err(|_| StreamError::AllocationFailed)?;
        state.admitted = true;
        Ok(())
    }

    pub(super) fn request_selection_job(
        &mut self,
        job: StreamJob,
    ) -> StreamResult<StreamingSelectionRequest> {
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
        let revision = draw
            .auxiliary
            .as_ref()
            .map_or(self.chart_states[&job.chart].revisions.selection, |aux| {
                aux.snapshot.desired
            });
        if draw.selection.revision == Some(revision)
            && draw.selection.view_revision == draw.expected_view_revision
        {
            if draw.selection.complete {
                return Ok(StreamingSelectionRequest::Complete { revision });
            }
            if draw.selection.failed {
                return Ok(StreamingSelectionRequest::Failed { revision });
            }
            if let Some(ticket) = draw.selection.pending {
                return self.selection_ready_request(ticket, revision);
            }
        }
        let (config, revision) = self.selection_config(job)?;
        if let Err(error) = self.prepare_selection_candidate(job, &config, revision) {
            self.stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .selection
                .failed = true;
            return Err(error);
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        if draw.selection.complete {
            return Ok(StreamingSelectionRequest::Complete { revision });
        }
        if draw.selection.failed {
            return Ok(StreamingSelectionRequest::Failed { revision });
        }
        let ticket = if let Some(ticket) = draw.selection.pending {
            ticket
        } else {
            let snapshot = draw.auto_snapshot();
            let series = snapshot.map_or(self.chart_states[&job.chart].series.as_slice(), |s| {
                s.series.as_slice()
            });
            let next = next_entry(&config, series, draw.selection.cursor, |id| {
                snapshot.map_or_else(
                    || self.streaming_sources.get(id).map(|s| s.len),
                    |s| s.sources.get(id).map(|s| s.len),
                )
            });
            let Some((entry, next_cursor)) = next else {
                let draw = self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap();
                draw.selection.ready = std::mem::take(&mut draw.selection.candidate);
                draw.selection.complete = true;
                draw.display_dirty = true;
                self.end_gpu_frame();
                return Ok(StreamingSelectionRequest::Complete { revision });
            };
            let columns = columns_for(&config, &series[entry.series], entry);
            let names: [String; 3] = std::array::from_fn(|i| columns.ids[i].to_owned());
            let rows = columns.rows;
            let offsets = columns.offsets;
            let count = columns.count;
            let ranges: [StreamSourceRange<'_>; 3] = std::array::from_fn(|i| StreamSourceRange {
                column: &names[i],
                offset: offsets[i],
                len: rows[i],
            });
            let StreamRequestStatus::Ready(ticket) =
                self.request_stream_columns(job, &ranges[..count])?
            else {
                return Ok(StreamingSelectionRequest::Backpressure { revision });
            };
            let runtime = self.stream_runtime.as_mut().unwrap();
            runtime
                .requests
                .iter_mut()
                .find(|request| request.ticket == ticket)
                .unwrap()
                .selection = true;
            let state = &mut runtime
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .selection;
            state.pending = Some(ticket);
            state.entry = Some(entry);
            state.next_cursor = next_cursor;
            state.last_issued = Some(ticket);
            ticket
        };
        self.selection_ready_request(ticket, revision)
    }

    fn selection_ready_request(
        &mut self,
        ticket: StreamTicket,
        revision: RenderRevision,
    ) -> StreamResult<StreamingSelectionRequest> {
        let ranges = self
            .stream_request_columns(ticket)?
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
        Ok(StreamingSelectionRequest::Ready {
            ticket: StreamingSelectionTicket { ticket, revision },
            revision,
            ranges,
        })
    }

    pub fn request_stream_selection_ranges(
        &mut self,
        chart: ChartId,
    ) -> Result<StreamingSelectionRequest> {
        let job = self
            .active_stream_job(chart)
            .ok_or_else(|| StreamRequestError::from(StreamError::Stale).into_figgy())?;
        self.request_selection_job(job)
            .map_err(StreamRequestError::into_figgy)
    }

    pub fn submit_stream_selection_ranges(
        &mut self,
        ticket: StreamingSelectionTicket,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<()> {
        let ordered = self
            .ordered_selection_ranges(ticket.ticket, sources)
            .map_err(StreamRequestError::into_figgy)?;
        self.submit_selection_supply(ticket.ticket, StreamSupply::Sources(&ordered), None)
            .map_err(StreamRequestError::into_figgy)
    }

    pub fn abandon_stream_selection(&mut self, ticket: StreamingSelectionTicket) -> Result<()> {
        self.validate_stream_job(ticket.ticket.job)
            .map_err(StreamRequestError::into_figgy)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == ticket.ticket.job)
            .unwrap();
        if draw.selection.revision != Some(ticket.revision)
            || draw.selection.last_issued != Some(ticket.ticket)
        {
            return Err(StreamRequestError::from(StreamError::Stale).into_figgy());
        }
        if draw.selection.complete {
            return Ok(());
        }
        self.discard_pending_selection(ticket.ticket.job)
            .map_err(StreamRequestError::into_figgy)?;
        let state = &mut self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == ticket.ticket.job)
            .unwrap()
            .selection;
        state.candidate.clear();
        state.entry = None;
        state.failed = true;
        self.end_gpu_frame();
        Ok(())
    }

    /// Release a pending provider admission without discarding the candidate or
    /// the visible suffix. A later request retries the same row with a new ticket.
    pub fn suspend_stream_selection(&mut self, chart: ChartId) -> Result<()> {
        let job = self
            .active_stream_job(chart)
            .ok_or_else(|| StreamRequestError::from(StreamError::Stale).into_figgy())?;
        self.discard_pending_selection(job)
            .map_err(StreamRequestError::into_figgy)?;
        let state = &mut self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap()
            .selection;
        state.entry = None;
        state.last_issued = None;
        Ok(())
    }

    fn ordered_selection_ranges<'a>(
        &mut self,
        ticket: StreamTicket,
        sources: &'a [crate::StreamRangeSourceBinding<'a>],
    ) -> StreamResult<Vec<StreamSourceInput<'a>>> {
        let requested = self.stream_request_columns(ticket)?;
        requested
            .iter()
            .map(|column| {
                resolve_stream_range_source(column, sources).map_err(StreamRequestError::from)
            })
            .collect()
    }

    pub(super) fn submit_selection_supply(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        explicit_view: Option<&ChartView>,
    ) -> StreamResult<()> {
        let (config, revision) = self.selection_config(ticket.job)?;
        let snapshot = self.auto_stream_snapshot(ticket.job);
        let view = snapshot
            .as_ref()
            .map(|s| &s.view)
            .or(explicit_view)
            .ok_or(StreamError::WrongState)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == ticket.job)
            .ok_or(StreamError::Stale)?;
        if draw.selection.pending != Some(ticket)
            || draw.selection.revision != Some(revision)
            || draw.selection.view_revision != draw.expected_view_revision
            || view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
        {
            return Err(StreamError::Stale.into());
        }
        let entry = draw.selection.entry.ok_or(StreamError::WrongState)?;
        let auxiliary = draw.auxiliary.clone();
        self.validate_stream_supply_kind(ticket, supply)?;
        let ranges: Vec<_> = self
            .stream_request_columns(ticket)?
            .iter()
            .map(|c| c.range)
            .collect();
        let series_config = snapshot.as_ref().map_or_else(
            || &self.chart_states[&ticket.job.chart].series[entry.series],
            |s| &s.series[entry.series],
        );
        let styles = snapshot.as_ref().map_or_else(
            || {
                &self.chart_states[&ticket.job.chart]
                    .prepared_styles
                    .as_ref()
                    .unwrap()
                    .styles
            },
            |s| &s.styles.styles,
        );
        let style = &styles[entry.series];
        let mapped = entry.field.is_none()
            && !entry.bar
            && matches!(config.draw_style, DrawStyle::Precise)
            && style.scatter_map.is_some();
        if auxiliary.is_none() {
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
                    chart_config: &config,
                    series: &[Series {
                        config: series_config,
                        style,
                    }],
                }],
            );
        }
        let headroom = if entry.field.is_some() {
            field::OVERHEAD
        } else {
            std::mem::size_of::<PrimitiveStyle>() as u64
                + if entry.bar {
                    std::mem::size_of::<data_render::DataSelectionGpu>() as u64
                } else {
                    0
                }
                + if mapped { 16 } else { 0 }
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("stream selection rows"),
            });
        let chunk =
            match self.accept_stream_supply_with_headroom(ticket, supply, &mut encoder, headroom) {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop(encoder);
                    self.end_gpu_frame();
                    return Err(error);
                }
            };
        let packet = (|| -> StreamResult<StreamSelectionPacket> {
            let (states, preparation) = self.preparation_parts();
            let series_config = snapshot.as_ref().map_or_else(
                || &states[&ticket.job.chart].series[entry.series],
                |s| &s.series[entry.series],
            );
            let styles = snapshot.as_ref().map_or_else(
                || {
                    &states[&ticket.job.chart]
                        .prepared_styles
                        .as_ref()
                        .unwrap()
                        .styles
                },
                |s| &s.styles.styles,
            );
            let style = &styles[entry.series];
            if let Some(cell) = entry.field {
                return field::packet(
                    &preparation,
                    view,
                    &config,
                    auxiliary
                        .as_ref()
                        .and_then(|a| a.pipelines.as_ref())
                        .unwrap_or(preparation.pipelines),
                    &chunk,
                    &ranges,
                    cell,
                );
            }
            let columns = columns_for(&config, series_config, entry);
            let lookup = |id: &str| -> Result<ColumnHandle> {
                let index = columns.ids[..columns.count]
                    .iter()
                    .position(|column| *column == id)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
                chunk
                    .column_handle(ranges[index])
                    .map_err(|error| StreamRequestError::from(error).into_figgy())
            };
            let x = lookup(&series_config.x_column)?;
            let y = lookup(&series_config.y_column)?;
            let bar = entry.bar.then(|| {
                if histogram_edge_column(series_config) == Some(series_config.x_column.as_str()) {
                    (x, y)
                } else {
                    (y, x)
                }
            });
            let tally = crate::gpu_memory::ChargeTally::new();
            let map = if mapped {
                Some(style.scatter_map.as_ref().unwrap().stream_bind_group(
                    preparation.device,
                    preparation.per_point_style_map_bgl,
                    u32::try_from(entry.index).map_err(|_| StreamError::TooLarge)?,
                    &tally,
                ))
            } else {
                None
            };
            let map_charge = mapped.then(|| {
                crate::gpu_memory::shared_charge(
                    tally,
                    preparation.gpu_ledger,
                    GpuResourceKind::Uniform,
                )
            });
            let layers = preparation.build_selection_layers(
                view,
                &config,
                &Series {
                    config: series_config,
                    style,
                },
                auxiliary
                    .as_ref()
                    .and_then(|a| a.pipelines.as_ref())
                    .unwrap_or(preparation.pipelines),
                SelectionColumns {
                    buffer: &chunk.work,
                    x,
                    y,
                    bar,
                    rows: SelectionRows {
                        global_start: entry.index,
                        refs: entry.filter,
                    },
                    mapped_point_bg: map.as_ref(),
                },
                None,
                matches!(config.draw_style, DrawStyle::Precise),
                lookup,
            )?;
            let tally = crate::gpu_memory::ChargeTally::new();
            tally.add(
                (layers.picked.len() + layers.selected_bars.len()) as u64
                    * std::mem::size_of::<PrimitiveStyle>() as u64,
            );
            let mut packet = PreparedSeries::from_layers(layers.into_series_layers(), None);
            packet._column_charge = Some(chunk.work.shared_charge());
            packet._stream_style_charge = Some(crate::gpu_memory::shared_charge(
                tally,
                &self.gpu_ledger,
                GpuResourceKind::Uniform,
            ));
            packet._stream_transform_charge = Some(view.transform_buffer.shared_charge());
            Ok(StreamSelectionPacket {
                packet,
                _map_charge: map_charge,
            })
        })();
        let packet = match packet {
            Ok(packet) => packet,
            Err(error) => {
                drop((encoder, chunk));
                self.discard_stream_recording(ticket)?;
                self.clear_selection_pending(ticket);
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let submitted = self.queue_stream_recording(ticket, encoder.finish());
        drop(chunk);
        match submitted {
            Ok(_) => {
                let state = &mut self
                    .stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == ticket.job)
                    .unwrap()
                    .selection;
                state.candidate.push(packet);
                state.cursor = state.next_cursor;
                state.pending = None;
                state.entry = None;
                self.end_gpu_frame();
                Ok(())
            }
            Err(error) => {
                drop(packet);
                self.discard_stream_recording(ticket)?;
                self.clear_selection_pending(ticket);
                self.end_gpu_frame();
                Err(error)
            }
        }
    }

    fn clear_selection_pending(&mut self, ticket: StreamTicket) {
        if let Some(draw) = self
            .stream_runtime
            .as_mut()
            .and_then(|runtime| runtime.draws.iter_mut().find(|draw| draw.job == ticket.job))
        {
            draw.selection.pending = None;
            draw.selection.entry = None;
        }
    }

    pub(super) fn pump_stream_selection_sources(
        &mut self,
        job: StreamJob,
        view: Option<&ChartView>,
        sources: &[crate::StreamSourceBinding<'_>],
    ) -> StreamResult<()> {
        if let StreamingSelectionRequest::Ready { ticket, .. } = self.request_selection_job(job)? {
            let ordered: Vec<_> = self
                .stream_request_columns(ticket.ticket)?
                .iter()
                .map(|column| {
                    resolve_stream_source(column, sources).map_err(StreamRequestError::from)
                })
                .collect::<StreamResult<_>>()?;
            self.submit_selection_supply(ticket.ticket, StreamSupply::Sources(&ordered), view)?;
        }
        Ok(())
    }
}

impl WindowedRenderer<'_> {
    pub fn request_stream_selection_ranges(
        &mut self,
        chart: ChartId,
    ) -> Result<StreamingSelectionRequest> {
        self.inner.request_stream_selection_ranges(chart)
    }
    pub fn submit_stream_selection_ranges(
        &mut self,
        ticket: StreamingSelectionTicket,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<()> {
        self.inner.submit_stream_selection_ranges(ticket, sources)
    }
    pub fn abandon_stream_selection(&mut self, ticket: StreamingSelectionTicket) -> Result<()> {
        self.inner.abandon_stream_selection(ticket)
    }
    pub fn suspend_stream_selection(&mut self, chart: ChartId) -> Result<()> {
        self.inner.suspend_stream_selection(chart)
    }
}

#[cfg(test)]
#[path = "streaming_selection_tests.rs"]
mod tests;
