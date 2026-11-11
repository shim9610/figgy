//! Per-series extras: selection highlights, value labels and parallel rows.

use super::*;

impl PrepareContext<'_> {
    /// A series' selection highlights: its picked rings, bars and fields.
    pub(super) fn selection_layers<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        bar: Option<&data_render::ColumnBarLayer<'a>>,
        category_bars: Option<&data_render::ColumnCategoryBarLayer<'a>>,
        violin: Option<&data_render::ColumnCategoryBarLayer<'a>>,
        box_points: Option<&data_render::ColumnCategoryBarLayer<'a>>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<selection_prepare::SelectionLayers<'a>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            products,
            styled,
            column_buffer,
            phase,
            idx,
            primitives,
            x_h,
            y_h,
            z_3d,
            ..
        } = *s;
        let series = &s.series;
        let camera_binding = |mapped: bool| self.camera_binding(s, mapped);
        let layers = if phase.is_some() {
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
                    bar: bar.map(|layer| (layer.edges, layer.values)),
                    category: category_bars.map(|layer| layer.data.clone()),
                    violin: violin.map(|layer| (layer.data.clone(), layer.count)),
                    box_points: box_points.map(|layer| layer.data.clone()),
                    rows: selection_prepare::SelectionRows::RESIDENT,
                    mapped_point_bg: None,
                    view3d: z_3d
                        .filter(|_| primitives.scatter || primitives.vector)
                        .map(|z| {
                            camera_binding(styled.is_none() && series.style.scatter_map.is_some())
                                .map(|binding| (z, binding))
                        })
                        .transpose()?,
                },
                products
                    .contours
                    .get(idx)
                    .and_then(|series| series.scratch.as_ref()),
                styled.is_none(),
                &mut lookup,
            )?
        };
        Ok(layers)
    }

    /// A bar, box or radar series' value labels over its own geometry.
    pub(super) fn value_label_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        category_bars: Option<&data_render::ColumnCategoryBarLayer<'a>>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<data_render::ColumnValueLabelLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            products,
            column_buffer,
            phase,
            idx,
            cfg,
            rt,
            primitives,
            ..
        } = *s;
        let series = &s.series;
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
                let data = match category_bars {
                    Some(layer) => Some(layer.data.clone()),
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
                // A radar series' lane among the panel's labelled radar
                // series, from their atlases' widest strings (#272).
                let lanes = radar_label_lanes(
                    chart_config,
                    s.series_specs.iter().map(|spec| spec.config),
                    cfg,
                    |index, _| {
                        Ok(products
                            .contours
                            .get(index)
                            .and_then(|series| series.value_labels.as_ref())
                            .map_or(0.0, |atlas| atlas.widest()))
                    },
                )?;
                match (
                    data,
                    self.value_label_data(
                        chart_config,
                        cfg,
                        atlas,
                        series.style.display_scale,
                        &lanes,
                    )?,
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
        Ok(value_labels)
    }

    /// A parallel coordinates series' rows and its picked row.
    pub(super) fn parallel_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<data_render::ColumnParallelLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            column_buffer,
            phase,
            cfg,
            rt,
            primitives,
            ..
        } = *s;
        let series = &s.series;
        // A parallel coordinates series' rows (#141): the axis lane and
        // range tables over the chart's parallel axes, the rows coloured
        // through the value map `ensure_value_map_scratch` resolved when
        // a colour column is named. Resident and precise only; the
        // picked row (a point selection of this series) is redrawn in
        // the selection's style over every body.
        let parallel = if primitives.parallel && phase.is_none() {
            let value_map = match rt.parallel().and_then(|p| p.color_column) {
                Some(_) => {
                    &self
                        .value_map_cache
                        .get(cfg.series_id.as_str())
                        .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            kind: crate::RuleKind::ParallelValueMapMissing,
                        })?
                        .buffers
                }
                None => self.neutral_point_value_map_buffers,
            };
            let data = build_parallel_data(
                &self.device,
                self.gpu_ledger,
                self.parallel_bgl,
                column_buffer,
                value_map,
                chart_config,
                cfg,
                &mut lookup,
            )?;
            let selected = selection_prepare::point_selection_visuals(
                chart_config,
                cfg,
                selection_prepare::SelectionRows::RESIDENT.refs,
            )
            .into_iter()
            .next()
            .zip(pipelines.line_parallel_selection.as_ref())
            .and_then(|(visual, pipeline)| {
                let row = u32::try_from(visual.point_index).ok()?;
                let style = PrimitiveStyle::from_color_with_width(
                    visual.color,
                    visual.width_px * series.style.display_scale,
                );
                let buffer = data_render::create_style_uniform_buffer(&self.device, &style);
                Some(data_render::ParallelSelection {
                    pipeline,
                    style_bg: data_render::create_style_bind_group(
                        &self.device,
                        self.style_bgl,
                        &buffer,
                    ),
                    row,
                })
            });
            Some(data_render::ColumnParallelLayer {
                pipeline: pipelines
                    .line_parallel
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured the parallel pipeline",
                    ))?,
                transform_bg: &view.transform_bg,
                style_bg: &series.style.line_bg,
                data,
                selected,
            })
        } else {
            None
        };
        Ok(parallel)
    }
}
