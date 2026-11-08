//! Bar layers: category bars, box plots, range bars, violins and histograms.

use super::*;

impl PrepareContext<'_> {
    /// A category bar, box plot or range bar series' bars, over its
    /// panel's group of the same kind.
    pub(super) fn category_bar_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<data_render::ColumnCategoryBarLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            series_specs,
            pipelines,
            column_buffer,
            phase,
            cfg,
            rt,
            primitives,
            ..
        } = *s;
        let series = &s.series;
        // Category bars: the group is this panel's category bar series;
        // the params and the override table are resolved against the
        // panel's categorical axis on every prepare, so a reordered axis
        // or an edited override is drawn at once. No streamed form.
        let category_bars = if primitives.category_bars && phase.is_none() {
            let bars = rt.category_bars().ok_or(FiggyError::internal(
                "the category bar primitive has a category bar config",
            ))?;
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
            )?;
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
                pipelines.bar_category.as_ref().ok_or(FiggyError::internal(
                    "prepare ensured category bar pipeline",
                ))?,
            ))
        } else if primitives.box_plot && phase.is_none() {
            // Box plots: the group is this panel's box plot series; the
            // params read the record's and the outlier column's member
            // lanes, and the style map resolves the overrides against the
            // panel's categorical axis on every prepare. No streamed form.
            let boxes = rt.box_plot().ok_or(FiggyError::internal(
                "the box plot primitive has a box plot config",
            ))?;
            let (params, lanes, vertices) = box_plot_params(
                chart_config,
                series_specs
                    .iter()
                    .filter(|s| {
                        effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
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
            )?;
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
                    .ok_or(FiggyError::internal("prepare ensured box plot pipeline"))?,
            ))
        } else if primitives.range_bars && phase.is_none() {
            // Range bars: the group is this panel's range bar series; the
            // params read every column's lane and the style map holds the
            // part colours. No streamed form.
            let (data, count) = range_bar_data(
                &self.device,
                self.gpu_ledger,
                self.category_bar_data_bgl,
                self.per_point_style_map_bgl,
                column_buffer,
                chart_config,
                series_specs
                    .iter()
                    .filter(|s| {
                        effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
                            .range_bars
                    })
                    .map(|s| s.config),
                cfg,
                series.style.display_scale,
                &mut lookup,
            )?;
            Some((
                data,
                count,
                data_render::RANGE_BAR_VERTICES,
                pipelines
                    .bar_range
                    .as_ref()
                    .ok_or(FiggyError::internal("prepare ensured range bar pipeline"))?,
            ))
        } else {
            None
        };
        Ok(category_bars.map(|(data, count, vertices, pipeline)| {
            data_render::ColumnCategoryBarLayer {
                pipeline,
                transform_bg: &view.transform_bg,
                style_bg: &series.style.bar_bg,
                data,
                count,
                vertices,
            }
        }))
    }

    /// A box plot series' violins and raw points.
    pub(super) fn box_extra_layers<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<BoxExtraLayers<'a>> {
        let SeriesInputs {
            view,
            chart_config,
            series_specs,
            pipelines,
            column_buffer,
            phase,
            cfg,
            primitives,
            ..
        } = *s;
        let series = &s.series;
        // A box plot's violins and raw points: their own group 3 over the
        // box group's layout, under and over its boxes. No streamed form.
        let box_group = || {
            series_specs
                .iter()
                .filter(|s| {
                    effective_series_primitives(&chart_config.draw_style, &s.config.render_type)
                        .box_plot
                })
                .map(|s| s.config)
        };
        let mut box_extra = |points: bool, pipeline: Option<&'a wgpu::RenderPipeline>| {
            box_extras_data(
                &self.device,
                self.gpu_ledger,
                self.category_bar_data_bgl,
                self.per_point_style_map_bgl,
                column_buffer,
                self.pool,
                chart_config,
                box_group(),
                cfg,
                series.style.display_scale,
                &mut lookup,
                points,
            )
            .and_then(|(data, count)| {
                Ok(data_render::ColumnCategoryBarLayer {
                    pipeline: pipeline
                        .ok_or(FiggyError::internal("prepare ensured the violin pipelines"))?,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.bar_bg,
                    data,
                    count,
                    vertices: data_render::VIOLIN_VERTICES,
                })
            })
        };
        let violin = (primitives.violin && phase.is_none())
            .then(|| box_extra(false, pipelines.bar_violin.as_ref()))
            .transpose()?;
        let box_points = (primitives.box_points && phase.is_none())
            .then(|| box_extra(true, pipelines.bar_box_points.as_ref()))
            .transpose()?;
        Ok((violin, box_points))
    }

    /// A histogram series' bars, read along the bar's own orientation.
    pub(super) fn bar_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
    ) -> Result<Option<data_render::ColumnBarLayer<'a>>> {
        let SeriesInputs {
            view,
            pipelines,
            column_buffer,
            phase,
            rt,
            primitives,
            x_h,
            y_h,
            ..
        } = *s;
        let series = &s.series;
        // Bars: the edge column and the value column are picked from the
        // bar's own orientation — never from the column lengths (design
        // §B.3). `Vertical` reads x as edges, `Horizontal` reads y.
        let bar = if primitives.bar
            && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Histogram)
        {
            let bar_cfg = extract_bar(rt)
                .ok_or(FiggyError::internal("primitives.bar implies a bar config"))?;
            let (edges, values) = match bar_cfg.orientation {
                crate::data_config::BarOrientation::Vertical => (x_h, y_h),
                crate::data_config::BarOrientation::Horizontal => (y_h, x_h),
            };
            let style_map = series.style.bar_map.as_ref();
            Some(data_render::ColumnBarLayer {
                envelope: if phase.is_none() {
                    Some(
                        pipelines
                            .bar_envelope
                            .as_ref()
                            .ok_or(FiggyError::internal(
                                "prepare ensured histogram envelope pipeline",
                            ))?
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
                            )?,
                    )
                } else {
                    None
                },
                pipeline: match style_map {
                    None => pipelines
                        .bar
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured bar pipeline"))?,
                    Some(_) => pipelines
                        .bar_mapped
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured mapped bar pipeline"))?,
                },
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
        Ok(bar)
    }
}
