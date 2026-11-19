//! Preparing a frame: the parts, the registered and mixed paths, validation.

use super::*;

impl Renderer {
    /// Draw multiple chart panels into one RenderPass.
    ///
    /// Facade over [`Self::prepare`] + [`Self::paint_prepared`] for hosts
    /// that own the renderer exclusively during their frame (winit loop,
    /// wasm wrapper, export): both phases run back to back under one
    /// `&mut self`, so no interleaving is possible and no token handling is
    /// needed. Hosts whose paint callback only provides `&self` (egui's
    /// `CallbackTrait::paint`, iced's `shader::Primitive::draw`) should call
    /// the two phases separately instead of wrapping the renderer in a
    /// `Mutex` — see the host-integration notes above.
    ///
    /// Takes a pass already opened by the host via `begin_render_pass`.
    /// `Renderer` owns the pool buffer, pipelines, and bind groups, so no
    /// external resources are needed.
    ///
    /// Returns `FiggyError::UnknownColumn` if a series references an unknown
    /// column id (or a stale handle).
    ///
    /// `target_size` is the color attachment pixel size of the current pass;
    /// panel_rect / data_area are clamped to it because wgpu validation would
    /// otherwise abort. Pass the surface pixel size reported by the host
    /// (winit, egui CallbackInfo, iced shader::Primitive, …).
    pub fn paint(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &mut [ChartDrawItem<'_>],
    ) -> Result<()> {
        let prepared = self.prepare(items)?;
        self.paint_prepared(pass, target_size, &prepared)
    }

    pub(in crate::renderer) fn preparation_parts(
        &mut self,
    ) -> (&HashMap<ChartId, ChartRenderState>, PrepareContext<'_>) {
        (
            &self.chart_states,
            PrepareContext {
                in_frame: false,
                device: &self.device,
                queue: &self.queue,
                caps: &self.caps,
                pool: &self.pool,
                monotonic: &self.monotonic,
                gpu_ledger: &self.gpu_ledger,
                style_bgl: &self.style_bgl,
                field_bgl: &self.field_bgl,
                surface_bgl: &self.surface_bgl,
                region_bgl: &self.region_bgl,
                vector_bgl: &self.vector_bgl,
                category_bar_data_bgl: &self.category_bar_data_bgl,
                parallel_bgl: &self.parallel_bgl,
                value_label_bgl: &self.value_label_bgl,
                field_cell_tables: &self.field_cell_tables,
                transform_bgl: &self.transform_bgl,
                per_point_style_map_bgl: &self.per_point_style_map_bgl,
                point_value_map_bgl: &self.point_value_map_bgl,
                neutral_point_value_map_bg: &self.neutral_point_value_map_bg,
                neutral_point_value_map_buffers: &self.neutral_point_value_map_buffers,
                view3d_value_map_bgl: &self.view3d_value_map_bgl,
                data_selection_bgl: &self.data_selection_bgl,
                star_data_bgl: &self.star_data_bgl,
                sampler: &self.sampler,
                quad_vb: &self.quad_vb,
                pipelines: &mut self.pipelines,
                field_cache: &mut self.field_cache,
                value_map_cache: &mut self.value_map_cache,
                vector_cache: &mut self.vector_cache,
                contour_cache: &mut self.contour_cache,
                arc_cache: &mut self.arc_cache,
                texture_bgl: &self.texture_bgl,
                target_sample_count: self.target_sample_count,
                resident_displays: &mut self.resident_displays,
                resident_display_serial: &mut self.resident_display_serial,
                contour_label_pipelines: &mut self.contour_label_pipelines,
                arc_pipelines: &mut self.arc_pipelines,
                surface_format: self.surface_format,
                renderer_identity: self.renderer_identity,
                target_pipeline_generation: self.target_pipeline_generation,
                #[cfg(test)]
                arc_chunk_override: self.arc_chunk_override,
            },
        )
    }

    /// Mutable half of the frame: finish every state change the draw needs,
    /// before any render pass opens.
    ///
    /// First redraws each stale view's text and axis raster (#265): a view
    /// drawn from another raster stamp of its chart, another panel rect or
    /// scale, or an older font generation. The raster draws the chart's
    /// selection ([`Chart::selected`]). A view whose chart has not changed
    /// is left as it is. [`Self::refresh_axis`] stays for a host that draws
    /// the raster itself; the next prepare then checks it once. Rebuilds no
    /// caller-owned styles: recreate affected ones with
    /// [`Self::create_style_for_series`] before calling this method.
    ///
    /// Runs the styled-pipeline ensure (compiles pipeline sets the items
    /// need but the cache lacks), writes each panel's data→NDC transform
    /// uniform from `item.chart_config`, and (re)dispatches the per-series
    /// GPU arc-length scans — submitting their compute encoder on the queue,
    /// which orders it before the host's later pass submission. Returns an
    /// owned [`PreparedFrame`] token snapshotting everything the immutable
    /// draw phase will reference.
    ///
    /// Because the transform uniform and the arc dispatch are written here
    /// from the same config snapshot, they cannot desync — calling
    /// [`Self::update_transform`] per frame becomes unnecessary (though
    /// harmless) for callers of this API.
    ///
    /// Contract: call after this frame's data uploads (`add_column`) and
    /// `ensure_target_format`; before the host records and
    /// submits the command buffer containing the render pass. Once a command
    /// buffer has been recorded from the token, submit it before any later
    /// mutation of the same [`ChartView`] or another `prepare` for that view.
    /// The returned token owns the resolved draw input. `paint_prepared`
    /// therefore receives no second
    /// copy of chart/view/series/style inputs. It rejects renderer resource
    /// changes that invalidate the captured GPU work: target-pipeline rebuild,
    /// pool layout change, replacement of any captured column allocation, or
    /// another transform/axis write to a captured `ChartView`.
    ///
    /// A chart whose layout waits for its fit
    /// ([`crate::LayoutState::NeedsFit`], #357) is fitted first, and the fit
    /// is written into it ([`Chart::fit_pending_layout`]); the draw then
    /// shows the chart as it is stored. [`Self::prepare_as_stored`] skips
    /// the fit.
    pub fn prepare(&mut self, items: &mut [ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        let result = self.prepare_items_unreported(items, DrawLayout::FitPending);
        self.reported(result)
    }

    /// [`Self::prepare`] drawing each chart exactly as stored (#357): a
    /// layout waiting for its fit is drawn with the margins it holds and
    /// stays [`crate::LayoutState::NeedsFit`].
    pub fn prepare_as_stored(&mut self, items: &mut [ChartDrawItem<'_>]) -> Result<PreparedFrame> {
        let result = self.prepare_items_unreported(items, DrawLayout::AsStored);
        self.reported(result)
    }

    pub(super) fn prepare_items_unreported(
        &mut self,
        items: &mut [ChartDrawItem<'_>],
        layout: DrawLayout,
    ) -> Result<PreparedFrame> {
        self.health.check()?;
        if layout == DrawLayout::FitPending {
            for item in items.iter_mut() {
                item.chart.fit_pending_layout();
            }
        }
        for item in items.iter_mut() {
            self.refresh_stale_view(item.view, item.chart)?;
        }
        let mut panels = Vec::new();
        panels.try_reserve_exact(items.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "chart prepare inputs",
                reason: error.to_string(),
            }
        })?;
        panels.extend(items.iter().map(|item| PanelDraw {
            view: &*item.view,
            chart_config: item.chart.config(),
            series: item.series,
        }));
        self.prepare_unreported(&panels)
    }

    /// Redraw `view`'s raster from `chart` when it was drawn from other
    /// content: another raster stamp, panel rect or scale, or font
    /// generation.
    fn refresh_stale_view(&mut self, view: &mut ChartView, chart: &Chart) -> Result<()> {
        let panel_rect = chart.config().chart_area.0;
        let scale = view.display_scale;
        let stamp = chart.raster_stamp().ok_or(FiggyError::CounterExhausted {
            counter: "chart raster stamp",
        })?;
        let source = Some(RasterSource {
            chart: RasterChart::Held { stamp },
            font_generation: crate::text_render::font_generation(),
            panel_rect,
            scale,
        });
        if view.raster_source == source {
            return Ok(());
        }
        self.refresh_axis_with_selection_unreported(
            view,
            chart,
            panel_rect,
            chart.selected(),
            scale,
        )?;
        view.raster_source = source;
        Ok(())
    }

    /// Redraw a registered chart's `view` from its stored document, display
    /// placement and selection when it was drawn from other content.
    fn refresh_stale_registered_view(&mut self, view: &mut ChartView, id: ChartId) -> Result<()> {
        let (config, panel_rect, scale) = self.chart_display_config(id)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let source = RasterSource {
            chart: RasterChart::Registered {
                chart: id,
                raster: state.revisions.raster,
                annotation: state.revisions.annotation,
            },
            font_generation: crate::text_render::font_generation(),
            panel_rect,
            scale,
        };
        if view.raster_source == Some(source) {
            return Ok(());
        }
        let selected = state.selected.and_then(|hit| {
            crate::select::HitMap::for_active_layer(&state.config)
                .get(hit)
                .map(|element| element.element_id())
        });
        let chart = Chart::new(config.into_owned());
        self.refresh_axis_with_selection_unreported(
            view,
            &chart,
            panel_rect,
            selected.as_deref(),
            scale,
        )?;
        view.raster_source = Some(source);
        Ok(())
    }

    pub(super) fn prepare_unreported(&mut self, items: &[PanelDraw<'_>]) -> Result<PreparedFrame> {
        self.health.check()?;
        // The one point where a frame's charts are known before any of its
        // pipelines is compiled (#299): a break, a probability axis or
        // polar coordinates on any layer of any chart move the renderer to
        // the full shaders.
        self.admit_full_shaders(
            items
                .iter()
                .any(|item| config_needs_full_shaders(item.chart_config)),
        );
        for item in items {
            for series in item.series {
                self.require_resident_series(series.config)?;
            }
        }
        self.preparation_parts().1.prepare(items)
    }

    /// Prepare renderer-owned charts without copying their Config or ordered
    /// SeriesConfig values. GPU styles are cached derivatives of those series;
    /// chart identity is always the explicit id, never inferred from resources.
    ///
    /// Each stale view's raster is redrawn first, as for [`Self::prepare`],
    /// from the stored chart, its display placement and its selection
    /// ([`Self::set_chart_selection`]). Mutating or removing a captured chart
    /// makes its token stale. Changes to unrelated charts do not invalidate
    /// the captured chart stamps.
    ///
    /// A chart whose layout waits for its fit
    /// ([`crate::LayoutState::NeedsFit`], #357) is fitted first and the fit
    /// is stored ([`Self::fit_pending_layout`]); a chart that draws a
    /// streamed source, and a monotonic chart, are drawn as stored.
    /// [`Self::prepare_registered_as_stored`] skips the fit.
    pub fn prepare_registered(
        &mut self,
        items: &mut [RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let result = self
            .fit_pending_registered(items.iter().map(|item| item.chart_id))
            .and_then(|()| self.prepare_registered_items_unreported(items));
        self.reported(result)
    }

    /// [`Self::prepare_registered`] drawing each chart exactly as stored
    /// (#357): a layout waiting for its fit stays
    /// [`crate::LayoutState::NeedsFit`].
    pub fn prepare_registered_as_stored(
        &mut self,
        items: &mut [RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        let result = self.prepare_registered_items_unreported(items);
        self.reported(result)
    }

    pub(in crate::renderer) fn prepare_registered_items_unreported(
        &mut self,
        items: &mut [RegisteredChartDrawItem<'_>],
    ) -> Result<PreparedFrame> {
        self.health.check()?;
        // Every id resolves before any view is redrawn: a failed prepare
        // leaves every view as it was.
        for item in items.iter() {
            if !self.chart_states.contains_key(&item.chart_id) {
                return Err(FiggyError::UnknownChart { id: item.chart_id });
            }
        }
        for item in items.iter_mut() {
            self.refresh_stale_registered_view(item.view, item.chart_id)?;
        }
        let mut panels = Vec::new();
        panels.try_reserve_exact(items.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "registered chart prepare inputs",
                reason: error.to_string(),
            }
        })?;
        panels.extend(items.iter().map(|item| RegisteredPanel {
            chart_id: item.chart_id,
            view: &*item.view,
        }));
        self.prepare_registered_unreported(&panels)
    }

    pub(super) fn prepare_registered_unreported(
        &mut self,
        items: &[RegisteredPanel<'_>],
    ) -> Result<PreparedFrame> {
        self.health.check()?;
        for item in items {
            self.admit_chart_shaders(item.chart_id);
        }
        self.prepare_registered_mixed(items)
            .map_err(streaming_runtime::StreamRequestError::into_figgy)
    }

    pub(in crate::renderer) fn prepare_registered_resident(
        &mut self,
        items: &[RegisteredPanel<'_>],
    ) -> Result<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "registered chart prepare inputs",
                reason: error.to_string(),
            };
        for item in items {
            self.admit_chart_shaders(item.chart_id);
        }
        // Resolve every id and fallible contour invariant before creating any
        // GPU styles or advancing the content revision of a supplied view.
        for item in items {
            let (_, _, scale) = self.chart_display_config(item.chart_id)?;
            let state = self
                .chart_states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            for series in &state.series {
                // A series whose source is not resident (a cancelled or
                // unpublished stream) is not drawn; the rest of the chart is
                // (#160).
                if self.require_resident_series(series).is_err() {
                    continue;
                }
                validate_contour_level_count(&series.series_id, &series.render_type)?;
                state
                    .config
                    .validate_series_on_polar(series)
                    .map_err(|reason| FiggyError::InvalidSeriesConfig {
                        series_id: series.series_id.clone(),
                        kind: reason,
                    })?;
                preflight_contour_label_spacing(
                    &series.series_id,
                    &series.render_type,
                    effective_series_primitives(&state.config.draw_style, &series.render_type)
                        .contour,
                    scale,
                )?;
            }
        }
        for item in items {
            if self
                .chart_states
                .get(&item.chart_id)
                .is_some_and(|state| state.display.is_some())
            {
                self.ensure_registered_display(item.chart_id)?;
            } else {
                self.ensure_registered_styles(item.chart_id)?;
            }
        }
        // Whether each series of each item is drawn: resident sources only,
        // and in a browser only once its pipelines have compiled (#299).
        let mut drawn = Vec::new();
        drawn.try_reserve_exact(items.len()).map_err(allocation)?;
        let mut complete = true;
        // What this frame needs becomes the screen of the compile queue.
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            queue.begin_frame();
        }
        for item in items {
            let state = self
                .chart_states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            let mut resident = Vec::new();
            resident
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            for (index, series) in state.series.iter().enumerate() {
                let ready = self.registered_series_compiled(item, state, index);
                complete &= ready;
                resident.push(self.require_resident_series(series).is_ok() && ready);
            }
            drawn.push(resident);
        }
        self.frame_complete = complete;
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            self.frame_settled = queue.settled();
        }
        let (states, mut preparation) = self.preparation_parts();
        let mut series_inputs = Vec::new();
        let mut stamps = Vec::new();
        series_inputs
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        stamps.try_reserve_exact(items.len()).map_err(allocation)?;
        for (item, resident) in items.iter().zip(&drawn) {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            let styles = &match &state.display {
                Some(display) => display.styles.as_ref().map(|(_, styles)| styles),
                None => state.prepared_styles.as_ref(),
            }
            .ok_or(FiggyError::internal("registered styles were prepared"))?
            .styles;
            let mut series = Vec::new();
            series
                .try_reserve_exact(state.series.len())
                .map_err(allocation)?;
            series.extend(
                state
                    .series
                    .iter()
                    .zip(styles)
                    .zip(resident)
                    .filter(|(_, resident)| **resident)
                    .map(|((config, style), _)| Series { config, style }),
            );
            series_inputs.push(series);
            stamps.push(PreparedChartStamp {
                chart_id: item.chart_id,
                desired: state.revisions.desired,
                data: state.revisions.data,
            });
        }
        let mut draws = Vec::new();
        draws.try_reserve_exact(items.len()).map_err(allocation)?;
        for (item, series) in items.iter().zip(&series_inputs) {
            let state = states
                .get(&item.chart_id)
                .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
            let chart_config = match state.display.as_ref().and_then(|d| d.placed.as_ref()) {
                Some(placed) => &placed.config,
                None => &state.config,
            };
            draws.push(PanelDraw {
                view: item.view,
                chart_config,
                series,
            });
        }
        let mut frame = preparation.prepare(&draws)?;
        frame.registered_charts = stamps;
        Ok(frame)
    }

    /// Select the current resident or owned streaming display for each chart
    /// from its explicit id. Paint receives only the prepared token.
    pub(crate) fn prepare_registered_mixed(
        &mut self,
        items: &[RegisteredPanel<'_>],
    ) -> streaming_runtime::StreamResult<PreparedFrame> {
        let allocation =
            |error: std::collections::TryReserveError| FiggyError::StateAllocationFailed {
                resource: "mixed chart prepare inputs",
                reason: error.to_string(),
            };
        for item in items {
            self.admit_chart_shaders(item.chart_id);
        }
        let mut resident = Vec::new();
        let mut displays = Vec::new();
        let mut streamed = Vec::new();
        // Prepared items a resident chart contributes: one per panel of its
        // layered document (`expand_for_render`); zero for a streamed chart.
        let mut panel_counts = Vec::new();
        resident
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        displays
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        streamed
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        panel_counts
            .try_reserve_exact(items.len())
            .map_err(allocation)?;
        self.sync_external_invalidations()?;
        self.service_stream_requests();
        for item in items {
            let config = self.chart_config(item.chart_id)?;
            let active = self.chart_has_stream_surface(item.chart_id);
            panel_counts.push(if active {
                0
            } else {
                render_panel_count(config)?
            });
            if active {
                self.preflight_chart_stream_display(item.chart_id, item.view)?;
            }
            if !active {
                resident.push(RegisteredPanel {
                    chart_id: item.chart_id,
                    view: item.view,
                });
            }
            streamed.push(active);
            displays.push(None);
        }
        let mut frame = self.prepare_registered_resident(&resident)?;
        for ((item, active), display) in items.iter().zip(streamed).zip(&mut displays) {
            if active {
                *display = Some(
                    self.prepare_chart_stream_display(item.chart_id, item.view)?
                        .ok_or(crate::streaming::StreamError::WrongState)?,
                );
            }
        }
        let mut resident_items = std::mem::take(&mut frame.items).into_iter();
        frame
            .items
            .try_reserve_exact(items.len() - resident.len() + panel_counts.iter().sum::<usize>())
            .map_err(allocation)?;
        frame
            .registered_charts
            .try_reserve_exact(items.len() - resident.len())
            .map_err(allocation)?;
        for (item, (display, count)) in items.iter().zip(displays.into_iter().zip(panel_counts)) {
            if let Some(display) = display {
                let state = self
                    .chart_states
                    .get(&item.chart_id)
                    .ok_or(FiggyError::UnknownChart { id: item.chart_id })?;
                frame.registered_charts.push(PreparedChartStamp {
                    chart_id: item.chart_id,
                    desired: state.revisions.desired,
                    data: state.revisions.data,
                });
                frame.items.push(PreparedItem {
                    view: PreparedView::capture(
                        item.view,
                        item.view
                            .content_revision
                            .load(std::sync::atomic::Ordering::Acquire),
                        state.config.annotations.visible,
                    ),
                    data_area: state
                        .config
                        .data_area()
                        .map(|area| area.0)
                        .unwrap_or(item.view.panel_rect),
                    // A streamed chart has no breaks (rule 8).
                    piece_transforms: no_piece_transforms(),
                    data_pieces: super::composite::DataPieces::whole(
                        state
                            .config
                            .data_area()
                            .map(|area| area.0)
                            .unwrap_or(item.view.panel_rect),
                    ),
                    axis_pipeline: self.pipelines.axis.clone(),
                    series: Vec::new(),
                    stream_display: Some(display),
                    resident_display: None,
                    column_fingerprint: 0,
                });
            } else {
                for _ in 0..count {
                    frame.items.push(resident_items.next().ok_or(
                        streaming_runtime::StreamRequestError::Scheduler(
                            crate::streaming::StreamError::WrongState,
                        ),
                    )?);
                }
            }
        }
        Ok(frame)
    }
}

impl Renderer {
    /// Whether the series at `index` of the registered chart `state` can be
    /// drawn in this frame without creating a pipeline the browser has not
    /// compiled yet (#299). A series whose pipelines are still waiting or
    /// compiling in the renderer's background queue is left out, and those
    /// compiles are moved ahead of the others, so the frame draws everything
    /// else at once and the chart fills in as they resolve. Natively, and
    /// for a renderer without a queue, every series is ready: its pipelines
    /// are created when it is drawn.
    #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
    pub(in crate::renderer) fn registered_series_compiled(
        &self,
        item: &RegisteredPanel<'_>,
        state: &ChartRenderState,
        index: usize,
    ) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            let Some(queue) = self
                .compile_queue
                .as_ref()
                .filter(|queue| queue.serves(self.surface_format, self.target_sample_count))
            else {
                return true;
            };
            let styles = match &state.display {
                Some(display) => display.styles.as_ref().map(|(_, styles)| styles),
                None => state.prepared_styles.as_ref(),
            };
            let (Some(styles), Some(config)) = (styles, state.series.get(index)) else {
                return true;
            };
            let Some(style) = styles.styles.get(index) else {
                return true;
            };
            let chart_config = match state.display.as_ref().and_then(|d| d.placed.as_ref()) {
                Some(placed) => &placed.config,
                None => &state.config,
            };
            let set = self.pipelines.shaders.set;
            let mut ready = true;
            series_pipeline_labels(
                item.view,
                chart_config,
                &Series { config, style },
                |label| {
                    use crate::compile_schedule::LabelReadiness;
                    match queue.readiness(set, label) {
                        // Waiting or compiling: leave the series out and
                        // issue its compiles first. They end in some
                        // outcome, which wakes the chart to draw again.
                        LabelReadiness::Pending => {
                            ready = false;
                            queue.prioritize(set, label);
                        }
                        // Compiled: the creation is a cache hit.
                        LabelReadiness::Ready => {}
                        // Ended otherwise (rejected, device lost, retired,
                        // cancelled): draw, so the creation reports the
                        // error through the usual path; never wait on it.
                        LabelReadiness::Ended => {}
                        // Not in the queue yet: a vector, 3D surface or
                        // ternary region module is scheduled now, ahead of
                        // the rest, and the series waits for it like any
                        // other; anything else the draw creates as before.
                        LabelReadiness::Untracked => match queue.schedule_on_demand(set, label) {
                            Ok(true) => ready = false,
                            Ok(false) => {}
                            Err(_) => {
                                // Not schedulable: the draw creates the
                                // pipeline, which reports any error through
                                // the usual path.
                            }
                        },
                    }
                    queue.mark_wanted(set, label);
                },
            );
            ready
        }
        #[cfg(not(target_arch = "wasm32"))]
        true
    }

    /// Whether the last screen frame drew every series it was handed
    /// (#299). False while a series waits for its pipelines' background
    /// compile in a browser; always true natively.
    pub fn frame_complete(&self) -> bool {
        self.frame_complete
    }

    /// The renderer's background pipeline compiles (#299): how many of its
    /// shader set's render pipelines are scheduled, compiled, compiling and
    /// failed, for the last screen frame (`screen`) and in all (`all`), and
    /// whether the last screen frame was complete. Cheap: no GPU call.
    pub fn pipeline_status(&self) -> PipelineStatus {
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
        let mut status = PipelineStatus {
            frame_complete: self.frame_complete,
            ..PipelineStatus::default()
        };
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            use crate::compile_schedule::{Counts, Scope};
            let counts = |scope| {
                let Counts {
                    total,
                    ready,
                    compiling,
                    failed,
                } = queue.counts(self.pipelines.shaders.set, scope);
                PipelineCounts {
                    total,
                    ready,
                    compiling,
                    failed,
                }
            };
            status.screen = counts(Scope::Screen);
            status.all = counts(Scope::All);
        }
        status
    }

    /// Every background compile of the renderer's shader set that ended
    /// other than compiled (#299), in schedule order: rejected by the
    /// browser, cut short by device loss, or cancelled. Empty natively.
    pub fn pipeline_failures(&self) -> Vec<PipelineFailure> {
        #[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
        let mut failures = Vec::new();
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            queue.for_each_failure(self.pipelines.shaders.set, |label, outcome| {
                failures.push(pipeline_failure(label, outcome));
            });
        }
        failures
    }

    /// Resolves once every background compile the last screen frame needed
    /// has ended (#299): a frame drawn then draws every series. It wakes on
    /// each compile's ending, never by polling, and always ends. `Err` is the
    /// first of those compiles that ended other than compiled; the frame
    /// still draws its series, creating the pipeline, which reports the
    /// error through the usual path. Natively, and with nothing pending, it
    /// resolves at once.
    pub async fn series_ready(&self) -> std::result::Result<(), PipelineFailure> {
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            return queue
                .wait_scope(
                    self.pipelines.shaders.set,
                    crate::compile_schedule::Scope::Screen,
                )
                .await
                .map_err(|(label, outcome)| pipeline_failure(&label, &outcome));
        }
        Ok(())
    }

    /// Resolves once every background compile scheduled so far has ended
    /// (#299). `Err` is the first that ended other than compiled; see
    /// [`Self::pipeline_failures`] for all of them. It always ends.
    pub async fn pipelines_ready(&self) -> std::result::Result<(), PipelineFailure> {
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.compile_queue {
            return queue
                .wait_scope(
                    self.pipelines.shaders.set,
                    crate::compile_schedule::Scope::All,
                )
                .await
                .map_err(|(label, outcome)| pipeline_failure(&label, &outcome));
        }
        Ok(())
    }

    /// A handle on this renderer's background compiles that does not
    /// borrow it (#315): see [`PipelineReadiness`]. Cheap: no GPU call.
    pub fn pipeline_readiness(&self) -> PipelineReadiness {
        PipelineReadiness {
            #[cfg(target_arch = "wasm32")]
            queue: self
                .compile_queue
                .as_ref()
                .map(crate::browser_compile::CompileQueue::downgrade),
        }
    }

    /// Whether a background compile has ended since the last screen frame
    /// left a series out (#299): a host loop that draws only when something
    /// changed draws again, so the chart fills in without a host call.
    pub fn compile_redraw_due(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            !self.frame_complete
                && self
                    .compile_queue
                    .as_ref()
                    .is_some_and(|queue| queue.settled() != self.frame_settled)
        }
        #[cfg(not(target_arch = "wasm32"))]
        false
    }

    /// Resolves when a background compile next changes state (#299), or at
    /// once when none is waiting or compiling. A chart that left a series
    /// out awaits this, then draws again. Every compile ends (ready,
    /// rejected, device lost, superseded or cancelled), so this never waits
    /// for good while a series is left out.
    #[cfg(target_arch = "wasm32")]
    pub async fn compile_progress(&self) {
        if let Some(queue) = &self.compile_queue {
            queue.changed().await;
        }
    }

    /// Whether any background compile is waiting or in flight (#299).
    pub fn compiles_pending(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            self.compile_queue
                .as_ref()
                .is_some_and(crate::browser_compile::CompileQueue::busy)
        }
        #[cfg(not(target_arch = "wasm32"))]
        false
    }
}

impl PipelineReadiness {
    /// The renderer's background compiles now, read without borrowing it
    /// (#315). Cheap: no GPU call.
    pub fn status(&self) -> ReadinessStatus {
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.queue {
            use crate::compile_schedule::{Counts, Scope};
            let counts = |scope| {
                queue.counts(scope).map(
                    |Counts {
                         total,
                         ready,
                         compiling,
                         failed,
                     }| PipelineCounts {
                        total,
                        ready,
                        compiling,
                        failed,
                    },
                )
            };
            return match (counts(Scope::Screen), counts(Scope::All)) {
                (Some(screen), Some(all)) => ReadinessStatus {
                    screen,
                    all,
                    renderer_dropped: false,
                },
                _ => ReadinessStatus {
                    renderer_dropped: true,
                    ..ReadinessStatus::default()
                },
            };
        }
        ReadinessStatus::default()
    }

    /// [`Renderer::series_ready`] without borrowing the renderer (#315):
    /// resolves once every background compile the renderer's last screen
    /// frame needed has ended. It draws nothing; the host's next frame
    /// draws the series. `Err` is the first that failed, or
    /// [`ReadinessError::RendererDropped`] once the renderer is gone.
    pub async fn series_ready(&self) -> std::result::Result<(), ReadinessError> {
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.queue {
            return wait_readiness(queue, crate::compile_schedule::Scope::Screen).await;
        }
        Ok(())
    }

    /// [`Renderer::pipelines_ready`] without borrowing the renderer (#315):
    /// resolves once every background compile scheduled so far has ended.
    /// `Err` as for [`Self::series_ready`].
    pub async fn pipelines_ready(&self) -> std::result::Result<(), ReadinessError> {
        #[cfg(target_arch = "wasm32")]
        if let Some(queue) = &self.queue {
            return wait_readiness(queue, crate::compile_schedule::Scope::All).await;
        }
        Ok(())
    }
}

/// Wait on a weakly held queue (#315), mapping how the wait ended.
#[cfg(target_arch = "wasm32")]
async fn wait_readiness(
    queue: &crate::browser_compile::WeakCompileQueue,
    scope: crate::compile_schedule::Scope,
) -> std::result::Result<(), ReadinessError> {
    use crate::browser_compile::WeakWaitEnd;
    queue.wait_scope(scope).await.map_err(|end| match end {
        WeakWaitEnd::Failed(label, outcome) => {
            ReadinessError::Failed(pipeline_failure(&label, &outcome))
        }
        WeakWaitEnd::Dropped => ReadinessError::RendererDropped,
    })
}

/// A compile that ended as `outcome`, as the public failure. Only an
/// ending other than compiled, superseded or retired reaches here.
#[cfg(target_arch = "wasm32")]
fn pipeline_failure(
    label: &str,
    outcome: &crate::compile_schedule::CompileOutcome,
) -> PipelineFailure {
    use crate::compile_schedule::{CompileOutcome, RejectReason};
    let (kind, message) = match outcome {
        CompileOutcome::Rejected { reason, message } => (
            match reason {
                RejectReason::Validation => PipelineFailureKind::Validation,
                RejectReason::Internal => PipelineFailureKind::Internal,
                RejectReason::Other => PipelineFailureKind::Rejected,
            },
            message.clone(),
        ),
        CompileOutcome::DeviceLost => (
            PipelineFailureKind::DeviceLost,
            "the GPU device was lost before the pipeline compiled".to_owned(),
        ),
        CompileOutcome::Cancelled
        | CompileOutcome::Ready
        | CompileOutcome::Superseded { .. }
        | CompileOutcome::Retired => (
            PipelineFailureKind::Cancelled,
            "the renderer was dropped before the pipeline compiled".to_owned(),
        ),
    };
    PipelineFailure {
        label: label.to_owned(),
        kind,
        message,
    }
}
