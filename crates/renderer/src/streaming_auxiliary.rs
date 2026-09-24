//! Exact, bounded replay operations independent of the displayed stream.
use super::*;

#[cfg(test)]
#[path = "streaming_auxiliary_tests.rs"]
mod tests;

pub(super) struct AuxiliaryPick {
    point: crate::gpu_pick::GpuStreamPick,
    data: Option<crate::gpu_data_pick::GpuStreamDataPick>,
    plan: Arc<PickChartPlan>,
}

impl AuxiliaryPick {
    pub(super) fn field_enabled(&self) -> bool { self.data.as_ref().is_some_and(|data| data.field_enabled()) }
    pub(super) fn field_query(&self, order: u32) -> Result<crate::gpu_data_pick::GpuStreamFieldQuery> {
        self.data.as_ref().ok_or(FiggyError::StaleStateToken { reason: "no typed field picker".into() })?
            .field_query(order).map_err(|error| FiggyError::GpuResourceAllocationFailed { resource: "stream field query", reason: error.to_string() })
    }
    pub(super) fn reduce_field(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.data.as_mut().expect("typed field pick").encode_field_candidate(encoder);
    }
    pub(super) fn commit_field(&mut self) {
        self.data.as_mut().expect("typed field pick").commit_field_candidate();
    }
}

pub(super) struct StreamAuxiliaryTarget {
    pub(super) snapshot: Arc<AutoStreamExecutionSnapshot>,
    pub(super) pipelines: Option<TargetPipelines>,
    transfer: Option<StreamTransfer>,
    _styles_charge: Option<crate::gpu_memory::SharedCharge>,
    // Picking never writes this target, but its cursor still retains the
    // captured texture handle for identity checks across screen replacement.
    _target_charge: Option<crate::gpu_memory::SharedCharge>,
    // Only completed snapshots may be shared: progressive fit needs exclusive ownership.
    _completed: Arc<AutoStreamExecutionSnapshot>,
}

impl WindowedRenderer<'_> {
    pub fn view_residency_status(&self, chart: ChartId) -> Result<crate::ViewResidencyStatus> {
        self.inner.view_residency_status(chart)
    }

    pub fn is_streaming_chart(&self, chart: ChartId) -> bool {
        self.inner.is_streaming_chart(chart)
    }

    pub async fn pick_chart_view_cache_at(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
    ) -> Result<Option<crate::PickedPoint>> {
        self.inner.pick_chart_view_cache(chart, position, distance).await
    }

    pub fn next_view_point_index(
        &self, chart: ChartId, source_id: Option<&str>, series_id: &str,
        current: usize, forward: bool,
    ) -> Option<usize> {
        self.inner.next_view_point_index(chart, source_id, series_id, current, forward)
    }

    pub fn begin_stream_export(
        &mut self,
        chart: ChartId,
        scale: f32,
        clear: crate::Color,
        max_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.inner
            .begin_stream_export(chart, scale, clear, max_chunk)
    }
    pub async fn begin_stream_pick_point(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
        max_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.inner
            .begin_stream_pick_point(chart, position, distance, max_chunk)
            .await
    }
    pub async fn begin_stream_pick_data(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
        max_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.inner
            .begin_stream_pick_data(chart, position, distance, max_chunk)
            .await
    }
    pub fn request_stream_operation_ranges(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.inner.request_stream_operation_ranges(operation)
    }
    pub fn submit_stream_operation_ranges(
        &mut self,
        operation: StreamingOperation,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.inner
            .submit_stream_operation_ranges(operation, sources)
    }
    pub fn set_stream_operation_chunk_budget(
        &mut self,
        operation: StreamingOperation,
        max_chunk: u64,
    ) -> Result<()> {
        self.inner
            .set_stream_operation_chunk_budget(operation, max_chunk)
    }
    pub async fn finish_stream_export(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<RasterImage> {
        self.inner.finish_stream_export(operation).await
    }
    pub async fn finish_stream_pick_point(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<Option<crate::PickedPoint>> {
        self.inner.finish_stream_pick_point(operation).await
    }
    pub async fn finish_stream_pick_data(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<Option<crate::PickedData>> {
        self.inner.finish_stream_pick_data(operation).await
    }
    pub fn cancel_stream_operation(&mut self, operation: StreamingOperation) -> Result<()> {
        self.inner.cancel_stream_operation(operation)
    }
    pub async fn cancel_stream_operation_and_wait(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<()> {
        self.inner.cancel_stream_operation_and_wait(operation).await
    }
}

impl Renderer {
    /// Navigate only the valid source rows retained in a completed packed
    /// view. This is metadata lookup; it never dispatches a pick or reads a
    /// provider/GPU buffer.
    pub fn next_view_point_index(
        &self, chart: ChartId, source_id: Option<&str>, series_id: &str,
        current: usize, forward: bool,
    ) -> Option<usize> {
        let draw = self.stream_runtime.as_ref()?.draws.iter().find(|draw| {
            draw.job.chart == chart && draw.auxiliary.is_none()
        })?;
        let cache = draw.view_cache.as_ref()?;
        let snapshot = draw.auto_snapshot()?;
        let series = snapshot.series.iter().position(|series| {
            series.series_id == series_id && series.source_id.as_deref() == source_id
        })?;
        let current = u32::try_from(current).ok()?;
        cache.next_source_index(series, current, forward).map(|index| index as usize)
    }

    /// Pick only the completed chart-local packed view. This never replays a
    /// source and never walks a resident closure in the global column pool.
    pub async fn pick_chart_view_cache(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
    ) -> Result<Option<crate::PickedPoint>> {
        self.service_stream_requests();
        let Some(draw) = self.stream_runtime.as_ref()
            .and_then(|runtime| runtime.draws.iter().find(|draw| {
                draw.job.chart == chart && draw.auxiliary.is_none() && draw.auto_terminal(runtime)
            })) else { return Ok(None); };
        let job = draw.job;
        let Some(cache) = draw.view_cache.as_ref().cloned() else { return Ok(None); };
        self.publish_auto_stream_completion(job).map_err(StreamRequestError::into_figgy)?;
        self.enable_gpu_picking_async().await?;
        let snapshot = self.auto_stream_snapshot(job)
            .ok_or(FiggyError::StaleStateToken { reason: "packed view snapshot disappeared".into() })?;
        let plan = PickChartPlan::new(&snapshot.document_config, &snapshot.series)?;
        let panel = snapshot.config.chart_area.0;
        let data_area_px = snapshot.config.data_area().ok().map(|area| {
            let area = area.0;
            [area.x as f32, area.y as f32, area.width as f32, area.height as f32]
        });
        let query = crate::gpu_pick::GpuPickQuery {
            transform: data_render::scatter_transform_from_config(&snapshot.config),
            chart_rect_px: [panel.x as f32, panel.y as f32, panel.width as f32, panel.height as f32],
            data_area_px,
            canvas_position_px: position,
            max_distance_px: distance,
        };
        // Give each packed page a distinct pick identity. The GPU reports a
        // page-local index; only this one small candidate is read back, then
        // the page's source-run table maps it to the original source index.
        // Ranking keeps the old equal-distance order: later series wins,
        // scatter before line, then the earliest source page.
        let mut pick_pages = Vec::new();
        pick_pages.try_reserve(cache.chunks.len()).map_err(|error| FiggyError::StateAllocationFailed {
            resource: "packed-view pick page index", reason: error.to_string(),
        })?;
        pick_pages.extend(cache.chunks.iter().enumerate().filter_map(|(index, page)| {
            (page.phase != StreamDrawPhase::Errorbar).then_some(index)
        }));
        pick_pages.sort_by_key(|&index| {
            let page = &cache.chunks[index];
            let phase_order = if page.phase == StreamDrawPhase::Scatter { 1 } else { 0 };
            let first_source = page.source_runs.first().map_or(u32::MAX, |run| run.source_start);
            (page.series, phase_order, std::cmp::Reverse(first_source))
        });
        let identities = pick_pages.iter().map(|&index| {
            let series = &snapshot.series[cache.chunks[index].series];
            (series.source_id.clone(), series.series_id.clone())
        }).collect();
        let mut pick = self.picker.ready_bundle()?.begin_stream(query, snapshot.display_scale, identities)?;
        // Each page keeps its own encoder: combining large gated scans and
        // reductions in one encoder can stall the browser WebGPU backend.
        // Submit a bounded set of command buffers together instead.
        const PAGES_PER_SUBMISSION: usize = 8;
        let mut charges = Vec::new();
        charges.try_reserve_exact(PAGES_PER_SUBMISSION).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "packed-view pick submission",
                reason: error.to_string(),
            }
        })?;
        let mut command_buffers = Vec::new();
        command_buffers.try_reserve_exact(PAGES_PER_SUBMISSION).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "packed-view pick command buffers",
                reason: error.to_string(),
            }
        })?;
        let encoded = pick_pages.iter().enumerate().try_for_each(|(order, &page_index)| -> Result<()> {
            let packed = &cache.chunks[page_index];
            let series = &snapshot.series[packed.series];
            let planned = plan.descriptors.iter().find(|descriptor| {
                descriptor.signature.series_id == series.series_id
                    && descriptor.signature.source_id == series.source_id
            }).ok_or(FiggyError::StaleStateToken { reason: "packed view pick descriptor missing".into() })?;
            let mut descriptor = planned.descriptor();
            match packed.phase {
                StreamDrawPhase::Scatter => descriptor.line_width_px = None,
                StreamDrawPhase::Line => descriptor.scatter = None,
                _ => return Err(FiggyError::StaleStateToken { reason: "unsupported packed view pick phase".into() }),
            }
            let phase_columns = stream_phase_columns(series, packed.phase);
            let handle = |name: &str| -> Result<data_render::ColumnHandle> {
                let index = phase_columns.ids[..phase_columns.count].iter()
                    .position(|id| *id == name)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: name.into() })?;
                packed.chunk.column_handle(packed.ranges[index]).map_err(|error| {
                    FiggyError::InvalidSeriesConfig {
                        series_id: series.series_id.clone(),
                        reason: format!("invalid packed view pick lane: {error:?}"),
                    }
                })
            };
            let count = u32::try_from(packed.ranges[0].len).map_err(|_| FiggyError::StaleStateToken {
                reason: "packed view pick count exceeds u32".into(),
            })?;
            let columns = crate::gpu_pick::GpuStreamPickColumns {
                x: handle(&series.x_column)?,
                y: handle(&series.y_column)?,
                style_index: None,
            };
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let charge = pick.encode_indexed_chunk(
                &mut encoder, &packed.chunk.work, descriptor, columns,
                u32::try_from(order).map_err(|_| FiggyError::StaleStateToken {
                    reason: "too many packed pick pages".into(),
                })?, 0,
                if packed.phase == StreamDrawPhase::Scatter { count } else { 0 },
                if packed.phase == StreamDrawPhase::Line { count.saturating_sub(1) } else { 0 },
            )?;
            if let Some(charge) = charge {
                command_buffers.push(encoder.finish());
                charges.push(charge);
                if charges.len() == PAGES_PER_SUBMISSION {
                    self.queue.submit(command_buffers.drain(..));
                    charges.clear();
                    self.end_gpu_frame();
                }
            }
            Ok(())
        });
        if let Err(error) = encoded {
            drop(command_buffers);
            drop(charges);
            drop(pick);
            self.end_gpu_frame();
            return Err(error);
        }
        if !charges.is_empty() {
            self.queue.submit(command_buffers.drain(..));
            drop(charges);
            self.end_gpu_frame();
        }
        let result = pick.finish()?.resolve_indexed().await?.map(|hit| -> Result<crate::PickedPoint> {
            let page_index = *pick_pages.get(hit.series_order as usize).ok_or(
                FiggyError::StaleStateToken { reason: "packed pick page disappeared".into() },
            )?;
            let source_index = cache.chunks[page_index].source_index(hit.point_index).ok_or(
                FiggyError::StaleStateToken { reason: "packed pick selected a separator".into() },
            )?;
            Ok(crate::PickedPoint {
                source_id: hit.source_id,
                series_id: hit.series_id,
                point_index: source_index as usize,
                distance_px: hit.distance_px,
            })
        }).transpose()?;
        self.end_gpu_frame();
        Ok(result)
    }

    /// Exact point/line replay against the completed display revision.
    pub async fn begin_stream_pick_point(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
        max_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.begin_stream_pick(chart, position, distance, max_chunk, false)
            .await
    }

    /// Exact tagged point/line/histogram replay; Pending is represented by the
    /// operation, never by a fabricated miss.
    pub async fn begin_stream_pick_data(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
        max_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.begin_stream_pick(chart, position, distance, max_chunk, true)
            .await
    }

    async fn begin_stream_pick(
        &mut self,
        chart: ChartId,
        position: [f32; 2],
        distance: f32,
        max_chunk: u64,
        typed: bool,
    ) -> Result<StreamingOperation> {
        self.service_stream_requests();
        self.chart_config(chart)?;
        let runtime = self
            .stream_runtime
            .as_ref()
            .ok_or_else(|| StreamRequestError::from(StreamError::WrongState).into_figgy())?;
        let job = runtime
            .draws
            .iter()
            .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none())
            .filter(|draw| draw.auto_terminal(runtime))
            .map(|draw| draw.job)
            .ok_or_else(|| StreamRequestError::from(StreamError::WrongState).into_figgy())?;
        if max_chunk == 0
            || runtime
                .draws
                .iter()
                .any(|draw| draw.job.chart == chart && draw.auxiliary.is_some())
        {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        self.publish_auto_stream_completion(job)
            .map_err(StreamRequestError::into_figgy)?;
        self.enable_gpu_picking_async().await?;
        let snapshot = self
            .auto_stream_snapshot(job)
            .ok_or_else(|| StreamRequestError::from(StreamError::Stale).into_figgy())?;
        let plan = Arc::new(PickChartPlan::new(
            &snapshot.document_config,
            &snapshot.series,
        )?);
        let scale = snapshot
            .styles
            .styles
            .first()
            .map_or(1.0, |style| style.display_scale);
        let panel = snapshot.config.chart_area.0;
        let chart_rect_px = [
            panel.x as f32,
            panel.y as f32,
            panel.width as f32,
            panel.height as f32,
        ];
        let data_area_px = snapshot.config.data_area().ok().map(|area| {
            let area = area.0;
            [
                area.x as f32,
                area.y as f32,
                area.width as f32,
                area.height as f32,
            ]
        });
        let transform = data_render::scatter_transform_from_config(&snapshot.config);
        let identities = || {
            snapshot
                .series
                .iter()
                .map(|series| (series.source_id.clone(), series.series_id.clone()))
                .collect()
        };
        let bytes = crate::gpu_pick::GpuStreamPick::PERSISTENT_BYTES
            + if typed {
                crate::gpu_data_pick::GpuStreamDataPick::PERSISTENT_BYTES
            } else {
                0
            };
        self.preflight_auxiliary_gpu_bytes(bytes)?;
        let point = self.picker.ready_bundle()?.begin_stream(
            crate::gpu_pick::GpuPickQuery {
                transform,
                chart_rect_px,
                data_area_px,
                canvas_position_px: position,
                max_distance_px: distance,
            },
            scale,
            identities(),
        )?;
        let data = if typed {
            Some(self.picker.ready_data_bundle()?.begin_stream(
                crate::gpu_data_pick::GpuDataPickQuery {
                    transform,
                    chart_rect_px,
                    data_area_px,
                    canvas_position_px: position,
                    max_distance_px: distance,
                },
                identities(),
            )?)
        } else {
            None
        };
        let runtime = self.stream_runtime.as_mut().unwrap();
        runtime
            .draws
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "stream auxiliary cursor",
                reason: error.to_string(),
            })?;
        let screen_draw = runtime
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let target = screen_draw.target.clone();
        let target_charge = screen_draw.surface.as_ref().map(|surface| surface.prefix().shared_charge());
        let id = runtime
            .scheduler
            .start_auxiliary(
                chart.sequence,
                SourceStamp(snapshot.data_revision.sequence),
                ViewEpoch(snapshot.view_revision.sequence),
            )
            .map_err(|error| StreamRequestError::from(error).into_figgy())?;
        let operation = StreamJob { chart, id };
        runtime.draws.push(StreamDrawCursor {
            job: operation,
            series: 0,
            phase_index: 0,
            offset: 0,
            max_primitives: max_chunk,
            max_primitives_limit: max_chunk,
            pending: None,
            preferred_work_bytes: 0,
            view_revision: Arc::clone(&snapshot.view.stream_revision),
            expected_view_revision: snapshot.view.stream_revision.load(Ordering::Acquire),
            display_view_revision: Arc::clone(&snapshot.view.content_revision),
            displayed_view_revision: snapshot.view.content_revision.load(Ordering::Acquire),
            target,
            reusable_work: None,
            surface: None,
            display_bind_group: None,
            surface_clear: None,
            display_dirty: false,
            display_serial: 0,
            hist_envelope: None,
            hist_series: None,
            hist_overlay: false,
            mode: StreamExecutionMode::Explicit,
            auxiliary: Some(Arc::new(StreamAuxiliaryTarget {
                snapshot: Arc::clone(&snapshot),
                pipelines: None,
                transfer: None,
                _styles_charge: None,
                _target_charge: target_charge,
                _completed: snapshot,
            })),
            auxiliary_pick: Some(AuxiliaryPick { point, data, plan }),
            arc: None,
            field: None,
            field_fit: None,
            field_fits: HashMap::new(),
            selection: selection::StreamSelectionState::default(),
            view_candidate: None,
            view_cache: None,
            view_rejection: None,
        });
        Ok(StreamingOperation(operation))
    }

    fn preflight_auxiliary_gpu_bytes(&self, bytes: u64) -> Result<()> {
        let total = self
            .gpu_memory_usage()
            .total_bytes()
            .checked_add(bytes)
            .ok_or_else(|| StreamRequestError::from(StreamError::Overflow).into_figgy())?;
        if total > self.memory_budget.unwrap_or(u64::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "stream auxiliary GPU memory",
                requested: total,
                limit: self.memory_budget.unwrap_or(u64::MAX),
            });
        }
        Ok(())
    }

    /// Adjust later range sizes without replacing the operation or a pending request.
    pub fn set_stream_operation_chunk_budget(
        &mut self,
        operation: StreamingOperation,
        max_chunk: u64,
    ) -> Result<()> {
        let job = self
            .validate_stream_operation(operation)
            .map_err(StreamRequestError::into_figgy)?;
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        if max_chunk == 0 || max_chunk > draw.max_primitives_limit {
            return Err(FiggyError::InvalidConfig {
                field: "stream operation chunk budget",
                reason: "must be nonzero and no larger than the initial operation cap",
            });
        }
        draw.max_primitives = max_chunk;
        Ok(())
    }

    fn take_finished_stream_pick(
        &mut self,
        operation: StreamingOperation,
        typed: bool,
    ) -> Result<AuxiliaryPick> {
        let job = self
            .validate_stream_operation(operation)
            .map_err(StreamRequestError::into_figgy)?;
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        if draw
            .auxiliary_pick
            .as_ref()
            .is_none_or(|pick| pick.data.is_some() != typed)
        {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        if self
            .request_chart_stream_draw(job)
            .map_err(StreamRequestError::into_figgy)?
            != StreamDrawRequestStatus::AllSubmitted
        {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        self.preflight_auxiliary_gpu_bytes(if typed {
            crate::gpu_data_pick::GpuStreamDataPick::READBACK_BYTES
        } else {
            crate::gpu_pick::GpuStreamPick::READBACK_BYTES
        })?;
        Ok(self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap()
            .auxiliary_pick
            .take()
            .unwrap())
    }

    pub async fn finish_stream_pick_point(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<Option<crate::PickedPoint>> {
        let pick = self.take_finished_stream_pick(operation, false)?;
        let result = match pick.point.finish() {
            Ok(ticket) => ticket.resolve().await.map_err(FiggyError::from),
            Err(error) => Err(error.into()),
        };
        self.cancel_stream_operation(operation)?;
        self.end_gpu_frame();
        result
    }

    pub async fn finish_stream_pick_data(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<Option<crate::PickedData>> {
        let pick = self.take_finished_stream_pick(operation, true)?;
        let mut data = pick.data.unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        data.encode_point_result(&mut encoder, &pick.point);
        self.queue.submit([encoder.finish()]);
        let result = match data.finish() {
            Ok(ticket) => ticket.resolve().await.map_err(FiggyError::from),
            Err(error) => Err(error.into()),
        };
        drop(pick.point);
        self.cancel_stream_operation(operation)?;
        self.end_gpu_frame();
        result
    }

    pub(super) fn submit_stream_pick_supply(
        &mut self,
        ticket: StreamTicket,
        supply: StreamSupply<'_>,
        series_index: usize,
        phase: StreamDrawPhase,
        columns: &[ColumnRange],
        offset: u64,
        primitives: u64,
    ) -> StreamResult<wgpu::SubmissionIndex> {
        let snapshot = self
            .auto_stream_snapshot(ticket.job)
            .ok_or(StreamError::Stale)?;
        let cfg = &snapshot.series[series_index];
        let pick = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == ticket.job)
            .unwrap()
            .auxiliary_pick
            .as_ref()
            .unwrap();
        let plan = Arc::clone(&pick.plan);
        let descriptor = plan.descriptors.iter().find(|desc| {
            desc.signature.series_id == cfg.series_id && desc.signature.source_id == cfg.source_id
        });
        let mut descriptor = descriptor.map(OwnedGpuPickSeriesDescriptor::descriptor);
        if let Some(desc) = &mut descriptor {
            match phase {
                StreamDrawPhase::Line => desc.scatter = None,
                StreamDrawPhase::Scatter => desc.line_width_px = None,
                _ => descriptor = None,
            }
        }
        let headroom = if let Some(desc) = &descriptor {
            pick.point
                .chunk_bytes(desc, columns[0].len as usize)
                .map_err(FiggyError::from)?
        } else if phase == StreamDrawPhase::Histogram && pick.data.is_some() {
            crate::gpu_data_pick::GpuStreamDataPick::CHUNK_BYTES
        } else {
            0
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let chunk =
            self.accept_stream_supply_with_headroom(ticket, supply, &mut encoder, headroom, None)?;
        let result = (|| -> Result<Option<crate::gpu_memory::SharedCharge>> {
            let handles = |id: &str| -> Result<data_render::ColumnHandle> {
                let phase_columns = stream_phase_columns(cfg, phase);
                let index = phase_columns.ids[..phase_columns.count]
                    .iter()
                    .position(|name| *name == id)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
                chunk.column_handle(columns[index]).map_err(|error| {
                    FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        reason: format!("invalid pick chunk: {error:?}"),
                    }
                })
            };
            let pick = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == ticket.job)
                .unwrap()
                .auxiliary_pick
                .as_mut()
                .unwrap();
            if let Some(descriptor) = descriptor {
                let style_index = descriptor
                    .scatter
                    .as_ref()
                    .and_then(|scatter| scatter.style_map.as_ref())
                    .and_then(|map| map.style_index_column.as_deref())
                    .map(handles)
                    .transpose()?;
                return Ok(pick.point.encode_chunk(
                    &mut encoder,
                    &chunk.work,
                    descriptor,
                    crate::gpu_pick::GpuStreamPickColumns {
                        x: handles(&cfg.x_column)?,
                        y: handles(&cfg.y_column)?,
                        style_index,
                    },
                    series_index as u32,
                    offset as u32,
                    if phase == StreamDrawPhase::Scatter {
                        primitives as u32
                    } else {
                        0
                    },
                    if phase == StreamDrawPhase::Line {
                        primitives as u32
                    } else {
                        0
                    },
                )?);
            }
            if phase == StreamDrawPhase::Histogram
                && let Some(data) = &mut pick.data
            {
                let bar = extract_bar(&cfg.render_type).ok_or_else(|| {
                    StreamRequestError::from(StreamError::WrongState).into_figgy()
                })?;
                let horizontal = bar.orientation == crate::data_config::BarOrientation::Horizontal;
                let (hi, lo) = crate::data::split_f64_to_f32_pair(bar.baseline);
                let (edges, values) = if horizontal {
                    (handles(&cfg.y_column)?, handles(&cfg.x_column)?)
                } else {
                    (handles(&cfg.x_column)?, handles(&cfg.y_column)?)
                };
                return Ok(data.encode_chunk(
                    &mut encoder,
                    &chunk.work,
                    crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order: series_index as u32,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::Histogram {
                            edges,
                            values,
                            bin_count: primitives as u32,
                            baseline: [hi, lo],
                            gap_px: bar.gap_px.max(0.0)
                                * snapshot.styles.styles[series_index].display_scale,
                            width_ratio: data_render::sanitize_bar_width_ratio(bar.width_ratio),
                            horizontal,
                            style_map: snapshot.styles.styles[series_index]
                                .bar_map
                                .as_ref()
                                .map(|map| map.bind_group.clone()),
                        },
                    },
                    offset as u32,
                )?);
            }
            Ok(None)
        })();
        let charge = match result {
            Ok(charge) => charge,
            Err(error) => {
                drop((encoder, chunk));
                self.discard_stream_recording(ticket)?;
                self.cancel_stream_operation(StreamingOperation(ticket.job))?;
                self.end_gpu_frame();
                return Err(error.into());
            }
        };
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop((charge, chunk));
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
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                self.discard_stream_recording(ticket)?;
                self.cancel_stream_operation(StreamingOperation(ticket.job))?;
                self.end_gpu_frame();
                Err(error)
            }
        }
    }

    /// Replay the completed streamed revision at export resolution. This leaves
    /// the screen execution, target, progress, and desired chart state unchanged.
    pub fn begin_stream_export(
        &mut self,
        chart: ChartId,
        scale: f32,
        clear: crate::Color,
        max_primitives_per_chunk: u64,
    ) -> Result<StreamingOperation> {
        self.begin_stream_export_inner(chart, scale, clear, max_primitives_per_chunk)
            .map_err(StreamRequestError::into_figgy)
    }

    fn begin_stream_export_inner(
        &mut self,
        chart: ChartId,
        scale: f32,
        clear: crate::Color,
        max_primitives: u64,
    ) -> StreamResult<StreamingOperation> {
        self.service_stream_requests();
        self.chart_config(chart)?;
        let runtime = self
            .stream_runtime
            .as_ref()
            .ok_or(StreamError::WrongState)?;
        let completed_job = runtime
            .draws
            .iter()
            .find(|draw| draw.job.chart == chart && draw.auxiliary.is_none())
            .filter(|draw| draw.auto_terminal(runtime))
            .map(|draw| draw.job)
            .ok_or(StreamError::WrongState)?;
        if max_primitives == 0
            || runtime
                .draws
                .iter()
                .any(|draw| draw.job.chart == chart && draw.auxiliary.is_some())
        {
            return Err(StreamError::WrongState.into());
        }
        self.publish_auto_stream_completion(completed_job)?;
        let completed = self
            .auto_stream_snapshot(completed_job)
            .ok_or(StreamError::WrongState)?;
        let scale = clamp_export_scale(scale);
        let state = &self.chart_states[&chart];
        let document_config = if state.revisions.data == completed.data_revision
            && state.revisions.series == completed.series_revision
            && state.revisions.view == completed.view_revision {
            state.config.clone()
        } else { completed.document_config.clone() };
        let original = document_config.chart_area.0;
        let size = (
            ((original.width as f32) * scale).round().max(1.0) as u32,
            ((original.height as f32) * scale).round().max(1.0) as u32,
        );
        validate_texture_extent(self.caps, "stream export dimensions", size.0, size.1)?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let sample_count = preferred_msaa_sample_count(self.caps, format);
        validate_target_sample_count(self.caps, format, sample_count)?;
        let mut config = document_config.scaled(scale);
        config.chart_area = crate::layout::ChartArea(Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        });
        let limits = self.stream_runtime.as_ref().unwrap().scheduler.limits();
        for series in &completed.series {
            PrepareContext::validate_stream_series(&config, series)?;
            for phase in stream_draw_phases(&config.draw_style, series)? {
                if *phase == StreamDrawPhase::Field {
                    field_runtime::preflight(&self.device, limits, &config, series, max_primitives, |id| completed.sources.get(id))?;
                    continue;
                }
                let columns = stream_phase_columns(series, *phase);
                let total = stream_phase_total(series, *phase, |id| {
                    completed.sources.get(id).map(|s| s.len)
                })?;
                if total == 0 {
                    continue;
                }
                if *phase == StreamDrawPhase::Line && arc_runtime::needs_arc(&config.draw_style, series)
                    && total >= u64::from(u32::MAX) {
                    return Err(StreamError::TooLarge.into());
                }
                let mut input = 0u64;
                let mut work = 0u64;
                for id in &columns.ids[..columns.count] {
                    let rows = if *phase == StreamDrawPhase::Line && arc_runtime::needs_arc(&config.draw_style, series) {
                        (total + 1).min(max_primitives).min(257)
                    } else {
                        stream_phase_request_len(series, *phase, id, total.min(max_primitives))?
                    };
                    input = input
                        .checked_add(
                            rows.checked_mul(completed.sources[*id].encoding.bytes_per_value())
                                .ok_or(StreamError::Overflow)?,
                        )
                        .ok_or(StreamError::Overflow)?;
                    work = work
                        .checked_add(rows.checked_mul(8).ok_or(StreamError::Overflow)?)
                        .ok_or(StreamError::Overflow)?;
                }
                if columns.count > limits.max_columns_per_request
                    || input > limits.max_chunk_bytes
                    || work.checked_mul(2).ok_or(StreamError::Overflow)?
                        > limits.max_in_flight_bytes
                    || work > self.device.limits().max_buffer_size
                    || work > u64::from(self.device.limits().max_storage_buffer_binding_size)
                {
                    return Err(StreamError::TooLarge.into());
                }
            }
        }
        // Admission precedes every export-owned GPU allocation. Mapped
        // scatter/errorbar bases keep their existing shared owners; remaining
        // style uniforms and histogram tables are charged to this operation.
        let mut untracked_style_bytes = 0u64;
        let mut mapped_style_bytes = 0u64;
        for series in &completed.series {
            untracked_style_bytes = untracked_style_bytes
                .checked_add(4 * std::mem::size_of::<PrimitiveStyle>() as u64)
                .ok_or(StreamError::Overflow)?;
            mapped_style_bytes = mapped_style_bytes
                .checked_add(mapped_stream_base_bytes(series, &self.device)?)
                .ok_or(StreamError::Overflow)?;
            if let Some(bar) = extract_bar(&series.render_type) {
                let overrides = bar.bar_style_overrides.as_ref().map_or(0, |rows| {
                    rows.iter()
                        .filter(|row| u32::try_from(row.index).is_ok())
                        .count()
                });
                if overrides != 0 {
                    let rows = (overrides as u64)
                        .checked_mul(std::mem::size_of::<data_render::BarStyleOverrideGpu>() as u64)
                        .ok_or(StreamError::Overflow)?;
                    if rows > self.device.limits().max_buffer_size
                        || rows > self.device.limits().max_storage_buffer_binding_size
                    {
                        return Err(StreamError::TooLarge.into());
                    }
                    untracked_style_bytes = untracked_style_bytes
                        .checked_add(rows)
                        .and_then(|bytes| {
                            bytes.checked_add(
                                (std::mem::size_of::<data_render::BarStyleSlotGpu>()
                                    + std::mem::size_of::<data_render::BarStyleMapMeta>())
                                    as u64,
                            )
                        })
                        .ok_or(StreamError::Overflow)?;
                }
            }
        }
        if matches!(
            config.draw_style,
            DrawStyle::Milkyway(_) | DrawStyle::Constellation(_)
        ) {
            untracked_style_bytes = untracked_style_bytes
                .checked_add(data_render::STYLED_TEXTURE_BYTES)
                .ok_or(StreamError::Overflow)?;
        }
        let surface_spec = StreamSurfaceSpec {
            width: size.0,
            height: size.1,
            format,
            sample_count,
        };
        let view_bytes = u64::from(size.0)
            .checked_mul(u64::from(size.1))
            .and_then(|pixels| pixels.checked_mul(8))
            .and_then(|bytes| {
                bytes.checked_add(std::mem::size_of::<data_render::ScatterTransform>() as u64)
            })
            .ok_or(StreamError::Overflow)?;
        let allocation_bytes = surface_spec
            .charged_bytes()?
            .checked_add(view_bytes)
            .and_then(|bytes| bytes.checked_add(untracked_style_bytes))
            .and_then(|bytes| bytes.checked_add(mapped_style_bytes))
            .ok_or(StreamError::Overflow)?;
        self.preflight_auxiliary_gpu_bytes(allocation_bytes)?;
        self.stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .try_reserve(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        let chart_view =
            self.create_chart_view(&Chart::new(config.clone()), config.chart_area.0)?;
        let expected_view_revision = chart_view.advance_stream_revision()?;
        let displayed_view_revision = chart_view.advance_content_revision()?;
        data_render::update_scatter_transform(
            &self.queue,
            &chart_view.transform_buffer,
            &data_render::scatter_transform_from_config(&config),
        );
        let mut styles = RegisteredChartStyles {
            series_revision: completed.series_revision,
            styles: completed
                .series
                .iter()
                .map(|series| self.create_style_for_series_scaled(series, scale))
                .collect(),
        };
        charge_mapped_stream_bases(&mut styles, &self.gpu_ledger);
        let tally = crate::gpu_memory::ChargeTally::new();
        tally.add(untracked_style_bytes);
        let styles_charge =
            crate::gpu_memory::shared_charge(tally, &self.gpu_ledger, GpuResourceKind::Uniform);
        if self.gpu_memory_usage().total_bytes() > self.memory_budget.unwrap_or(u64::MAX) {
            return Err(StreamError::TooLarge.into());
        }
        let mut pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            format,
            sample_count,
        );
        let series: Vec<_> = completed
            .series
            .iter()
            .zip(&styles.styles)
            .map(|(config, style)| Series { config, style })
            .collect();
        pipelines.ensure_precise_variants_for_items(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            &self.data_selection_bgl,
            &self.field_bgl,
            self.contour_label_pipelines.as_ref(),
            format,
            &[ChartDrawItem {
                view: &chart_view,
                chart_config: &config,
                series: &series,
            }],
        );
        pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            format,
            &[ChartDrawItem {
                view: &chart_view,
                chart_config: &config,
                series: &series,
            }],
        );
        let transfer = StreamTransfer::new(&self.device, format, sample_count)?;
        let surface = StreamSurface::new(
            &self.device,
            &self.gpu_ledger,
            &transfer,
            surface_spec,
            self.memory_budget.unwrap_or(u64::MAX),
            self.pool
                .gpu_bytes()
                .checked_add(self.pool.retired_bytes())
                .ok_or(StreamError::Overflow)?,
        )?;
        let snapshot = Arc::new(AutoStreamExecutionSnapshot {
            desired: completed.desired,
            data_revision: completed.data_revision,
            series_revision: completed.series_revision,
            view_revision: completed.view_revision,
            target: StreamTargetKey::new(size, format, sample_count, clear, max_primitives),
            document_config,
            display_scale: scale,
            config,
            series: completed.series.clone(),
            sources: completed.sources.clone(),
            styles,
            view: chart_view,
            handoff: None,
        });
        let auxiliary = Arc::new(StreamAuxiliaryTarget {
            snapshot: Arc::clone(&snapshot),
            pipelines: Some(pipelines),
            transfer: Some(transfer),
            _styles_charge: Some(styles_charge),
            _target_charge: None,
            _completed: completed,
        });
        let target = wgpu::Texture::clone(surface.prefix());
        let target_view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream export initialization"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&auxiliary.pipelines.as_ref().unwrap().axis);
            pass.set_bind_group(0, &snapshot.view.grid_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let runtime = self.stream_runtime.as_mut().unwrap();
        let id = runtime.scheduler.start_auxiliary(
            chart.sequence,
            SourceStamp(snapshot.data_revision.sequence),
            ViewEpoch(snapshot.view_revision.sequence),
        )?;
        let job = StreamJob { chart, id };
        runtime.draws.push(StreamDrawCursor {
            job,
            series: 0,
            phase_index: 0,
            offset: 0,
            max_primitives,
            max_primitives_limit: max_primitives,
            pending: None,
            preferred_work_bytes: 0,
            view_revision: Arc::clone(&snapshot.view.stream_revision),
            expected_view_revision,
            display_view_revision: Arc::clone(&snapshot.view.content_revision),
            displayed_view_revision,
            target,
            reusable_work: None,
            surface: Some(surface),
            display_bind_group: None,
            surface_clear: None,
            display_dirty: false,
            display_serial: 0,
            hist_envelope: None,
            hist_series: None,
            hist_overlay: false,
            mode: StreamExecutionMode::Explicit,
            auxiliary: Some(auxiliary),
            auxiliary_pick: None,
            arc: None,
            field: None,
            field_fit: None,
            field_fits: HashMap::new(),
            selection: selection::StreamSelectionState::default(),
            view_candidate: None,
            view_cache: None,
            view_rejection: None,
        });
        self.queue.submit([encoder.finish()]);
        Ok(StreamingOperation(job))
    }

    fn validate_stream_operation(&self, operation: StreamingOperation) -> StreamResult<StreamJob> {
        let job = operation.0;
        self.validate_stream_job(job)?;
        if !self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .any(|draw| draw.job == job && draw.auxiliary.is_some())
        {
            return Err(StreamError::Stale.into());
        }
        Ok(job)
    }

    /// Request the next exact source ranges, sharing display backpressure.
    pub fn request_stream_operation_ranges(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<crate::AutoStreamingRangeRequest> {
        self.service_stream_requests();
        let job = self
            .validate_stream_operation(operation)
            .map_err(StreamRequestError::into_figgy)?;
        let revision = self.auto_stream_snapshot(job).unwrap().desired;
        let mut request = self
            .request_chart_stream_draw(job)
            .map_err(StreamRequestError::into_figgy)?;
        let (submitted_primitives, total_primitives) = self
            .stream_progress_counts(job)
            .map_err(StreamRequestError::into_figgy)?;
        if request == StreamDrawRequestStatus::AllSubmitted
            && self.stream_runtime.as_ref().unwrap().draws.iter().find(|draw| draw.job == job).unwrap().auxiliary_pick.is_none() {
            match self.request_selection_job(job).map_err(StreamRequestError::into_figgy)? {
                StreamingSelectionRequest::Ready { ticket, .. } => request = StreamDrawRequestStatus::Ready(ticket.raw()),
                StreamingSelectionRequest::Backpressure { .. } => request = StreamDrawRequestStatus::Backpressure,
                StreamingSelectionRequest::Complete { .. } => {},
                StreamingSelectionRequest::Failed { .. } => return Err(StreamRequestError::from(StreamError::WrongState).into_figgy()),
            }
        }
        Ok(match request {
            StreamDrawRequestStatus::Backpressure => {
                crate::AutoStreamingRangeRequest::Backpressure {
                    revision,
                    submitted_primitives,
                    total_primitives,
                }
            }
            StreamDrawRequestStatus::AllSubmitted => {
                if self
                    .stream_runtime
                    .as_ref()
                    .unwrap()
                    .scheduler
                    .has_slots_for_job(job.id)
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
                }
            }
            StreamDrawRequestStatus::Ready(ticket) => {
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
                crate::AutoStreamingRangeRequest::Ready {
                    revision,
                    submitted_primitives,
                    total_primitives,
                    ranges,
                }
            }
        })
    }

    /// Borrow only the requested ranges through staging and GPU submission.
    pub fn submit_stream_operation_ranges(
        &mut self,
        operation: StreamingOperation,
        sources: &[crate::StreamRangeSourceBinding<'_>],
    ) -> Result<crate::StreamingProgress> {
        self.service_stream_requests();
        let job = self
            .validate_stream_operation(operation)
            .map_err(StreamRequestError::into_figgy)?;
        let ticket = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .and_then(|draw| draw.pending.or(draw.selection.pending()))
            .ok_or_else(|| StreamRequestError::from(StreamError::WrongState).into_figgy())?;
        let (ordered, count) = {
            let requested = self
                .stream_request_columns(ticket)
                .map_err(StreamRequestError::into_figgy)?;
            let first = requested
                .first()
                .ok_or_else(|| StreamRequestError::from(StreamError::WrongState).into_figgy())?;
            if requested.len() > 7 {
                return Err(StreamRequestError::from(StreamError::TooLarge).into_figgy());
            }
            let first = resolve_stream_range_source(first, sources)?;
            let mut ordered = [first; 7];
            for (index, column) in requested.iter().enumerate().skip(1) {
                ordered[index] = resolve_stream_range_source(column, sources)?;
            }
            (ordered, requested.len())
        };
        if self.stream_runtime.as_ref().unwrap().requests.iter().any(|request| request.ticket == ticket && request.selection) {
            self.submit_selection_supply(ticket, StreamSupply::Sources(&ordered[..count]), None).map_err(StreamRequestError::into_figgy)?;
        } else {
            self.submit_chart_stream_surface_sources(ticket, &ordered[..count], None).map_err(StreamRequestError::into_figgy)?;
        }
        let (submitted_primitives, total_primitives) = self
            .stream_progress_counts(job)
            .map_err(StreamRequestError::into_figgy)?;
        Ok(crate::StreamingProgress::Submitted {
            submitted_primitives,
            total_primitives,
        })
    }

    /// Resolve and read back the replay at its own resolution. All source
    /// ranges must already have been submitted; completion may be awaited here.
    pub async fn finish_stream_export(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<RasterImage> {
        let job = self
            .validate_stream_operation(operation)
            .map_err(StreamRequestError::into_figgy)?;
        if self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap()
            .auxiliary
            .as_ref()
            .unwrap()
            .transfer
            .is_none()
        {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        if self
            .request_chart_stream_draw(job)
            .map_err(StreamRequestError::into_figgy)?
            != StreamDrawRequestStatus::AllSubmitted
        {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        if !matches!(self.request_selection_job(job).map_err(StreamRequestError::into_figgy)?, StreamingSelectionRequest::Complete { .. }) {
            return Err(StreamRequestError::from(StreamError::WrongState).into_figgy());
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let auxiliary = draw.auxiliary.as_ref().unwrap();
        let surface = draw.surface.as_ref().unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        surface
            .record_display(auxiliary.transfer.as_ref().unwrap(), &mut encoder, |pass| {
                let view = &auxiliary.snapshot.view;
                let size = (surface.prefix().width(), surface.prefix().height());
                if let (Some(panel), Some(data)) = (
                    data_render::clamp_rect_to_target(view.panel_rect, size),
                    data_render::clamp_rect_to_target(auxiliary.snapshot.config.data_area().map_or(view.panel_rect, |area| area.0), size),
                ) {
                    pass.set_viewport(panel.x as f32, panel.y as f32, panel.width as f32, panel.height as f32, 0.0, 1.0);
                    pass.set_scissor_rect(data.x, data.y, data.width, data.height);
                    for selected in &draw.selection.ready { data_render::issue_series_picked(pass, &selected.packet.layers()); }
                    pass.set_scissor_rect(panel.x, panel.y, panel.width, panel.height);
                }
                pass.set_pipeline(&auxiliary.pipelines.as_ref().unwrap().axis);
                pass.set_bind_group(0, &auxiliary.snapshot.view.decoration_bind_group, &[]);
                pass.draw(0..3, 0..1);
            })
            .map_err(|error| StreamRequestError::from(error).into_figgy())?;
        let texture = wgpu::Texture::clone(surface.resolved());
        self.queue.submit([encoder.finish()]);
        let result = self.read_stream_export_rgba(&texture).await;
        drop(texture);
        self.cancel_stream_operation(operation)?;
        self.end_gpu_frame();
        result
    }

    /// Stop new requests immediately. Submitted resources remain charged until
    /// the queue completion boundary, independently of the screen execution.
    pub fn cancel_stream_operation(&mut self, operation: StreamingOperation) -> Result<()> {
        let job = operation.0;
        if job.chart.renderer_identity != self.renderer_identity {
            return Err(StreamRequestError::from(StreamError::Stale).into_figgy());
        }
        let Some(runtime) = self.stream_runtime.as_mut() else {
            return Ok(());
        };
        runtime.scheduler.cancel(job.id);
        runtime.draws.retain(|draw| draw.job != job);
        runtime
            .requests
            .retain(|request| runtime.scheduler.contains_ticket(request.ticket.ticket));
        Ok(())
    }

    pub async fn cancel_stream_operation_and_wait(
        &mut self,
        operation: StreamingOperation,
    ) -> Result<()> {
        self.cancel_stream_operation(operation)?;
        self.end_gpu_frame();
        #[cfg(target_arch = "wasm32")]
        self.wait_submitted_work().await;
        #[cfg(not(target_arch = "wasm32"))]
        self.wait_idle();
        self.service_stream_requests();
        Ok(())
    }

    async fn read_stream_export_rgba(&mut self, target: &wgpu::Texture) -> Result<RasterImage> {
        let (width, height) = (target.width(), target.height());
        let row = u64::from(width) * 4;
        let padded = row.div_ceil(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
            * u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let remaining = self
            .memory_budget
            .unwrap_or(u64::MAX)
            .saturating_sub(self.gpu_memory_usage().total_bytes());
        let rows =
            (self.caps.max_buffer_size.min(remaining) / padded).min(u64::from(height)) as u32;
        if rows == 0 {
            return Err(FiggyError::GpuResourceLimit {
                resource: "stream export readback row",
                requested: padded,
                limit: remaining.min(self.caps.max_buffer_size),
            });
        }
        let readback = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Readback,
            create_buffer_checked(
                &self.device,
                &wgpu::BufferDescriptor {
                    label: Some("stream export readback"),
                    size: padded * u64::from(rows),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                },
                "stream export readback",
            )?,
        );
        let len = row
            .checked_mul(u64::from(height))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "stream export output",
                requested: row.saturating_mul(u64::from(height)),
                limit: usize::MAX as u64,
            })?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(len)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "stream export output",
                reason: error.to_string(),
            })?;
        rgba.resize(len, 0);
        let mut y = 0;
        while y < height {
            let count = rows.min(height - y);
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded as u32),
                        rows_per_image: Some(count),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height: count,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit([encoder.finish()]);
            let slice = readback.slice(..padded * u64::from(count));
            let (tx, rx) = futures_channel::oneshot::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
            #[cfg(not(target_arch = "wasm32"))]
            self.wait_idle();
            rx.await
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "stream export mapping",
                    reason: error.to_string(),
                })?
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "stream export mapping",
                    reason: error.to_string(),
                })?;
            let mapped = slice.get_mapped_range().map_err(|error| {
                FiggyError::GpuResourceAllocationFailed {
                    resource: "stream export mapping",
                    reason: error.to_string(),
                }
            })?;
            for local in 0..count {
                let src = &mapped[(u64::from(local) * padded) as usize..][..row as usize];
                let dst = &mut rgba[(u64::from(y + local) * row) as usize..][..row as usize];
                dst.copy_from_slice(src);
                for pixel in dst.chunks_exact_mut(4) {
                    let alpha = pixel[3];
                    if alpha != 0 && alpha != 255 {
                        let alpha = f32::from(alpha) / 255.0;
                        for channel in &mut pixel[..3] {
                            *channel =
                                (f32::from(*channel) / alpha).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
            drop(mapped);
            readback.unmap();
            y += count;
        }
        Ok(RasterImage {
            width,
            height,
            rgba,
        })
    }
}
