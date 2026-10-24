//! `FiggyChart`: construction, lifecycle, the document (config and
//! series), and the frame loop.

use super::*;

pub(super) const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

pub(super) fn emit_init_progress(on_event: Option<&js_sys::Function>, event: InitEvent) {
    let Some(callback) = on_event else {
        return;
    };
    let payload = js_sys::Object::new();
    let phase = match event.phase {
        InitPhase::Started => "started",
        InitPhase::Finished => "finished",
    };
    let _ = js_sys::Reflect::set(&payload, &"scope".into(), &JsValue::from_str(event.scope));
    let _ = js_sys::Reflect::set(&payload, &"stage".into(), &JsValue::from_str(event.stage));
    let _ = js_sys::Reflect::set(&payload, &"phase".into(), &JsValue::from_str(phase));
    let _ = callback.call1(&JsValue::UNDEFINED, &payload);
}

pub(super) async fn create_chart_kernel(
    canvas: HtmlCanvasElement,
    on_event: Option<js_sys::Function>,
) -> Result<FiggyChart, JsValue> {
    console_error_panic_hook::set_once();

    let (mut renderer, w, h) = {
        let on_event_ref = on_event.as_ref();
        let mut observer = |event| emit_init_progress(on_event_ref, event);
        let (w, h) = (canvas.width().max(1), canvas.height().max(1));
        let mut renderer = Renderer::for_window_async_observed(
            wgpu::SurfaceTarget::Canvas(canvas),
            (w, h),
            POOL_CAPACITY,
            &mut observer,
        )
        .await
        .map_err(js_err)?;
        renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
        (renderer, w, h)
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Started),
    );

    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });
    let chart_id = renderer
        .register_chart(config.clone(), Vec::new())
        .map_err(js_err)?;
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(
            &chart,
            Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
        )
        .map_err(js_err)?;

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Finished),
    );
    let mut chart = FiggyChart {
        renderer,
        chart_id,
        view,
        surface_size: (w, h),
        styles: Vec::new(),
        labels: Vec::new(),
        legend_auto_managed: true,
        columns: HashMap::new(),
        column_revisions: HashMap::new(),
        next_column_revision: 1,
        pending_stream_handoff: None,
        series_extents: HashMap::new(),
        alive: Rc::new(Cell::new(true)),
        color_seq: 0,
        hitmap: HitMap::standard_chart(),
        hitmap_layer: (renderer::LayerRef::Base, 0, None),
        dragging: false,
        drag_pointer: [0.0; 2],
        resizing: None,
        cycle: renderer::ColorCycle::Classic,
        clear_color: Color::WHITE,
        view_dirty: false,
        redraw_pending: true,
        last_presented_stamp: None,
        selected_annotation: None,
        annotation_selection_dirty: false,
        annotation_pointer: false,
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Started),
    );
    chart.first_frame_ready().await?;
    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Finished),
    );
    Ok(chart)
}

/// Parameter metadata for one `draw_style` mode — a JSON array of
/// `{key, min, max, default, integer}`. Ranges are the RECOMMENDED
/// slider spans (the SSoT accepts values beyond them; the renderer
/// applies only safety guards), and they come from the model crate, so
/// hosts never hardcode them.
///
/// JS: `const specs = JSON.parse(draw_style_param_specs("constellation"));`
/// Valid modes: `draw_style_modes()`.
#[wasm_bindgen]
pub fn draw_style_param_specs(mode: &str) -> Result<String, JsValue> {
    let specs = renderer::config::DrawStyle::param_specs_for_mode(mode).ok_or_else(|| {
        js_err(format!(
            "unknown draw_style mode {mode:?} (valid: {})",
            renderer::config::DrawStyle::mode_tags().join(", ")
        ))
    })?;
    serde_json::to_string(specs).map_err(js_err)
}

/// Every valid `draw_style` mode tag, as a JSON string array.
#[wasm_bindgen]
pub fn draw_style_modes() -> Result<String, JsValue> {
    serde_json::to_string(renderer::config::DrawStyle::mode_tags()).map_err(js_err)
}

pub(super) struct PreparedSeriesExtentCache {
    pub(super) extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
    pub(super) ticket_jobs: Vec<(renderer::GpuSeriesExtentTicket, Rc<SeriesExtentJob>)>,
}

// ------------------------------------------------------------------
// FiggyChart — low-level wasm kernel bound to one canvas.
// ------------------------------------------------------------------

#[wasm_bindgen]
pub struct FiggyChart {
    pub(super) renderer: WindowedRenderer<'static>,
    pub(super) chart_id: ChartId,
    pub(super) view: ChartView,
    /// Current WebGPU surface size in physical canvas pixels. This is a
    /// viewport property, not the exported document size.
    pub(super) surface_size: (u32, u32),
    pub(super) styles: Vec<ChartStyle>,
    /// 1:1 with renderer-owned series order. Legacy plain-label fallback
    /// for explicit legend reset; rendered series/config remain the SSoT.
    pub(super) labels: Vec<Option<String>>,
    /// True while the legend follows the wrapper's auto row layout.
    /// Direct `set_config` legend edits turn this off so later series
    /// changes preserve user-authored text instead of deleting rows.
    pub(super) legend_auto_managed: bool,
    /// Registered columns: id to logical value count.
    pub(super) columns: HashMap<String, usize>,
    /// Successful changed-upload revision for each live GPU column.
    pub(super) column_revisions: HashMap<String, u64>,
    pub(super) next_column_revision: u64,
    /// Prepared wrapper metadata for a core handoff that is not live
    /// until the renderer reports its atomic completion.
    pub(super) pending_stream_handoff: Option<PendingStreamHandoffMetadata>,
    /// Already-submitted exact GPU drawable-domain reductions. Cache
    /// identity includes the normalized primitive mode and every active
    /// role-specific column revision.
    pub(super) series_extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
    pub(super) alive: Rc<Cell<bool>>,
    /// Monotonic color assignment for newly registered series.
    pub(super) color_seq: usize,
    pub(super) hitmap: HitMap,
    /// The (active layer, overlay count) the hit map was registered for.
    /// Either change rebuilds it with `HitMap::for_active_layer`.
    pub(super) hitmap_layer: (renderer::LayerRef, usize, Option<usize>),
    pub(super) dragging: bool,
    /// The pointer in document coordinates while a chart element drags:
    /// the press point advanced by every move. Polar axes and tick
    /// labels move relative to it.
    pub(super) drag_pointer: [f32; 2],
    pub(super) resizing: Option<ModelResizeHandle>,
    pub(super) cycle: renderer::ColorCycle,
    pub(super) clear_color: Color,
    pub(super) view_dirty: bool,
    /// Host-owned surface state not represented by renderer chart
    /// revisions (currently the clear color). Starts true so the initial
    /// surface frame is never skipped.
    pub(super) redraw_pending: bool,
    /// Renderer-issued identity of the state last presented onscreen.
    pub(super) last_presented_stamp: Option<ChartRenderStamp>,
    /// The annotation whose selection box the overlay draws. Host state
    /// like the decoration selection; not part of the config JSON.
    pub(super) selected_annotation: Option<usize>,
    /// The annotation selection changed since the overlay was drawn.
    pub(super) annotation_selection_dirty: bool,
    /// The pointer press that started the current drag or handle drag
    /// landed on the selected annotation (the annotation layer's route),
    /// so `on_move` edits the annotation instead of a chart element.
    pub(super) annotation_pointer: bool,
}

impl FiggyChart {
    pub(super) fn chart_config(&self) -> &renderer::Config {
        self.renderer
            .chart_config(self.chart_id)
            .expect("FiggyChart keeps its renderer-owned chart registered")
    }

    pub(super) fn chart_series(&self) -> &[SeriesConfig] {
        self.renderer
            .chart_series(self.chart_id)
            .expect("FiggyChart keeps its renderer-owned chart registered")
    }

    pub(super) fn chart_selection(&self) -> Option<HitId> {
        self.renderer
            .chart_selection(self.chart_id)
            .expect("FiggyChart keeps its renderer-owned chart registered")
    }

    pub(super) fn replace_chart_config(&mut self, config: renderer::Config) -> Result<(), JsValue> {
        self.renderer
            .set_chart_config(self.chart_id, config)
            .map_err(js_err)?;
        self.sync_hitmap();
        Ok(())
    }

    pub(super) fn replace_chart_state(
        &mut self,
        config: renderer::Config,
        series: Vec<SeriesConfig>,
    ) -> Result<(), JsValue> {
        self.renderer
            .set_chart_state(self.chart_id, config, series)
            .map_err(js_err)?;
        self.sync_hitmap();
        Ok(())
    }

    /// The hit map's registration set depends only on the active layer,
    /// the overlay count and the active subplot's overlay count. Rebuild
    /// it when any of them changed; the registry has already dropped any
    /// `HitId` of the previous map. A layer change also clears the
    /// annotation selection: it is the element selection of the
    /// annotation layer, one level below the layer like the registry's
    /// `HitId`.
    pub(super) fn sync_hitmap(&mut self) {
        let config = self.chart_config();
        let layer = config.active_layer();
        let subplot_overlays = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map(|subplot| subplot.config.overlays.len())
            }
            _ => None,
        };
        let key = (layer, config.overlays.len(), subplot_overlays);
        if key != self.hitmap_layer {
            let layer_changed = key.0 != self.hitmap_layer.0;
            self.hitmap = HitMap::for_active_layer(config);
            self.hitmap_layer = key;
            self.dragging = false;
            self.resizing = None;
            self.annotation_pointer = false;
            if layer_changed {
                self.set_selected_annotation(None);
            }
        }
    }

    /// The display scale and where the whole document lands on the
    /// surface.
    pub(super) fn display_scale_and_panel(&self) -> (f32, Rect) {
        let logical = self.chart_config().document_area();
        renderer::fit_display_panel((logical.width, logical.height), self.surface_size)
    }

    pub(super) fn display_config(&self) -> (renderer::config::Config, Rect, f32) {
        renderer::display_config_for_surface(self.chart_config(), self.surface_size)
    }

    pub(super) fn display_delta_to_document(&self, dx: f32, dy: f32) -> (f32, f32) {
        let (scale, _) = self.display_scale_and_panel();
        if scale > 0.0 {
            (dx / scale, dy / scale)
        } else {
            (dx, dy)
        }
    }

    /// A display point in document coordinates: the inverse of
    /// `display_config_for_surface`'s scale and placement.
    pub(super) fn display_point_to_document(&self, x: f32, y: f32) -> [f32; 2] {
        let (scale, panel) = self.display_scale_and_panel();
        let logical = self.chart_config().document_area();
        if scale > 0.0 {
            [
                logical.x as f32 + (x - panel.x as f32) / scale,
                logical.y as f32 + (y - panel.y as f32) / scale,
            ]
        } else {
            [x, y]
        }
    }

    pub(super) fn label_text(&self, label: &str) -> RichText {
        let content = &self.chart_config().legend.content;
        RichText {
            segments: rich_segments_from_text(label),
            color: content.color,
            font_size: content.font_size,
            font: content.font.clone(),
            align: content.align,
        }
    }

    pub(super) fn rich_text_to_plain(rt: &RichText) -> String {
        rt.segments.iter().map(|s| s.text).collect()
    }

    pub(super) fn fallback_label(&self, i: usize) -> Option<RichText> {
        self.chart_series()
            .get(i)
            .and_then(|cfg| cfg.label.clone())
            .or_else(|| self.labels.get(i)?.as_ref().map(|s| self.label_text(s)))
    }

    /// Explicit reset: rebuild every auto legend row from SeriesConfig.label
    /// first, falling back to the wrapper's legacy string labels.
    pub(super) fn rebuild_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
        let entries: Vec<(SeriesConfig, RichText)> = self
            .chart_series()
            .iter()
            .cloned()
            .enumerate()
            .filter_map(|(i, cfg)| Some((cfg, self.fallback_label(i)?)))
            .collect();

        let mut config = self.chart_config().clone();
        config.legend.content.segments.clear();
        for (cfg, label) in entries {
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(
                    &cfg,
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
                label.segments,
            );
        }
        config.legend.visible = !config.legend.content.segments.is_empty();
        self.replace_chart_config(config)?;
        self.legend_auto_managed = true;
        Ok(())
    }

    pub(super) fn styles_for_surface(
        &mut self,
        surface_size: (u32, u32),
    ) -> Result<Vec<ChartStyle>, JsValue> {
        let logical = self.chart_config().document_area();
        let (scale, _) = renderer::fit_display_panel((logical.width, logical.height), surface_size);
        let series = self.chart_series().to_vec();
        series
            .iter()
            .map(|cfg| {
                self.renderer
                    .create_style_for_series_scaled(cfg, scale)
                    .map_err(js_err)
            })
            .collect()
    }

    pub(super) fn request_host_redraw(&mut self) {
        self.redraw_pending = true;
    }
    pub(super) fn publish_series_extents_if_engine_ready(
        &mut self,
        series_cfgs: &[SeriesConfig],
        revisions: &HashMap<String, u64>,
        retry_failed: bool,
    ) -> Result<(), JsValue> {
        if !self.renderer.errorbar_extent_engine_ready() {
            return Ok(());
        }
        let prepared = self.prepare_series_extent_cache(series_cfgs, revisions, retry_failed)?;
        self.publish_series_extent_cache(prepared);
        Ok(())
    }

    pub(super) fn prepare_series_extent_cache(
        &self,
        series_cfgs: &[SeriesConfig],
        revisions: &HashMap<String, u64>,
        retry_failed: bool,
    ) -> Result<PreparedSeriesExtentCache, JsValue> {
        let mut resident_series = Vec::new();
        resident_series
            .try_reserve(series_cfgs.len())
            .map_err(js_err)?;
        resident_series.extend(
            series_cfgs
                .iter()
                .filter(|series| {
                    referenced_columns(series).into_iter().all(|id| {
                        matches!(
                            self.renderer.logical_column(id),
                            Some(renderer::LogicalColumn::Resident(_))
                        )
                    })
                })
                .cloned(),
        );
        let requests =
            active_series_extent_requests(&resident_series, revisions).map_err(js_err)?;
        let (extents, pending) =
            select_series_extent_job_map(requests, &self.series_extents, retry_failed)
                .map_err(js_err)?;
        let mut ticket_jobs = Vec::new();
        ticket_jobs.try_reserve(pending.len()).map_err(js_err)?;

        for request in pending {
            let job = extents.get(&request.key).cloned().ok_or_else(|| {
                js_err("pending series extent key lost its prepared job before publication")
            })?;
            match self.renderer.begin_series_fit_extent(&request.series) {
                Ok(Some(ticket)) => {
                    #[cfg(test)]
                    record_series_extent_submit();
                    ticket_jobs.push((ticket, job));
                }
                Ok(None) => job.complete_terminal_failure(
                    "active GPU extent request resolved to a metadata-only series".into(),
                ),
                Err(error) => job.complete_terminal_failure(error.to_string()),
            }
        }

        Ok(PreparedSeriesExtentCache {
            extents,
            ticket_jobs,
        })
    }

    pub(super) fn publish_series_extent_cache(&mut self, prepared: PreparedSeriesExtentCache) {
        self.series_extents = prepared.extents;
        for (ticket, job) in prepared.ticket_jobs {
            Self::spawn_series_extent(ticket, job);
        }
    }

    /// Removing series cannot create a new extent identity. Retire stale
    /// jobs without allocating or submitting new GPU work.
    pub(super) fn retire_series_extent_cache(&mut self) {
        let series_cfgs = self
            .renderer
            .chart_series(self.chart_id)
            .expect("FiggyChart keeps its renderer-owned chart registered");
        let revisions = &self.column_revisions;
        self.series_extents.retain(|key, _| {
            series_cfgs
                .iter()
                .any(|cfg| series_extent_key_matches_config(key, cfg, revisions))
        });
    }

    pub(super) fn spawn_series_extent(
        ticket: renderer::GpuSeriesExtentTicket,
        job: Rc<SeriesExtentJob>,
    ) {
        spawn_local(async move {
            match ticket.resolve().await {
                Ok(extent) => job.complete_success(extent.map(|extent| SeriesFitExtent {
                    x: FitExtent {
                        min: extent.x.min,
                        max: extent.x.max,
                        min_positive: extent.x.min_positive,
                    },
                    y: FitExtent {
                        min: extent.y.min,
                        max: extent.y.max,
                        min_positive: extent.y.min_positive,
                    },
                })),
                Err(error) => job.complete_retryable_failure(error.to_string()),
            }
        });
    }

    pub(super) fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
        for id in referenced_columns(cfg) {
            if self.renderer.logical_column(id).is_none() {
                return Err(js_err(format!(
                    "series '{}' references unregistered column '{id}'",
                    cfg.series_id
                )));
            }
        }
        Ok(())
    }
}

impl Drop for FiggyChart {
    fn drop(&mut self) {
        self.alive.set(false);
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// Bind the low-level chart kernel to `canvas` (uses the canvas's
    /// current pixel size). Ordinary web hosts should prefer
    /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
    /// JS: `const chart = await FiggyChart.create(canvas);`
    pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, None).await
    }

    /// Same as [`Self::create`], reporting each init stage to `on_event`
    /// as `{ scope, stage, phase }`. Do not call other kernel methods
    /// from the callback.
    pub async fn create_with_progress(
        canvas: HtmlCanvasElement,
        on_event: js_sys::Function,
    ) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, Some(on_event)).await
    }

    /// Register a font (TTF/OTF/TTC bytes) for SSoT `font` family names.
    /// Returns the family names the file declares — use them verbatim in
    /// `content.font` / label styles. Registered fonts win over native
    /// system fonts. A character the chosen face cannot draw falls back
    /// per character to registered fonts (in registration order), then
    /// the bundled Liberation Sans, then, on desktop only, installed
    /// system fonts. Resolution through registered and bundled faces is
    /// therefore identical on web and desktop; installed system fonts
    /// are a desktop-only fallback. Already-drawn text re-rasterizes on
    /// the next `frame()`.
    ///
    /// JS: `chart.register_font(new Uint8Array(await (await fetch(url)).arrayBuffer()))`
    pub fn register_font(&mut self, bytes: &[u8]) -> Result<Vec<String>, JsValue> {
        let families =
            renderer::text_render::register_font_bytes(bytes.to_vec()).map_err(js_err)?;
        // Text may already be on screen in the fallback font — force a
        // decoration re-raster so the registration is visible.
        Ok(families)
    }

    /// Opt in to pool growth on an otherwise out-of-space upload. Growth
    /// remains bounded by the device storage-binding limit and the
    /// configured renderer memory budget.
    pub fn set_pool_auto_growth(&mut self, enabled: bool) {
        self.renderer.set_pool_growth_policy(if enabled {
            GrowthPolicy::OnAllocFailure
        } else {
            GrowthPolicy::Fixed
        });
    }

    /// Read-only accounting. These are requested GPU allocations, not a
    /// driver VRAM measurement; retired bytes remain charged until the
    /// queue completion callback fires.
    pub fn gpu_memory_status(&self) -> String {
        let usage = self.renderer.gpu_memory_usage();
        let pool = self.renderer.pool();
        let mut sources: Vec<_> = self
            .columns
            .iter()
            .map(|(id, &length)| {
                let (residency, encoded_bytes) = match self.renderer.logical_column(id) {
                    Some(renderer::LogicalColumn::Resident(slot)) => {
                        ("resident", Some(slot.byte_size))
                    }
                    Some(renderer::LogicalColumn::Streamed(_)) => ("streamed", None),
                    None => ("missing", None),
                };
                serde_json::json!({
                    "id": id,
                    "length": length,
                    "revision": self.column_revisions.get(id).map(u64::to_string),
                    "residency": residency,
                    "encoded_bytes": encoded_bytes,
                })
            })
            .collect();
        sources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let resources: Vec<_> = GpuResourceKind::ALL
            .into_iter()
            .map(|kind| {
                serde_json::json!({
                    "kind": kind.label(),
                    "live_bytes": usage.live_bytes_of(kind),
                    "retired_bytes": usage.retired_bytes_of(kind),
                })
            })
            .collect();
        serde_json::json!({
            "total_bytes": usage.total_bytes(),
            "live_bytes": usage.live_bytes(),
            "retired_bytes": usage.retired_bytes(),
            "memory_budget_bytes": self.renderer.memory_budget(),
            "pool": {
                "capacity_bytes": pool.capacity(),
                "used_bytes": pool.used_bytes(),
                "free_bytes": pool.free_bytes(),
                "largest_free_bytes": pool.largest_free_region(),
                "backup_bytes": pool.backup_bytes(),
                "retired_bytes": pool.retired_bytes(),
                "auto_growth": pool.growth_policy == GrowthPolicy::OnAllocFailure,
            },
            "resources": resources,
            "sources": sources,
        })
        .to_string()
    }

    /// Compact live resident columns, drop the unused pool slab, and wait
    /// for the device completion fence before reporting reclaimed bytes.
    /// The 16 MiB startup slab remains as the minimum reusable pool.
    pub async fn release_unused_gpu_memory(&mut self) -> Result<String, JsValue> {
        let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        if matches!(
            status.status,
            renderer::StreamingState::Active
                | renderer::StreamingState::AllSubmitted
                | renderer::StreamingState::Cancelling
        ) {
            return Err(js_err(
                "finish or cancel the active stream before GPU pool cleanup",
            ));
        }
        self.renderer
            .release_unused_pool_memory(POOL_CAPACITY)
            .await
            .map_err(column_registration_error)?;
        Ok(self.gpu_memory_status())
    }

    // ---- series registry (id-keyed upsert / unregister) ----

    /// Register or update a line series over two registered columns.
    ///
    /// Upsert by `series_id`: a new id takes the next color of the active
    /// cycle; an existing id is replaced in place and keeps its color.
    /// Non-empty `label` adds/updates the legend row.
    pub fn add_line_series(
        &mut self,
        series_id: &str,
        x_column: &str,
        y_column: &str,
        line_width: f32,
        label: &str,
    ) -> Result<(), JsValue> {
        let current = self.chart_series().to_vec();
        let existing = current.iter().position(|c| c.series_id == series_id);
        let color = match existing {
            Some(i) => match &current[i].render_type {
                DataRenderType::Line { line } => line.line_color,
                _ => self.cycle.color(i),
            },
            None => self.cycle.color(self.color_seq),
        };
        let next_color_seq = if existing.is_none() {
            self.color_seq
                .checked_add(1)
                .ok_or_else(|| js_err("series color sequence exhausted"))?
        } else {
            self.color_seq
        };
        let label_changed = !label.is_empty();
        let rich_label = if label_changed {
            Some(self.label_text(label))
        } else {
            existing.and_then(|i| current[i].label.clone())
        };
        let plain_label = if label_changed {
            Some(label.to_string())
        } else {
            existing.and_then(|i| self.labels[i].clone())
        };

        let cfg = SeriesConfig {
            series_id: series_id.into(),
            source_id: None,
            label: rich_label.clone(),
            x_column: x_column.into(),
            y_column: y_column.into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: color,
                    line_width: line_width.max(0.5),
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
            },
        };
        self.ensure_columns_exist(&cfg)?;
        let (scale, _) = self.display_scale_and_panel();
        let mut proposed = current;
        if let Some(index) = existing {
            proposed[index] = cfg.clone();
        } else {
            proposed.try_reserve(1).map_err(js_err)?;
            proposed.push(cfg.clone());
        }
        let prepared_extents = if self.renderer.errorbar_extent_engine_ready() {
            Some(self.prepare_series_extent_cache(&proposed, &self.column_revisions, false)?)
        } else {
            None
        };
        let mut next_styles = Vec::new();
        next_styles.try_reserve(proposed.len()).map_err(js_err)?;
        for series in &proposed {
            next_styles.push(
                self.renderer
                    .create_style_for_series_scaled(series, scale)
                    .map_err(js_err)?,
            );
        }
        let mut next_labels = self.labels.clone();
        if let Some(index) = existing {
            next_labels[index] = plain_label;
        } else {
            next_labels.try_reserve(1).map_err(js_err)?;
            next_labels.push(plain_label);
        }
        let mut next_config = self.chart_config().clone();
        if label_changed && let Some(label) = rich_label {
            match existing {
                Some(index) => renderer::config::set_legend_entry_label(
                    &mut next_config.legend.content,
                    index,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
                None => renderer::config::append_legend_entry_rich(
                    &mut next_config.legend.content,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
            }
            next_config.legend.visible = true;
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut next_config.legend.content,
                &proposed,
                next_config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }

        self.replace_chart_state(next_config, proposed)?;
        self.styles = next_styles;
        self.labels = next_labels;
        self.color_seq = next_color_seq;
        if let Some(prepared_extents) = prepared_extents {
            self.publish_series_extent_cache(prepared_extents);
        }
        Ok(())
    }

    /// Set / change / remove a series' legend label. `'\n'` breaks lines;
    /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments.
    /// Empty string removes the legend row. Returns `true` when the
    /// series exists.
    pub fn set_series_label(&mut self, series_id: &str, label: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series().to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        let mut config = self.chart_config().clone();
        let mut labels = self.labels.clone();
        if label.is_empty() {
            labels[i] = None;
            series[i].label = None;
            if renderer::config::remove_legend_entry(&mut config.legend.content, i)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            let label = self.label_text(label);
            labels[i] = Some(Self::rich_text_to_plain(&label));
            series[i].label = Some(label.clone());
            renderer::config::set_legend_entry_label(
                &mut config.legend.content,
                i,
                renderer::config::series_symbol_segments(
                    &series[i],
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
                label.segments,
            );
            config.legend.visible = true;
        }
        self.replace_chart_state(config, series)?;
        self.labels = labels;
        Ok(true)
    }

    /// Unregister a series. Columns stay registered. Auto-managed legends
    /// remove its row; freely edited legends preserve their text and only
    /// synchronize recognized series symbols.
    /// Returns `true` when the series existed.
    pub fn remove_series(&mut self, series_id: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series().to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        series.remove(i);
        let mut config = self.chart_config().clone();
        if self.legend_auto_managed {
            if renderer::config::remove_legend_entry(&mut config.legend.content, i)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &series,
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        self.replace_chart_state(config, series)?;
        self.styles.remove(i);
        self.labels.remove(i);
        self.retire_series_extent_cache();
        Ok(true)
    }

    /// Replace one contour series' levels with axis-native nice levels.
    /// The resulting explicit values remain the series SSoT.
    pub fn set_contour_nice_levels(
        &mut self,
        series_id: &str,
        target_count: usize,
        use_colormap_colors: bool,
    ) -> Result<u32, JsValue> {
        let colorbar = self
            .chart_config()
            .colorbar
            .clone()
            .ok_or_else(|| js_err("contour levels need Config.colorbar"))?;
        let mut series = self.chart_series().to_vec();
        let cfg = series
            .iter_mut()
            .find(|cfg| cfg.series_id == series_id)
            .ok_or_else(|| js_err("no series with this id in the chart"))?;
        let contour = match &mut cfg.render_type {
            DataRenderType::Contour { contour, .. }
            | DataRenderType::HeatmapContour { contour, .. } => contour,
            _ => return Err(js_err("series does not have contour levels")),
        };
        contour
            .set_nice_levels(&colorbar.axis, target_count)
            .map_err(|error| js_err(format!("{error:?}")))?;
        if use_colormap_colors {
            contour.set_colormap_colors(&colorbar);
        }
        let count = u32::try_from(contour.levels.len()).map_err(js_err)?;
        let json = serde_json::to_string(&series).map_err(js_err)?;
        self.set_series(&json)?;
        self.request_host_redraw();
        Ok(count)
    }

    /// Report the effective primitive/grid extent currently drawn.
    pub fn series_draw_info(&self, series_id: &str) -> Result<String, JsValue> {
        let info = self
            .renderer
            .series_draw_info(self.chart_id, series_id)
            .map_err(js_err)?;
        serde_json::to_string(&serde_json::json!({
            "drawn_count": info.drawn_count,
            "cols": info.cols,
            "rows": info.rows,
            "truncated": info.truncated,
        }))
        .map_err(js_err)
    }

    // ---- SSoT I/O ----
    //
    // The whole option tree (`Config`) is plain data; these round-trip it
    // as JSON so a host can read it, edit anything — axis scale, tick
    // shape/length, colors, fonts, label text — and hand it back. The
    // standard flow: auto-fit first, then refine via the SSoT.
    // Full schema reference: crates/web/SCHEMA.md.

    /// Serialize the full chart option SSoT to a JSON string.
    /// JS: `const cfg = JSON.parse(chart.get_config());`
    pub fn get_config(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_config()).map_err(js_err)
    }

    /// Replace the whole option SSoT from JSON. Marks everything dirty —
    /// the next `frame()` re-rasters the chrome and refreshes the
    /// transform, exactly like any other config edit.
    /// Use `get_config`, edit only the desired fields, then submit the
    /// complete config here for selective control. For example, writing
    /// only axis `min` / `max` preserves tick and label settings, whereas
    /// the auto-fit helpers derive those settings along with the range.
    /// This call does not auto-fit ranges or fit the title band.
    /// JS: `chart.set_config(JSON.stringify(cfg));`
    pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
        let mut new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
        let old_config = self.chart_config();
        let legend_content_changed = old_config.legend.content != new_cfg.legend.content;
        let colormap_changed = old_config.colorbar.as_ref().map(|bar| &bar.colormap)
            != new_cfg.colorbar.as_ref().map(|bar| &bar.colormap);
        let series = self.chart_series().to_vec();
        // A heatmap's legend mark shows the colour map: an auto-managed
        // legend follows a new map, a host-written one stays as written.
        if colormap_changed && !legend_content_changed && self.legend_auto_managed {
            renderer::config::update_legend_symbols_preserving_text(
                &mut new_cfg.legend.content,
                &series,
                new_cfg.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        let (_, _, scale) = renderer::display_config_for_surface(&new_cfg, self.surface_size);
        let mut new_styles = Vec::new();
        new_styles.try_reserve(series.len()).map_err(js_err)?;
        for cfg in &series {
            new_styles.push(
                self.renderer
                    .create_style_for_series_scaled(cfg, scale)
                    .map_err(js_err)?,
            );
        }
        self.replace_chart_config(new_cfg)?;
        if legend_content_changed {
            self.legend_auto_managed = false;
        }
        self.styles = new_styles;
        Ok(())
    }

    /// Serialize the series declarations (columns, render type, styles).
    pub fn get_series(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_series()).map_err(js_err)
    }

    /// Replace the series declarations from JSON. Column references must
    /// already be registered; GPU styles are rebuilt, legend labels are
    /// kept for series ids that survive.
    ///
    /// `Contour` and `HeatmapContour` accept `contour.levels` lists with
    /// 0 through 1024 entries. A list with 1025 or more entries rejects the
    /// JSON call with a JavaScript exception; the previously committed
    /// chart config, series declarations, and GPU styles remain active.
    /// The WASM facade neither adds a smaller limit nor truncates levels.
    /// Contour labels use the same 1024 capacity for automatic and explicit
    /// placement. `spacing_px` must be finite and positive even for hidden
    /// labels or explicit anchors; rejection preserves the prior state.
    pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
        let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
        for cfg in &new_series {
            self.ensure_columns_exist(cfg)?;
        }
        let old_series = self.chart_series().to_vec();
        let mut new_labels = Vec::new();
        new_labels.try_reserve(new_series.len()).map_err(js_err)?;
        for cfg in &new_series {
            new_labels.push(
                cfg.label
                    .as_ref()
                    .map(Self::rich_text_to_plain)
                    .or_else(|| {
                        old_series
                            .iter()
                            .position(|old| old.series_id == cfg.series_id)
                            .and_then(|i| self.labels[i].clone())
                    }),
            );
        }
        let (scale, _) = self.display_scale_and_panel();
        let mut new_styles = Vec::new();
        new_styles.try_reserve(new_series.len()).map_err(js_err)?;
        for cfg in &new_series {
            new_styles.push(
                self.renderer
                    .create_style_for_series_scaled(cfg, scale)
                    .map_err(js_err)?,
            );
        }
        let mut config = self.chart_config().clone();
        if self.legend_auto_managed {
            let existing = renderer::config::legend_entry_count(&config.legend.content);
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &new_series,
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
            if existing > new_series.len() {
                for index in (new_series.len()..existing).rev() {
                    renderer::config::remove_legend_entry(&mut config.legend.content, index);
                }
            }
            for index in existing..new_series.len() {
                let label = new_series[index].label.clone().or_else(|| {
                    new_labels[index]
                        .as_ref()
                        .map(|label| self.label_text(label))
                });
                if let Some(label) = label {
                    renderer::config::append_legend_entry_rich(
                        &mut config.legend.content,
                        renderer::config::series_symbol_segments(
                            &new_series[index],
                            config.colorbar.as_ref().map(|bar| &bar.colormap),
                        ),
                        label.segments,
                    );
                }
            }
            config.legend.visible = !config.legend.content.segments.is_empty();
        }
        let new_len = new_series.len();
        self.replace_chart_state(config, new_series)?;
        self.labels = new_labels;
        self.styles = new_styles;
        self.color_seq = new_len.max(self.color_seq);
        let extent_series = self.chart_series().to_vec();
        let extent_revisions = self.column_revisions.clone();
        self.publish_series_extents_if_engine_ready(&extent_series, &extent_revisions, false)?;
        Ok(())
    }

    // ---- frame / resize / export ----

    /// Wait until previously submitted GPU work has finished. Does not
    /// draw. Used to separate an earlier compute submit from the next
    /// render submit when measuring startup.
    pub async fn wait_submitted_work(&self) {
        self.renderer.wait_submitted_work().await;
    }

    /// Compile the drawable-series fit-bound engine without drawing. On wasm
    /// this uses `createComputePipelineAsync`. Series registration does
    /// not do this; `auto_fit_all` does.
    pub async fn ensure_extent_engine(&mut self) -> Result<(), JsValue> {
        self.renderer
            .ensure_errorbar_extent_engine()
            .await
            .map_err(js_err)
    }

    /// Submit the current chart if it is dirty, then wait until that GPU
    /// work has finished. Hosts should keep a loading overlay up until
    /// this resolves. The wait does not remove a main-thread Dawn/driver
    /// freeze; it only reports first-submit completion.
    ///
    /// JS: `await chart.first_frame_ready();`
    pub async fn first_frame_ready(&mut self) -> Result<(), JsValue> {
        self.frame()?;
        renderer::init::wait_browser_submitted_work(self.renderer.queue())
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Alias for [`Self::first_frame_ready`].
    pub async fn warm_up(&mut self) -> Result<(), JsValue> {
        self.first_frame_ready().await
    }

    /// Prepare the resident Cartesian pipeline groups while reporting the
    /// current stage as `{ scope, stage, phase }` objects. Separate bounded
    /// renderers, nonresident matrix/tile pipelines and future chart-specific
    /// allocations are outside this scope. The chart is mutably
    /// borrowed until the Promise settles; hosts must keep their normal
    /// async busy gate active and queue chart work rather than calling the
    /// kernel from the callback.
    pub async fn prewarm_all_with_progress(
        &mut self,
        on_event: js_sys::Function,
    ) -> Result<(), JsValue> {
        let mut observer = |event| emit_init_progress(Some(&on_event), event);
        self.renderer
            .prewarm_all_observed(&mut observer)
            .await
            .map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Prepare the same resident Cartesian groups as prewarm_all_with_progress,
    /// without a progress callback. The host must prevent other calls on this
    /// kernel until the Promise settles; wasm-bindgen does not queue them.
    pub async fn prewarm_all(&mut self) -> Result<(), JsValue> {
        self.renderer.prewarm_all().await.map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Process pending pool maintenance and draw only when visual state is
    /// dirty. A clean rAF tick returns before touching the pool or surface.
    pub fn frame(&mut self) -> Result<(), JsValue> {
        self.renderer
            .sync_external_invalidations()
            .map_err(js_err)?;
        let current_stamp = self
            .renderer
            .chart_render_stamp(self.chart_id)
            .map_err(js_err)?;
        let renderer_dirty = current_stamp.needs_draw_since(self.last_presented_stamp.as_ref());
        let raster_dirty = current_stamp.needs_raster_since(self.last_presented_stamp.as_ref());
        // The annotation layer has its own raster revision: an item edit
        // or a changed annotation selection redraws the two annotation
        // textures only; `refresh_axis` redoes them with the axis raster.
        let annotation_dirty = current_stamp
            .needs_annotation_raster_since(self.last_presented_stamp.as_ref())
            || self.annotation_selection_dirty;
        let decision = frame_decision(
            renderer_dirty,
            raster_dirty,
            self.view_dirty,
            self.redraw_pending,
            self.renderer.has_pending_maintenance(),
        );
        let refresh_raster = match decision {
            FrameDecision::Clean => return Ok(()),
            FrameDecision::MaintenanceOnly => {
                let _ = self
                    .renderer
                    .process_pending_maintenance()
                    .map_err(js_err)?;
                return Ok(());
            }
            FrameDecision::Draw { refresh_raster } => refresh_raster,
        };

        let _ = self
            .renderer
            .process_pending_maintenance()
            .map_err(js_err)?;
        let current_stamp = self
            .renderer
            .chart_render_stamp(self.chart_id)
            .map_err(js_err)?;

        // Browser resize is preview zoom, not a document mutation. The
        // stored chart remains the export SSoT; the live canvas renders a
        // scaled/letterboxed display chart derived from it.
        let (display_config, panel_rect, _) = self.display_config();
        let display_chart = Chart::new(display_config);
        if refresh_raster {
            let selected_id: Option<String> = self
                .chart_selection()
                .and_then(|id| self.hitmap.get(id).map(|el| el.element_id()));
            self.renderer
                .refresh_axis_with_selection(
                    &mut self.view,
                    &display_chart,
                    panel_rect,
                    selected_id.as_deref(),
                )
                .map_err(js_err)?;
        }
        if (annotation_dirty && !refresh_raster)
            || self.view.annotation_selected() != self.selected_annotation
        {
            self.renderer
                .refresh_annotations(
                    &mut self.view,
                    &display_chart,
                    panel_rect,
                    self.selected_annotation,
                )
                .map_err(js_err)?;
        }

        if self.renderer.is_streaming_chart(self.chart_id) {
            self.renderer
                .draw_registered(
                    self.clear_color,
                    &[RegisteredChartDrawItem {
                        chart_id: self.chart_id,
                        view: &self.view,
                    }],
                )
                .map_err(js_err)?;
        } else {
            let series_configs = self.chart_series().to_vec();
            let series: Vec<Series<'_>> = self
                .styles
                .iter()
                .zip(series_configs.iter())
                // A cancelled stream (or an unpublished residency upload)
                // has no data surface. Its logical registration is not a
                // resident allocation; keep drawing decorations and any
                // independent resident series without requesting it from
                // the column pool.
                .filter(|(_, config)| {
                    !referenced_columns(config).into_iter().any(|id| {
                        matches!(
                            self.renderer.logical_column(id),
                            Some(renderer::LogicalColumn::Streamed(_))
                        )
                    })
                })
                .map(|(style, config)| Series { config, style })
                .collect();
            let items = [ChartDrawItem {
                view: &self.view,
                chart_config: display_chart.config(),
                series: &series,
            }];
            self.renderer
                .draw(self.clear_color, &items)
                .map_err(js_err)?;
        }

        // Only a successfully submitted/presented draw advances the
        // renderer-issued onscreen stamp. Any earlier failure leaves the
        // frame dirty so the next persistent rAF retries it.
        consume_successful_frame(&mut self.view_dirty, &mut self.redraw_pending);
        self.annotation_selection_dirty = false;
        self.last_presented_stamp = Some(current_stamp);
        Ok(())
    }

    /// Set the WebGPU surface clear color used behind the chart panel.
    /// Components are linear 0..1 RGBA floats. This is host/app state:
    /// it does not change the chart Config JSON.
    pub fn set_clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.clear_color = Color::from_rgba(
            r.clamp(0.0, 1.0),
            g.clamp(0.0, 1.0),
            b.clamp(0.0, 1.0),
            a.clamp(0.0, 1.0),
        );
        self.request_host_redraw();
    }

    /// Resize the swap chain viewport. The chart Config keeps its
    /// document/export `chart_area`; live rendering scales that logical
    /// document into this surface.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        let (w, h) = (width.max(1), height.max(1));
        let styles = self.styles_for_surface((w, h))?;
        self.renderer.resize(w, h).map_err(js_err)?;
        self.surface_size = (w, h);
        self.view_dirty = true;
        self.styles = styles;
        Ok(())
    }
}
