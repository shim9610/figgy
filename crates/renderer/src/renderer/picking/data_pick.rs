//! The typed data pick: what each series draws under the cursor, with its value labels.

use super::*;

/// The one branch of [`Renderer::pick_chart_data`] that answers for a
/// series, by its effective primitives. The browser's compile ahead of a
/// typed pick reads the kernels from the same route (#327).
#[derive(Clone, Copy)]
pub(super) enum DataPickRoute {
    Histogram,
    BoxPlot,
    RangeBars,
    RangeLinks,
    CategoryBars,
    Parallel,
    Vector,
    Mesh,
    /// A field or contour; a series that draws neither (a line, a
    /// scatter) answers through the point pick only.
    Field,
}

impl DataPickRoute {
    pub(super) fn of(primitives: EffectiveSeriesPrimitives) -> Self {
        if primitives.bar {
            Self::Histogram
        } else if primitives.box_plot {
            Self::BoxPlot
        } else if primitives.range_bars {
            Self::RangeBars
        } else if primitives.range_links {
            Self::RangeLinks
        } else if primitives.category_bars {
            Self::CategoryBars
        } else if primitives.parallel {
            Self::Parallel
        } else if primitives.vector {
            Self::Vector
        } else if primitives.mesh || primitives.mesh_contour {
            Self::Mesh
        } else {
            Self::Field
        }
    }

    /// The kernels a series on this route can dispatch.
    pub(super) fn kernels(
        self,
        primitives: EffectiveSeriesPrimitives,
    ) -> crate::gpu_data_pick::DataPickKernelSet {
        use crate::gpu_data_pick::{DataPickKernel as K, DataPickKernelSet as Set};
        match self {
            Self::Histogram => Set::EMPTY.with(K::Histogram),
            Self::BoxPlot if primitives.violin => Set::EMPTY.with(K::BoxPlot).with(K::Violin),
            Self::BoxPlot => Set::EMPTY.with(K::BoxPlot),
            Self::RangeBars => Set::EMPTY.with(K::RangeBar),
            Self::RangeLinks => Set::EMPTY.with(K::RangeLink),
            Self::CategoryBars => Set::EMPTY.with(K::CategoryBar),
            Self::Parallel => Set::EMPTY.with(K::Parallel),
            Self::Vector => Set::EMPTY.with(K::VectorCell),
            Self::Mesh => Set::EMPTY.with(K::TriMesh),
            Self::Field if primitives.field || primitives.contour => {
                Set::EMPTY.with(K::FieldContour).with(K::FieldCell)
            }
            Self::Field => Set::EMPTY,
        }
    }
}

impl Renderer {
    /// The category bar group 3 series `cfg`'s value labels are placed over
    /// at the pick's display transform: the bar or box group's params and
    /// style map (as the draw resolves them), or a radar series' category
    /// and value columns. `None` for a series whose bars, boxes or polygon
    /// are not drawn.
    pub(super) fn value_label_pick_group(
        &mut self,
        config: &Config,
        display_config: &Config,
        series: &[SeriesConfig],
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<data_render::CategoryBarData>> {
        let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
        let lookup = |pool: &crate::data_render::ColumnPool, id: &str| {
            pool.handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
        };
        if primitives.category_bars
            && let Some(bars) = cfg.render_type.category_bars()
        {
            let group = series.iter().filter(|s| {
                effective_series_primitives(&config.draw_style, &s.render_type).category_bars
            });
            let params = category_bar_params(
                display_config,
                group.clone(),
                cfg,
                scale,
                |id| lookup(&self.pool, id),
                |id| self.pool.slot(id).map(|slot| slot.min),
            )?;
            let style_map = crate::renderer::style::category_bar_style_map(
                &self.device,
                &self.per_point_style_map_bgl,
                category_bar_categories(display_config).unwrap_or(&[]),
                bars,
                scale,
            )?;
            return data_render::create_category_bar_data_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.category_bar_data_bgl,
                self.pool.buffer(),
                &params,
                params.members as usize,
                |push| {
                    category_bar_lanes(
                        display_config,
                        group,
                        cfg,
                        |id| lookup(&self.pool, id),
                        push,
                    )
                },
                style_map,
            )
            .map(Some);
        }
        if primitives.box_plot
            && let Some(boxes) = cfg.render_type.box_plot()
        {
            let (params, lanes, _) = box_plot_params(
                display_config,
                series.iter().filter(|s| {
                    effective_series_primitives(&config.draw_style, &s.render_type).box_plot
                }),
                cfg,
                scale,
                &self.pool,
                |id| lookup(&self.pool, id),
            )?;
            let style_map = crate::renderer::style::box_plot_style_map(
                &self.device,
                &self.per_point_style_map_bgl,
                category_bar_categories(display_config).unwrap_or(&[]),
                boxes,
                scale,
            )?;
            return data_render::create_category_bar_data_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.category_bar_data_bgl,
                self.pool.buffer(),
                &params,
                lanes.len(),
                |push| {
                    lanes.iter().for_each(|&lane| push(lane));
                    Ok(())
                },
                style_map,
            )
            .map(Some);
        }
        if primitives.range_bars {
            let (data, _) = range_bar_data(
                &self.device,
                &self.gpu_ledger,
                &self.category_bar_data_bgl,
                &self.per_point_style_map_bgl,
                self.pool.buffer(),
                display_config,
                series.iter().filter(|s| {
                    effective_series_primitives(&config.draw_style, &s.render_type).range_bars
                }),
                cfg,
                scale,
                |id| lookup(&self.pool, id),
            )?;
            return Ok(Some(data));
        }
        if primitives.line && cfg.render_type.radar().is_some() {
            let pool_buffer = self.pool.buffer().clone();
            let (_, preparation) = self.preparation_parts();
            let mut lookup = |id: &str| lookup(preparation.pool, id);
            return preparation
                .radar_label_columns(
                    display_config,
                    series.iter(),
                    cfg,
                    &pool_buffer,
                    scale,
                    &mut lookup,
                )
                .map(Some);
        }
        Ok(None)
    }

    /// The typed data pick kernels a pick of chart `id` at `request`
    /// dispatches (#327): the chart routed and its layer's series read as
    /// [`Self::pick_chart_data`] does, each by its [`DataPickRoute`], with
    /// `pick_value_labels` for a series whose value labels are drawn over
    /// bars, boxes or a radar polygon. A 3D chart picks by its ID render
    /// and needs none.
    #[cfg(any(test, target_arch = "wasm32"))]
    pub(in crate::renderer) fn data_pick_kernels_for(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> Result<crate::gpu_data_pick::DataPickKernelSet> {
        use crate::gpu_data_pick::DataPickKernelSet;
        self.admit_chart_shaders(id);
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        let routed = subplot_route(&state.config, &displayed, request.canvas_position_px);
        if let Some(layer) = routed {
            displayed.active_layer = layer;
        }
        let inputs = match routed {
            Some(layer) => layer_pick_inputs(&state.config, layer, &state.series)?,
            None => active_layer_pick_inputs(&state.config, &state.series)?,
        };
        let display_config = active_layer_pick_inputs(&displayed, &state.series)?.panel;
        if display_config.view_3d().is_some() {
            return Ok(DataPickKernelSet::EMPTY);
        }
        let mut needed = DataPickKernelSet::EMPTY;
        for cfg in inputs.series.iter() {
            needed = needed.union(series_data_pick_kernels(&inputs.panel, cfg, |cfg| {
                value_label_atlas_input(&display_config, cfg).is_some()
            }));
        }
        Ok(needed)
    }

    /// Every typed data pick kernel a pick of chart `id` can dispatch,
    /// wherever it lands (#363): for each layer a pick can reach (the
    /// active layer and, from the base layer, each subplot), each series'
    /// kernels as [`Self::data_pick_kernels_for`] reads them, with the value
    /// label kernel for a series that draws value labels selected or not. A
    /// superset of what any one pick needs; empty for a 3D layer.
    pub(in crate::renderer) fn data_pick_kernels_of_chart(
        &self,
        id: ChartId,
    ) -> Result<crate::gpu_data_pick::DataPickKernelSet> {
        use crate::gpu_data_pick::DataPickKernelSet;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let mut needed = DataPickKernelSet::EMPTY;
        let mut layer = |inputs: LayerPickInputs<'_>| -> Result<()> {
            if inputs.panel.view_3d().is_some() {
                return Ok(());
            }
            for cfg in inputs.series.iter() {
                needed = needed.union(series_data_pick_kernels(&inputs.panel, cfg, |cfg| {
                    [false, true].into_iter().any(|selected| {
                        data_render::value_label_kind(&cfg.render_type, selected).is_some()
                    })
                }));
            }
            Ok(())
        };
        layer(active_layer_pick_inputs(&state.config, &state.series)?)?;
        if state.config.active_layer() == crate::LayerRef::Base {
            for index in 0..state.config.subplots.len() {
                layer(layer_pick_inputs(
                    &state.config,
                    crate::LayerRef::Subplot(index),
                    &state.series,
                )?)?;
            }
        }
        Ok(needed)
    }

    /// Drop the typed data pick kernels no registered chart can dispatch
    /// any more (#363). Run by the host's release of unused GPU memory,
    /// never on its own: a removed chart's kernels stay until then, so the
    /// same or a similar chart registered again reuses them instead of
    /// compiling, and a live chart's are never dropped. While any chart
    /// draws from a stream, whose typed picks also run the stream's
    /// reduction kernels, every kernel is kept.
    pub(in crate::renderer) fn release_unused_data_pick_kernels(&mut self) {
        use crate::gpu_data_pick::DataPickKernelSet;
        let streamed = self.chart_states.values().any(|state| {
            state
                .series
                .iter()
                .any(|series| self.require_resident_series(series).is_err())
        });
        if streamed {
            return;
        }
        let mut live = DataPickKernelSet::EMPTY;
        for &id in self.chart_states.keys() {
            match self.data_pick_kernels_of_chart(id) {
                Ok(kernels) => live = live.union(kernels),
                // A chart whose kernels cannot be read keeps every kernel.
                Err(_) => return,
            }
        }
        self.picker.data_kernels.retain(live);
    }

    /// Submit one typed data pick across points/lines, histogram bins, heatmap
    /// cells, and contour levels.
    ///
    /// The ticket answers the registered chart's pick (the nearest datum
    /// and the ink hit, [`Self::pick_chart`]) and the typed hit separately
    /// ([`crate::DataPickResult`], #381): the selected element is the
    /// nearest datum when the ink hits and the chart has a series with
    /// point data, the typed hit otherwise. A datum never competes with a
    /// body, bar or cell by distance.
    ///
    /// Source values stay in `ColumnPool`. The CPU supplies only current
    /// render metadata (lane bases, counts, styles and identity indices), while
    /// the bar/field render shaders themselves compute the exact hit geometry.
    pub fn pick_chart_data(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> Result<crate::GpuDataPickTicket> {
        let result = self.pick_chart_data_unreported(id, request);
        self.reported(result)
    }

    pub(super) fn pick_chart_data_unreported(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> Result<crate::GpuDataPickTicket> {
        self.health.check()?;
        // A chart with an axis break, a probability axis or polar
        // coordinates is picked with the full shaders only (#299).
        self.admit_chart_shaders(id);
        // A 3D chart's point pick binds the series' registered styles (#132),
        // built here when the chart has not been drawn yet.
        if self
            .chart_states
            .get(&id)
            .is_some_and(|state| state.config.view_3d().is_some())
        {
            self.ensure_registered_styles(id)?;
        }
        let (config, series, display_config, styles, layer) =
            self.typed_layer_inputs(id, request, None)?;
        // `pick_chart` moves the cursor itself, so it gets the request as
        // given; everything else here reads it moved.
        let given = request;
        let (request, moved) = pick_request_outside_gaps(&display_config, request);
        // The layer's DataLess series pick on the CPU (#203, #234); a gap
        // moved the request out of reach, so none answers there.
        let cpu = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            pick_energy_levels(&display_config, &series, &state.series, &request)?
        };

        // A 3D chart's surfaces, markers and strokes are picked by their ID
        // render: the draw's own rasterizer and depth test, so a hidden
        // quad or point never answers.
        if display_config.view_3d().is_some() {
            return self.pick_surface(&display_config, &series, &styles, &request);
        }
        // The registered chart's pick first (#381): every series' nearest
        // datum and the ink hit, routed to the same subplot. Its datum and
        // the typed hits below are separate answers; neither competes with
        // the other by distance.
        let point_ticket = self.pick_chart_inked(id, given)?;
        let data_bundle = self.picker.ready_data_bundle()?;
        if data_bundle.set == ShaderSet::Basic && config_needs_full_shaders(&display_config) {
            return Err(FiggyError::internal(
                "a chart with an axis break, a probability axis or polar coordinates is picked with the full shaders",
            ));
        }

        let data_ticket = self
            .encode_typed_layer(&config, &series, &display_config, request, moved)?
            .submit();
        // The group's other layers with body or fill series: their typed
        // picks are encoded and not run; the ticket runs the one whose
        // layer has the ink owner, when that owner is a body (#381).
        let deferred = self.deferred_typed_layers(id, given, layer)?;
        Ok(crate::gpu_data_pick::GpuDataPickTicket::new(
            point_ticket,
            data_ticket,
            cpu,
            layer,
            deferred,
        ))
    }

    /// The typed data pick's inputs for chart `id` at `request` (#381): the
    /// layer's panel and series, its panel as displayed, its series' 3D
    /// pick styles, and the layer. `layer` names the layer to pick; `None`
    /// picks the active one, or the subplot drawn at the cursor.
    #[allow(clippy::type_complexity)]
    fn typed_layer_inputs(
        &self,
        id: ChartId,
        request: GpuPickRequest,
        layer: Option<crate::LayerRef>,
    ) -> Result<(
        Config,
        Vec<SeriesConfig>,
        Config,
        Vec<Option<super::surface_pick::PickStyle3d>>,
        crate::LayerRef,
    )> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        for series in &state.series {
            self.require_resident_series(series)?;
        }
        // The typed query answers for the active layer only, with that
        // layer's panel (its Y range) and the series it scales, so its
        // paint orders agree with the point query's registry. The query
        // geometry is the panel as displayed: the document is scaled and
        // placed first and expanded after, exactly as it is drawn.
        let mut displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        // A pointer over a subplot's frame (an inset over the parent, or
        // any subplot elsewhere, #203, #205) belongs to the subplot drawn
        // on top there: on the base layer it is routed there.
        // A layer named by the caller is picked as it is drawn, on its
        // own panel (#381: the ink owner's layer). It is one of the layers
        // the document draws as a panel (`deferred_typed_layers`), so it is
        // read from the stored document without copying it (#442).
        let routed = match layer {
            Some(_) => None,
            None => subplot_route(&state.config, &displayed, request.canvas_position_px),
        };
        if let Some(layer) = layer.or(routed) {
            displayed.active_layer = layer;
        }
        let active = layer
            .or(routed)
            .unwrap_or_else(|| state.config.active_layer());
        let picked_layer = match active {
            crate::LayerRef::Annotations => crate::LayerRef::Base,
            layer => layer,
        };
        let inputs = match layer.or(routed) {
            Some(layer) => layer_pick_inputs(&state.config, layer, &state.series)?,
            None => active_layer_pick_inputs(&state.config, &state.series)?,
        };
        // The panel as displayed: `displayed` itself when the document is
        // its own panel, moved rather than copied (#442).
        let display_panel = match active_layer_pick_inputs(&displayed, &state.series)?.panel {
            std::borrow::Cow::Owned(panel) => Some(panel),
            std::borrow::Cow::Borrowed(_) => None,
        };
        let display_config = display_panel.unwrap_or(displayed);
        // The prepared styles of the layer's series, for a 3D chart's
        // point picks (#132): a series not prepared yet has none.
        let styles: Vec<Option<super::surface_pick::PickStyle3d>> = inputs
            .series
            .iter()
            .map(|cfg| {
                let position = state
                    .series
                    .iter()
                    .position(|s| s.series_id == cfg.series_id)?;
                // The styles the chart is drawn with: at the display
                // scale when the pick is in that display (#160).
                let display = state.display.as_ref().and_then(|display| {
                    display
                        .styles
                        .as_ref()
                        .filter(|(bits, _)| *bits == request.display_scale.to_bits())
                });
                match display {
                    Some((_, styles)) => Some(styles),
                    None => state.prepared_styles.as_ref(),
                }
                .and_then(|prepared| prepared.styles.get(position))
                .map(super::surface_pick::PickStyle3d::of)
            })
            .collect();
        Ok((
            inputs.panel.into_owned(),
            inputs.series.into_owned(),
            display_config,
            styles,
            picked_layer,
        ))
    }

    /// The typed data pick of one layer (`config`, `series`, its panel as
    /// displayed), encoded and not submitted: every series' typed geometry
    /// and its value labels (#381).
    fn encode_typed_layer(
        &mut self,
        config: &Config,
        series: &[SeriesConfig],
        display_config: &Config,
        request: GpuPickRequest,
        moved: [f32; 2],
    ) -> Result<crate::gpu_data_pick::EncodedDataPick> {
        let mut data_series = Vec::new();
        // One entry per series and one per series' value labels.
        data_series
            .try_reserve_exact(series.len().saturating_mul(2))
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed data pick series",
                reason: error.to_string(),
            })?;
        // In drawing order, ranked by it (#357): a range link series listed
        // after its bars draws, and so picks, under them.
        let order = crate::data_config::series_draw_order(series.len(), |i| &series[i]);
        for (rank, cfg) in order.enumerate().map(|(rank, i)| (rank, &series[i])) {
            let paint_order = u32::try_from(rank).map_err(|_| FiggyError::InvalidSeriesConfig {
                series_id: cfg.series_id.clone(),
                kind: crate::RuleKind::SeriesPaintOrderOverflow,
            })?;

            let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
            match DataPickRoute::of(primitives) {
                DataPickRoute::Histogram => {
                    self.histogram_data_pick(request, cfg, paint_order, &mut data_series)?;
                }
                DataPickRoute::BoxPlot => self.box_plot_data_pick(
                    &config,
                    &display_config,
                    &series,
                    request,
                    cfg,
                    paint_order,
                    primitives,
                    &mut data_series,
                )?,
                DataPickRoute::RangeBars => self.range_bar_data_pick(
                    &config,
                    &display_config,
                    &series,
                    request,
                    cfg,
                    paint_order,
                    &mut data_series,
                )?,
                DataPickRoute::RangeLinks => self.range_link_data_pick(
                    &config,
                    &display_config,
                    &series,
                    request,
                    cfg,
                    paint_order,
                    &mut data_series,
                )?,
                DataPickRoute::CategoryBars => self.category_bar_data_pick(
                    &config,
                    &display_config,
                    &series,
                    request,
                    cfg,
                    paint_order,
                    &mut data_series,
                )?,
                DataPickRoute::Parallel => {
                    self.parallel_data_pick(&display_config, cfg, paint_order, &mut data_series)?;
                }
                DataPickRoute::Vector => {
                    self.vector_data_pick(&config, request, cfg, paint_order, &mut data_series)?;
                }
                DataPickRoute::Mesh => self.mesh_data_pick(
                    &config,
                    request,
                    cfg,
                    paint_order,
                    primitives,
                    &mut data_series,
                )?,
                DataPickRoute::Field => self.field_data_pick(
                    &config,
                    &display_config,
                    request,
                    cfg,
                    paint_order,
                    primitives,
                    &mut data_series,
                )?,
            }
        }

        // Value labels: drawn over every series of the panel, so they pick
        // with a paint order after every series and win the ties against
        // the data under them. The same atlas, uniform and group 3 the draw
        // places them with, at the pick's display transform and scale.
        for (index, cfg) in series.iter().enumerate() {
            let Some(atlas) = self
                .preparation_parts()
                .1
                .ensure_value_label_scratch(&display_config, cfg)?
            else {
                continue;
            };
            let Some(data) = self.value_label_pick_group(
                &config,
                &display_config,
                &series,
                cfg,
                request.display_scale,
            )?
            else {
                continue;
            };
            // A radar series' labels are placed with those of its label
            // group's members, as the draw places them (#272): their atlases
            // are resolved first and stay resident through the frame batch.
            let mut preparation = self.preparation_parts().1;
            let labels = preparation.frame_batch(|preparation| {
                if cfg.render_type.radar().is_some() {
                    for (_, member) in radar_label_members(&display_config, series.iter()) {
                        preparation.ensure_value_label_scratch(&display_config, member)?;
                    }
                }
                preparation.value_label_data(
                    &display_config,
                    series.iter(),
                    cfg,
                    &atlas,
                    request.display_scale,
                    |_, member| Ok(preparation.resident_value_label_atlas(&display_config, member)),
                )
            })?;
            let Some(labels) = labels else {
                continue;
            };
            if labels.count == 0 {
                continue;
            }
            let paint_order = u32::try_from(series.len() + index).map_err(|_| {
                FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::ValueLabelPaintOrderOverflow,
                }
            })?;
            data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
                geometry: crate::gpu_data_pick::GpuDataPickGeometry::ValueLabels { data, labels },
            });
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
        let data_bundle = self.picker.ready_data_bundle()?;
        Ok(data_bundle.encode(
            &mut self.picker.data_kernels,
            &self.pool,
            crate::gpu_data_pick::GpuDataPickQuery {
                transform: data_render::scatter_transform_from_config(&display_config),
                chart_rect_px: [
                    chart_rect.x as f32,
                    chart_rect.y as f32,
                    chart_rect.width as f32,
                    chart_rect.height as f32,
                ],
                data_area_px,
                canvas_position_px: request.canvas_position_px,
                max_distance_px: request.max_distance_px,
            },
            data_series,
        )?)
    }

    /// The typed picks of the layers drawn with `picked`'s group (the base
    /// layer and its overlays, or a subplot and its overlays) other than
    /// `picked`, for each that has a body or fill series, encoded and not
    /// run (#381). A layer drawn as no panel of its own, a 3D layer and a
    /// layer that needs the full shaders while the picker has the basic
    /// ones are left out.
    fn deferred_typed_layers(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
        picked: crate::LayerRef,
    ) -> Result<Vec<(crate::LayerRef, crate::gpu_data_pick::EncodedDataPick)>> {
        let group = |layer: crate::LayerRef| match (picked, layer) {
            (
                crate::LayerRef::Base | crate::LayerRef::Overlay(_),
                crate::LayerRef::Base | crate::LayerRef::Overlay(_),
            ) => true,
            (
                crate::LayerRef::Subplot(at) | crate::LayerRef::SubplotOverlay(at, _),
                crate::LayerRef::Subplot(index) | crate::LayerRef::SubplotOverlay(index, _),
            ) => at == index,
            _ => false,
        };
        let layers = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            let displayed = place_document(
                &state.config,
                request.display_scale,
                request.display_panel_px,
            );
            let count = render_panel_count(&displayed)?;
            let mut layers = displayed.panel_layers();
            layers.truncate(count);
            layers.retain(|&layer| layer != picked && group(layer));
            layers.dedup();
            layers
        };
        let mut deferred = Vec::new();
        deferred.try_reserve_exact(layers.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "deferred typed data picks",
                reason: error.to_string(),
            }
        })?;
        let set = self.picker.ready_data_bundle()?.set;
        for layer in layers {
            let (config, series, display_config, _, _) =
                self.typed_layer_inputs(id, request, Some(layer))?;
            let bodies = series.iter().any(|cfg| {
                series_data_pick_kernels(&config, cfg, |_| false)
                    != crate::gpu_data_pick::DataPickKernelSet::EMPTY
            });
            if !bodies
                || display_config.view_3d().is_some()
                || (set == ShaderSet::Basic && config_needs_full_shaders(&display_config))
            {
                continue;
            }
            let (request, moved) = pick_request_outside_gaps(&display_config, request);
            let encoded =
                self.encode_typed_layer(&config, &series, &display_config, request, moved)?;
            deferred.push((layer, encoded));
        }
        Ok(deferred)
    }
}

/// The typed data pick kernels a pick of series `cfg` on `panel`
/// dispatches (#327): its route's kernels, and the value label kernel when
/// the series is one `value_label_pick_group` places labels over and
/// `labelled` says it draws them.
fn series_data_pick_kernels(
    panel: &Config,
    cfg: &SeriesConfig,
    labelled: impl Fn(&SeriesConfig) -> bool,
) -> crate::gpu_data_pick::DataPickKernelSet {
    use crate::gpu_data_pick::DataPickKernel;
    let primitives = effective_series_primitives(&panel.draw_style, &cfg.render_type);
    let mut needed = DataPickRoute::of(primitives).kernels(primitives);
    let placed = primitives.category_bars
        || primitives.box_plot
        || primitives.range_bars
        || (primitives.line && cfg.render_type.radar().is_some());
    if placed && labelled(cfg) {
        needed = needed.with(DataPickKernel::ValueLabels);
    }
    needed
}
