//! The typed data pick: what each series draws under the cursor, with its value labels.

use super::*;

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
                .radar_label_group(cfg, &pool_buffer, scale, &mut lookup)
                .map(Some);
        }
        Ok(None)
    }

    /// Submit one typed data pick across points/lines, histogram bins, heatmap
    /// cells, and contour levels.
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
        // A 3D chart's point pick binds the series' registered styles (#132),
        // built here when the chart has not been drawn yet.
        if self
            .chart_states
            .get(&id)
            .is_some_and(|state| state.config.view_3d().is_some())
        {
            self.ensure_registered_styles(id)?;
        }
        let (config, series, display_config, members, styles) = {
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
            let routed = subplot_route(&state.config, &displayed, request.canvas_position_px);
            if let Some(config) = &routed {
                displayed.active_layer = config.active_layer;
            }
            let inputs =
                active_layer_pick_inputs(routed.as_ref().unwrap_or(&state.config), &state.series)?;
            let display_config = active_layer_pick_inputs(&displayed, &state.series)?.panel;
            let members = inputs.member_count();
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
            (inputs.panel, inputs.series, display_config, members, styles)
        };
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
        // Submit the established point/line query first. It stays point-only,
        // so the compatibility API and its registry lifecycle are unchanged.
        // It routes the same cursor to the same subplot, so the points it
        // answers are the ones drawn on top there.
        let point_ticket = self.pick_chart(id, given)?;
        let data_bundle = self.picker.ready_data_bundle()?;

        let mut data_series = Vec::new();
        // One entry per series and one per series' value labels.
        data_series
            .try_reserve_exact(series.len().saturating_mul(2))
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed data pick series",
                reason: error.to_string(),
            })?;
        let mut point_orders = Vec::new();
        point_orders
            .try_reserve_exact(series.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed point pick paint order",
                reason: error.to_string(),
            })?;

        for (index, cfg) in series.iter().enumerate() {
            let paint_order =
                u32::try_from(index).map_err(|_| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    kind: crate::RuleKind::SeriesPaintOrderOverflow,
                })?;
            point_orders.push(crate::gpu_data_pick::PointPaintOrder {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
            });

            let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
            if primitives.bar {
                let bar = extract_bar(&cfg.render_type).ok_or(FiggyError::internal(
                    "effective histogram primitive has a bar config",
                ))?;
                let (edges_id, values_id, horizontal) = match bar.orientation {
                    crate::data_config::BarOrientation::Vertical => {
                        (&cfg.x_column, &cfg.y_column, false)
                    }
                    crate::data_config::BarOrientation::Horizontal => {
                        (&cfg.y_column, &cfg.x_column, true)
                    }
                };
                let edges =
                    self.pool
                        .handle_for(edges_id)
                        .ok_or_else(|| FiggyError::UnknownColumn {
                            id: edges_id.clone(),
                        })?;
                let values =
                    self.pool
                        .handle_for(values_id)
                        .ok_or_else(|| FiggyError::UnknownColumn {
                            id: values_id.clone(),
                        })?;
                let bin_count = edges.len_values.saturating_sub(1).min(values.len_values);
                if bin_count != 0 {
                    let bin_count =
                        u32::try_from(bin_count).map_err(|_| FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            kind: crate::RuleKind::HistogramBinCountOverflow,
                        })?;
                    let (baseline_hi, baseline_lo) =
                        crate::data::split_f64_to_f32_pair(bar.baseline);
                    let style_map = self
                        .create_bar_style_map_for(bar, request.display_scale)?
                        .map(|map| map.bind_group);
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::Histogram {
                            edges,
                            values,
                            bin_count,
                            baseline: [baseline_hi, baseline_lo],
                            gap_px: bar.gap_px.max(0.0) * request.display_scale,
                            width_ratio: data_render::sanitize_bar_width_ratio(bar.width_ratio),
                            horizontal,
                            style_map,
                        },
                    });
                }
                continue;
            }

            if primitives.box_plot {
                // The boxes the box plot entries draw at the pick's display
                // transform, from the same params and style map as the draw.
                let boxes = cfg.render_type.box_plot().ok_or(FiggyError::internal(
                    "the box plot primitive has a box plot config",
                ))?;
                let (params, lanes, _) = box_plot_params(
                    &display_config,
                    series.iter().filter(|s| {
                        effective_series_primitives(&config.draw_style, &s.render_type).box_plot
                    }),
                    cfg,
                    request.display_scale,
                    &self.pool,
                    |id| {
                        self.pool
                            .handle_for(id)
                            .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                    },
                )?;
                if params.count != 0 {
                    let style_map = crate::renderer::style::box_plot_style_map(
                        &self.device,
                        &self.per_point_style_map_bgl,
                        category_bar_categories(&display_config).unwrap_or(&[]),
                        boxes,
                        request.display_scale,
                    )?;
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::BoxPlot {
                            data: data_render::create_category_bar_data_bind_group(
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
                            )?,
                        },
                    });
                }
                // A violin's bodies, from the same params and style map as
                // its draw, after the boxes: a box painted over its violin
                // wins a tie. Its raw points are picked by the point picker.
                if primitives.violin {
                    let (data, segments) = box_extras_data(
                        &self.device,
                        &self.gpu_ledger,
                        &self.category_bar_data_bgl,
                        &self.per_point_style_map_bgl,
                        self.pool.buffer(),
                        &self.pool,
                        &display_config,
                        series.iter().filter(|s| {
                            effective_series_primitives(&config.draw_style, &s.render_type).box_plot
                        }),
                        cfg,
                        request.display_scale,
                        |id| {
                            self.pool
                                .handle_for(id)
                                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                        },
                        false,
                    )?;
                    if segments != 0 {
                        data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                            source_id: cfg.source_id.clone(),
                            series_id: cfg.series_id.clone(),
                            paint_order,
                            geometry: crate::gpu_data_pick::GpuDataPickGeometry::Violin { data },
                        });
                    }
                }
                continue;
            }

            if primitives.range_bars {
                // The bars the range bar entries draw at the pick's display
                // transform, from the same params and style map as the draw.
                let (data, count) = range_bar_data(
                    &self.device,
                    &self.gpu_ledger,
                    &self.category_bar_data_bgl,
                    &self.per_point_style_map_bgl,
                    self.pool.buffer(),
                    &display_config,
                    series.iter().filter(|s| {
                        effective_series_primitives(&config.draw_style, &s.render_type).range_bars
                    }),
                    cfg,
                    request.display_scale,
                    |id| {
                        self.pool
                            .handle_for(id)
                            .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                    },
                )?;
                if count != 0 {
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::RangeBars { data },
                    });
                }
                continue;
            }

            if primitives.category_bars {
                // The bars the category bar module draws at the pick's
                // display transform, resolved like the draw's.
                let bars = cfg.render_type.category_bars().ok_or(FiggyError::internal(
                    "the category bar primitive has a category bar config",
                ))?;
                let group = series.iter().filter(|s| {
                    effective_series_primitives(&config.draw_style, &s.render_type).category_bars
                });
                let lookup = |id: &str| {
                    self.pool
                        .handle_for(id)
                        .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                };
                let params = category_bar_params(
                    &display_config,
                    group.clone(),
                    cfg,
                    request.display_scale,
                    lookup,
                    |id| self.pool.slot(id).map(|slot| slot.min),
                )?;
                if params.count != 0 {
                    let style_map = crate::renderer::style::category_bar_style_map(
                        &self.device,
                        &self.per_point_style_map_bgl,
                        category_bar_categories(&display_config).unwrap_or(&[]),
                        bars,
                        request.display_scale,
                    )?;
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::CategoryBars {
                            data: data_render::create_category_bar_data_bind_group(
                                &self.device,
                                &self.gpu_ledger,
                                &self.category_bar_data_bgl,
                                self.pool.buffer(),
                                &params,
                                params.members as usize,
                                |push| {
                                    category_bar_lanes(&display_config, group, cfg, lookup, push)
                                },
                                style_map,
                            )?,
                        },
                    });
                }
                continue;
            }

            if primitives.parallel {
                // The rows the parallel entries draw at the pick's display
                // transform, over the same lane and range tables (#141);
                // the colour is not read, so the neutral value map binds.
                let lookup = |id: &str| {
                    self.pool
                        .handle_for(id)
                        .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                };
                let data = build_parallel_data(
                    &self.device,
                    &self.gpu_ledger,
                    &self.parallel_bgl,
                    self.pool.buffer(),
                    &self.neutral_point_value_map_buffers,
                    &display_config,
                    cfg,
                    lookup,
                )?;
                if data.instances() != 0 {
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::Parallel { data },
                    });
                }
                continue;
            }

            if primitives.vector {
                // A grid vector picks the arrows it draws, as the cells they
                // are drawn for: the vector shader's own pick entry walks
                // them with the draw's group 2 at the pick's display scale.
                // A point vector is a point pick (`pick_chart`).
                self.ensure_vector_scratch(&config, cfg, request.display_scale)?;
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal("vector scratch was just ensured"))?;
                if scratch.input == data_render::vector::VectorInput::Grid && scratch.count != 0 {
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::VectorGrid {
                            vector_bg: scratch.vector_bg.clone(),
                            charge: Arc::clone(&scratch.charge),
                        },
                    });
                }
                continue;
            }
            if primitives.mesh || primitives.mesh_contour {
                // A triangle mesh picks the triangles and strokes it draws
                // (#210): the mesh entry walks them over the draw's group 2
                // at the pick's display scale.
                self.ensure_mesh_scratch(&config, cfg, request.display_scale)?;
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal("mesh scratch was just ensured"))?;
                if !scratch.drawable {
                    continue;
                }
                let [r, g, b, a] = scratch.signature.nan_color.map(f32::from_bits);
                let nan_color_premul =
                    PrimitiveStyle::from_color(Color { r, g, b, a }).color_premul;
                let contour_width_px = cfg
                    .tri_mesh()
                    .and_then(|mesh| mesh.contour.as_ref())
                    .map_or(0.0, |contour| contour.line.line_width.max(0.0))
                    * request.display_scale;
                data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    paint_order,
                    geometry: crate::gpu_data_pick::GpuDataPickGeometry::TriMesh {
                        field_bg: scratch.field_bg.clone(),
                        charge: Arc::clone(&scratch.charge),
                        has_fill: primitives.mesh,
                        has_contour: primitives.mesh_contour,
                        contour_width_px,
                        nan_color_premul,
                    },
                });
                continue;
            }
            if !(primitives.field || primitives.contour) {
                continue;
            }
            let has_fill = primitives.field;
            let has_contour = primitives.contour;
            let mut nan_color_premul = [0.0; 4];
            let (field_bg, charge, drawable) = if has_fill {
                self.ensure_field_scratch(&config, cfg)?;
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal("field scratch was just ensured"))?;
                // The fill's `nan_color`, premultiplied as its style uniform
                // holds it (`PrimitiveStyle::from_color`).
                let [r, g, b, a] = scratch.signature.nan_color.map(f32::from_bits);
                nan_color_premul = PrimitiveStyle::from_color(Color { r, g, b, a }).color_premul;
                (
                    scratch.field_bg.clone(),
                    Arc::clone(&scratch.charge),
                    scratch.drawable,
                )
            } else {
                self.ensure_contour_scratch(cfg, &config, request.display_scale)?;
                let scratch = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .and_then(ScratchEntry::contour)
                    .ok_or(FiggyError::internal("contour scratch was just ensured"))?;
                (
                    scratch.field_bg.clone(),
                    Arc::clone(&scratch.charge),
                    scratch.drawable,
                )
            };
            if !drawable {
                continue;
            }
            let contour_width_px = extract_contour(&cfg.render_type)
                .map_or(0.0, |contour| contour.line.line_width.max(0.0))
                * request.display_scale;
            // The label placement the contour draws its gaps from at this
            // display transform: the draw's own snapshot (reused after a frame
            // at the same transform, dispatched once otherwise).
            let label = if has_contour && series_wants_contour_labels(&cfg.render_type) {
                if has_fill {
                    self.ensure_contour_scratch(cfg, &config, request.display_scale)?;
                }
                self.preparation_parts().1.contour_pick_label(
                    cfg,
                    &display_config,
                    request.display_scale,
                )?
            } else {
                None
            };
            data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
                geometry: crate::gpu_data_pick::GpuDataPickGeometry::Field {
                    field_bg,
                    charge,
                    has_fill,
                    has_contour,
                    contour_width_px,
                    nan_color_premul,
                    label,
                },
            });
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
            let Some(labels) = self.preparation_parts().1.value_label_data(
                &display_config,
                cfg,
                &atlas,
                request.display_scale,
            )?
            else {
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
        let data_ticket = data_bundle.submit(
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
        )?;
        Ok(crate::gpu_data_pick::GpuDataPickTicket::new(
            point_ticket,
            data_ticket,
            point_orders,
            members,
            cpu,
        ))
    }
}
