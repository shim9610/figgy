//! Exact scalar cursor lookup through the field shader's bounded axis walk.
use super::*;
use crate::data_render::stream_field::{self as field, Action, SourceRole};

impl Renderer {
    pub(super) fn request_stream_field_pick(
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
        if draw.field.is_none() {
            let mut state = self.new_stream_field(job, true)?;
            let budget = self.stream_field_budget(80 + u64::from(state.max_pairs) * 16)?;
            if self
                .gpu_memory_usage()
                .total_bytes()
                .checked_add(80 + field::PIXEL_BYTES + 64 + u64::from(state.max_pairs) * 16)
                .ok_or(StreamError::Overflow)?
                > budget.max_renderer_bytes
            {
                return Err(StreamError::TooLarge.into());
            }
            let draw = self
                .stream_runtime
                .as_ref()
                .unwrap()
                .draws
                .iter()
                .find(|draw| draw.job == job)
                .unwrap();
            let query = draw
                .auxiliary_pick
                .as_ref()
                .ok_or(StreamError::WrongState)?
                .field_query(u32::try_from(draw.series).map_err(|_| StreamError::TooLarge)?)?;
            let mut tile = field::PickTile::new(
                &self.device,
                Arc::clone(&state.resources),
                self.stream_field_budget(u64::from(state.max_pairs) * 16)?,
                state.max_pairs,
                query,
            )?;
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let step = tile.initialize(&mut encoder)?;
            self.queue.submit([encoder.finish()]);
            tile.commit(step)?;
            state.pick = Some(tile);
            self.stream_runtime
                .as_mut()
                .unwrap()
                .draws
                .iter_mut()
                .find(|draw| draw.job == job)
                .unwrap()
                .field = Some(state);
        }
        let draw = self
            .stream_runtime
            .as_ref()
            .unwrap()
            .draws
            .iter()
            .find(|draw| draw.job == job)
            .unwrap();
        let Action::Source(request) = draw
            .field
            .as_ref()
            .unwrap()
            .pick
            .as_ref()
            .unwrap()
            .action()?
        else {
            return Err(StreamError::WrongState.into());
        };
        let snapshot = draw.auto_snapshot().ok_or(StreamError::WrongState)?;
        let series = &snapshot.series[draw.series];
        let id = match request.role {
            SourceRole::Axis(0) => &series.x_column,
            SourceRole::Axis(_) => &series.y_column,
            SourceRole::ZColumn(_) => return Err(StreamError::WrongState.into()),
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

    pub(super) fn submit_stream_field_pick(
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
            .unwrap()
            .pick
            .as_ref()
            .unwrap()
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
            let (step, complete) = draw
                .field
                .as_mut()
                .unwrap()
                .pick
                .as_mut()
                .unwrap()
                .record_source(
                    &self.device,
                    &mut encoder,
                    budget,
                    request,
                    &chunk.work,
                    u32::try_from(handle.offset / 8).map_err(|_| StreamError::TooLarge)?,
                    chunk.work.shared_charge(),
                )?;
            if complete {
                draw.auxiliary_pick
                    .as_mut()
                    .unwrap()
                    .reduce_field(&mut encoder);
            }
            Ok((step, complete))
        })();
        let (step, complete) = match recorded {
            Ok(value) => value,
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
        match result {
            Ok(submission) => {
                draw.field
                    .as_mut()
                    .unwrap()
                    .pick
                    .as_mut()
                    .unwrap()
                    .commit(step)?;
                draw.pending = None;
                if complete {
                    draw.auxiliary_pick.as_mut().unwrap().commit_field();
                    draw.offset = 1;
                }
                self.end_gpu_frame();
                Ok(submission)
            }
            Err(error) => {
                draw.field
                    .as_mut()
                    .unwrap()
                    .pick
                    .as_mut()
                    .unwrap()
                    .discard(step)?;
                self.discard_stream_recording(ticket)?;
                self.end_gpu_frame();
                Err(error)
            }
        }
    }
}
