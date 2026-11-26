//! [`Renderer::begin_stream_export`]'s phases, split out move-only (#341):
//! the per-phase admission checks, the allocation admission, the export
//! target's view, styles, pipelines and surface, and its cleared first
//! frame. Streaming is not extended (rule 8).

use super::*;

impl Renderer {
    /// Every phase of the completed series fits the scheduler's limits.
    pub(super) fn check_stream_export_phases(
        &self,
        completed: &AutoStreamExecutionSnapshot,
        config: &Config,
        max_primitives: u64,
    ) -> StreamResult<()> {
        let limits = live_runtime(&self.stream_runtime)?.scheduler.limits();
        for series in &completed.series {
            PrepareContext::validate_stream_series(&config, series)?;
            for phase in stream_draw_phases(&config.draw_style, series)? {
                if *phase == StreamDrawPhase::Field {
                    field_runtime::preflight(
                        &self.device,
                        limits,
                        &config,
                        series,
                        max_primitives,
                        |id| completed.sources.get(id),
                    )?;
                    continue;
                }
                let columns = stream_phase_columns(series, *phase);
                let total = stream_phase_total(series, *phase, |id| {
                    completed.sources.get(id).map(|s| s.len)
                })?;
                if total == 0 {
                    continue;
                }
                if *phase == StreamDrawPhase::Line
                    && arc_runtime::needs_arc(&config.draw_style, series)
                    && total >= u64::from(u32::MAX)
                {
                    return Err(StreamError::TooLarge.into());
                }
                let mut input = 0u64;
                let mut work = 0u64;
                for id in &columns.ids[..columns.count] {
                    let rows = if *phase == StreamDrawPhase::Line
                        && arc_runtime::needs_arc(&config.draw_style, series)
                    {
                        (total + 1).min(max_primitives).min(257)
                    } else {
                        stream_phase_request_len(*phase, total.min(max_primitives))?
                    };
                    input = input
                        .checked_add(
                            rows.checked_mul(completed.sources[*id].encoding.bytes_per_value())
                                .ok_or(StreamError::Overflow)?,
                        )
                        .ok_or(StreamError::Overflow)?;
                    work = work
                        .checked_add(rows.checked_mul(8).ok_or(StreamError::Overflow)?)
                        .ok_or(StreamError::Overflow)?;
                }
                if columns.count > limits.max_columns_per_request
                    || input > limits.max_chunk_bytes
                    || work.checked_mul(2).ok_or(StreamError::Overflow)?
                        > limits.max_in_flight_bytes
                    || work > self.device.limits().max_buffer_size
                    || work > u64::from(self.device.limits().max_storage_buffer_binding_size)
                {
                    return Err(StreamError::TooLarge.into());
                }
            }
        }
        Ok(())
    }

    /// The export's style and surface bytes, admitted before any of them is
    /// allocated.
    pub(super) fn admit_stream_export(
        &mut self,
        completed: &AutoStreamExecutionSnapshot,
        config: &Config,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> StreamResult<(u64, StreamSurfaceSpec)> {
        // Admission precedes every export-owned GPU allocation. Each
        // ChartStyle charges its own uniforms and mapped-style buffers.
        let mut untracked_style_bytes = 0u64;
        let mut style_bytes = 0u64;
        for series in &completed.series {
            style_bytes = style_bytes
                .checked_add(self.style_allocation_bytes(series)?)
                .ok_or(StreamError::Overflow)?;
        }
        if matches!(
            config.draw_style,
            DrawStyle::Milkyway(_) | DrawStyle::Constellation(_)
        ) {
            untracked_style_bytes = untracked_style_bytes
                .checked_add(data_render::STYLED_TEXTURE_BYTES)
                .ok_or(StreamError::Overflow)?;
        }
        let surface_spec = StreamSurfaceSpec {
            width: size.0,
            height: size.1,
            format,
            sample_count,
        };
        let view_bytes = u64::from(size.0)
            .checked_mul(u64::from(size.1))
            .and_then(|pixels| pixels.checked_mul(8))
            .and_then(|bytes| {
                bytes.checked_add(std::mem::size_of::<data_render::ScatterTransform>() as u64)
            })
            .ok_or(StreamError::Overflow)?;
        let allocation_bytes = surface_spec
            .charged_bytes()?
            .checked_add(view_bytes)
            .and_then(|bytes| bytes.checked_add(untracked_style_bytes))
            .and_then(|bytes| bytes.checked_add(style_bytes))
            .ok_or(StreamError::Overflow)?;
        self.preflight_auxiliary_gpu_bytes(allocation_bytes)?;
        live_runtime_mut(&mut self.stream_runtime)?
            .draws
            .try_reserve(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        Ok((untracked_style_bytes, surface_spec))
    }

    /// The export target's chart view, styles, pipelines, transfer and
    /// surface.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub(super) fn create_stream_export_target(
        &self,
        config: &Config,
        completed: &AutoStreamExecutionSnapshot,
        scale: f32,
        untracked_style_bytes: u64,
        format: wgpu::TextureFormat,
        sample_count: u32,
        surface_spec: StreamSurfaceSpec,
    ) -> StreamResult<(
        ChartView,
        u64,
        u64,
        RegisteredChartStyles,
        crate::gpu_memory::SharedCharge,
        TargetPipelines,
        StreamTransfer,
        StreamSurface,
    )> {
        let chart_view =
            self.create_chart_view(&Chart::stored(config.clone()), config.chart_area.0)?;
        let expected_view_revision = chart_view.advance_stream_revision()?;
        let displayed_view_revision = chart_view.advance_content_revision()?;
        data_render::update_scatter_transform(
            &self.queue,
            &chart_view.transform_buffer,
            &data_render::scatter_transform_from_config(&config),
        );
        let styles = RegisteredChartStyles {
            series_revision: completed.series_revision,
            styles: completed
                .series
                .iter()
                .map(|series| self.create_style_for_series_scaled_inner(series, scale))
                .collect::<Result<_>>()?,
        };
        let tally = crate::gpu_memory::ChargeTally::new();
        tally.add(untracked_style_bytes);
        let styles_charge =
            crate::gpu_memory::shared_charge(tally, &self.gpu_ledger, GpuResourceKind::Uniform);
        if self.gpu_memory_usage().total_bytes() > self.gpu_ledger.budget_limit() {
            return Err(StreamError::TooLarge.into());
        }
        let mut pipelines = create_target_pipelines(
            &self.device,
            None,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            format,
            sample_count,
            self.pipelines.shaders.set,
        );
        let series: Vec<_> = completed
            .series
            .iter()
            .zip(&styles.styles)
            .map(|(config, style)| Series { config, style })
            .collect();
        pipelines.ensure_precise_variants_for_items(
            &self.device,
            None,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            &self.point_value_map_bgl,
            [&self.surface_bgl, &self.view3d_value_map_bgl],
            &self.data_selection_bgl,
            &self.field_bgl,
            &self.vector_bgl,
            &self.star_data_bgl,
            &self.category_bar_data_bgl,
            &self.parallel_bgl,
            self.contour_label_pipelines.as_ref(),
            format,
            &[PanelDraw {
                view: &chart_view,
                chart_config: &config,
                series: &series,
            }],
            // Streaming keeps its plain contour pass (rule 8).
            None,
        );
        pipelines.ensure_styles_for_items(
            &self.device,
            None,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            format,
            &[PanelDraw {
                view: &chart_view,
                chart_config: &config,
                series: &series,
            }],
        );
        let transfer = StreamTransfer::new(&self.device, format, sample_count)?;
        let surface = StreamSurface::new(
            &self.device,
            &self.gpu_ledger,
            &transfer,
            surface_spec,
            self.gpu_ledger.budget_limit(),
        )?;
        Ok((
            chart_view,
            expected_view_revision,
            displayed_view_revision,
            styles,
            styles_charge,
            pipelines,
            transfer,
            surface,
        ))
    }

    /// The export target cleared to `clear`, with the axis grid and the
    /// annotation underlay drawn, recorded into a new encoder.
    pub(super) fn clear_stream_export_target(
        &self,
        surface: &StreamSurface,
        auxiliary: &Arc<StreamAuxiliaryTarget>,
        snapshot: &Arc<AutoStreamExecutionSnapshot>,
        clear: crate::Color,
    ) -> StreamResult<(wgpu::Texture, wgpu::CommandEncoder)> {
        let axis = &auxiliary
            .pipelines
            .as_ref()
            .ok_or(FiggyError::internal(
                "an export target is built with its pipelines",
            ))?
            .axis;
        let target = wgpu::Texture::clone(surface.prefix());
        let target_view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream export initialization"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(axis);
            pass.set_bind_group(0, &snapshot.view.grid_bind_group, &[]);
            pass.draw(0..3, 0..1);
            if let (Some(underlay), _) = snapshot
                .view
                .annotation_bind_groups(snapshot.annotations_visible())
            {
                pass.set_bind_group(0, underlay, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        Ok((target, encoder))
    }
}
