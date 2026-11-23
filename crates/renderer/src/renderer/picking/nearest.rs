//! The nearest-datum pick of a registered chart (#381): every pickable
//! layer at the cursor in one registry, each series placed where its own
//! panel draws it.

use super::*;

impl Renderer {
    /// Each series' datum nearest to the cursor, and the nearest of them,
    /// on every pickable layer drawn at the cursor (#381): the base layer
    /// and its overlays, or, over a subplot, that subplot and its overlays.
    ///
    /// Each series is placed with the projection its panel draws it with
    /// (each layer's own axes, a line offset member's own panel, a 3D
    /// chart's camera), and its distance is the screen distance from the
    /// cursor to the datum. There is no pick reach:
    /// `request.max_distance_px` is not used, and a series the draw places
    /// any datum of always answers. No style is read: a line is picked by
    /// its points, a marker by its centre.
    ///
    /// The nearest datum is the nearest of all, whatever its layer; an
    /// exact distance tie goes to the active layer, then to the series
    /// drawn later (on top). [`crate::gpu_pick::NearestPick::per_series`]
    /// keeps every series' answer with its layer, so a host can apply a
    /// rule of its own (the active layer only, for example). Whether the
    /// cursor hits anything is not decided here.
    ///
    /// The typed data picks ([`Self::pick_chart_data`]) and the DataLess
    /// series (energy levels, Feynman diagrams) are not part of this pick.
    /// A chart whose series read a streamed source is refused with
    /// [`crate::gpu_pick::GpuPickError::ColumnNotResident`] (rule 8), as
    /// [`Self::pick_chart`] refuses it; a streamed display keeps its own
    /// pick.
    pub fn pick_chart_nearest(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::NearestPickTicket, crate::gpu_pick::GpuPickError>
    {
        let result = self.pick_chart_nearest_unreported(id, request);
        self.reported_as(result, crate::gpu_pick::GpuPickError::from_figgy)
    }

    pub(super) fn pick_chart_nearest_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::NearestPickTicket, crate::gpu_pick::GpuPickError>
    {
        self.pick_chart_nearest_grouped(id, request)
            .map(|(ticket, _, _)| ticket)
    }

    /// [`Self::pick_chart_nearest`], with the subplot whose layer group it
    /// searched (`None`: the base layer and its overlays) and the layer an
    /// exact tie goes to.
    fn pick_chart_nearest_grouped(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<
        (
            crate::gpu_pick::NearestPickTicket,
            Option<usize>,
            crate::LayerRef,
        ),
        crate::gpu_pick::GpuPickError,
    > {
        self.pick_health()?;
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        // A streamed source is refused (rule 8), as every registered-chart
        // pick refuses one.
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        let displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        // The layer group at the cursor: over a subplot (on the base layer)
        // the subplot drawn there, otherwise the active layer's group.
        let active = subplot_route(&state.config, &displayed, request.canvas_position_px)
            .unwrap_or_else(|| state.config.active_layer());
        let (first, overlays, subplot) = match active {
            crate::LayerRef::Subplot(index) | crate::LayerRef::SubplotOverlay(index, _) => (
                crate::LayerRef::Subplot(index),
                state.config.subplots[index].config.overlays.len(),
                Some(index),
            ),
            _ => (crate::LayerRef::Base, state.config.overlays.len(), None),
        };
        let active = match active {
            crate::LayerRef::Annotations => crate::LayerRef::Base,
            layer => layer,
        };
        let series = pickable_series(&self.pool, &state.series);
        let monotonic_rows = self
            .monotonic
            .pick_rows(&series)
            .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let mut descriptors = Vec::new();
        let mut slots = Vec::new();
        descriptors.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "nearest pick plan",
            }
        })?;
        slots.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "nearest pick slots",
            }
        })?;
        let mut needs_full = false;
        // The layers drawn as panels of their own: a document squeezed
        // below its margins draws its base panel alone, with every series
        // on it (`expand_for_render`), and a layer it does not draw is not
        // planned again, so each series is planned once.
        let panel_count =
            render_panel_count(&displayed).map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let panel_layers = displayed.panel_layers();
        for k in 0..=overlays {
            let layer = match (k, subplot) {
                (0, _) => first,
                (k, Some(index)) => crate::LayerRef::SubplotOverlay(index, k - 1),
                (k, None) => crate::LayerRef::Overlay(k - 1),
            };
            let drawn_as = panel_layers
                .get(panel_index(&displayed, panel_count, layer))
                .copied();
            if k > 0 && drawn_as != Some(layer) {
                continue;
            }
            // The group's first layer is planned as the panel draws it: the
            // base panel when the document fell back to it.
            let layer = drawn_as.unwrap_or(layer);
            let inputs = layer_pick_inputs(&displayed, layer, &series)?;
            let plan = PickChartPlan::new(&inputs.panel, &inputs.series)?;
            let first_member = inputs.series.len() - inputs.member_count();
            for descriptor in plan.descriptors {
                // A line offset member is placed where its own panel draws
                // it; every other series where its layer's panel does.
                let position = inputs
                    .series
                    .iter()
                    .position(|item| item.series_id == descriptor.series_id);
                let panel = position
                    .and_then(|position| position.checked_sub(first_member))
                    .map_or(&*inputs.panel, |member| &inputs.member_panels[member]);
                let transform = data_render::scatter_transform_from_config(panel);
                needs_full |= transform.needs_full_shaders();
                let (moved, _) = pick_request_outside_gaps(panel, request);
                let chart_rect = panel.chart_area.0;
                let query = crate::gpu_pick::GpuPickQuery {
                    transform,
                    chart_rect_px: [
                        chart_rect.x as f32,
                        chart_rect.y as f32,
                        chart_rect.width as f32,
                        chart_rect.height as f32,
                    ],
                    data_area_px: None,
                    canvas_position_px: moved.canvas_position_px,
                    max_distance_px: 0.0,
                    draw_style: crate::gpu_pick::GpuPickDrawStyle::of(&panel.draw_style),
                };
                // A 3D panel: its camera mapped onto the cursor's pixel, as
                // the 3D ID pick maps it (`Renderer::pick_surface`), and the
                // series' height column.
                let view3d = panel.view_3d().and_then(|_| {
                    let (px, py) = (
                        f64::from(moved.canvas_position_px[0]),
                        f64::from(moved.canvas_position_px[1]),
                    );
                    let target = ((px - 0.5, py - 0.5), (1.0, 1.0));
                    let camera = data_render::surface_mesh::view3d_gpu(panel, target, target)?;
                    let heights = position
                        .and_then(|position| panel.drawn_z_column(&inputs.series[position]))
                        .and_then(|z| self.pool.handle_for(z))
                        .and_then(|handle| {
                            Some((
                                u32::try_from(handle.offset / 4).ok()?,
                                u32::try_from(handle.len_values).ok()?,
                            ))
                        })
                        .unwrap_or((0, 0));
                    Some((camera, heights))
                });
                slots.push(crate::gpu_pick::NearestSlot {
                    query,
                    layer,
                    view3d,
                });
                descriptors.push(descriptor);
            }
        }
        let set = self.picker.ready_bundle()?.set;
        if needs_full && set == ShaderSet::Basic {
            return Err(crate::gpu_pick::GpuPickError::Internal(
                "a chart with an axis break, a probability axis or polar coordinates is picked with the full shaders",
            ));
        }
        self.picker
            .prepare_plan(&self.pool, id, PickChartPlan { descriptors })?;
        let picker = &mut self.picker;
        let active_registry =
            picker
                .active
                .as_ref()
                .ok_or(crate::gpu_pick::GpuPickError::Internal(
                    "successful chart preparation publishes an active picker registry",
                ))?;
        let pipelines = match &mut picker.nearest {
            Some(pipelines) if pipelines.set() == set => pipelines,
            slot => slot.insert(crate::gpu_pick::NearestPickPipelines::new(set)),
        };
        let mut ticket = active_registry.engine.pick_nearest(
            pipelines,
            &self.pool,
            request.display_scale,
            &slots,
            active,
        )?;
        ticket.monotonic = monotonic_rows;
        Ok((ticket, subplot, active))
    }

    /// The pick of a registered chart (#381), [`Self::pick_chart`]: the
    /// nearest-datum pick of [`Self::pick_chart_nearest`], and a read of
    /// the ink masks of the same layer group around the cursor. The cursor
    /// hits when ink lies within `request.max_distance_px` canvas pixels
    /// of it; the series that owns the nearest inked texel decides the hit
    /// (an exact tie goes to the active layer, then to the panel and the
    /// series drawn later).
    pub(super) fn pick_chart_inked(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        let (nearest, subplot, active_layer) = self.pick_chart_nearest_grouped(id, request)?;
        let reach = request.max_distance_px;
        let cursor = request.canvas_position_px;
        if !reach.is_finite() || reach < 0.0 || !cursor.iter().all(|c| c.is_finite()) {
            return Ok(crate::gpu_pick::GpuPickTicket::from_nearest(nearest, None));
        }
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let layers = state.config.panel_layers();
        let in_group = |layer: crate::LayerRef| match (subplot, layer) {
            (None, crate::LayerRef::Base | crate::LayerRef::Overlay(_)) => true,
            (
                Some(index),
                crate::LayerRef::Subplot(at) | crate::LayerRef::SubplotOverlay(at, _),
            ) => at == index,
            _ => false,
        };
        let mut masks = Vec::new();
        masks.try_reserve_exact(layers.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "ink read masks",
            }
        })?;
        for mask in self.ink_masks.iter().filter(|mask| mask.chart == id) {
            let layer = layers
                .get(mask.panel)
                .copied()
                .unwrap_or(crate::LayerRef::Base);
            if !in_group(layer) {
                continue;
            }
            // The active layer's panels first, then the panel drawn later.
            let rank = (u64::from(layer == active_layer) << 32)
                | u64::try_from(mask.panel).unwrap_or(u64::from(u32::MAX));
            masks.push(crate::gpu_pick::InkMaskRead {
                texture: &mask.texture,
                origin: [i64::from(mask.panel_rect.x), i64::from(mask.panel_rect.y)],
                layer,
                rank,
                owners: &mask.owners,
            });
        }
        let active = self
            .picker
            .active
            .as_ref()
            .ok_or(crate::gpu_pick::GpuPickError::Internal(
                "successful chart preparation publishes an active picker registry",
            ))?;
        let ink =
            crate::gpu_pick::InkRead::submit(&active.engine.handles(), &masks, cursor, reach)?;
        Ok(crate::gpu_pick::GpuPickTicket::from_nearest(nearest, ink))
    }
}
