//! Marker layers: scatter markers, error bars and stems.

use super::*;

impl PrepareContext<'_> {
    /// A series' markers: precise, mapped, styled, ternary or 3D.
    pub(super) fn scatter_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<ColumnScatterLayer<'a>>> {
        let SeriesInputs {
            view,
            pipelines,
            styled,
            column_buffer,
            phase,
            picks,
            cfg,
            rt,
            primitives,
            x_h,
            y_h,
            z_3d,
            ..
        } = *s;
        let series = &s.series;
        let scatter_pick = picks.scatter;
        let camera_binding = |mapped: bool| self.camera_binding(s, mapped);
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
                            kind: crate::RuleKind::ScatterStyleMapIndex,
                        });
                    };
                    let h = lookup(column)?;
                    let count = x_h.len_values.min(y_h.len_values);
                    if h.len_values < count {
                        return Err(FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            kind: crate::RuleKind::ScatterStyleIndexShort {
                                column: column.to_string(),
                                len: h.len_values as u64,
                                need: count as u64,
                            },
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
            let scatter_pick =
                scatter_pick.ok_or(FiggyError::internal("prepare ensured scatter pipeline"))?;
            // A ternary series' markers: the ternary entries with its
            // third component.
            let ternary_c = rt.ternary_c_column().map(&mut lookup).transpose()?;
            let ternary = |kind: TernaryPipeline| {
                pipelines.ternary[kind as usize]
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured the ternary scatter pipeline",
                    ))
            };
            // A 3D series' markers: the 3D entries with its height and
            // the panel's camera, beside the value map when mapped
            // (#132).
            let view3d = |kind: View3dPipeline| {
                pipelines.view3d[kind as usize]
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured the 3D scatter pipeline",
                    ))
            };
            let scatter_3d = z_3d
                .map(|z| camera_binding(precise_style_map.is_some()).map(|binding| (z, binding)))
                .transpose()?;
            Some(ColumnScatterLayer {
                pipeline: match (precise_style_map, ternary_c, scatter_3d.is_some()) {
                    (Some(_), _, true) => view3d(View3dPipeline::ScatterMapped)?,
                    (None, _, true) => view3d(View3dPipeline::Scatter)?,
                    (Some(_), Some(_), false) => ternary(TernaryPipeline::ScatterMapped)?,
                    (None, Some(_), false) => ternary(TernaryPipeline::Scatter)?,
                    (Some(_), None, false) => {
                        pipelines
                            .scatter_mapped
                            .as_ref()
                            .ok_or(FiggyError::internal(
                                "prepare ensured mapped scatter pipeline",
                            ))?
                    }
                    (None, None, false) => scatter_pick.pipeline,
                },
                third: scatter_3d.as_ref().map(|(z, _)| *z).or(ternary_c),
                view3d: scatter_3d.map(|(_, binding)| binding),
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
        Ok(scatter)
    }

    /// A series' error bars, symmetric or asymmetric on either axis.
    pub(super) fn errorbar_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    ) -> Result<Option<ColumnErrorBarDraw<'a>>> {
        let SeriesInputs {
            view,
            pipelines,
            styled,
            column_buffer,
            phase,
            picks,
            cfg,
            rt,
            constellation_only,
            x_h,
            y_h,
            ..
        } = *s;
        let series = &s.series;
        let errorbar_pipe = picks.errorbar;
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
                    let errorbar_style_index =
                        match (precise_errorbar_style_map, extract_errorbar_style(rt)) {
                            (Some(map), Some(eb)) if map.has_index => {
                                let Some(column) = eb.error_bar_style_index_column.as_ref() else {
                                    return Err(FiggyError::InvalidSeriesConfig {
                                        series_id: cfg.series_id.clone(),
                                        kind: crate::RuleKind::ErrorBarStyleMapIndex,
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
                                        kind: crate::RuleKind::ErrorBarStyleIndexShort {
                                            column: column.to_string(),
                                            len: h.len_values as u64,
                                            need: count as u64,
                                        },
                                    });
                                }
                                Some(h)
                            }
                            _ => None,
                        };
                    Some(ColumnErrorBarDraw {
                        pipeline: match precise_errorbar_style_map {
                            None => errorbar_pipe
                                .ok_or(FiggyError::internal("prepare ensured errorbar pipeline"))?,
                            Some(_) => {
                                pipelines
                                    .errorbar_mapped
                                    .as_ref()
                                    .ok_or(FiggyError::internal(
                                        "prepare ensured mapped errorbar pipeline",
                                    ))?
                            }
                        },
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
        Ok(errorbar)
    }

    /// A stem series' stems.
    pub(super) fn stem_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
    ) -> Result<Option<ColumnErrorBarDraw<'a>>> {
        let SeriesInputs {
            view,
            column_buffer,
            phase,
            picks,
            constellation_only,
            primitives,
            x_h,
            y_h,
            ..
        } = *s;
        let series = &s.series;
        let stem_pipe = picks.stem;
        // The stems (#206): the error-bar module's second draw, the
        // error slots bound to the anchor columns as filler (the stem
        // reads only x and y). Every row is drawn (rule 10). A streamed
        // phase never draws one (rule 8: refused before a cursor).
        let stem = match (series.style.stem_bg.as_ref(), phase) {
            (Some(stem_bg), None) if !constellation_only && primitives.scatter => {
                Some(ColumnErrorBarDraw {
                    pipeline: stem_pipe
                        .ok_or(FiggyError::internal("prepare ensured stem pipeline"))?,
                    transform_bg: &view.transform_bg,
                    style_bg: stem_bg,
                    style_map_bg: None,
                    pool_buffer: column_buffer,
                    x: x_h,
                    y: y_h,
                    err_y_lo: y_h,
                    err_y_hi: y_h,
                    err_x_lo: x_h,
                    err_x_hi: x_h,
                    style_index: None,
                })
            }
            _ => None,
        };
        Ok(stem)
    }
}
