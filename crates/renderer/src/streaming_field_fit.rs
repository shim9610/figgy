//! Compact endpoint replay for the existing GPU field-lattice fit reducer.
//! Only GPU-derived bounds return to the CPU; original coordinates never do.
use super::*;
use crate::gpu_errorbar::{
    GpuFieldExtentColumns, GpuFieldExtentMode, GpuSeriesExtentTicket, GpuSeriesFitMode,
};

pub(super) struct StreamFieldFit {
    buffer: TrackedBuffer,
    indices: [[u32; 4]; 2],
    count: u32,
    cells: u32,
    ordinal: u32,
    mode: GpuFieldExtentMode,
    readback: Option<GpuSeriesExtentTicket>,
}

pub(super) fn all_fit_inputs_ready(
    snapshot: &AutoStreamExecutionSnapshot,
    statistics: &HashMap<ColumnId, StreamStatisticsCache>,
    fields: &HashMap<usize, Option<crate::gpu_errorbar::GpuSeriesExtent>>,
) -> bool {
    snapshot.series.iter().enumerate().all(|(index, series)| {
        if is_stream_heatmap(&series.render_type) {
            fields.contains_key(&index)
                || field_runtime::field_total(series, |id| snapshot.sources.get(id).map(|s| s.len))
                    .ok()
                    == Some(0)
        } else {
            let mut ready = true;
            visit_series_columns(series, &mut |id| {
                ready &= snapshot.sources.get(id).is_some_and(|source| {
                    statistics
                        .get(id)
                        .is_some_and(|cache| cache.covers(0..source.len))
                });
            });
            ready
        }
    })
}

fn compact_indices(mode: GpuFieldExtentMode, cells: u32) -> ([u32; 4], u32, u32) {
    match mode {
        GpuFieldExtentMode::EdgesCells => ([0, cells, 0, 0], 2, 1),
        GpuFieldExtentMode::CentersSamples => ([0, cells - 1, 0, 0], 2, 2),
        GpuFieldExtentMode::CentersCells => {
            ([0, 1.min(cells - 1), cells.max(2) - 2, cells - 1], 4, 4)
        }
        GpuFieldExtentMode::EdgesSamples => ([0, 1, cells - 1, cells], 4, 3),
    }
}

fn extent_error(error: crate::gpu_errorbar::GpuErrorbarError) -> StreamRequestError {
    FiggyError::GpuResourceAllocationFailed {
        resource: "stream field fit",
        reason: error.to_string(),
    }
    .into()
}

impl Renderer {
    fn new_stream_field_fit(&self, job: StreamJob) -> StreamResult<StreamFieldFit> {
        let snapshot = self.auto_stream_snapshot(job);
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        let series = snapshot
            .as_ref()
            .map_or(&self.chart_states[&job.chart].series[draw.series], |s| {
                &s.series[draw.series]
            });
        let (extent, _, _) = field_runtime::extent(series, |id| {
            snapshot.as_ref().map_or_else(
                || self.streaming_sources.get(id).map(|s| s.len),
                |snapshot| snapshot.sources.get(id).map(|s| s.len),
            )
        })?;
        let matrix = extract_matrix(&series.render_type).ok_or(StreamError::WrongState)?;
        let (xc, yc) = match matrix.orientation {
            crate::data_config::MatrixOrientation::ColumnsAreX => (extent.cols, extent.rows),
            crate::data_config::MatrixOrientation::ColumnsAreY => (extent.rows, extent.cols),
        };
        let Some(GpuSeriesFitMode::Field(mode)) =
            GpuSeriesFitMode::from_render_type(&series.render_type)
        else {
            return Err(StreamError::WrongState.into());
        };
        let (x, count, cells) = compact_indices(mode, xc as u32);
        let (y, _, _) = compact_indices(mode, yc as u32);
        if self
            .gpu_memory_usage()
            .total_bytes()
            .checked_add(64 + 16 + GpuSeriesExtentTicket::FIELD_BYTES)
            .ok_or(StreamError::Overflow)?
            > self.memory_budget.unwrap_or(u64::MAX)
        {
            return Err(StreamError::TooLarge.into());
        }
        // gpu-alloc: ErrorbarScratch
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream field compact endpoints"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(StreamFieldFit {
            buffer: TrackedBuffer::new(&self.gpu_ledger, GpuResourceKind::ErrorbarScratch, buffer),
            indices: [x, y],
            count,
            cells,
            ordinal: 0,
            mode,
            readback: None,
        })
    }

    pub(super) fn request_stream_field_fit(
        &mut self,
        job: StreamJob,
    ) -> StreamResult<StreamDrawRequestStatus> {
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        if draw.field_fit.is_none() {
            let fit = self.new_stream_field_fit(job)?;
            self.stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .field_fit = Some(fit);
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let fit = draw.field_fit.as_ref().unwrap();
        if fit.ordinal == fit.count * 2 {
            if fit.readback.is_none() {
                if self
                    .gpu_memory_usage()
                    .total_bytes()
                    .checked_add(GpuSeriesExtentTicket::FIELD_BYTES)
                    .ok_or(StreamError::Overflow)?
                    > self.memory_budget.unwrap_or(u64::MAX)
                {
                    return Err(StreamError::TooLarge.into());
                }
                if self.errorbar_extent_engine.is_none() {
                    self.errorbar_extent_engine =
                        Some(crate::gpu_errorbar::GpuErrorbarExtentEngine::new_tracked(
                            &self.device,
                            Arc::clone(&self.gpu_ledger),
                        ));
                }
                let handle = |offset| data_render::ColumnHandle {
                    generation: 0,
                    offset,
                    byte_size: u64::from(fit.count) * 8,
                    len_values: fit.count as usize,
                };
                let ticket = self
                    .errorbar_extent_engine
                    .as_ref()
                    .unwrap()
                    .begin_field(
                        &self.device,
                        &self.queue,
                        &fit.buffer,
                        fit.mode,
                        GpuFieldExtentColumns {
                            x: handle(0),
                            y: handle(32),
                            x_cells: fit.cells,
                            y_cells: fit.cells,
                        },
                    )
                    .map_err(extent_error)?;
                self.stream_runtime
                    .as_mut()
                    .unwrap()
                    .draws
                    .iter_mut()
                    .find(|draw| draw.job == job)
                    .unwrap()
                    .field_fit
                    .as_mut()
                    .unwrap()
                    .readback = Some(ticket);
            }
            let draw = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap();
            let resolved = draw
                .field_fit
                .as_mut()
                .unwrap()
                .readback
                .as_mut()
                .unwrap()
                .try_resolve();
            let resolved = match resolved {
                Ok(resolved) => resolved,
                Err(error) => {
                    draw.field_fit.as_mut().unwrap().readback = None;
                    return Err(extent_error(error));
                }
            };
            let Some(extent) = resolved else {
                return Ok(StreamDrawRequestStatus::Backpressure);
            };
            draw.field_fits.insert(draw.series, extent);
            draw.field_fit = None;
            self.end_gpu_frame();
            if let Some(config) = self.pending_auto_fit_config(job)? {
                self.restart_auto_stream_with_config(job, config)?;
                return Ok(StreamDrawRequestStatus::Backpressure);
            }
            return self.request_stream_field(job);
        }
        let axis = (fit.ordinal / fit.count) as usize;
        let offset = u64::from(fit.indices[axis][(fit.ordinal % fit.count) as usize]);
        let snapshot = self.auto_stream_snapshot(job);
        let series = snapshot
            .as_ref()
            .map_or(&self.chart_states[&job.chart].series[draw.series], |s| {
                &s.series[draw.series]
            });
        let id = if axis == 0 {
            &series.x_column
        } else {
            &series.y_column
        }
        .clone();
        match self.request_stream_columns(
            job,
            &[StreamSourceRange {
                column: &id,
                offset,
                len: 1,
            }],
        )? {
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
        }
    }

    pub(super) fn submit_stream_field_fit(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        explicit_view: Option<&ChartView>,
        target: &wgpu::Texture,
    ) -> StreamResult<wgpu::SubmissionIndex> {
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
            .ok_or(StreamError::WrongState)?;
        if draw.pending != Some(ticket)
            || target != &draw.target
            || !Arc::ptr_eq(&view.stream_revision, &draw.view_revision)
            || view.stream_revision.load(Ordering::Acquire) != draw.expected_view_revision
        {
            return Err(StreamError::Stale.into());
        }
        self.validate_stream_supply_kind(ticket, supply)?;
        let ranges = self.stream_request_columns(ticket)?;
        if ranges.len() != 1 || ranges[0].range.len != 1 {
            return Err(StreamError::InvalidRange.into());
        }
        let range = ranges[0].range;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let chunk = match self.accept_stream_supply_with_headroom(
            ticket,
            supply,
            &mut encoder,
            GpuSeriesExtentTicket::FIELD_BYTES,
            None,
        ) {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(encoder);
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let handle = chunk.column_handle(range)?;
        let fit = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == ticket.job)
            .unwrap()
            .field_fit
            .as_ref()
            .unwrap();
        let axis = fit.ordinal / fit.count;
        let dest = u64::from(axis) * 32 + u64::from(fit.ordinal % fit.count) * 8;
        encoder.copy_buffer_to_buffer(&chunk.work, handle.offset, &fit.buffer, dest, 8);
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop(chunk);
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
                draw.field_fit.as_mut().unwrap().ordinal += 1;
                draw.pending = None;
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                Err(error)
            }
        }
    }
}
