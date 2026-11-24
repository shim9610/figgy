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
                value_label_face(&display_config, cfg).is_some()
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
        self.picker.release_typed_picks();
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
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        for series in &state.series {
            self.require_resident_series(series)?;
        }
        let labels = ValueLabelSource::of(id, &state.revisions);
        let desired = state.revisions.desired;
        // The chart's config and series are lent to the typed pick, not
        // cloned (#443): the registry is set aside while the pick encodes,
        // which reads none of it, and put back whatever the outcome.
        let states = std::mem::take(&mut self.chart_states);
        let typed = states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })
            .and_then(|state| {
                with_typed_layer_inputs(
                    state,
                    request,
                    None,
                    |config, series, display_config, styles, layer| {
                        self.typed_pick_encode(
                            (config, series, display_config, styles, layer),
                            &state.series,
                            request,
                            (id, desired, labels),
                        )
                    },
                )
            });
        self.chart_states = states;
        let (encoded, cpu, layer) = match typed? {
            TypedPick::Surface(ticket) => return Ok(ticket),
            TypedPick::Encoded {
                encoded,
                cpu,
                layer,
            } => (encoded, cpu, layer),
        };
        // `pick_chart` moves the cursor itself, so it gets the request as
        // given.
        // The registered chart's pick (#381): every series' nearest datum
        // and the ink hit, routed to the same subplot. Its datum and the
        // typed hits are separate answers; neither competes with the other
        // by distance.
        let point_ticket = self.pick_chart_inked(id, request)?;
        let data_ticket = encoded.submit();
        // The group's other layers with body or fill series: their typed
        // picks are encoded and not run; the ticket runs the one whose
        // layer has the ink owner, when that owner is a body (#381).
        let deferred = self.deferred_typed_layers(id, request, layer)?;
        Ok(crate::gpu_data_pick::GpuDataPickTicket::new(
            point_ticket,
            data_ticket,
            cpu,
            layer,
            deferred,
        ))
    }

    /// The typed half of a pick of a registered chart over its lent
    /// inputs ([`with_typed_layer_inputs`]): the CPU picks of its DataLess
    /// series, a 3D chart's ID render, or the typed kernels encoded and not
    /// submitted.
    #[allow(clippy::type_complexity)]
    fn typed_pick_encode(
        &mut self,
        (config, series, display_config, styles, layer): (
            &Config,
            &[SeriesConfig],
            &Config,
            &[Option<super::surface_pick::PickStyle3d>],
            crate::LayerRef,
        ),
        all_series: &[SeriesConfig],
        given: GpuPickRequest,
        (id, desired, labels): (ChartId, RenderRevision, ValueLabelSource),
    ) -> Result<TypedPick> {
        // Everything here reads the request moved out of the gaps.
        let (request, moved) = pick_request_outside_gaps(display_config, given);
        // The layer's DataLess series pick on the CPU (#203, #234); a gap
        // moved the request out of reach, so none answers there.
        let cpu = pick_energy_levels(display_config, series, all_series, &request)?;
        // A 3D chart's surfaces, markers and strokes are picked by their ID
        // render: the draw's own rasterizer and depth test, so a hidden
        // quad or point never answers.
        if display_config.view_3d().is_some() {
            return self
                .pick_surface(display_config, series, styles, &request)
                .map(TypedPick::Surface);
        }
        let data_bundle = self.picker.ready_data_bundle()?;
        if data_bundle.set == ShaderSet::Basic && config_needs_full_shaders(display_config) {
            return Err(FiggyError::internal(
                "a chart with an axis break, a probability axis or polar coordinates is picked with the full shaders",
            ));
        }
        let encoded = self.encode_typed_layer(
            (id, layer, desired),
            config,
            series,
            display_config,
            request,
            moved,
            labels,
        )?;
        Ok(TypedPick::Encoded {
            encoded,
            cpu,
            layer,
        })
    }

    /// The typed data pick of chart `id`'s layer `layer` (`config`,
    /// `series`, its panel as displayed), encoded and not submitted: every
    /// series' typed geometry and its value labels (#381). The prepared
    /// work is reused from the picker's cache while its key's generations
    /// hold and no other pick holds it (#445); only the cursor is written.
    #[allow(clippy::too_many_arguments)]
    fn encode_typed_layer(
        &mut self,
        (id, layer, desired): (ChartId, crate::LayerRef, RenderRevision),
        config: &Config,
        series: &[SeriesConfig],
        display_config: &Config,
        request: GpuPickRequest,
        moved: [f32; 2],
        labels: ValueLabelSource,
    ) -> Result<crate::gpu_data_pick::EncodedDataPick> {
        // `desired` is read by the caller: the registry is lent while a
        // typed pick encodes (#443).
        let key = super::picker::TypedPickKey {
            chart: id,
            layer,
            desired,
            visual: self.visual_revision,
            pool_layout: self.pool.layout_generation(),
            font: crate::text_render::font_generation(),
            set: self.picker.ready_data_bundle()?.set,
            display_scale: request.display_scale,
            display_panel_px: request.display_panel_px,
        };
        let prepared = match self.picker.reusable_typed_pick(&key) {
            Some(prepared) => prepared,
            None => {
                let data_series =
                    self.typed_layer_series(config, series, display_config, request, labels)?;
                let data_bundle = self.picker.ready_data_bundle()?;
                let prepared =
                    data_bundle.prepare(&mut self.picker.data_kernels, &self.pool, data_series)?;
                self.picker.keep_typed_pick(key, &prepared)?;
                prepared
            }
        };

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
            &prepared,
            &mut self.picker.data_readbacks,
            crate::gpu_data_pick::GpuDataPickQuery {
                transform: data_render::scatter_transform_from_config(display_config),
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
        )?)
    }

    /// Every series' typed geometry and value labels of one layer, for
    /// [`Self::encode_typed_layer`] to prepare.
    fn typed_layer_series(
        &mut self,
        config: &Config,
        series: &[SeriesConfig],
        display_config: &Config,
        request: GpuPickRequest,
        labels: ValueLabelSource,
    ) -> Result<Vec<crate::gpu_data_pick::GpuDataPickSeries>> {
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
        let slot = Some((labels, ValueLabelRole::Pick));
        for (index, cfg) in series.iter().enumerate() {
            let Some(atlas) =
                self.preparation_parts()
                    .1
                    .ensure_value_label_scratch(display_config, cfg, slot)?
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
            let cells = preparation.frame_batch(|preparation| {
                if cfg.render_type.radar().is_some() {
                    for (_, member) in radar_label_members(display_config, series.iter()) {
                        preparation.ensure_value_label_scratch(display_config, member, slot)?;
                    }
                }
                preparation.ensure_value_label_data(
                    display_config,
                    series.iter(),
                    cfg,
                    &atlas,
                    request.display_scale,
                    slot,
                    |preparation, _, member| {
                        Ok(preparation.resident_value_label_atlas(display_config, member, slot))
                    },
                )
            })?;
            let Some(labels) = cells else {
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

        Ok(data_series)
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
        let (labels, desired) = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            (
                ValueLabelSource::of(id, &state.revisions),
                state.revisions.desired,
            )
        };
        // As the active layer's pick: the registry is lent, not cloned.
        let states = std::mem::take(&mut self.chart_states);
        let encoded = states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })
            .and_then(|state| {
                for layer in layers {
                    let encoded = with_typed_layer_inputs(
                        state,
                        request,
                        Some(layer),
                        |config, series, display_config, _, _| {
                            let bodies = series.iter().any(|cfg| {
                                series_data_pick_kernels(config, cfg, |_| false)
                                    != crate::gpu_data_pick::DataPickKernelSet::EMPTY
                            });
                            if !bodies
                                || display_config.view_3d().is_some()
                                || (set == ShaderSet::Basic
                                    && config_needs_full_shaders(display_config))
                            {
                                return Ok(None);
                            }
                            let (request, moved) =
                                pick_request_outside_gaps(display_config, request);
                            self.encode_typed_layer(
                                (id, layer, desired),
                                config,
                                series,
                                display_config,
                                request,
                                moved,
                                labels,
                            )
                            .map(Some)
                        },
                    )?;
                    if let Some(encoded) = encoded {
                        deferred.push((layer, encoded));
                    }
                }
                Ok(())
            });
        self.chart_states = states;
        encoded?;
        Ok(deferred)
    }
}

/// The typed data pick's inputs for the registered chart `state` at
/// `request` (#381), lent to `f`: the layer's panel and series, its panel
/// as displayed, its series' 3D pick styles, and the layer. `layer` names
/// the layer to pick; `None` picks the active one, or the subplot drawn at
/// the cursor. The series are borrowed from the chart, never cloned: a
/// pick copies no label string (#443).
fn with_typed_layer_inputs<R>(
    state: &ChartRenderState,
    request: GpuPickRequest,
    layer: Option<crate::LayerRef>,
    f: impl FnOnce(
        &Config,
        &[SeriesConfig],
        &Config,
        &[Option<super::surface_pick::PickStyle3d>],
        crate::LayerRef,
    ) -> Result<R>,
) -> Result<R> {
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
    // own panel (#381: the ink owner's layer).
    // A layer named by the caller may be out of range and falls back as
    // `Config::active_layer` does, so it is set on a copy; a subplot
    // route always names a subplot the document has.
    let named = layer.map(|layer| {
        let mut config = state.config.clone();
        config.active_layer = layer;
        config
    });
    let routed = match &named {
        Some(_) => None,
        None => subplot_route(&state.config, &displayed, request.canvas_position_px),
    };
    if let Some(layer) = named.as_ref().map(|config| config.active_layer).or(routed) {
        displayed.active_layer = layer;
    }
    let active = match (&named, routed) {
        (Some(config), _) => config.active_layer(),
        (None, Some(layer)) => layer,
        (None, None) => state.config.active_layer(),
    };
    let picked_layer = match active {
        crate::LayerRef::Annotations => crate::LayerRef::Base,
        layer => layer,
    };
    let inputs = match (&named, routed) {
        (Some(config), _) => active_layer_pick_inputs(config, &state.series)?,
        (None, Some(layer)) => layer_pick_inputs(&state.config, layer, &state.series)?,
        (None, None) => active_layer_pick_inputs(&state.config, &state.series)?,
    };
    let display_inputs = active_layer_pick_inputs(&displayed, &state.series)?;
    let display_config = &*display_inputs.panel;
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
    f(
        &inputs.panel,
        &inputs.series,
        display_config,
        &styles,
        picked_layer,
    )
}

/// The typed half of a registered chart's pick.
enum TypedPick {
    /// A 3D chart's ID render: the whole answer.
    Surface(crate::GpuDataPickTicket),
    /// The typed kernels, encoded and not submitted, with the CPU picks of
    /// the layer's DataLess series and the layer picked.
    Encoded {
        encoded: crate::gpu_data_pick::EncodedDataPick,
        cpu: Option<crate::gpu_data_pick::RankedPickedData>,
        layer: crate::LayerRef,
    },
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
