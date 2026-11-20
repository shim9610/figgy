//! Submitting a pick: the gate, exact, arc and reduce passes.

use super::*;

impl GpuPickEngine {
    /// Submit one exact pick query and return an owned async readback ticket.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn pick(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
    ) -> Result<GpuPickTicket, GpuPickError> {
        self.pick_with_display_scale(pool, query, 1.0)
    }

    /// Submit one exact pick query using the display's uniform pixel scale.
    ///
    /// Series descriptors remain in logical-document pixels. This scale is
    /// applied once by the query shader to scatter radii, line half-widths,
    /// and the matching conservative gate widths. Cursor coordinates,
    /// `max_distance_px`, and returned distances remain physical canvas pixels.
    /// The registered series, in the order a pick addresses them.
    pub(crate) fn series_identities(&self) -> GpuPickSeriesIdentities {
        GpuPickSeriesIdentities(Arc::clone(&self.registry.identities))
    }

    #[cfg(test)]
    pub(crate) fn pick_with_display_scale(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let mut exact = PickExactPipelines::default();
        for key in self.exact_variants(&query) {
            exact.ensure(&self.bundle, key)?;
        }
        // No line offset members: the member transform is never asked for.
        let transform = query.transform;
        self.pick_with_arcs(
            pool,
            query,
            display_scale,
            &[],
            &[],
            0,
            &|_| transform,
            &exact,
        )
    }

    /// The exact module and variant each registered series needs under
    /// `query`, in registry order: create them
    /// ([`PickExactPipelines::ensure`]) before [`Self::pick_with_arcs`].
    pub(crate) fn exact_variants<'a>(
        &'a self,
        query: &'a GpuPickQuery,
    ) -> impl Iterator<Item = PickExactKey> + 'a {
        self.registry
            .slots
            .iter()
            .map(move |slot| series_exact_variant(&slot.gpu, query))
    }

    /// The bundle the exact pipelines are created against.
    pub(crate) fn bundle(&self) -> &PickPipelineBundle {
        &self.bundle
    }

    /// [`Self::pick_with_display_scale`] with each registered series' arc
    /// prefix, by registry position: the per-point cumulative arc length the
    /// line shader drew the series with (`line_arc.wgsl`) for the query's
    /// transform, which places its dash phase and sketch wobble. A missing
    /// entry, or a buffer too short for the series, picks without it.
    /// `exact` must hold every variant [`Self::exact_variants`] names.
    ///
    /// The last `members` registered series are line offset members
    /// (`Config::line_offsets`), each drawn by a panel of its own with the
    /// transform `member_transform` gives for it, in `line_offsets.series`
    /// order, so the last one paints on top. The pick resolves to what is
    /// painted under the cursor: the last member whose hit is at distance 0
    /// (inside its painted part) answers first; with none, the nearest hit
    /// within reach of any series does, as for every other chart.
    pub(crate) fn pick_with_arcs(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
        arcs: &[Option<wgpu::Buffer>],
        rings: &[Option<wgpu::Buffer>],
        members: usize,
        member_transform: &dyn Fn(usize) -> ScatterTransform,
        exact: &PickExactPipelines,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !display_scale.is_finite()
            || display_scale <= 0.0
            || !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(GpuPickTicket::ready_none());
            }
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(GpuPickTicket::ready_none());
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }

        let members = members.min(self.registry.slots.len());
        let first_member = self.registry.slots.len() - members;
        for (series_order, slot) in self.registry.slots.iter().enumerate() {
            Self::validate_series_columns(pool, slot)?;
            let series = &slot.gpu;
            let mut params = query_params(
                series,
                &slot.identity.series_id,
                &query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &slot.identity.series_id)?,
                    lane_base(series.y_handle, &slot.identity.series_id)?,
                    series.style_index_base,
                ],
                [
                    0,
                    series.point_count,
                    // A closed line owns its closing segment too.
                    if series.line_closed {
                        series.point_count
                    } else {
                        series.point_count.saturating_sub(1)
                    },
                    0,
                ],
                series_arc(arcs, series_order, series.point_count).is_some(),
            );
            // A line offset member is tested where its own panel draws it.
            if let Some(member) = series_order.checked_sub(first_member) {
                params.transform = member_transform(member);
            }
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        }

        // The final candidate, then each member's own candidate.
        let readback_tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &readback_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick detached readback"),
                size: CANDIDATE_BYTES * (1 + members as u64),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "detached pick readback",
        )?;
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy exact GPU pick encoder"),
                });
        // Reduction storage may intentionally be larger than the live registry
        // after replace/remove. Clear every slot so a prior query's candidate
        // can never participate as a stale tail entry.
        encoder.clear_buffer(&self.registry.reduce.candidates, 0, None);
        let has_gated_series = self.registry.slots.iter().any(|slot| !slot.gpu.direct_scan);
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick X gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_x);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick Y gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_y);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick candidate scan"),
                timestamp_writes: None,
            });
            for (series_order, slot) in self.registry.slots.iter().enumerate() {
                let series = &slot.gpu;
                let key = series_exact_variant(series, &query);
                pass.set_pipeline(exact.get(key).ok_or(GpuPickError::InvalidGpuResult)?);
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                // The pass keeps the bind group alive until it is submitted.
                let arc = series_arc(arcs, series_order, series.point_count);
                let ring = series_ring(rings, series_order, series.point_count);
                let group = (arc.is_some() || ring.is_some())
                    .then(|| self.bundle.arc_bind_group(arc, ring));
                pass.set_bind_group(
                    2,
                    group.as_ref().unwrap_or(&self.bundle.arc_placeholder),
                    &[],
                );
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick per-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                pass.set_bind_group(0, &series.reduce_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, slot) in self.registry.slots.iter().enumerate() {
            let series = &slot.gpu;
            encoder.copy_buffer_to_buffer(
                &series.result,
                0,
                &self.registry.reduce.candidates,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick cross-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            pass.set_bind_group(0, &self.registry.reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        for (member, slot) in self.registry.slots[first_member..].iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                &slot.gpu.result,
                0,
                &readback,
                (1 + member as u64) * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        self.bundle.queue.submit(std::iter::once(encoder.finish()));

        Ok(GpuPickTicket {
            monotonic: Vec::new(),
            state: GpuPickTicketState::Pending {
                readback: GpuReadback::map(
                    &self.bundle.device,
                    &self.bundle.health,
                    &readback,
                    None,
                ),
                identities: Arc::clone(&self.registry.identities),
                members,
                _charge: readback_tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
                _scratch_charge: None,
            },
        })
    }
}
