//! Line layers: strokes, area fills, ternary regions and star passes.

use super::*;

impl PrepareContext<'_> {
    /// A series' stroke: the precise or styled strip, a streamline's
    /// mapped stroke, a ternary or 3D stroke.
    pub(super) fn line_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<ColumnLineLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            products,
            styled,
            column_buffer,
            phase,
            picks,
            idx,
            cfg,
            rt,
            primitives,
            x_h,
            y_h,
            z_3d,
            ..
        } = *s;
        let series = &s.series;
        let line_pick = picks.line;
        let camera_binding = |mapped: bool| self.camera_binding(s, mapped);
        let interpolation =
            extract_line(rt).map_or(LineInterpolation::Linear, |line| line.interpolation);
        // A radar polygon closes its outline, and its fill on a full
        // circle (in a sector the closing trapezoid would cover the
        // others).
        let closed = rt.radar().is_some();
        let line = if primitives.line
            && phase.is_none_or(|phase| phase == streaming_runtime::StreamDrawPhase::Line)
        {
            let line_pick =
                line_pick.ok_or(FiggyError::internal("prepare ensured line pipeline"))?;
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
                    let scratch = self.value_map_cache.get(cfg.series_id.as_str()).ok_or(
                        FiggyError::internal(
                            "prepare ensured this streamline colour map's value map",
                        ),
                    )?;
                    Some(data_render::LineValueMap {
                        column: lookup(&map.column)?,
                        neutral_bg: self.neutral_point_value_map_bg,
                        value_map_bg: &scratch.bind_group,
                    })
                }
                _ => None,
            };
            // A ternary series' stroke: the ternary entry with its third
            // component (never mapped, never styled).
            let ternary_c = rt.ternary_c_column().map(&mut lookup).transpose()?;
            // A 3D series' stroke: the 3D entry with its height and the
            // panel's camera (#132).
            let line_3d = z_3d
                .map(|z| camera_binding(false).map(|binding| (z, binding)))
                .transpose()?;
            Some(ColumnLineLayer {
                pipeline: if line_3d.is_some() {
                    pipelines.view3d[View3dPipeline::Line as usize]
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured the 3D line pipeline"))?
                } else if value_map.is_some() {
                    pipelines
                        .line_mapped
                        .as_ref()
                        .ok_or(FiggyError::internal("prepare ensured mapped line pipeline"))?
                } else if ternary_c.is_some() {
                    pipelines.ternary[TernaryPipeline::Line as usize]
                        .as_ref()
                        .ok_or(FiggyError::internal(
                            "prepare ensured the ternary line pipeline",
                        ))?
                } else {
                    line_pick.pipeline
                },
                third: line_3d.as_ref().map(|(z, _)| *z).or(ternary_c),
                view3d: line_3d.map(|(_, binding)| binding),
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
                        crate::line_step::arc_pieces(chart_config.polar()),
                    ),
                ),
                texture_bg: line_pick.texture_bg,
                closed,
                value_map,
            })
        } else {
            None
        };
        Ok(line)
    }

    /// The area fill under or beside a drawn line.
    pub(super) fn area_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        line_drawn: bool,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<data_render::ColumnAreaLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            series_specs,
            pipelines,
            column_buffer,
            phase,
            cfg,
            rt,
            x_h,
            y_h,
            ..
        } = *s;
        let series = &s.series;
        let interpolation =
            extract_line(rt).map_or(LineInterpolation::Linear, |line| line.interpolation);
        let closed_fill = rt.radar().is_some() && polar_fill_closes(chart_config);
        // The area fill under / beside the line. Resident only: the fill
        // between two series needs index-aligned columns that no streamed
        // chunking can guarantee, and `validate_stream_series` has already
        // rejected a streamed fill before this point. The second y column
        // is the referenced series' own, resolved from the same series
        // list; `validate_line_fill` checked the reference and its x
        // column before any GPU work.
        let area = if line_drawn
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
                            kind: crate::RuleKind::LineFillTargetMissing {
                                target: target.to_string(),
                            },
                        })?;
                    if target.x_column != cfg.x_column {
                        return Err(FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            kind: crate::RuleKind::LineFillXColumn {
                                target: target.series_id.clone(),
                                x_column: cfg.x_column.clone(),
                                target_x_column: target.x_column.clone(),
                            },
                        });
                    }
                    lookup(&target.y_column)?
                }
            };
            Some(data_render::ColumnAreaLayer {
                pipeline: pipelines
                    .area
                    .as_ref()
                    .ok_or(FiggyError::internal("prepare ensured line area pipeline"))?,
                transform_bg: &view.transform_bg,
                style_bg: series.style.area_bg.as_ref().ok_or_else(|| {
                    FiggyError::InvalidSeriesConfig {
                        series_id: cfg.series_id.clone(),
                        kind: crate::RuleKind::LineFillStyleMissing,
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
                        crate::line_step::arc_pieces(chart_config.polar()),
                    ),
                ),
                closed: closed_fill,
            })
        } else {
            None
        };
        Ok(area)
    }

    /// A ternary series' region fill and its ring-start scan.
    pub(super) fn region_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<data_render::ternary_region::ColumnTernaryRegionLayer<'a>>> {
        let SeriesInputs {
            view,
            chart_config,
            pipelines,
            column_buffer,
            phase,
            cfg,
            rt,
            x_h,
            y_h,
            ..
        } = *s;
        // A ternary series' region fill: the closed polygons its samples
        // outline, drawn into the composite's stencil attachment before
        // every series' fill and body. Resident only (streamed ternary
        // sources are rejected).
        let region = match (rt.ternary_region(), rt.ternary_c_column()) {
            (Some(style), Some(c)) if chart_config.ternary().is_some() && phase.is_none() => {
                let c_h = lookup(c)?;
                let params =
                    ternary_region_params(&cfg.series_id, style.color_premul(), x_h, y_h, c_h)?;
                // Each ring's first row, scanned before the draw reads it
                // for the closing edges (#49).
                let scan = pipelines
                    .ternary_region_scan
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured the ternary region pipelines",
                    ))?;
                let (ring_starts, ring_charge) = data_render::ternary_region::scan_ring_starts(
                    &self.device,
                    &self.queue,
                    &self.gpu_ledger,
                    scan,
                    column_buffer,
                    &params,
                    self.caps.max_compute_workgroups_per_dimension,
                )?
                .ok_or_else(|| ring_scan_too_large(&cfg.series_id))?;
                let (region_bg, charge) =
                    data_render::ternary_region::create_ternary_region_bind_group(
                        &self.gpu_ledger,
                        &self.device,
                        &self.region_bgl,
                        column_buffer,
                        &params,
                        &ring_starts,
                    )?;
                let pipeline =
                    |pass: data_render::ternary_region::TernaryRegionPass| {
                        pipelines.ternary_region[pass as usize].as_ref().ok_or(
                            FiggyError::internal("prepare ensured the ternary region pipelines"),
                        )
                    };
                Some(data_render::ternary_region::ColumnTernaryRegionLayer {
                    stencil: pipeline(data_render::ternary_region::TernaryRegionPass::Stencil)?,
                    cover: pipeline(data_render::ternary_region::TernaryRegionPass::Cover)?,
                    transform_bg: &view.transform_bg,
                    region_bg,
                    count: params.count,
                    charge,
                    ring_charge,
                })
            }
            _ => None,
        };
        Ok(region)
    }

    /// A Milkyway line's star pass over its arc scan.
    pub(super) fn line_extra_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        line_drawn: bool,
    ) -> Result<Option<data_render::ColumnStarLayer<'a>>> {
        let SeriesInputs {
            view,
            products,
            picks,
            idx,
            ..
        } = *s;
        let series = &s.series;
        let stars_pick = picks.stars;
        // Star pass: per-series GPU state is the owned snapshot the
        // prepare phase cloned out of the arc scratch (indirect args +
        // VS bind group) — read from the token, never from `arc_cache`,
        // so an eviction/rebuild between prepare and paint cannot drop
        // or desync it. A missing star (n < 2, or a
        // theoretically-impossible prepare miss) just skips the stars —
        // the ribbon still draws.
        let line_extra = match (line_drawn, &stars_pick) {
            (true, Some(sp)) => products
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
        Ok(line_extra)
    }
}
