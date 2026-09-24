//! Renderer-owned Heatmap continuation. The GPU helper owns only resolved
//! immutable uniforms and one bounded tile; source identity stays in the job.
use super::*;
use crate::data_render::stream_field::{self as field, Action, SourceRole};

pub(super) struct StreamFieldState {
    pub(super) resources: Arc<field::Resources>,
    shape: field::TileShape,
    viewport: [f32; 4],
    scissor: [u32; 4],
    next: [u32; 2],
    pub(super) max_pairs: u32,
    tile: Option<field::Tile>,
    pub(super) pick: Option<field::PickTile>,
}

pub(super) fn extent(
    series: &SeriesConfig,
    mut len: impl FnMut(&str) -> Option<u64>,
) -> StreamResult<(MatrixExtent, u32, u32)> {
    let matrix = extract_matrix(&series.render_type).ok_or(StreamError::WrongState)?;
    let x = u32::try_from(len(&series.x_column).ok_or(StreamError::InvalidRange)?)
        .map_err(|_| StreamError::TooLarge)?;
    let y = u32::try_from(len(&series.y_column).ok_or(StreamError::InvalidRange)?)
        .map_err(|_| StreamError::TooLarge)?;
    let result = matrix_extent(matrix, x as usize, y as usize, |id| {
        len(id).and_then(|n| usize::try_from(n).ok()).unwrap_or(0)
    });
    Ok((result, x, y))
}

pub(super) fn field_total(
    series: &SeriesConfig,
    len: impl FnMut(&str) -> Option<u64>,
) -> StreamResult<u64> {
    let (extent, _, _) = extent(series, len)?;
    let fill = extract_field_fill(&series.render_type).ok_or(StreamError::WrongState)?;
    let minimum = if matches!(fill.shading, crate::data_config::Shading::Interpolated) {
        2
    } else {
        1
    };
    Ok(u64::from(extent.cols >= minimum && extent.rows >= minimum))
}

fn pair_limit(device: &wgpu::Device, limits: StreamLimits, requested: u64) -> StreamResult<u32> {
    let pairs = requested
        .max(2)
        .min(limits.max_chunk_bytes / 8)
        .min(limits.max_in_flight_bytes / 16)
        .min(device.limits().max_buffer_size / 8)
        .min(u64::from(device.limits().max_storage_buffer_binding_size) / 8)
        .min(u64::from(u32::MAX));
    if pairs < 2 {
        return Err(StreamError::TooLarge.into());
    }
    Ok(pairs as u32)
}

pub(super) fn preflight<'a>(
    device: &wgpu::Device,
    limits: StreamLimits,
    _config: &Config,
    series: &SeriesConfig,
    requested: u64,
    mut source: impl FnMut(&str) -> Option<&'a crate::StreamColumn>,
) -> StreamResult<()> {
    pair_limit(device, limits, requested)?;
    let matrix = extract_matrix(&series.render_type).ok_or(StreamError::WrongState)?;
    for id in [&series.x_column, &series.y_column]
        .into_iter()
        .chain(&matrix.columns)
    {
        let column = source(id).ok_or(StreamError::InvalidRange)?;
        u32::try_from(column.len).map_err(|_| StreamError::TooLarge)?;
    }
    field_total(series, |id| source(id).map(|column| column.len))?;
    Ok(())
}

impl Renderer {
    pub(super) fn stream_field_budget(&self, headroom_bytes: u64) -> StreamResult<field::Budget> {
        Ok(field::Budget {
            max_renderer_bytes: self.memory_budget.unwrap_or(u64::MAX),
            pool_bytes: self
                .pool
                .gpu_bytes()
                .checked_add(self.pool.retired_bytes())
                .ok_or(StreamError::Overflow)?,
            headroom_bytes,
        })
    }

    pub(super) fn new_stream_field(
        &self,
        job: StreamJob,
        picking: bool,
    ) -> StreamResult<StreamFieldState> {
        let snapshot = self.auto_stream_snapshot(job);
        let runtime = self.stream_runtime.as_ref().unwrap();
        let draw = runtime
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        let state = &self.chart_states[&job.chart];
        let config = snapshot
            .as_ref()
            .map_or(&state.config, |snapshot| &snapshot.config);
        let series = snapshot
            .as_ref()
            .map_or(&state.series[draw.series], |snapshot| {
                &snapshot.series[draw.series]
            });
        let source_len = |id: &str| {
            snapshot.as_ref().map_or_else(
                || self.streaming_sources.get(id).map(|source| source.len),
                |snapshot| snapshot.sources.get(id).map(|source| source.len),
            )
        };
        let (extent, x_len, y_len) = extent(series, source_len)?;
        let matrix = extract_matrix(&series.render_type).ok_or(StreamError::WrongState)?;
        let fill = extract_field_fill(&series.render_type).ok_or(StreamError::WrongState)?;
        let bar = config.colorbar.as_ref().ok_or(StreamError::WrongState)?;
        let log_z = matches!(bar.axis.scale, AxisScale::Logarithmic);
        let bound = |value: f64| {
            let (hi, lo) =
                crate::data::split_f64_to_f32_pair(if log_z { value.log10() } else { value });
            [hi, lo]
        };
        let flags = if matches!(
            matrix.orientation,
            crate::data_config::MatrixOrientation::ColumnsAreY
        ) {
            data_render::FIELD_FLAG_COLUMNS_ARE_Y
        } else {
            0
        } | if matches!(matrix.grid_layout, crate::data_config::GridLayout::Centers) {
            data_render::FIELD_FLAG_CENTERS
        } else {
            0
        } | if matches!(fill.shading, crate::data_config::Shading::Interpolated) {
            data_render::FIELD_FLAG_INTERPOLATED
        } else {
            0
        } | if matches!(fill.mode, crate::data_config::FillMode::Bands) {
            data_render::FIELD_FLAG_BANDS
        } else {
            0
        } | if log_z {
            data_render::FIELD_FLAG_LOG_Z
        } else {
            0
        };
        let stops = crate::gpu_contour::try_collect(
            "stream field color stops",
            bar.colormap.stops().iter().map(|c| [c.r, c.g, c.b, c.a]),
        )?;
        let tables = data_render::build_field_lookup_tables(&stops, &[])?;
        let params = data_render::FieldParamsGpu {
            x_base: 0,
            y_base: 0,
            x_len,
            y_len,
            cols: u32::try_from(extent.cols).map_err(|_| StreamError::TooLarge)?,
            rows: u32::try_from(extent.rows).map_err(|_| StreamError::TooLarge)?,
            level_count: 0,
            stop_count: u32::try_from(stops.len()).map_err(|_| StreamError::TooLarge)?,
            flags,
            opacity: fill.opacity.clamp(0.0, 1.0),
            line_width_px: 0.0,
            level_color_count: 0,
            z_min: bound(bar.axis.min),
            z_max: bound(bar.axis.max),
        };
        let size = (draw.target.width(), draw.target.height());
        let data_rect = config.data_area().map_err(|_| StreamError::InvalidRange)?.0;
        let (panel, data) = if picking {
            // Resident picking is a chart-space query, independent of a raster target.
            (config.chart_area.0, data_rect)
        } else {
            (
                data_render::clamp_rect_to_target(config.chart_area.0, size)
                    .ok_or(StreamError::InvalidRange)?,
                data_render::clamp_rect_to_target(data_rect, size)
                    .ok_or(StreamError::InvalidRange)?,
            )
        };
        let max_pairs = pair_limit(
            &self.device,
            runtime.scheduler.limits(),
            draw.max_primitives,
        )?;
        let upload = u64::from(max_pairs) * 16;
        let metadata = (std::mem::size_of::<data_render::ScatterTransform>()
            + std::mem::size_of::<PrimitiveStyle>()
            + std::mem::size_of::<data_render::FieldParamsGpu>()) as u64
            + (tables.stops.len() * std::mem::size_of::<[f32; 4]>()
                + tables.metadata.len()
                    * std::mem::size_of::<data_render::ContourLookupMetadataGpu>())
                as u64;
        let available = self
            .memory_budget
            .unwrap_or(u64::MAX)
            .checked_sub(self.gpu_memory_usage().total_bytes())
            .and_then(|n| n.checked_sub(metadata))
            .and_then(|n| n.checked_sub(upload))
            .ok_or(StreamError::TooLarge)?;
        let shape = field::TileShape::new(
            &self.device.limits(),
            if picking {
                [1, 1]
            } else {
                [data.width, data.height]
            },
            if picking {
                1
            } else {
                draw.target.sample_count()
            },
            available.min(runtime.scheduler.limits().max_in_flight_bytes),
        )?;
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("stream field runtime"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("data_render/field_columnar.wgsl").into(),
                ),
            });
        let pipelines = Arc::new(field::Pipelines::new(
            &self.device,
            &shader,
            draw.target.format(),
            if picking {
                1
            } else {
                draw.target.sample_count()
            },
        )?);
        let resources = field::Resources::new(
            &self.device,
            pipelines,
            Arc::clone(&self.gpu_ledger),
            self.stream_field_budget(
                upload + shape.state_bytes()? + 64 + if picking { 80 } else { 0 },
            )?,
            &data_render::scatter_transform_from_config(config),
            &PrimitiveStyle::from_color(bar.nan_color),
            &params,
            &tables,
        )?;
        Ok(StreamFieldState {
            resources,
            shape,
            max_pairs,
            viewport: [
                panel.x as f32,
                panel.y as f32,
                panel.width as f32,
                panel.height as f32,
            ],
            scissor: [data.x, data.y, data.width, data.height],
            next: [data.x, data.y],
            tile: None,
            pick: None,
        })
    }

    pub(super) fn request_stream_field(
        &mut self,
        job: StreamJob,
    ) -> StreamResult<StreamDrawRequestStatus> {
        let snapshot = self.auto_stream_snapshot(job);
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?;
        let config = snapshot
            .as_ref()
            .map_or(&self.chart_states[&job.chart].config, |snapshot| {
                &snapshot.config
            });
        let size = (draw.target.width(), draw.target.height());
        let data = config.data_area().map_err(|_| StreamError::InvalidRange)?.0;
        if data_render::clamp_rect_to_target(config.chart_area.0, size).is_none()
            || data_render::clamp_rect_to_target(data, size).is_none()
        {
            draw.offset = 1;
            return Ok(StreamDrawRequestStatus::Backpressure);
        }
        drop(snapshot);
        let needs_state = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .ok_or(StreamError::WrongState)?
            .field
            .is_none();
        if needs_state {
            let field = self.new_stream_field(job, false)?;
            self.stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .field = Some(field);
        }
        let budget = self.stream_field_budget(0)?;
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == job)
            .unwrap();
        let field = draw.field.as_mut().unwrap();
        if field.tile.is_none() {
            let rect = [
                field.next[0],
                field.next[1],
                field
                    .shape
                    .width
                    .min(field.scissor[0] + field.scissor[2] - field.next[0]),
                field
                    .shape
                    .height
                    .min(field.scissor[1] + field.scissor[3] - field.next[1]),
            ];
            let mut tile = field::Tile::new(
                &self.device,
                Arc::clone(&field.resources),
                field::Budget {
                    headroom_bytes: u64::from(field.max_pairs) * 16,
                    ..budget
                },
                field.shape,
                field.viewport,
                field.scissor,
                rect,
                field.max_pairs,
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let step =
                tile.record_init(&mut encoder, &draw.target.create_view(&Default::default()))?;
            self.queue.submit([encoder.finish()]);
            tile.commit(step)?;
            field.tile = Some(tile);
        }
        let Action::Source(request) = field.tile.as_ref().unwrap().action()? else {
            return Err(StreamError::WrongState.into());
        };
        let snapshot = self.auto_stream_snapshot(job);
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let series = snapshot
            .as_ref()
            .map_or(&self.chart_states[&job.chart].series[draw.series], |s| {
                &s.series[draw.series]
            });
        let id = match request.role {
            SourceRole::Axis(0) => &series.x_column,
            SourceRole::Axis(_) => &series.y_column,
            SourceRole::ZColumn(column) => {
                &extract_matrix(&series.render_type)
                    .ok_or(StreamError::WrongState)?
                    .columns[column as usize]
            }
        }
        .clone();
        match self.request_stream_columns(
            job,
            &[StreamSourceRange {
                column: &id,
                offset: u64::from(request.start),
                len: u64::from(request.len),
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

    pub(super) fn submit_stream_field_supply(
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
        let Action::Source(request) = draw
            .field
            .as_ref()
            .and_then(|field| field.tile.as_ref())
            .ok_or(StreamError::WrongState)?
            .action()?
        else {
            return Err(StreamError::WrongState.into());
        };
        self.validate_stream_supply_kind(ticket, supply)?;
        let requested = self.stream_request_columns(ticket)?;
        if requested.len() != 1
            || requested[0].range.offset != u64::from(request.start)
            || requested[0].range.len != u64::from(request.len)
        {
            return Err(StreamError::InvalidRange.into());
        }
        let range = requested[0].range;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let chunk = match self.accept_stream_supply_with_headroom(
            ticket,
            supply,
            &mut encoder,
            field::STEP_BYTES,
            None,
        ) {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(encoder);
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let recorded = (|| -> StreamResult<(field::RecordedStep, bool)> {
            let handle = chunk.column_handle(range)?;
            let budget = self.stream_field_budget(0)?;
            let draw = self
                .stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == ticket.job)
                .unwrap();
            let tile = draw.field.as_mut().unwrap().tile.as_mut().unwrap();
            let mut step = tile.record_source(
                &self.device,
                &mut encoder,
                budget,
                request,
                &chunk.work,
                u32::try_from(handle.offset / 8).map_err(|_| StreamError::TooLarge)?,
                chunk.work.shared_charge(),
            )?;
            let final_tile = tile.record_final_after_source(
                &mut encoder,
                &target.create_view(&Default::default()),
                &mut step,
            )?;
            Ok((step, final_tile))
        })();
        let (step, final_tile) = match recorded {
            Ok(step) => step,
            Err(error) => {
                drop((encoder, chunk));
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                return Err(error);
            }
        };
        let result = self.queue_stream_recording(ticket, encoder.finish());
        drop(chunk);
        let draw = self
            .stream_runtime
            .as_mut()
            .unwrap()
            .draws
            .iter_mut()
            .find(|draw| draw.job == ticket.job)
            .unwrap();
        let field = draw.field.as_mut().unwrap();
        match result {
            Ok(submission) => {
                field.tile.as_mut().unwrap().commit(step)?;
                draw.pending = None;
                if final_tile {
                    field.tile = None;
                    field.next[0] += field.shape.width;
                    if field.next[0] >= field.scissor[0] + field.scissor[2] {
                        field.next[0] = field.scissor[0];
                        field.next[1] += field.shape.height;
                    }
                    if field.next[1] >= field.scissor[1] + field.scissor[3] {
                        draw.offset = 1;
                    }
                    draw.display_dirty = true;
                }
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                field.tile.as_mut().unwrap().discard(step)?;
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "streaming_field_runtime_tests.rs"]
mod tests;
