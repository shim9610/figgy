//! The prototype pick entries (#381): the chart pick of
//! [`Renderer::pick_chart`] resolved by one on-demand render pass into a
//! 1×1 ID target (`gpu_pick/id_pass.rs`), or by the nearest-datum kernel
//! (`gpu_pick/nearest_pass.rs`), instead of the gate, exact and reduce
//! passes. Measurement only: no host calls them yet.

use super::*;

impl Renderer {
    /// [`Self::pick_chart`] through the pick ID pass (#381 prototype),
    /// measuring `geometry`. The registry, the query, the arc prefixes and
    /// the ring starts are prepared as [`Self::pick_chart`] prepares them.
    #[doc(hidden)]
    pub fn pick_chart_id_prototype(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
        geometry: crate::gpu_pick::IdPickGeometry,
    ) -> std::result::Result<crate::gpu_pick::IdPickTicket, crate::gpu_pick::GpuPickError> {
        let result = self
            .pick_chart_prototype_unreported(id, request, PrototypePick::Id(geometry))
            .and_then(|ticket| match ticket {
                PrototypeTicket::Id(ticket) => Ok(ticket),
                PrototypeTicket::Nearest(_) => Err(crate::gpu_pick::GpuPickError::Internal(
                    "an ID pick returns an ID ticket",
                )),
            });
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    /// [`Self::pick_chart`] through the nearest-datum kernel (#381
    /// prototype): the datum nearest to the cursor within
    /// `request.max_distance_px`, by the draw's projection alone, and each
    /// series' own nearest datum.
    #[doc(hidden)]
    pub fn pick_chart_nearest_prototype(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::NearestPickTicket, crate::gpu_pick::GpuPickError>
    {
        let result = self
            .pick_chart_prototype_unreported(id, request, PrototypePick::Nearest)
            .and_then(|ticket| match ticket {
                PrototypeTicket::Nearest(ticket) => Ok(ticket),
                PrototypeTicket::Id(_) => Err(crate::gpu_pick::GpuPickError::Internal(
                    "a nearest-datum pick returns a nearest-datum ticket",
                )),
            });
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    /// How many nearest-datum pipelines this renderer has created (#381
    /// prototype): one per column kind.
    #[doc(hidden)]
    pub fn pick_nearest_pipelines_created(&self) -> usize {
        self.picker
            .nearest
            .as_ref()
            .map_or(0, crate::gpu_pick::NearestPickPipelines::created)
    }

    /// How many pick ID pipelines this renderer has created (#381
    /// prototype): at most one per column kind and geometry.
    #[doc(hidden)]
    pub fn pick_id_pipelines_created(&self) -> usize {
        self.picker
            .id
            .as_ref()
            .map_or(0, crate::gpu_pick::IdPickPipelines::created)
    }

    fn pick_chart_prototype_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
        mode: PrototypePick,
    ) -> std::result::Result<PrototypeTicket, crate::gpu_pick::GpuPickError> {
        self.pick_health()?;
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
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
        let series = pickable_series(&self.pool, &state.series);
        let monotonic_rows = self
            .monotonic
            .pick_rows(&series)
            .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        self.picker.prepare_chart(
            &self.pool,
            id,
            routed.as_ref().unwrap_or(&state.config),
            &series,
        )?;
        let inputs = active_layer_pick_inputs(&displayed, &series)?;
        let (display_config, layer_series) = (&inputs.panel, &inputs.series);
        let (request, moved) = pick_request_outside_gaps(display_config, request);
        let transform = data_render::scatter_transform_from_config(display_config);
        let set = self.picker.ready_bundle()?.set;
        if transform.needs_full_shaders() && set == ShaderSet::Basic {
            return Err(crate::gpu_pick::GpuPickError::Internal(
                "a chart with an axis break, a probability axis or polar coordinates is picked with the full shaders",
            ));
        }
        let first_member = layer_series.len() - inputs.member_count();
        let slots = self
            .picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?
            .engine
            .series_identities();
        // The arc prefixes and ring starts by registry position, as the
        // exact pick binds them. Prototype: the same two lists the exact
        // pick builds (`Renderer::pick_chart`).
        let mut arcs: Vec<Option<wgpu::Buffer>> = Vec::new();
        arcs.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick arc prefixes",
            }
        })?;
        let mut rings: Vec<Option<wgpu::Buffer>> = Vec::new();
        rings.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick ring starts",
            }
        })?;
        let walks_arcs = matches!(mode, PrototypePick::Id(_));
        for series_id in slots.series_ids().filter(|_| walks_arcs) {
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
                    self.preparation_parts().1.ensure_arc_prefix(
                        &series.series_id,
                        &series.x_column,
                        &series.y_column,
                        match &series.render_type {
                            crate::data_config::DataRenderType::Ternary(
                                crate::data_config::TernarySeries { c_column, .. },
                            ) => Some(super::prepare::ArcThird::Ternary(c_column.as_str())),
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
                .map_err(|error| crate::gpu_pick::GpuPickError::Gpu(error.to_string()))?
                .flatten()
                .map(|arc| arc.prefix.0);
            arcs.push(arc);
            let ring = match series {
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
        let device = self.device.clone();
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
        let geometry = match mode {
            PrototypePick::Id(geometry) => geometry,
            PrototypePick::Nearest => {
                let pipelines = match &mut picker.nearest {
                    Some(pipelines) if pipelines.set() == set => pipelines,
                    slot => slot.insert(crate::gpu_pick::NearestPickPipelines::new(set)),
                };
                let mut ticket = active.engine.pick_nearest(
                    pipelines,
                    &self.pool,
                    query,
                    request.display_scale,
                    inputs.member_count(),
                    &|member| inputs.member_transform(member),
                )?;
                ticket.monotonic = monotonic_rows;
                return Ok(PrototypeTicket::Nearest(ticket));
            }
        };
        let pipelines = match &mut picker.id {
            Some(pipelines) if pipelines.set() == set => pipelines,
            slot => slot.insert(crate::gpu_pick::IdPickPipelines::new(&device, set)),
        };
        let mut ticket = active.engine.pick_id(
            pipelines,
            geometry,
            &self.pool,
            query,
            request.display_scale,
            &arcs,
            &rings,
            inputs.member_count(),
            &|member| inputs.member_transform(member),
        )?;
        ticket.monotonic = monotonic_rows;
        Ok(PrototypeTicket::Id(ticket))
    }
}

/// Which prototype pick to run (#381).
#[derive(Clone, Copy)]
enum PrototypePick {
    Id(crate::gpu_pick::IdPickGeometry),
    Nearest,
}

enum PrototypeTicket {
    Id(crate::gpu_pick::IdPickTicket),
    Nearest(crate::gpu_pick::NearestPickTicket),
}
