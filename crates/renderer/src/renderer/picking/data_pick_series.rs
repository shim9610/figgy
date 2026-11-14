//! Typed data pick entries of each series kind.

use super::*;

impl Renderer {
    /// A histogram series' bins for the typed data pick, at the pick's display transform.
    pub(super) fn histogram_data_pick(
        &mut self,
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
        let bar = extract_bar(&cfg.render_type).ok_or(FiggyError::internal(
            "effective histogram primitive has a bar config",
        ))?;
        let (edges_id, values_id, horizontal) = match bar.orientation {
            crate::data_config::BarOrientation::Vertical => (&cfg.x_column, &cfg.y_column, false),
            crate::data_config::BarOrientation::Horizontal => (&cfg.y_column, &cfg.x_column, true),
        };
        let edges = self
            .pool
            .handle_for(edges_id)
            .ok_or_else(|| FiggyError::UnknownColumn {
                id: edges_id.clone(),
            })?;
        let values = self
            .pool
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
            let (baseline_hi, baseline_lo) = crate::data::split_f64_to_f32_pair(bar.baseline);
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
        Ok(())
    }

    /// A box plot series' boxes for the typed data pick, at the pick's display transform.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn box_plot_data_pick(
        &mut self,
        config: &Config,
        display_config: &Config,
        series: &[SeriesConfig],
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        primitives: EffectiveSeriesPrimitives,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
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
        Ok(())
    }

    /// A range bar series' bars for the typed data pick, at the pick's display transform.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn range_bar_data_pick(
        &mut self,
        config: &Config,
        display_config: &Config,
        series: &[SeriesConfig],
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
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
        Ok(())
    }

    /// A category bar series' bars for the typed data pick, at the pick's display transform.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn category_bar_data_pick(
        &mut self,
        config: &Config,
        display_config: &Config,
        series: &[SeriesConfig],
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
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
                        |push| category_bar_lanes(&display_config, group, cfg, lookup, push),
                        style_map,
                    )?,
                },
            });
        }
        Ok(())
    }

    /// A parallel coordinates series' rows for the typed data pick, at the pick's display transform.
    pub(super) fn parallel_data_pick(
        &mut self,
        display_config: &Config,
        cfg: &SeriesConfig,
        paint_order: u32,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
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
        Ok(())
    }

    /// A vector series' arrows for the typed data pick, at the pick's display transform.
    pub(super) fn vector_data_pick(
        &mut self,
        config: &Config,
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
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
        Ok(())
    }

    /// A triangle mesh series' triangles and contours for the typed data pick, at the pick's display transform.
    pub(super) fn mesh_data_pick(
        &mut self,
        config: &Config,
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        primitives: EffectiveSeriesPrimitives,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
        // A triangle mesh picks the triangles and strokes it draws
        // (#210): the mesh entry walks them over the draw's group 2
        // at the pick's display scale.
        self.ensure_mesh_scratch(&config, cfg, request.display_scale)?;
        let scratch = self
            .field_cache
            .get(cfg.series_id.as_str())
            .ok_or(FiggyError::internal("mesh scratch was just ensured"))?;
        if !scratch.drawable {
            return Ok(());
        }
        let [r, g, b, a] = scratch.signature.nan_color.map(f32::from_bits);
        let nan_color_premul = PrimitiveStyle::from_color(Color { r, g, b, a }).color_premul;
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
        Ok(())
    }

    /// A field or contour series' cells and levels for the typed data pick, at the pick's display transform.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn field_data_pick(
        &mut self,
        config: &Config,
        display_config: &Config,
        request: GpuPickRequest,
        cfg: &SeriesConfig,
        paint_order: u32,
        primitives: EffectiveSeriesPrimitives,
        data_series: &mut Vec<crate::gpu_data_pick::GpuDataPickSeries>,
    ) -> Result<()> {
        if !(primitives.field || primitives.contour) {
            return Ok(());
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
            return Ok(());
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
        Ok(())
    }
}
