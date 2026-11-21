//! [`Renderer::submit_stream_arc_supply`]'s phases, split out move-only
//! (#341): a completed leaf's uniforms and scan passes, and the drawing
//! replay's line prefix. Streaming is not extended (rule 8).

use super::*;

impl Renderer {
    /// A completed leaf's uniforms, charged to their headroom, and its leaf
    /// and upper-tree scan passes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_stream_arc_leaf(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &ArcBuffers,
        leaf: ArcReplayLeaf,
        drawing: bool,
        last_leaf: bool,
        canonical_chunk: ArcReplayChunk,
        headroom: u64,
        save_carry: bool,
    ) -> StreamResult<SharedCharge> {
        let tally = ChargeTally::new();
        let params = |label, contents: &[u8]| {
            charged_buffer_init(
                &tally,
                &self.device,
                &wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::UNIFORM,
                },
            )
        };
        let leaf_params = params(
            "stream arc leaf params",
            bytemuck::bytes_of(&leaf.arc_params(0, (Y_OFFSET / 4) as u32)),
        )?;
        let replay_params = params(
            "stream arc leaf identity",
            bytemuck::bytes_of(&leaf.replay_params()),
        )?;
        let top_params = (!drawing && last_leaf)
            .then(|| {
                params(
                    "stream arc top params",
                    bytemuck::bytes_of(&ArcParams {
                        len: canonical_chunk.block_count(),
                        x_base: 0,
                        y_base: 0,
                        start: 0,
                        mode: 0,
                        c_base: 0,
                        flags: 0,
                        _pad: 0,
                    }),
                )
            })
            .transpose()?;
        let upper_params = (!drawing && last_leaf)
            .then(|| {
                params(
                    "stream arc upper params",
                    bytemuck::bytes_of(&ArcParams {
                        len: canonical_chunk.block_count().div_ceil(WG),
                        x_base: 0,
                        y_base: 0,
                        start: 0,
                        mode: 0,
                        c_base: 0,
                        flags: 0,
                        _pad: 0,
                    }),
                )
            })
            .transpose()?;
        if tally.bytes() != headroom {
            return Err(
                FiggyError::internal("stream arc leaf uniforms charge their headroom").into(),
            );
        }
        let charge = shared_charge(tally, &self.gpu_ledger, GpuResourceKind::ArcScan);
        let pipelines = self
            .arc_pipelines
            .as_ref()
            .ok_or(FiggyError::internal("a stream arc has its pipelines"))?;
        let storage = pipelines.replay_storage_bind_group(
            &self.device,
            &buffers.pool,
            &buffers.arc,
            &buffers.sums,
            &leaf_params,
            &buffers.carry,
        );
        let replay =
            pipelines.replay_control_bind_group(&self.device, &buffers.top, &replay_params);
        if drawing && leaf.local_start != 0 {
            encoder.copy_buffer_to_buffer(
                &buffers.predecessor,
                0,
                &buffers.arc,
                0,
                data_render::line_arc::ARC_PAIR_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("stream canonical arc leaf"),
                timestamp_writes: None,
            });
            if drawing {
                pipelines.record_replay_leaf_output(
                    &mut pass,
                    &buffers.transform,
                    &storage,
                    &replay,
                    leaf,
                    canonical_chunk.start != 0,
                    save_carry,
                );
            } else {
                pipelines.record_replay_leaf_total(
                    &mut pass,
                    &buffers.transform,
                    &storage,
                    &replay,
                );
            }
        }
        // Both exist exactly when `!drawing && last_leaf`.
        if let (Some(top_params), Some(upper_params)) = (&top_params, &upper_params) {
            let top = pipelines.replay_storage_bind_group(
                &self.device,
                &buffers.pool,
                &buffers.top,
                &buffers.upper,
                top_params,
                &buffers.carry,
            );
            let upper = pipelines.replay_storage_bind_group(
                &self.device,
                &buffers.pool,
                &buffers.upper,
                &buffers.sink,
                upper_params,
                &buffers.carry,
            );
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("stream canonical arc upper tree"),
                timestamp_writes: None,
            });
            pipelines.record_replay_top(
                &mut pass,
                &buffers.transform,
                &top,
                &upper,
                canonical_chunk,
            );
        }
        Ok(charge)
    }

    /// The drawing replay's line prefix: the leaf's last arc pair kept as
    /// the next leaf's predecessor, and the leaf's segments drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_stream_arc_prefix(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &ArcBuffers,
        leaf: ArcReplayLeaf,
        snapshot: &Option<Arc<AutoStreamExecutionSnapshot>>,
        series_index: usize,
        ticket: StreamTicket,
        view: &ChartView,
        target: &wgpu::Texture,
        auxiliary: &Option<Arc<auxiliary::StreamAuxiliaryTarget>>,
    ) -> StreamResult<()> {
        encoder.copy_buffer_to_buffer(
            &buffers.arc,
            u64::from(leaf.supply_len - 1) * data_render::line_arc::ARC_PAIR_BYTES,
            &buffers.predecessor,
            0,
            data_render::line_arc::ARC_PAIR_BYTES,
        );
        let (states, preparation) = self.preparation_parts();
        let (config, series, style) = match snapshot.as_ref() {
            Some(snapshot) => (
                &snapshot.config,
                &snapshot.series[series_index],
                &snapshot.styles.styles[series_index],
            ),
            None => {
                let state = states
                    .get(&ticket.job.chart)
                    .ok_or(FiggyError::UnknownChart {
                        id: ticket.job.chart,
                    })?;
                let styles = state
                    .prepared_styles
                    .as_ref()
                    .ok_or(FiggyError::internal("validated explicit stream styles"))?;
                (
                    &state.config,
                    &state.series[series_index],
                    &styles.styles[series_index],
                )
            }
        };
        let handle = |offset| ColumnHandle {
            generation: 0,
            offset,
            byte_size: u64::from(leaf.supply_len) * 8,
            len_values: leaf.supply_len as usize,
        };
        let packet = preparation.build_stream_arc_series(
            view,
            config,
            &Series {
                config: series,
                style,
            },
            &buffers.pool,
            handle(0),
            handle(Y_OFFSET),
            (
                buffers.arc.clone(),
                u64::from(leaf.supply_len) * data_render::line_arc::ARC_PAIR_BYTES,
            ),
            Arc::clone(&buffers.charge),
            auxiliary
                .as_ref()
                .and_then(|a| a.pipelines.as_ref())
                .unwrap_or(preparation.pipelines),
        )?;
        let size = (target.width(), target.height());
        let panel = data_render::clamp_rect_to_target(view.panel_rect, size);
        let data = data_render::clamp_rect_to_target(
            config.data_area().map_or(view.panel_rect, |area| area.0),
            size,
        );
        if let (Some(panel), Some(data)) = (panel, data) {
            let target_view = target.create_view(&wgpu::TextureViewDescriptor {
                mip_level_count: Some(1),
                ..Default::default()
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stream canonical line prefix"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(
                panel.x as f32,
                panel.y as f32,
                panel.width as f32,
                panel.height as f32,
                0.0,
                1.0,
            );
            pass.set_scissor_rect(data.x, data.y, data.width, data.height);
            data_render::issue_series_data(&mut pass, &packet.layers());
        }
        Ok(())
    }
}
