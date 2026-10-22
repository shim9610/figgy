//! Streamed series preparation: validation and the stream layer builders.

use super::*;

impl PrepareContext<'_> {
    /// A single exact pass; composite series and global-context primitives must
    /// be scheduled separately rather than repeated once per source chunk.
    pub(in crate::renderer) fn validate_stream_series(
        config: &Config,
        series: &SeriesConfig,
    ) -> Result<()> {
        let unsupported = || FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: "stream pass does not yet support contours or arc-driven stars".into(),
        };
        let primitives = effective_series_primitives(&config.draw_style, &series.render_type);
        if primitives.contour
            || (primitives.field && !streaming_runtime::is_stream_heatmap(&series.render_type))
            || (primitives.line && matches!(config.draw_style, DrawStyle::Milkyway(_)))
        {
            return Err(unsupported());
        }
        // Streaming is not extended to polar charts.
        if config.polar.is_some() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "polar charts require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // A fill between two series needs index-aligned chunks of two streams,
        // which nonresident streaming cannot guarantee, and a stepped line's
        // arc prefix is not part of the streamed canonical scan. Both are
        // resident-only by design, rejected here rather than drawn wrong.
        if primitives.line
            && let Some(line) = extract_line(&series.render_type)
            && (line.fill.is_some() || !line.interpolation.is_linear())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "line fill and step interpolation require GPU-resident columns; \
                         streamed sources are not supported"
                    .into(),
            });
        }
        // Streaming is not extended to vector series: their shader reads the
        // resident pool as storage.
        if has_vector(&series.render_type) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "vector series require GPU-resident columns; streamed sources are \
                         not supported"
                    .into(),
            });
        }
        // The value maps read index-aligned columns on every draw; streaming
        // is not extended to them.
        if let Some(scatter) = extract_scatter(&series.render_type)
            && (scatter.point_color_map.is_some() || scatter.point_size_map.is_some())
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.series_id.clone(),
                reason: "point colour and size maps require GPU-resident columns; streamed \
                         sources are not supported"
                    .into(),
            });
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::renderer) fn build_stream_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
    ) -> Result<PreparedSeries> {
        self.build_stream_series_with_pipelines(
            view,
            config,
            series,
            chunk,
            columns,
            phase,
            self.pipelines,
        )
    }

    pub(in crate::renderer) fn build_stream_arc_series(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        pool: &wgpu::Buffer,
        x: ColumnHandle,
        y: ColumnHandle,
        arc: ArcPrefix,
        charge: crate::gpu_memory::SharedCharge,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            pool,
            |id| {
                if id == series.config.x_column {
                    Ok(x)
                } else if id == series.config.y_column {
                    Ok(y)
                } else {
                    Err(FiggyError::UnknownColumn { id: id.into() })
                }
            },
            Some(streaming_runtime::StreamDrawPhase::Line),
        )?;
        let mut packet = PreparedSeries::from_layers(
            layers.remove(0),
            Some(charge),
            Some(Arc::clone(&series.style._charge)),
        );
        if let Some(line) = &mut packet.line {
            line.arc = Some(arc);
        }
        Ok(packet)
    }

    pub(in crate::renderer) fn build_stream_series_with_pipelines(
        &self,
        view: &ChartView,
        config: &Config,
        series: &Series<'_>,
        chunk: &crate::streaming_upload::RecordedChunk,
        columns: &[crate::streaming::ColumnRange],
        phase: streaming_runtime::StreamDrawPhase,
        pipelines: &TargetPipelines,
    ) -> Result<PreparedSeries> {
        Self::validate_stream_series(config, series.config)?;
        if phase == streaming_runtime::StreamDrawPhase::Field {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed field requires the bounded tile executor".into(),
            });
        }
        if phase == streaming_runtime::StreamDrawPhase::Line
            && extract_line(&series.config.render_type).is_some_and(|line| {
                !matches!(line.line_style, LineStylePreset::Solid)
                    || style_variant(&config.draw_style)
                        .is_some_and(|variant| variant.needs_arc_prefix)
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed line requires a prepared canonical arc prefix".into(),
            });
        }
        let expected = streaming_runtime::stream_phase_columns(series.config, phase);
        let first = columns
            .first()
            .ok_or_else(|| FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive has no supplied columns".into(),
            })?;
        let bins = if phase == streaming_runtime::StreamDrawPhase::Histogram {
            if streaming_runtime::histogram_edge_column(series.config)
                == Some(series.config.x_column.as_str())
            {
                first.len.saturating_sub(1)
            } else {
                first.len
            }
        } else {
            first.len.saturating_sub(phase.halo())
        };
        if columns.len() != expected.count
            || columns.iter().enumerate().any(|(index, column)| {
                column.column != index as u64
                    || column.offset != first.offset
                    || column.len
                        != if phase == streaming_runtime::StreamDrawPhase::Histogram {
                            bins.saturating_add(u64::from(
                                streaming_runtime::histogram_edge_column(series.config)
                                    == Some(expected.ids[index]),
                            ))
                        } else {
                            bins.saturating_add(phase.halo())
                        }
            })
        {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: series.config.series_id.clone(),
                reason: "streamed primitive ranges must describe the same global rows".into(),
            });
        }
        let specs = [Series {
            config: series.config,
            style: series.style,
        }];
        let mut layers = self.build_series_layers_with_columns(
            view,
            config,
            &specs,
            pipelines,
            PreparedSeriesProducts {
                arcs: &[],
                contours: &[],
            },
            pipelines.style_set(&config.draw_style),
            &chunk.work,
            |id| {
                let index = expected.ids[..expected.count]
                    .iter()
                    .position(|column| *column == id)
                    .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
                let range = columns[index];
                chunk
                    .column_handle(range)
                    .map_err(|error| FiggyError::InvalidSeriesConfig {
                        series_id: series.config.series_id.clone(),
                        reason: format!("invalid streamed column span: {error:?}"),
                    })
            },
            Some(phase),
        )?;
        let mut prepared = PreparedSeries::from_layers(
            layers.remove(0),
            None,
            Some(Arc::clone(&series.style._charge)),
        );
        prepared._column_charge = Some(chunk.work.shared_charge());
        Ok(prepared)
    }
}
