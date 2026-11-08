//! Enabling GPU picking, preparing it for a chart, and the chart pick.

use super::*;

impl Renderer {
    /// A lost device or an unreported GPU error, as a pick error.
    pub(super) fn pick_health(&self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.health
            .check()
            .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))
    }

    /// Enable exact GPU picking for this renderer.
    ///
    /// This is idempotent. The immutable compute pipeline bundle is created
    /// only by the first successful call; chart registries remain lazy.
    pub fn enable_gpu_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_unreported();
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn enable_gpu_picking_unreported(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed(&mut noop)
    }

    /// Enable exact GPU picking while reporting timestamp-free pipeline stages.
    pub fn enable_gpu_picking_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_observed_unreported(observer);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn enable_gpu_picking_observed_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        self.picker.enable_observed(
            self.device.clone(),
            self.queue.clone(),
            Arc::clone(&self.gpu_ledger),
            self.health.clone(),
            crate::gpu_data_pick::DataPickShaders {
                bar: &self.pipelines.shaders.bar,
                field: &self.pipelines.shaders.field,
                vector_grid: &self.pipelines.shaders.vector_grid,
                line: &self.pipelines.shaders.line,
            },
            &self.transform_bgl,
            &self.field_bgl,
            &self.per_point_style_map_bgl,
            &self.vector_bgl,
            &self.category_bar_data_bgl,
            &self.parallel_bgl,
            observer,
        )
    }

    pub async fn enable_gpu_picking_async(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.enable_gpu_picking_async_unreported().await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) async fn enable_gpu_picking_async_unreported(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed_async(&mut noop).await
    }

    pub async fn enable_gpu_picking_observed_async(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self
            .enable_gpu_picking_observed_async_unreported(observer)
            .await;
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) async fn enable_gpu_picking_observed_async_unreported(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        self.picker
            .enable_observed_async(
                self.device.clone(),
                self.queue.clone(),
                Arc::clone(&self.gpu_ledger),
                self.health.clone(),
                crate::gpu_data_pick::DataPickShaders {
                    bar: &self.pipelines.shaders.bar,
                    field: &self.pipelines.shaders.field,
                    vector_grid: &self.pipelines.shaders.vector_grid,
                    line: &self.pipelines.shaders.line,
                },
                &self.transform_bgl,
                &self.field_bgl,
                &self.per_point_style_map_bgl,
                &self.vector_bgl,
                &self.category_bar_data_bgl,
                &self.parallel_bgl,
                observer,
            )
            .await
    }

    /// Prepare the renderer-owned picker cache for one chart.
    pub fn prepare_gpu_picking_for_chart(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let result = self.prepare_gpu_picking_for_chart_unreported(id);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn prepare_gpu_picking_for_chart_unreported(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        self.picker
            .prepare_chart(&self.pool, id, &state.config, &state.series)
    }

    /// Submit one exact pick against renderer-owned chart state.
    pub fn pick_chart(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let result = self.pick_chart_unreported(id, request);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn pick_chart_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        // On the base layer a pointer over a subplot's frame belongs to the
        // subplot drawn on top there (#205), inside the parent chart or
        // outside it: the query runs on that subplot's layer, its registry
        // prepared from the same routed config.
        let routed = subplot_route(&state.config, &displayed, request.canvas_position_px);
        if let Some(config) = &routed {
            displayed.active_layer = config.active_layer;
        }
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        self.picker.prepare_chart(
            &self.pool,
            id,
            routed.as_ref().unwrap_or(&state.config),
            &state.series,
        )?;
        let inputs = active_layer_pick_inputs(&displayed, &state.series)?;
        let (display_config, layer_series) = (&inputs.panel, &inputs.series);
        let (request, moved) = pick_request_outside_gaps(display_config, request);
        let transform = data_render::scatter_transform_from_config(display_config);
        // The line offset members are the last registered series, each
        // drawn (and its arc prefix scanned) at its own panel's transform.
        let first_member = layer_series.len() - inputs.member_count();
        // The arc prefix each registered line was drawn with at this
        // transform, by registry position: its dash phase and sketch wobble
        // walk it. The draw's own cache key, so a pick after a frame reuses
        // the frame's scan and a pick before one runs the same scan.
        let slots = self
            .picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?
            .engine
            .series_identities();
        let mut arcs: Vec<Option<wgpu::Buffer>> = Vec::new();
        arcs.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick arc prefixes",
            }
        })?;
        // A region series' ring starts, the scan its draw reads (#49).
        let mut rings: Vec<Option<wgpu::Buffer>> = Vec::new();
        rings.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick ring starts",
            }
        })?;
        for series_id in slots.series_ids() {
            let position = layer_series
                .iter()
                .position(|series| series.series_id == series_id);
            let member_transform = position
                .and_then(|position| position.checked_sub(first_member))
                .map(|member| inputs.member_transform(member));
            let series_transform = member_transform.as_ref().unwrap_or(&transform);
            let series = position.map(|position| &layer_series[position]);
            let arc = series
                .filter(|series| line_pick_walks_arc(&display_config.draw_style, series))
                .map(|series| {
                    let interpolation = extract_line(&series.render_type)
                        .map_or(LineInterpolation::Linear, |line| line.interpolation);
                    // A streamline's arrowheads scan with their slot pitch,
                    // the draw's key at the same display scale.
                    self.preparation_parts().1.ensure_arc_prefix(
                        &series.series_id,
                        &series.x_column,
                        &series.y_column,
                        match &series.render_type {
                            crate::data_config::DataRenderType::Ternary { c_column, .. } => {
                                Some(super::prepare::ArcThird::Ternary(c_column.as_str()))
                            }
                            _ => None,
                        },
                        series_transform,
                        super::prepare::streamline_arrow_pitch(
                            &series.render_type,
                            request.display_scale,
                        ),
                        interpolation,
                    )
                })
                .transpose()
                // The scan the draw runs fails the pick with its message.
                .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))?
                .flatten()
                .map(|arc| arc.prefix.0);
            arcs.push(arc);
            let ring = match series {
                // The scan the draw runs: its failure (a lane past the
                // 32-bit range, rows past the dispatch grid) fails the pick
                // with the draw's message.
                Some(series) => self
                    .preparation_parts()
                    .1
                    .region_ring_starts(display_config, series)
                    .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))?,
                None => None,
            };
            rings.push(ring);
        }
        let chart_rect = display_config.chart_area.0;
        let data_area_px = display_config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32 + moved[0],
                rect.y as f32 + moved[1],
                rect.width as f32,
                rect.height as f32,
            ]
        });
        let query = crate::gpu_pick::GpuPickQuery {
            transform,
            chart_rect_px: [
                chart_rect.x as f32,
                chart_rect.y as f32,
                chart_rect.width as f32,
                chart_rect.height as f32,
            ],
            data_area_px,
            canvas_position_px: request.canvas_position_px,
            max_distance_px: request.max_distance_px,
            draw_style: crate::gpu_pick::GpuPickDrawStyle::of(&display_config.draw_style),
        };
        let picker = &mut self.picker;
        let active = picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?;
        if active.chart_id != id {
            return Err(crate::gpu_pick::GpuPickError::Internal(
                "the active picker registry is the prepared chart's",
            ));
        }
        // The exact pipelines these series need, compiled on the first pick
        // that needs each.
        for key in active.engine.exact_variants(&query) {
            picker.exact.ensure(active.engine.bundle(), key)?;
        }
        active.engine.pick_with_arcs(
            &self.pool,
            query,
            request.display_scale,
            &arcs,
            &rings,
            inputs.member_count(),
            &|member| inputs.member_transform(member),
            &picker.exact,
        )
    }
}
