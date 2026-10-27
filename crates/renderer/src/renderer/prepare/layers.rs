//! Series layer building: build_series_layers and its per-kind arms.

use super::*;

impl PrepareContext<'_> {
    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `prepared` supplies prepare-time arc and star
    /// products without a live `arc_cache` read. `styled` is the panel's
    /// resolved [`StyleSet`] — it selects the stylized pipeline variants
    /// (and the line strip's vertex count); `None` (precise mode, or the
    /// theoretically-impossible cache miss) selects the precise pipelines.
    /// Everything else is identical between the modes.
    pub(in crate::renderer) fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
    ) -> Result<(Vec<data_render::SeriesLayers<'a>>, HashMap<ColumnId, u64>)> {
        let pool = &self.pool;
        let mut column_sources = HashMap::new();
        let mut lookup = |id: &str| -> Result<ColumnHandle> {
            let handle = pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            let allocation_epoch = pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            column_sources.entry(id.into()).or_insert(allocation_epoch);
            Ok(handle)
        };

        let layers = self.build_series_layers_with_columns(
            view,
            chart_config,
            series_specs,
            pipelines,
            products,
            styled,
            pool.buffer(),
            &mut lookup,
            None,
        )?;
        Ok((layers, column_sources))
    }

    /// Column locations and their lifetime authority are resolved by the caller;
    /// geometry and style interpretation remain shared with resident rendering.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn build_series_layers_with_columns<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
        column_buffer: &'a wgpu::Buffer,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
        phase: Option<streaming_runtime::StreamDrawPhase>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        // Resolve the style set into per-primitive picks once. `stars` is the
        // constellation's arc-driven indirect star pass over the same polyline.
        #[derive(Clone, Copy)]
        struct LinePick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            verts: u32,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        #[derive(Clone, Copy)]
        struct StarsPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: &'p wgpu::BindGroup,
        }
        #[derive(Clone, Copy)]
        struct ScatterPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                pipelines.line.as_ref().map(|pipeline| LinePick {
                    pipeline,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                pipelines.scatter.as_ref().map(|pipeline| ScatterPick {
                    pipeline,
                    texture_bg: None,
                }),
                pipelines.errorbar.as_ref(),
            ),
            Some(StyleSet::Sketch {
                line,
                scatter,
                errorbar,
                line_verts,
            }) => (
                Some(LinePick {
                    pipeline: line,
                    verts: *line_verts,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: scatter,
                    texture_bg: None,
                }),
                Some(errorbar),
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                Some(LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(StarsPick {
                    pipeline: &c.stars,
                    texture_bg: &c.star_tex_bg,
                }),
                Some(ScatterPick {
                    pipeline: &c.planets,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(&c.jets),
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                Some(LinePick {
                    pipeline: &c.line,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: &c.stars,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                None,
            ),
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let primitives = effective_series_primitives(&chart_config.draw_style, rt);
            let x_h = lookup(&cfg.x_column)?;
            // A box plot's `y_column` is its record, not a column: its
            // member columns are looked up with its params below.
            let y_h = if rt.box_plot().is_some() {
                x_h
            } else {
                lookup(&cfg.y_column)?
            };
            // A radar polygon has one vertex per category: its columns are
            // data, so their lengths are checked here against the pool.
            if rt.radar().is_some() {
                radar_columns_fit(chart_config, cfg, x_h.len_values, y_h.len_values)?;
            }

            // A field: the group-2 state was resolved in `ensure_field_scratches`
            // (the prepare phase's `&mut self` half) and is reused whole while
            // nothing it depends on changes.
            let field = if primitives.field {
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this field's scratch");
                Some(data_render::ColumnFieldLayer {
                    pipeline: pipelines
                        .field
                        .as_ref()
                        .expect("prepare ensured field pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                })
            } else {
                None
            };

            // Contour lines: the same quad the field draws, through
            // `fs_contour`. The group-2 tables were resolved in
            // `ensure_contour_scratches`; the isolines come out of the grid
            // itself, so there is nothing per-level about the draw call.
            let contour = if primitives.contour {
                let scratch = products
                    .contours
                    .get(idx)
                    .and_then(|series| series.scratch.as_ref())
                    .expect("prepare captured this contour occurrence");
                let label = scratch
                    .label
                    .as_ref()
                    .zip(pipelines.contour_label.as_ref())
                    .map(|(snapshot, pipeline)| data_render::ColumnContourLabel {
                        pipeline,
                        snapshot: snapshot.clone(),
                    });
                let pipeline = if label.is_some() {
                    pipelines
                        .contour_labelled
                        .as_ref()
                        .expect("prepare ensured labelled contour pipeline")
                } else {
                    pipelines
                        .contour
                        .as_ref()
                        .expect("prepare ensured contour pipeline")
                };
                Some(data_render::ColumnContourLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    field_bg: scratch.field_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    drawable: scratch.drawable,
                    // The label pipeline is compiled only when some series wants
                    // labels, so a `None` here means this series' labels are not
                    // drawable this frame — never that the state is missing.
                    label,
                })
            } else {
                None
            };

            // Arrows: the group-2 state was resolved in `ensure_vector_scratches`
            // and is reused whole while nothing it depends on changes.
            let vector = if primitives.vector && phase.is_none() {
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this vector's scratch");
                let pipeline = match scratch.input {
                    data_render::vector::VectorInput::Points => pipelines.vector_points.as_ref(),
                    data_render::vector::VectorInput::Grid => pipelines.vector_grid.as_ref(),
                    data_render::vector::VectorInput::Path => pipelines.vector_path.as_ref(),
                }
                .expect("prepare ensured vector pipeline");
                Some(data_render::vector::ColumnVectorLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    vector_bg: scratch.vector_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    count: scratch.count,
                    indirect: None,
                })
            } else if primitives.line
                && phase.is_none()
                && rt
                    .streamlines()
                    .is_some_and(|streamlines| streamlines.arrows.is_some())
            {
                // Streamline arrowheads: the vector glyph on the arc slots of
                // the line's arc scan, drawn indirect in the vector slot,
                // under the stroke (#51, D5). A series too short to scan has
                // no slots and draws no heads.
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .expect("prepare ensured this streamline's arrowhead scratch");
                products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .and_then(|arc| arc.star.as_ref())
                    .map(|slots| data_render::vector::ColumnVectorLayer {
                        pipeline: pipelines
                            .vector_path
                            .as_ref()
                            .expect("prepare ensured the path vector pipeline"),
                        transform_bg: &view.transform_bg,
                        style_bg: scratch.style_bg.clone(),
                        vector_bg: scratch.vector_bg.clone(),
                        charge: Arc::clone(&scratch.charge),
                        count: scratch.count,
                        indirect: Some((&slots.indirect, &slots.vs_bg)),
                    })
            } else {
                None
            };

            // Category bars: the group is this panel's category bar series;
            // the params and the override table are resolved against the
            // panel's categorical axis on every prepare, so a reordered axis
            // or an edited override is drawn at once. No streamed form.
            let category_bars = if primitives.category_bars && phase.is_none() {
                let bars = rt
                    .category_bars()
                    .expect("the category bar primitive has a category bar config");
                let group = series_specs
                    .iter()
                    .filter(|s| {
                        effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
                            .category_bars
                    })
                    .map(|s| s.config);
                let params = category_bar_params(
                    chart_config,
                    group.clone(),
                    cfg,
                    series.style.display_scale,
                    &mut lookup,
                    |id| self.pool.slot(id).map(|slot| slot.min),
                )?;
                let style_map = crate::renderer::style::category_bar_style_map(
                    &self.device,
                    self.per_point_style_map_bgl,
                    category_bar_categories(chart_config).unwrap_or(&[]),
                    bars,
                    series.style.display_scale,
                );
                let data = data_render::create_category_bar_data_bind_group(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    column_buffer,
                    &params,
                    params.members as usize,
                    |push| category_bar_lanes(chart_config, group, cfg, &mut lookup, push),
                    style_map,
                )?;
                Some((
                    data,
                    params.count,
                    data_render::CATEGORY_BAR_VERTICES,
                    pipelines
                        .bar_category
                        .as_ref()
                        .expect("prepare ensured category bar pipeline"),
                ))
            } else if primitives.box_plot && phase.is_none() {
                // Box plots: the group is this panel's box plot series; the
                // params read the record's and the outlier column's member
                // lanes, and the style map resolves the overrides against the
                // panel's categorical axis on every prepare. No streamed form.
                let boxes = rt
                    .box_plot()
                    .expect("the box plot primitive has a box plot config");
                let (params, lanes, vertices) = box_plot_params(
                    chart_config,
                    series_specs
                        .iter()
                        .filter(|s| {
                            effective_series_primitives(
                                &chart_config.draw_style,
                                &s.config.render_type,
                            )
                            .box_plot
                        })
                        .map(|s| s.config),
                    cfg,
                    series.style.display_scale,
                    self.pool,
                    &mut lookup,
                )?;
                let style_map = crate::renderer::style::box_plot_style_map(
                    &self.device,
                    self.per_point_style_map_bgl,
                    category_bar_categories(chart_config).unwrap_or(&[]),
                    boxes,
                    series.style.display_scale,
                );
                let data = data_render::create_category_bar_data_bind_group(
                    &self.device,
                    self.gpu_ledger,
                    self.category_bar_data_bgl,
                    column_buffer,
                    &params,
                    lanes.len(),
                    |push| {
                        lanes.iter().for_each(|&lane| push(lane));
                        Ok(())
                    },
                    style_map,
                )?;
                Some((
                    data,
                    params.count,
                    vertices,
                    pipelines
                        .bar_box_plot
                        .as_ref()
                        .expect("prepare ensured box plot pipeline"),
                ))
            } else {
                None
            };

            // Bars: the edge column and the value column are picked from the
            // bar's own orientation — never from the column lengths (design
            // §B.3). `Vertical` reads x as edges, `Horizontal` reads y.
            let bar = if primitives.bar
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Histogram)
            {
                let bar_cfg = extract_bar(rt).expect("primitives.bar implies a bar config");
                let (edges, values) = match bar_cfg.orientation {
                    crate::data_config::BarOrientation::Vertical => (x_h, y_h),
                    crate::data_config::BarOrientation::Horizontal => (y_h, x_h),
                };
                let style_map = series.style.bar_map.as_ref();
                Some(data_render::ColumnBarLayer {
                    envelope: phase.is_none().then(|| {
                        pipelines
                            .bar_envelope
                            .as_ref()
                            .expect("prepare ensured histogram envelope pipeline")
                            .prepare(
                                &self.device,
                                &self.queue,
                                &self.gpu_ledger,
                                column_buffer,
                                edges,
                                values,
                                match bar_cfg.orientation {
                                    crate::data_config::BarOrientation::Vertical => {
                                        view.panel_rect.width
                                    }
                                    crate::data_config::BarOrientation::Horizontal => {
                                        view.panel_rect.height
                                    }
                                },
                                &view.transform_bg,
                                &series.style.bar_bg,
                                style_map.map(|map| &map.bind_group),
                            )
                    }),
                    pipeline: style_map.map_or_else(
                        || {
                            pipelines
                                .bar
                                .as_ref()
                                .expect("prepare ensured bar pipeline")
                        },
                        |_| {
                            pipelines
                                .bar_mapped
                                .as_ref()
                                .expect("prepare ensured mapped bar pipeline")
                        },
                    ),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    style_map_bg: style_map.map(|map| &map.bind_group),
                    pool_buffer: column_buffer,
                    edges,
                    values,
                })
            } else {
                None
            };

            let interpolation =
                extract_line(rt).map_or(LineInterpolation::Linear, |line| line.interpolation);
            // A radar polygon closes its outline, and its fill on a full
            // circle (in a sector the closing trapezoid would cover the
            // others).
            let closed = rt.radar().is_some();
            let closed_fill = closed && polar_fill_closes(chart_config);
            let line = if primitives.line
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Line)
            {
                let line_pick = line_pick.expect("prepare ensured line pipeline");
                let arc = products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .map(|arc| arc.prefix.clone());
                // A streamline series' colour column: its stroke draws through
                // the mapped line pipeline with the value map
                // `ensure_value_map_scratch` resolved (precise only, never
                // streamed).
                let value_map = match rt.streamlines().and_then(|s| s.color_map) {
                    Some(map) if styled.is_none() && phase.is_none() => {
                        let scratch = self
                            .value_map_cache
                            .get(cfg.series_id.as_str())
                            .expect("prepare ensured this streamline colour map's value map");
                        Some(data_render::LineValueMap {
                            column: lookup(&map.column)?,
                            neutral_bg: self.neutral_point_value_map_bg,
                            value_map_bg: &scratch.bind_group,
                        })
                    }
                    _ => None,
                };
                Some(ColumnLineLayer {
                    pipeline: if value_map.is_some() {
                        pipelines
                            .line_mapped
                            .as_ref()
                            .expect("prepare ensured mapped line pipeline")
                    } else {
                        line_pick.pipeline
                    },
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    arc,
                    // One strip per step piece (or polar arc piece), joined
                    // per `strip_slot`.
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        line_pick.verts,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                    texture_bg: line_pick.texture_bg,
                    closed,
                    value_map,
                })
            } else {
                None
            };

            // The area fill under / beside the line. Resident only: the fill
            // between two series needs index-aligned columns that no streamed
            // chunking can guarantee, and `validate_stream_series` has already
            // rejected a streamed fill before this point. The second y column
            // is the referenced series' own, resolved from the same series
            // list; `validate_line_fill` checked the reference and its x
            // column before any GPU work.
            let area = if line.is_some()
                && phase.is_none()
                && let Some(line_config) = extract_line(rt)
                && let Some(fill) = line_config.fill.as_ref()
            {
                let base = match fill {
                    LineFill::ToBaseline(_) => y_h,
                    LineFill::ToSeries(target) => {
                        let target = series_specs
                            .iter()
                            .map(|other| other.config)
                            .find(|other| other.series_id == *target)
                            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill references series `{target}`, which is not in \
                                     this chart"
                                ),
                            })?;
                        if target.x_column != cfg.x_column {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "line fill to series `{}` requires the same x column: \
                                     `{}` vs `{}`",
                                    target.series_id, cfg.x_column, target.x_column
                                ),
                            });
                        }
                        lookup(&target.y_column)?
                    }
                };
                Some(data_render::ColumnAreaLayer {
                    pipeline: pipelines
                        .area
                        .as_ref()
                        .expect("prepare ensured line area pipeline"),
                    transform_bg: &view.transform_bg,
                    style_bg: series.style.area_bg.as_ref().ok_or_else(|| {
                        FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            reason: "series style was created without its line fill; \
                                     recreate the style for the current series config"
                                .into(),
                        }
                    })?,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    base,
                    verts_per_instance: crate::line_step::strip_vertex_count(
                        data_render::LINE_PRECISE_VERTICES_PER_INSTANCE,
                        crate::line_step::mode_word(
                            interpolation,
                            crate::line_step::arc_pieces(chart_config.polar.as_ref()),
                        ),
                    ),
                    closed: closed_fill,
                })
            } else {
                None
            };

            // Star pass: per-series GPU state is the owned snapshot the
            // prepare phase cloned out of the arc scratch (indirect args +
            // VS bind group) — read from the token, never from `arc_cache`,
            // so an eviction/rebuild between prepare and paint cannot drop
            // or desync it. A missing star (n < 2, or a
            // theoretically-impossible prepare miss) just skips the stars —
            // the ribbon still draws.
            let line_extra = match (&line, &stars_pick) {
                (Some(_), Some(sp)) => products
                    .arcs
                    .get(idx)
                    .and_then(|series| series.arc.as_ref())
                    .and_then(|a| a.star.as_ref())
                    .map(|star| data_render::ColumnStarLayer {
                        pipeline: sp.pipeline,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.line_bg,
                        texture_bg: sp.texture_bg,
                        star_bg: &star.vs_bg,
                        indirect: &star.indirect,
                    }),
                _ => None,
            };

            let scatter = if primitives.scatter
                && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Scatter)
            {
                let precise_style_map = if styled.is_none() {
                    series.style.scatter_map.as_ref()
                } else {
                    None
                };
                let style_index = match (precise_style_map, extract_scatter(rt).as_deref()) {
                    (Some(map), Some(sc)) if map.has_index => {
                        let Some(column) = sc.point_style_index_column.as_ref() else {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: "scatter style map expects an index column".into(),
                            });
                        };
                        let h = lookup(column)?;
                        let count = x_h.len_values.min(y_h.len_values);
                        if h.len_values < count {
                            return Err(FiggyError::InvalidSeriesConfig {
                                series_id: cfg.series_id.clone(),
                                reason: format!(
                                    "style index column {column:?} has {} values, but scatter uses {count}",
                                    h.len_values
                                ),
                            });
                        }
                        Some(h)
                    }
                    _ => None,
                };
                let (color_value, size_value, value_map_bg) = self.scatter_value_map_columns(
                    cfg,
                    precise_style_map.is_some(),
                    x_h.len_values.min(y_h.len_values),
                    &mut lookup,
                )?;
                let scatter_pick = scatter_pick.expect("prepare ensured scatter pipeline");
                Some(ColumnScatterLayer {
                    pipeline: precise_style_map.map_or(scatter_pick.pipeline, |_| {
                        pipelines
                            .scatter_mapped
                            .as_ref()
                            .expect("prepare ensured mapped scatter pipeline")
                    }),
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    style_map_bg: precise_style_map.map(|m| &m.bind_group),
                    value_map_bg,
                    quad_vb: &self.quad_vb,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    style_index,
                    color_value,
                    size_value,
                    texture_bg: scatter_pick.texture_bg,
                })
            } else {
                None
            };

            let errorbar = if constellation_only
                || phase.is_some_and(|phase| phase != streaming_runtime::StreamDrawPhase::Errorbar)
            {
                None
            } else {
                match (extract_err_y(rt), extract_err_x(rt)) {
                    (None, None) => None,
                    (ey_opt, ex_opt) => {
                        let (ey_lo, ey_hi) = match ey_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            // The direction flag disables these attributes
                            // before the shader reads them. Reuse the anchor
                            // column so the bound slice still covers every draw.
                            None => (y_h, y_h),
                        };
                        let (ex_lo, ex_hi) = match ex_opt {
                            Some(ErrorRef::Symmetric { column }) => {
                                let h = lookup(column)?;
                                (h, h)
                            }
                            Some(ErrorRef::Asymmetric { lower, upper }) => {
                                (lookup(lower)?, lookup(upper)?)
                            }
                            None => (x_h, x_h),
                        };
                        let precise_errorbar_style_map = if styled.is_none() {
                            series.style.errorbar_map.as_ref()
                        } else {
                            None
                        };
                        let errorbar_style_index = match (
                            precise_errorbar_style_map,
                            extract_errorbar_style(rt),
                        ) {
                            (Some(map), Some(eb)) if map.has_index => {
                                let Some(column) = eb.error_bar_style_index_column.as_ref() else {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: "errorbar style map expects an index column".into(),
                                    });
                                };
                                let h = lookup(column)?;
                                let count = [
                                    x_h.len_values,
                                    y_h.len_values,
                                    ey_lo.len_values,
                                    ey_hi.len_values,
                                    ex_lo.len_values,
                                    ex_hi.len_values,
                                ]
                                .into_iter()
                                .min()
                                .unwrap_or(0);
                                if h.len_values < count {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        reason: format!(
                                            "errorbar style index column {column:?} has {} values, but errorbar uses {count}",
                                            h.len_values
                                        ),
                                    });
                                }
                                Some(h)
                            }
                            _ => None,
                        };
                        Some(ColumnErrorBarDraw {
                            pipeline: precise_errorbar_style_map.map_or(
                                errorbar_pipe.expect("prepare ensured errorbar pipeline"),
                                |_| {
                                    pipelines
                                        .errorbar_mapped
                                        .as_ref()
                                        .expect("prepare ensured mapped errorbar pipeline")
                                },
                            ),
                            transform_bg: &view.transform_bg,
                            style_bg: &series.style.errorbar_bg,
                            style_map_bg: precise_errorbar_style_map.map(|m| &m.bind_group),
                            pool_buffer: column_buffer,
                            x: x_h,
                            y: y_h,
                            err_y_lo: ey_lo,
                            err_y_hi: ey_hi,
                            err_x_lo: ex_lo,
                            err_x_hi: ex_hi,
                            style_index: errorbar_style_index,
                        })
                    }
                }
            };

            let selection_prepare::SelectionLayers {
                picked,
                selected_bars,
                selected_fields,
            } = if phase.is_some() {
                selection_prepare::SelectionLayers {
                    picked: Vec::new(),
                    selected_bars: Vec::new(),
                    selected_fields: Vec::new(),
                }
            } else {
                self.build_selection_layers(
                    view,
                    chart_config,
                    series,
                    pipelines,
                    selection_prepare::SelectionColumns {
                        buffer: column_buffer,
                        x: x_h,
                        y: y_h,
                        bar: bar.as_ref().map(|layer| (layer.edges, layer.values)),
                        category: category_bars.as_ref().map(|(data, ..)| data.clone()),
                        rows: selection_prepare::SelectionRows::RESIDENT,
                        mapped_point_bg: None,
                    },
                    products
                        .contours
                        .get(idx)
                        .and_then(|series| series.scratch.as_ref()),
                    styled.is_none(),
                    &mut lookup,
                )?
            };

            // Value labels: the atlas this prepare resolved for the series,
            // placed over the series' own category bar group 3 (a bar or
            // box series') or its category and value columns (a radar
            // series'), with its uniform at the style's display scale. Only
            // a series that draws its bars, boxes or polygon draws labels.
            let atlas = products
                .contours
                .get(idx)
                .and_then(|series| series.value_labels.as_ref());
            let value_labels = match atlas {
                Some(atlas) if phase.is_none() => {
                    let data = match &category_bars {
                        Some((data, ..)) => Some(data.clone()),
                        None if rt.radar().is_some() && primitives.line => {
                            Some(self.radar_label_group(
                                cfg,
                                column_buffer,
                                series.style.display_scale,
                                &mut lookup,
                            )?)
                        }
                        None => None,
                    };
                    match (
                        data,
                        self.value_label_data(chart_config, cfg, atlas, series.style.display_scale),
                        pipelines.bar_value_label.as_ref(),
                    ) {
                        (Some(data), Some(labels), Some(pipeline)) => {
                            Some(data_render::ColumnValueLabelLayer {
                                pipeline,
                                transform_bg: &view.transform_bg,
                                labels,
                                data,
                                selected: selected_value_label(chart_config, cfg)
                                    .zip(pipelines.bar_value_label_selection.as_ref()),
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };

            let category_bars = category_bars.map(|(data, count, vertices, pipeline)| {
                data_render::ColumnCategoryBarLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    data,
                    count,
                    vertices,
                }
            });
            out.push(data_render::SeriesLayers {
                area,
                field,
                bar,
                category_bars,
                contour,
                vector,
                errorbar,
                line,
                line_extra,
                scatter,
                selected_bars,
                selected_fields,
                picked,
                value_labels,
            });
        }
        Ok(out)
    }
}
